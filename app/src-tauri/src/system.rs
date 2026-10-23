//! Elevation (run as administrator) and reading the start of a workspace file.
//! Whether Debrief runs elevated is answered by the tools plugin's `elevation`
//! tool; restarting elevated is the shell's (it owns the process).

use crate::windows::CurrentWindow;
use crate::{err, AppState, CmdResult, State};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Largest preview returned by `read_file_head`.
const MAX_HEAD_BYTES: usize = 256 * 1024;

/// The operating system Debrief runs on, so the UI can offer platform-specific tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElevationInfo {
    pub elevated: bool,
    /// Whether `restart_elevated` is available on this platform.
    pub can_restart_elevated: bool,
    pub platform: Platform,
}

/// Asks the `elevation` tool; without it (safe mode, plugin disabled or
/// quarantined) the shell checks its own process token, so "Restart as
/// administrator" is not offered to an elevated Debrief.
#[tauri::command]
pub async fn elevation<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    window: CurrentWindow,
    state: State<'_, AppState>,
) -> CmdResult<ElevationInfo> {
    let answer = crate::tools::invoke(&app, &window, state.safe_mode, "elevation", "info", serde_json::json!({})).await;
    Ok(elevation_from(answer, process_elevated))
}

/// The tool's answer, or what the shell knows itself (`elevated` asks the process token).
fn elevation_from(answer: CmdResult<serde_json::Value>, elevated: impl FnOnce() -> bool) -> ElevationInfo {
    match answer.and_then(|v| crate::error::lift(serde_json::from_value::<ElevationInfo>(v).map_err(err))) {
        Ok(info) => info,
        Err(e) => {
            log::debug!("Elevation tool unavailable ({e}); checking the process token");
            ElevationInfo {
                elevated: elevated(),
                can_restart_elevated: cfg!(windows),
                platform: Platform::current(),
            }
        }
    }
}

/// Whether this process runs elevated: the UAC token on Windows.
#[cfg(windows)]
pub(crate) fn process_elevated() -> bool {
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

/// Whether this process runs elevated: its effective user is root.
#[cfg(not(windows))]
pub(crate) fn process_elevated() -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let pid = Pid::from_u32(std::process::id());
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().with_user(UpdateKind::Always),
    );
    let root: Option<sysinfo::Uid> = "0".parse().ok();
    sys.process(pid)
        .and_then(|p| p.effective_user_id().or_else(|| p.user_id()))
        .is_some_and(|uid| Some(uid) == root.as_ref())
}

/// Relaunches Debrief as administrator (Windows UAC prompt), reopening the calling window's autosaved session
/// when `reopen_session`, and exits this instance. Elsewhere: an error. The command line is built in Rust
/// ([`crate::lifecycle::relaunch_args`]): nothing the webview sends reaches the elevated process.
#[tauri::command]
pub async fn restart_elevated<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: CurrentWindow,
    lifecycle: State<'_, crate::lifecycle::Lifecycle>,
    reopen_session: bool,
) -> CmdResult<()> {
    let exe = std::env::current_exe().map_err(err)?;
    let session = crate::lifecycle::session_to_reopen(&state, &lifecycle, reopen_session);
    let args = crate::lifecycle::relaunch_args(false, session.as_deref());
    // The elevated process must become the running instance, not hand over to this one.
    crate::instance::release(&app);
    relaunch_elevated(&exe, &args)?;
    log::info!(Reopen = !args.is_empty(); "Restarting as administrator");
    crate::window_manager::quit(&app);
    Ok(())
}

