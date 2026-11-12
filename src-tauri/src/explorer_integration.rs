//! Registers the bundled Seafile Explorer extension (`seafile_ext64.dll`, from
//! the official Seafile Desktop MSI, Apache-2.0) so Explorer shows overlay icons.
//! The extension talks to our pipe server in `shell_ext.rs`.
//!
//! Registry layout copied from the MSI `Registry` table (overlay handlers only;
//! the context-menu handler is not registered because its commands — share
//! link, lock, private share — are not implemented by this app).
//!
//! Writing to HKLM needs admin rights, so install/uninstall run a generated
//! PowerShell script elevated through UAC (`ShellExecuteExW` + "runas").
//! The DLL is copied to `%ProgramData%\SeafileMeteredClient\SeafileExt` so the
//! registration stays valid when the portable app folder is moved or updated
//! (Explorer keeps the loaded DLL locked).

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

const DLL_NAME: &str = "seafile_ext64.dll";
const OVERLAYS_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers";
const APPROVED_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved";
const CLSID_KEY: &str = r"SOFTWARE\Classes\CLSID";

/// (CLSID suffix, overlay name). The 3 leading spaces + quotes in the key name
/// make Windows sort these first among the ~15 overlay slots.
const OVERLAYS: [(&str, &str); 8] = [
    ("609", "01SeafileExtIconSynced"),
    ("607", "02SeafileExtIconSyncing"),
    ("60A", "03SeafileExtIconPartialSynced"),
    ("60D", "04SeafileExtIconLockedByOthers"),
    ("60E", "05SeafileExtIconLockedByMe"),
    ("60C", "06SeafileExtIconReadOnly"),
    ("60B", "07SeafileExtIconCloud"),
    ("608", "08SeafileExtIconError"),
];

fn clsid(suffix: &str) -> String {
    format!("{{AD201805-4E05-4F2F-B0DE-D0381E6AE{suffix}}}")
}

fn overlay_key_name(name: &str) -> String {
    format!("   \".{name}\"")
}

fn installed_dll_path() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("SeafileMeteredClient").join("SeafileExt").join(DLL_NAME)
}

fn bundled_dll_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("binaries")
        .join(DLL_NAME))
}

// ---- Status -------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum IntegrationState {
    NotInstalled,
    /// Registered and pointing to our copy of the DLL.
    Installed,
    /// Some overlay keys are missing or the DLL file is gone.
    Broken,
    /// Registered by another program (e.g. the official Seafile Desktop).
    OtherOwner,
}

#[derive(Serialize)]
pub struct IntegrationStatus {
    pub state: IntegrationState,
    pub dll_path: Option<String>,
    pub bundled_available: bool,
}

fn registered_dll(suffix: &str, name: &str) -> Option<String> {
    let lm = windows_registry::LOCAL_MACHINE;
    let id = lm
        .open(format!(r"{OVERLAYS_KEY}\{}", overlay_key_name(name)))
        .ok()?
        .get_string("")
        .ok()?;
    if !id.eq_ignore_ascii_case(&clsid(suffix)) {
        return None;
    }
    lm.open(format!(r"{CLSID_KEY}\{id}\InProcServer32"))
        .ok()?
        .get_string("")
        .ok()
}

fn same_path(a: &str, b: &Path) -> bool {
    a.eq_ignore_ascii_case(&b.to_string_lossy())
}

pub fn status(app: &AppHandle) -> IntegrationStatus {
    let ours = installed_dll_path();
    let found: Vec<Option<String>> = OVERLAYS.iter().map(|(s, n)| registered_dll(s, n)).collect();
    let first = found.iter().flatten().next().cloned();

    let state = match &first {
        None => IntegrationState::NotInstalled,
        Some(p) if !same_path(p, &ours) => IntegrationState::OtherOwner,
        Some(p) => {
            let complete = found.iter().all(|f| f.as_deref().is_some_and(|x| same_path(x, &ours)));
            if complete && Path::new(p).exists() {
                IntegrationState::Installed
            } else {
                IntegrationState::Broken
            }
        }
    };
    IntegrationStatus {
        state,
        dll_path: first,
        bundled_available: bundled_dll_path(app).map(|p| p.exists()).unwrap_or(false),
    }
}

