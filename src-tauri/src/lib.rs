mod activity;
mod commands;
mod ignore_rules;
mod daemon;
mod data_location;
mod network_watcher;
mod seaf_rpc;
mod shell_ext;
mod state;
mod sync_controller;
mod tray;

use std::sync::Arc;

use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_autostart::ManagerExt;

use state::AppState;

pub const EVT_DAEMON_ERROR: &str = "daemon-error";
/// Passed by the Windows Run key so a sign-in launch can stay in the tray.
const AUTOSTART_ARG: &str = "--autostart";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // Must be first: a second launch just focuses the running instance.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("Seafile Metered Client")
                .arg(AUTOSTART_ARG)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let (location, paths, warning) = data_location::startup_location();
            if let Some(w) = &warning {
                eprintln!("[data-location] {w}");
            }
            let state = Arc::new(AppState::new(location, paths, warning)?);
            app.manage(state.clone());
            tray::setup(app.handle())?;

            // The main window is not created at startup (tauri.conf.json `create: false`),
            // so a minimized start never loads WebView2 at all.
            let from_autostart = std::env::args().any(|a| a == AUTOSTART_ARG);
            if !(from_autostart && state.preferences().start_minimized) {
                tray::show_main_window(app.handle());
            }
            // Re-register so the Run key follows the exe if the portable folder was moved.
            let launcher = app.autolaunch();
            if launcher.is_enabled().unwrap_or(false)
                && let Err(e) = launcher.enable()
            {
                eprintln!("[autostart] cannot refresh Run key: {e}");
            }

            // Serve the Seafile Explorer extension (overlay icons).
            shell_ext::start(app.handle().clone(), state.clone());
            // Transfer rates + sync history (idles until the daemon is up).
            activity::spawn(app.handle().clone(), state.clone());

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            network_watcher::spawn(tx);

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = daemon::start(&handle, &state).await {
                    eprintln!("[daemon] {e}");
                    *state.daemon_error.lock().unwrap() = Some(e.clone());
                    let _ = handle.emit(EVT_DAEMON_ERROR, e);
                }

                // The first event is the current cost: reconcile the daemon with stored rules.
                let mut initial = true;
                while let Some(cost) = rx.recv().await {
                    sync_controller::handle_network_change(&handle, &state, cost, initial).await;
                    initial = false;
                }
            });
            Ok(())
        })
        // Closing a window destroys it (Settings is owned by main and goes with it).
        // The app keeps running in the tray: see RunEvent::ExitRequested below.
        .invoke_handler(tauri::generate_handler![
            commands::login,
            commands::logout,
            commands::get_session,
            commands::get_cloud_libraries,
            commands::get_libraries,
            commands::setup_library_sync,
            commands::toggle_library_pause,
            commands::toggle_bypass_metered,
            commands::force_sync,
            commands::unsync_library,
            commands::open_library_folder,
            commands::open_library_on_cloud,
            commands::get_library_errors,
            commands::get_transfer_rate,
            commands::get_sync_history,
            commands::get_history_settings,
            commands::set_history_settings,
            commands::clear_sync_history,
            commands::open_settings_window,
            commands::get_general_settings,
            commands::get_temp_ignore_patterns,
            commands::set_general_settings,
            commands::get_network_status,
            commands::get_daemon_error,
            commands::pick_folder,
            commands::get_storage_settings,
            commands::move_data_location,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| match event {
        // `code: None` = the last window was closed. Stay in the tray with no
        // webview alive; only an explicit `app.exit()` (tray "Exit") quits.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        RunEvent::ExitRequested { .. } | RunEvent::Exit => {
            if let Some(state) = app.try_state::<Arc<AppState>>() {
                daemon::stop(&state);
            }
            // Clear overlay icons, as seafile-applet does on exit.
            shell_ext::refresh_explorer();
        }
        _ => {}
    });
}