/// The ShellExecute parameter string: each argument quoted (Windows paths cannot contain `"`).
#[cfg_attr(not(windows), allow(dead_code))]
fn quoted(args: &[std::ffi::OsString]) -> String {
    args.iter()
        .map(|a| format!("\"{}\"", a.to_string_lossy().replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The decoded first bytes of a file that is part of the workspace (a source
/// or an unrecognised file), for the format wizard. Other paths are refused so
/// the webview cannot read arbitrary files.
#[tauri::command]
pub async fn read_file_head(state: CurrentWindow, path: String, max_bytes: Option<usize>) -> CmdResult<String> {
    // Async (off the main thread): the workspace lock may be held by an import, and the file read may be slow.
    let summary = crate::workspace_commands::summary_of(&state).await?;
    crate::sync::blocking_anyhow("reading the file", move || {
        let known = summary
            .sources
            .iter()
            .map(|s| s.path.as_str())
            .chain(summary.unrecognized.iter().map(|u| u.path.as_str()));
        if !known.into_iter().any(|p| p == path) {
            anyhow::bail!("only files in the workspace can be previewed");
        }
        let limit = max_bytes.unwrap_or(64 * 1024).clamp(1, MAX_HEAD_BYTES);
        let mut bytes = Vec::with_capacity(limit);
        use std::io::Read;
        std::fs::File::open(Path::new(&path))?.take(limit as u64).read_to_end(&mut bytes)?;
        debrief_core::importers::decode(&bytes).ok_or_else(|| anyhow::anyhow!("this file is binary"))
    })
    .await
}

#[cfg(windows)]
fn relaunch_elevated(exe: &Path, args: &[std::ffi::OsString]) -> CmdResult<()> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let (verb, file, params) = (HSTRING::from("runas"), HSTRING::from(exe.as_os_str()), HSTRING::from(quoted(args)));
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
fn relaunch_elevated(_exe: &Path, _args: &[std::ffi::OsString]) -> CmdResult<()> {
    Err("Restarting as administrator is only available on Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_reports_platform_support() {
        let app = crate::test_support::test_app_with_commands(None);
        let main = crate::test_support::window(&app, "main");
        let info = crate::tools::tests::ipc(&main, "elevation", serde_json::json!({})).unwrap();
        let info: ElevationInfo = serde_json::from_value(info).unwrap();
        assert_eq!(info.can_restart_elevated, cfg!(windows));
        assert_eq!(info.platform == Platform::Windows, cfg!(windows));
        #[cfg(target_os = "macos")]
        assert_eq!(info.platform, Platform::Macos);
        #[cfg(not(windows))]
        assert!(relaunch_elevated(Path::new("x"), &[]).is_err());
        // An answer that is not elevation info, or none, falls back to the process token.
        let fallback = elevation_from(Ok(serde_json::json!(3)), || false);
        assert!(!fallback.elevated && fallback.platform == Platform::current());
        assert!(elevation_from(Err("no tool".into()), || true).elevated);
        assert!(!elevation_from(Err("no tool".into()), || false).elevated);
        // The tool's answer wins; the token is not even asked.
        let answer = serde_json::json!({ "elevated": true, "canRestartElevated": false, "platform": "linux" });
        assert!(elevation_from(Ok(answer), || panic!("not asked")).elevated);
    }

    #[test]
    fn elevated_parameters_quote_each_argument() {
        let args: Vec<std::ffi::OsString> = vec!["--".into(), r#"C:\a b\"x".debriefproj"#.into()];
        assert_eq!(quoted(&args), r#""--" "C:\a b\x.debriefproj""#);
        assert_eq!(quoted(&[]), "");
    }

    #[cfg(not(windows))]
    #[test]
    fn the_process_check_matches_the_effective_user() {
        let out = debrief_core::process::quiet_command("id").arg("-u").output().unwrap();
        let root = String::from_utf8_lossy(&out.stdout).trim() == "0";
        assert_eq!(process_elevated(), root);
    }

    #[test]
    fn platform_serialises_lowercase() {
        let names: Vec<String> = [Platform::Windows, Platform::Macos, Platform::Linux]
            .iter()
            .map(|p| serde_json::to_string(p).unwrap())
            .collect();
        assert_eq!(names, ["\"windows\"", "\"macos\"", "\"linux\""]);
        let json = serde_json::to_value(elevation_from(Err("x".into()), || false)).unwrap();
        assert!(json["platform"].is_string() && json.get("canRestartElevated").is_some());
    }
}
