//! Children that need administrator rights (ADR 0017 §7), e.g. ETW capture.
//!
//! [`decide`] is the launcher's pure decision. On Windows an elevated child is started with `ShellExecuteExW`
//! and the `runas` verb (the UAC prompt); it cannot get a stdin pipe, so its [`Bootstrap`] goes into a file in a
//! fresh private folder named on its command line (`--imt-bootstrap <file>`), which the child reads and deletes.
//! The host's endpoint admits the Administrators group (the elevated child may run as another account), and the
//! host checks that the process that connected is the one `ShellExecuteExW` started, besides the token.
//! Elsewhere elevation is refused with a message ([`IpcError::Denied`]): IMT has no setuid helper.

use crate::launch::Bootstrap;
use crate::process::SpawnSpec;
use crate::IpcError;

/// The command-line flag naming an elevated child's bootstrap file.
pub const BOOTSTRAP_FLAG: &str = "--imt-bootstrap";

/// Whether a child needs administrator rights.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Elevation {
    #[default]
    None,
    /// It must run elevated; `reason` is shown with the prompt's outcome ("ETW capture needs administrator rights").
    Required { reason: String },
}

/// How the launcher starts a child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// A normal child process (with IMT's rights: elevated already, or no elevation needed).
    Plain,
    /// Through the administrator prompt (Windows).
    RunAs,
}

/// The launcher's decision for this platform.
pub fn decide(elevation: &Elevation, elevated_now: bool) -> Result<LaunchMode, IpcError> {
    decide_on(elevation, elevated_now, cfg!(windows))
}

/// [`decide`] for a given platform (tests).
pub fn decide_on(elevation: &Elevation, elevated_now: bool, windows: bool) -> Result<LaunchMode, IpcError> {
    match elevation {
        Elevation::None => Ok(LaunchMode::Plain),
        Elevation::Required { .. } if elevated_now => Ok(LaunchMode::Plain),
        Elevation::Required { .. } if windows => Ok(LaunchMode::RunAs),
        Elevation::Required { reason } => Err(IpcError::Denied(format!(
            "{reason}; start IMT with administrator rights (for example with sudo) to use it"
        ))),
    }
}

/// Whether this process runs with administrator rights (elevated token on Windows, euid 0 on Unix).
pub fn is_elevated() -> bool {
    platform::is_elevated()
}

/// A child started through the administrator prompt.
pub struct ElevatedProcess {
    #[cfg(windows)]
    inner: std::sync::Arc<platform::Handle>,
    pid: u32,
    label: String,
    /// The private folder of the bootstrap file (removed on drop; the child deletes the file itself).
    _dir: Option<TempDir>,
}

impl ElevatedProcess {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// A probe for "has it exited, and how" usable from another thread.
    pub fn probe(&self) -> Box<dyn Fn() -> Option<String> + Send> {
        #[cfg(windows)]
        {
            let h = self.inner.clone();
            Box::new(move || h.exit_code().map(|c| format!("exit code {c}")))
        }
        #[cfg(not(windows))]
        Box::new(|| None)
    }

    pub fn exited(&self) -> bool {
        (self.probe())().is_some()
    }

    pub fn exit_text(&self) -> String {
        match (self.probe())() {
            Some(s) => format!("{} exited ({s})", self.label),
            None => format!("{} closed the connection", self.label),
        }
    }

    /// Ends it when IMT may (it also exits when its connection closes).
    pub fn kill(&self) {
        #[cfg(windows)]
        self.inner.terminate();
    }
}

/// A folder removed on drop.
pub(crate) struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Writes `bootstrap` into a fresh private folder; returns the folder and the file.
fn write_bootstrap(bootstrap: &Bootstrap) -> Result<(TempDir, std::path::PathBuf), IpcError> {
    let fail = |e: std::io::Error| IpcError::Spawn(format!("cannot prepare the elevated start: {e}"));
    let dir = std::env::temp_dir().join(format!("imt-elevate-{}", crate::endpoint::random_hex(8).map_err(fail)?));
    std::fs::create_dir(&dir).map_err(fail)?;
    let dir = TempDir(dir);
    let file = dir.0.join("bootstrap.json");
    let json = serde_json::to_vec(bootstrap).map_err(|e| IpcError::Spawn(e.to_string()))?;
    std::fs::write(&file, json).map_err(fail)?;
    Ok((dir, file))
}

