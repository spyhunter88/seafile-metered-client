use std::sync::{Arc, Mutex};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewWindow, WebviewWindowBuilder, Wry};

use crate::activity::format_rate;
use crate::state::{AppState, NetworkCost};
use crate::sync_controller;

/// Overall sync state shown as a badge on the tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayStatus {
    #[default]
    Normal,
    Syncing,
    Error,
}

impl TrayStatus {
    fn label(self) -> &'static str {
        match self {
            TrayStatus::Normal => "Status: Up to date",
            TrayStatus::Syncing => "Status: Syncing…",
            TrayStatus::Error => "Status: Sync error",
        }
    }

    fn color(self) -> [u8; 3] {
        match self {
            TrayStatus::Normal => [0x22, 0xc5, 0x5e],  // green
            TrayStatus::Syncing => [0x25, 0x63, 0xeb], // blue
            TrayStatus::Error => [0xef, 0x44, 0x44],   // red
        }
    }
}

/// Unbadged app icon as straight RGBA.
struct BaseIcon {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

struct TrayHandles {
    status: MenuItem<Wry>,
    base_icon: Option<BaseIcon>,
    sync_status: Mutex<TrayStatus>,
}

fn network_label(cost: NetworkCost) -> &'static str {
    match cost {
        NetworkCost::Unmetered => "Network: Unmetered (Wi-Fi/Ethernet)",
        NetworkCost::Metered => "Network: Metered (Hotspot)",
    }
}

pub const MAIN_WINDOW: &str = "main";

/// Shows the main window, creating it if needed.
///
/// The window is destroyed (not hidden) when closed so WebView2 and its
/// renderer processes are released while the app sits in the tray. It is
/// declared in tauri.conf.json with `create: false` and built from that config.
pub fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        focus(&w);
        return;
    }
    // Building a webview window from a tray / single-instance callback on the
    // main thread can deadlock on Windows (wry#583), so do it off-thread.
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // A second request may have created it while this task was queued.
        if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
            focus(&w);
            return;
        }
        let Some(config) = app.config().app.windows.iter().find(|w| w.label == MAIN_WINDOW).cloned() else {
            eprintln!("[window] no '{MAIN_WINDOW}' window in tauri.conf.json");
            return;
        };
        match WebviewWindowBuilder::from_config(&app, &config).and_then(|b| b.build()) {
            Ok(w) => focus(&w),
            Err(e) => eprintln!("[window] cannot create main window: {e}"),
        }
    });
}

fn focus(w: &WebviewWindow) {
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", network_label(NetworkCost::Unmetered), false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open Seafile Metered Client", true, None::<&str>)?;
    let pause_all = MenuItem::with_id(app, "pause_all", "Pause All", true, None::<&str>)?;
    let resume_all = MenuItem::with_id(app, "resume_all", "Resume All", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Exit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &PredefinedMenuItem::separator(app)?,
            &status,
            &PredefinedMenuItem::separator(app)?,
            &pause_all,
            &resume_all,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let base_icon = app.default_window_icon().map(|i| BaseIcon {
        rgba: i.rgba().to_vec(),
        width: i.width(),
        height: i.height(),
    });

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Seafile Metered Client")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main_window(app),
            "pause_all" | "resume_all" => {
                let paused = event.id.as_ref() == "pause_all";
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<Arc<AppState>>().inner().clone();
                    sync_controller::set_all_user_paused(&app, &state, paused).await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(base) = &base_icon {
        builder = builder.icon(badged_icon(base, TrayStatus::default()));
    }
    builder.build(app)?;

    app.manage(TrayHandles {
        status,
        base_icon,
        sync_status: Mutex::new(TrayStatus::default()),
    });
    refresh_tooltip(app);
    Ok(())
}

pub fn set_network(app: &AppHandle, cost: NetworkCost) {
    if let Some(h) = app.try_state::<TrayHandles>() {
        let _ = h.status.set_text(network_label(cost));
    }
    refresh_tooltip(app);
}

/// Updates the tray badge; does nothing when the status is unchanged.
pub fn set_sync_status(app: &AppHandle, status: TrayStatus) {
    let Some(h) = app.try_state::<TrayHandles>() else { return };
    {
        let mut current = h.sync_status.lock().unwrap();
        if *current == status {
            return;
        }
        *current = status;
    }
    if let (Some(tray), Some(base)) = (app.tray_by_id("main"), &h.base_icon) {
        let _ = tray.set_icon(Some(badged_icon(base, status)));
    }
    refresh_tooltip(app);
}

/// Rebuilds the tooltip from the current status, network cost and transfer rates.
/// (Windows truncates tray tooltips at 127 characters.)
pub fn refresh_tooltip(app: &AppHandle) {
    let Some(tray) = app.tray_by_id("main") else { return };
    let Some(state) = app.try_state::<Arc<AppState>>() else { return };
    let status = app
        .try_state::<TrayHandles>()
        .map(|h| *h.sync_status.lock().unwrap())
        .unwrap_or_default();
    let rate = state.transfer_rate();
    let tip = format!(
        "Seafile Metered Client v{}\n{}\n{}\n↑ {}   ↓ {}",
        app.package_info().version,
        status.label(),
        network_label(state.network_cost()),
        format_rate(rate.upload),
        format_rate(rate.download),
    );
    let _ = tray.set_tooltip(Some(tip));
}

/// Draws a coloured dot with a white ring in the bottom-right corner of the icon.
/// Sizes are relative to the icon so the badge stays visible after Windows
/// scales the icon down to 16/20/24 px.
fn badged_icon(base: &BaseIcon, status: TrayStatus) -> Image<'static> {
    let (w, h) = (base.width, base.height);
    let mut px = base.rgba.clone();
    let size = w.min(h) as f32;
    let r_outer = size * 0.24;
    let r_inner = r_outer - size * 0.06;
    let cx = w as f32 - r_outer - size * 0.01;
    let cy = h as f32 - r_outer - size * 0.01;
    // Anti-aliasing width of roughly one pixel at 32 px.
    let aa = (size / 32.0).max(1.0);
    let color = status.color();

    let y0 = (cy - r_outer - aa).floor().max(0.0) as u32;
    let x0 = (cx - r_outer - aa).floor().max(0.0) as u32;
    for y in y0..h {
        for x in x0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let cover = ((r_outer - d) / aa + 0.5).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            // White ring blending into the status colour inside.
            let inner = ((r_inner - d) / aa + 0.5).clamp(0.0, 1.0);
            let i = ((y * w + x) * 4) as usize;
            let dst_a = px[i + 3] as f32 / 255.0;
            let out_a = cover + dst_a * (1.0 - cover);
            for k in 0..3 {
                let src = 255.0 + (color[k] as f32 - 255.0) * inner;
                let dst = px[i + k] as f32;
                px[i + k] = ((src * cover + dst * dst_a * (1.0 - cover)) / out_a).round() as u8;
            }
            px[i + 3] = (out_a * 255.0).round() as u8;
        }
    }
    Image::new_owned(px, w, h)
}