// ---- Install / uninstall --------------------------------------------------------

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn overlays_ps_array() -> String {
    OVERLAYS
        .iter()
        .map(|(suffix, name)| {
            format!(
                "@{{ Clsid = {}; Key = {}; Desc = {} }}",
                ps_quote(&clsid(suffix)),
                ps_quote(&overlay_key_name(name)),
                ps_quote(&name[2..]),
            )
        })
        .collect::<Vec<_>>()
        .join(",\n  ")
}

const SCRIPT_HEADER: &str = r#"
$ErrorActionPreference = 'Stop'
$lm = [Microsoft.Win32.RegistryKey]::OpenBaseKey('LocalMachine', 'Registry64')
"#;

fn install_script(src: &Path, dll: &Path, log: &Path) -> String {
    format!(
        r#"{SCRIPT_HEADER}
$log = {log}
try {{
  $src = {src}
  $dll = {dll}
  $dir = Split-Path -Parent $dll
  New-Item -ItemType Directory -Force -Path $dir | Out-Null

  $same = (Test-Path -LiteralPath $dll) -and ((Get-FileHash -LiteralPath $dll).Hash -eq (Get-FileHash -LiteralPath $src).Hash)
  if (-not $same) {{
    # A loaded DLL cannot be overwritten but can be renamed.
    if (Test-Path -LiteralPath $dll) {{
      Move-Item -LiteralPath $dll -Destination ($dll + '.' + [guid]::NewGuid().ToString('N') + '.old') -Force
    }}
    Copy-Item -LiteralPath $src -Destination $dll -Force
  }}
  Get-ChildItem -LiteralPath $dir -Filter '*.old' | Remove-Item -Force -ErrorAction SilentlyContinue

  $items = @(
  {items}
  )
  $approved = $lm.CreateSubKey({approved})
  foreach ($o in $items) {{
    $k = $lm.CreateSubKey({clsid_key} + '\' + $o.Clsid)
    $k.SetValue('', 'SeafileExt'); $k.Close()
    $k = $lm.CreateSubKey({clsid_key} + '\' + $o.Clsid + '\InProcServer32')
    $k.SetValue('', $dll); $k.SetValue('ThreadingModel', 'Apartment'); $k.Close()
    $k = $lm.CreateSubKey({overlays} + '\' + $o.Key)
    $k.SetValue('', $o.Clsid); $k.Close()
    $approved.SetValue($o.Clsid, $o.Desc)
  }}
  $approved.Close()
  exit 0
}} catch {{
  ($_ | Out-String) | Set-Content -LiteralPath $log -Encoding UTF8
  exit 1
}}
"#,
        log = ps_quote(&log.to_string_lossy()),
        src = ps_quote(&src.to_string_lossy()),
        dll = ps_quote(&dll.to_string_lossy()),
        items = overlays_ps_array(),
        approved = ps_quote(APPROVED_KEY),
        clsid_key = ps_quote(CLSID_KEY),
        overlays = ps_quote(OVERLAYS_KEY),
    )
}

fn uninstall_script(dll: &Path, log: &Path) -> String {
    format!(
        r#"{SCRIPT_HEADER}
$log = {log}
try {{
  $dll = {dll}
  $items = @(
  {items}
  )
  foreach ($o in $items) {{
    # Only remove registrations that point to our DLL (leave Seafile Desktop's alone).
    $srv = $lm.OpenSubKey({clsid_key} + '\' + $o.Clsid + '\InProcServer32')
    $path = if ($srv) {{ $srv.GetValue('') }} else {{ $null }}
    if ($srv) {{ $srv.Close() }}
    if ($path -and ($path -ieq $dll)) {{
      $lm.DeleteSubKeyTree({clsid_key} + '\' + $o.Clsid, $false)
      $lm.DeleteSubKeyTree({overlays} + '\' + $o.Key, $false)
      $a = $lm.OpenSubKey({approved}, $true)
      if ($a) {{ $a.DeleteValue($o.Clsid, $false); $a.Close() }}
    }}
  }}
  if (Test-Path -LiteralPath $dll) {{
    try {{ Remove-Item -LiteralPath $dll -Force }}
    catch {{ Move-Item -LiteralPath $dll -Destination ($dll + '.' + [guid]::NewGuid().ToString('N') + '.old') -Force }}
  }}
  exit 0
}} catch {{
  ($_ | Out-String) | Set-Content -LiteralPath $log -Encoding UTF8
  exit 1
}}
"#,
        log = ps_quote(&log.to_string_lossy()),
        dll = ps_quote(&dll.to_string_lossy()),
        items = overlays_ps_array(),
        approved = ps_quote(APPROVED_KEY),
        clsid_key = ps_quote(CLSID_KEY),
        overlays = ps_quote(OVERLAYS_KEY),
    )
}

/// Runs a PowerShell script elevated (UAC prompt) and waits for it.
fn run_elevated_script(script: &str) -> Result<(), String> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED};
    use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    use windows::Win32::UI::Shell::{
        SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    use windows::core::{HSTRING, PCWSTR};

    let tmp = std::env::temp_dir().join(format!("smc-explorer-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let script_path = tmp.join("explorer-integration.ps1");
    let log_path = tmp.join("explorer-integration.log");
    let _ = std::fs::remove_file(&log_path);
    // Script text already references `log_path`; callers build it with the same path.
    let script = script.replace("__LOG_PATH__", &log_path.to_string_lossy().replace('\'', "''"));
    // BOM so Windows PowerShell 5 reads non-ASCII paths correctly.
    std::fs::write(&script_path, format!("\u{FEFF}{script}")).map_err(|e| e.to_string())?;

    let verb = HSTRING::from("runas");
    let file = HSTRING::from("powershell.exe");
    let params = HSTRING::from(format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\"",
        script_path.display()
    ));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };

    let result = unsafe { ShellExecuteExW(&mut info) };
    let outcome = match result {
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => Err("Administrator permission was declined".into()),
        Err(e) => Err(format!("Cannot start elevated PowerShell: {e}")),
        Ok(()) => {
            let mut code = 1u32;
            unsafe {
                WaitForSingleObject(info.hProcess, INFINITE);
                let _ = GetExitCodeProcess(info.hProcess, &mut code);
                let _ = CloseHandle(info.hProcess);
            }
            if code == 0 {
                Ok(())
            } else {
                let detail = std::fs::read_to_string(&log_path).unwrap_or_default();
                Err(format!("Registration script failed (exit {code}). {}", detail.trim()))
            }
        }
    };
    let _ = std::fs::remove_dir_all(&tmp);
    outcome
}

const LOG_PLACEHOLDER: &str = "__LOG_PATH__";

pub async fn install(app: &AppHandle) -> Result<(), String> {
    let src = bundled_dll_path(app)?;
    if !src.exists() {
        return Err(format!("{DLL_NAME} not found at {}", src.display()));
    }
    let script = install_script(&src, &installed_dll_path(), Path::new(LOG_PLACEHOLDER));
    tauri::async_runtime::spawn_blocking(move || run_elevated_script(&script))
        .await
        .map_err(|e| e.to_string())??;
    crate::shell_ext::refresh_explorer();
    Ok(())
}

pub async fn uninstall() -> Result<(), String> {
    let script = uninstall_script(&installed_dll_path(), Path::new(LOG_PLACEHOLDER));
    tauri::async_runtime::spawn_blocking(move || run_elevated_script(&script))
        .await
        .map_err(|e| e.to_string())??;
    crate::shell_ext::refresh_explorer();
    Ok(())
}

/// Explorer only reads overlay handlers at startup.
pub fn restart_explorer() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("taskkill")
        .args(["/f", "/im", "explorer.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("Cannot stop Explorer: {e}"))?;
    std::thread::sleep(std::time::Duration::from_millis(800));
    std::process::Command::new("explorer.exe")
        .spawn()
        .map_err(|e| format!("Cannot start Explorer: {e}"))?;
    Ok(())
}