/// Reads and deletes an elevated child's bootstrap file (the child side).
pub fn take_bootstrap(file: &std::path::Path) -> std::io::Result<Bootstrap> {
    let text = std::fs::read(file)?;
    let _ = std::fs::remove_file(file);
    if text.len() > 64 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the bootstrap file is too large",
        ));
    }
    serde_json::from_slice(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad bootstrap: {e}")))
}

/// One argument quoted for a Windows command line (the rules `CommandLineToArgvW` undoes).
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\u{b}', '"']) {
        return arg.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

/// Starts `spec`'s program through the administrator prompt with `bootstrap` (Windows only).
pub fn run_as(spec: &SpawnSpec, bootstrap: &Bootstrap) -> Result<ElevatedProcess, IpcError> {
    let (dir, file) = write_bootstrap(bootstrap)?;
    let mut args: Vec<String> = spec.args.clone();
    args.push(BOOTSTRAP_FLAG.into());
    args.push(file.to_string_lossy().into_owned());
    let params = args.iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ");
    platform::run_as(spec, &params).map(|(handle, pid)| {
        #[cfg(not(windows))]
        let () = handle;
        ElevatedProcess {
            #[cfg(windows)]
            inner: handle,
            pid,
            label: spec.label.clone(),
            _dir: Some(dir),
        }
    })
}

