//! Elevation (run as administrator) and reading the start of a workspace file.

use crate::{err, AppState, CmdResult, State};
use serde::Serialize;
use std::path::Path;

/// Largest preview returned by `read_file_head`.
const MAX_HEAD_BYTES: usize = 256 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElevationInfo {
    pub elevated: bool,
    /// Whether `restart_elevated` is available on this platform.
    pub can_restart_elevated: bool,
}

#[tauri::command]
pub fn elevation() -> ElevationInfo {
    ElevationInfo {
        elevated: is_elevated(),
        can_restart_elevated: cfg!(windows),
    }
}

/// Relaunches IMT as administrator (Windows UAC prompt) with `reopen` paths
/// (e.g. the saved project) and exits this instance. Elsewhere: an error.
#[tauri::command]
pub fn restart_elevated<R: tauri::Runtime>(app: tauri::AppHandle<R>, reopen: Vec<String>) -> CmdResult<()> {
    let exe = std::env::current_exe().map_err(err)?;
    relaunch_elevated(&exe, &reopen)?;
    log::info!(Paths = reopen.len() as u64; "Restarting as administrator");
    app.exit(0);
    Ok(())
}

/// The decoded first bytes of a file that is part of the workspace (a source
/// or an unrecognised file), for the format wizard. Other paths are refused so
/// the webview cannot read arbitrary files.
#[tauri::command]
pub fn read_file_head(state: State<'_, AppState>, path: String, max_bytes: Option<usize>) -> CmdResult<String> {
    let summary = state.workspace.read().map_err(err)?.summary();
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
        #[cfg(not(windows))]
        assert!(relaunch_elevated(Path::new("x"), &[]).is_err());
    }
}
