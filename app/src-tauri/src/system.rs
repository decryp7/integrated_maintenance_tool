//! Elevation (run as administrator) and reading the start of a workspace file.

use crate::windows::CurrentWindow;
use crate::{err, CmdResult};
use serde::Serialize;
use std::path::Path;

/// Largest preview returned by `read_file_head`.
const MAX_HEAD_BYTES: usize = 256 * 1024;

/// The operating system IMT runs on, so the UI can offer platform-specific tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Macos,
    Linux,
}

impl Platform {
    /// The platform this binary was built for (other Unixes report as Linux).
    pub const fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Macos
        } else {
            Platform::Linux
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElevationInfo {
    pub elevated: bool,
    /// Whether `restart_elevated` is available on this platform.
    pub can_restart_elevated: bool,
    pub platform: Platform,
}

#[tauri::command]
pub fn elevation() -> ElevationInfo {
    ElevationInfo {
        elevated: is_elevated(),
        can_restart_elevated: cfg!(windows),
        platform: Platform::current(),
    }
}

/// Relaunches IMT as administrator (Windows UAC prompt) with `reopen` paths
/// (e.g. the saved project) and exits this instance. Elsewhere: an error.
#[tauri::command]
pub fn restart_elevated<R: tauri::Runtime>(app: tauri::AppHandle<R>, reopen: Vec<String>) -> CmdResult<()> {
    let exe = std::env::current_exe().map_err(err)?;
    // The elevated process must become the running instance, not hand over to this one.
    crate::instance::release(&app);
    relaunch_elevated(&exe, &reopen)?;
    log::info!(Paths = reopen.len() as u64; "Restarting as administrator");
    crate::window_manager::quit(&app);
    Ok(())
}

/// The decoded first bytes of a file that is part of the workspace (a source
/// or an unrecognised file), for the format wizard. Other paths are refused so
/// the webview cannot read arbitrary files.
#[tauri::command]
pub fn read_file_head(state: CurrentWindow, path: String, max_bytes: Option<usize>) -> CmdResult<String> {
    let summary = state.store_read().summary();
    let known = summary
        .sources
        .iter()
        .map(|s| s.path.as_str())
        .chain(summary.unrecognized.iter().map(|u| u.path.as_str()));
    if !known.into_iter().any(|p| p == path) {
        return Err("only files in the workspace can be previewed".into());
    }
    let limit = max_bytes.unwrap_or(64 * 1024).clamp(1, MAX_HEAD_BYTES);
    let mut bytes = Vec::with_capacity(limit);
    use std::io::Read;
    std::fs::File::open(Path::new(&path))
        .map_err(err)?
        .take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    imt_core::importers::decode(&bytes).ok_or_else(|| "this file is binary".to_string())
}

#[cfg(windows)]
fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: plain Win32 calls with owned, correctly sized buffers; the token handle is closed.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
fn is_elevated() -> bool {
    // Root on Unix-like systems.
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

#[cfg(windows)]
fn relaunch_elevated(exe: &Path, args: &[String]) -> CmdResult<()> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let params: Vec<String> = args.iter().map(|a| format!("\"{}\"", a.replace('"', ""))).collect();
    let (verb, file, params) = (
        HSTRING::from("runas"),
        HSTRING::from(exe.as_os_str()),
        HSTRING::from(params.join(" ")),
    );
    // SAFETY: all strings outlive the call; ShellExecuteW does not retain them.
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(params.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success; the user may also decline the UAC prompt.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err("the administrator prompt was declined or failed".into())
    }
}

#[cfg(not(windows))]
fn relaunch_elevated(_exe: &Path, _args: &[String]) -> CmdResult<()> {
    Err("Restarting as administrator is only available on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_reports_platform_support() {
        let info = elevation();
        assert_eq!(info.can_restart_elevated, cfg!(windows));
        assert_eq!(info.platform == Platform::Windows, cfg!(windows));
        #[cfg(target_os = "macos")]
        assert_eq!(info.platform, Platform::Macos);
        #[cfg(not(windows))]
        assert!(relaunch_elevated(Path::new("x"), &[]).is_err());
    }

    #[test]
    fn platform_serialises_lowercase() {
        let names: Vec<String> = [Platform::Windows, Platform::Macos, Platform::Linux]
            .iter()
            .map(|p| serde_json::to_string(p).unwrap())
            .collect();
        assert_eq!(names, ["\"windows\"", "\"macos\"", "\"linux\""]);
        let json = serde_json::to_value(elevation()).unwrap();
        assert!(json["platform"].is_string() && json.get("canRestartElevated").is_some());
    }
}