#[cfg(windows)]
mod platform {
    use crate::process::SpawnSpec;
    use crate::IpcError;
    use std::sync::Arc;
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, GetProcessId, OpenProcessToken, TerminateProcess, WaitForSingleObject,
    };
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    pub fn is_elevated() -> bool {
        // SAFETY: plain Win32 calls on the current process' token with valid out pointers; the handle is closed.
        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut e = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut e as *mut _ as *mut std::ffi::c_void),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
            .is_ok();
            let _ = CloseHandle(token);
            ok && e.TokenIsElevated != 0
        }
    }

    /// The process handle `ShellExecuteExW` returned.
    pub struct Handle(HANDLE);

    // SAFETY: a process handle may be used and closed from any thread.
    unsafe impl Send for Handle {}
    // SAFETY: the calls made through a shared reference (wait, exit code, terminate) are thread-safe.
    unsafe impl Sync for Handle {}

    impl Handle {
        pub fn exit_code(&self) -> Option<u32> {
            // SAFETY: the handle is valid until `Drop`.
            unsafe {
                if WaitForSingleObject(self.0, 0) != WAIT_OBJECT_0 {
                    return None;
                }
                let mut code = 0u32;
                GetExitCodeProcess(self.0, &mut code).ok().map(|_| code)
            }
        }

        pub fn terminate(&self) {
            // SAFETY: as above; failure (no terminate right on an elevated process) is ignored: the child also
            // exits when its connection closes.
            unsafe {
                let _ = TerminateProcess(self.0, 1);
            }
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: closed once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub fn run_as(spec: &SpawnSpec, params: &str) -> Result<(Arc<Handle>, u32), IpcError> {
        const ERROR_CANCELLED: i32 = 1223;
        let verb = HSTRING::from("runas");
        let file = HSTRING::from(spec.exe.as_os_str());
        let params = HSTRING::from(params);
        let dir = spec.current_dir.as_ref().map(|d| HSTRING::from(d.as_os_str()));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            lpDirectory: dir.as_ref().map(|d| PCWSTR(d.as_ptr())).unwrap_or(PCWSTR::null()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        // SAFETY: every pointer in `info` refers to an HSTRING that outlives the call.
        match unsafe { ShellExecuteExW(&mut info) } {
            Ok(()) if !info.hProcess.is_invalid() => {
                // SAFETY: a valid process handle we own from now on.
                let pid = unsafe { GetProcessId(info.hProcess) };
                Ok((Arc::new(Handle(info.hProcess)), pid))
            }
            Ok(()) => Err(IpcError::Spawn(format!("{} started without a process handle", spec.label))),
            Err(e) if e.code().0 & 0xFFFF == ERROR_CANCELLED => Err(IpcError::Denied(format!(
                "{} needs administrator rights and the administrator prompt was declined",
                spec.label
            ))),
            Err(e) => Err(IpcError::Spawn(format!(
                "cannot start {} with administrator rights ({}): {e}",
                spec.label,
                spec.exe.display()
            ))),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::process::SpawnSpec;
    use crate::IpcError;

    pub fn is_elevated() -> bool {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() == 0 }
    }

    pub fn run_as(spec: &SpawnSpec, _params: &str) -> Result<((), u32), IpcError> {
        Err(IpcError::Denied(format!(
            "{} needs administrator rights; start IMT with them to use it",
            spec.label
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_decides_by_need_rights_and_platform() {
        let need = Elevation::Required {
            reason: "ETW capture needs administrator rights".into(),
        };
        assert_eq!(decide_on(&Elevation::None, false, true).unwrap(), LaunchMode::Plain);
        assert_eq!(decide_on(&Elevation::None, true, false).unwrap(), LaunchMode::Plain);
        assert_eq!(decide_on(&need, true, true).unwrap(), LaunchMode::Plain, "already elevated");
        assert_eq!(decide_on(&need, true, false).unwrap(), LaunchMode::Plain, "root on Unix");
        assert_eq!(decide_on(&need, false, true).unwrap(), LaunchMode::RunAs);
        match decide_on(&need, false, false) {
            Err(IpcError::Denied(m)) => assert!(m.starts_with("ETW capture needs administrator rights;"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert!(!IpcError::Denied("x".into()).is_process_failure());
        // Whatever this test runs as, the probe answers.
        let _ = is_elevated();
    }

    #[test]
    fn arguments_are_quoted_for_command_line_to_argv() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("C:\\Program Files\\IMT\\x.json"), "\"C:\\Program Files\\IMT\\x.json\"");
        assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_arg("ends\\ with\\"), "\"ends\\ with\\\\\"");
        assert_eq!(quote_arg("q\\\"x y"), "\"q\\\\\\\"x y\"");
    }

    #[test]
    fn the_bootstrap_file_is_read_once_and_deleted() {
        let b = Bootstrap {
            address: "pipe".into(),
            token: "00".repeat(32),
            sandbox: crate::sandbox::SandboxSpec::locked(),
            max_frame: 4096,
            role: "elevated".into(),
        };
        let (dir, file) = write_bootstrap(&b).unwrap();
        assert!(file.starts_with(&dir.0));
        assert_eq!(take_bootstrap(&file).unwrap(), b);
        assert!(!file.exists(), "deleted after reading");
        assert!(take_bootstrap(&file).is_err());
        let folder = dir.0.clone();
        drop(dir);
        assert!(!folder.exists());
        // Garbage and oversized files are refused.
        let (dir, file) = write_bootstrap(&b).unwrap();
        std::fs::write(&file, b"nope").unwrap();
        assert_eq!(take_bootstrap(&file).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        std::fs::write(&file, vec![b' '; 70_000]).unwrap();
        assert!(take_bootstrap(&file).is_err());
        drop(dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn elevation_is_refused_off_windows() {
        let spec = SpawnSpec::new("/bin/true", "the collector");
        let b = Bootstrap {
            address: "a".into(),
            token: String::new(),
            sandbox: Default::default(),
            max_frame: 1024,
            role: String::new(),
        };
        assert!(matches!(run_as(&spec, &b), Err(IpcError::Denied(_))));
    }
}
