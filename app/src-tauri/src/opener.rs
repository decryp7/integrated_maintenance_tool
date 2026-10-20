//! Hands files and folders to the OS: the program associated with a file, an editor at a line, the
//! file manager. The command lines are pure functions of the platform ([`Os`]) so every variant is
//! tested on any machine; [`start`] runs one detached (through `imt_core::process::quiet_command`, no
//! shell, each argument separate) and a reaper thread stops it after [`OPENER_TIMEOUT`].

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long an opener helper (file manager, `open`, `xdg-open`, the editor's launcher) may run before it is killed.
pub(crate) const OPENER_TIMEOUT: Duration = Duration::from_secs(30);

/// The platform a command line is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Os {
    Windows,
    Mac,
    Linux,
}

impl Os {
    pub(crate) const CURRENT: Os = if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Linux
    };
}

/// A program and its arguments, each passed as its own argument (never through a shell).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invocation {
    pub program: OsString,
    pub args: Vec<OsString>,
}

impl Invocation {
    fn new(program: impl AsRef<OsStr>, args: impl IntoIterator<Item = OsString>) -> Self {
        Invocation {
            program: program.as_ref().to_os_string(),
            args: args.into_iter().collect(),
        }
    }

    fn command(&self) -> std::process::Command {
        let mut cmd = imt_core::process::quiet_command(&self.program);
        cmd.args(&self.args);
        cmd
    }
}

/// `path` as the OS tools expect it: Windows verbatim paths (`\\?\C:\x`, `\\?\UNC\srv\share`) lose
/// their prefix, which Explorer and editors do not understand.
pub(crate) fn plain_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// The file manager showing `file` selected (Explorer, Finder; on Linux its folder, as file managers
/// have no common "select" argument).
pub(crate) fn reveal(os: Os, file: &Path) -> Invocation {
    let file = plain_path(file);
    match os {
        Os::Windows => {
            let mut arg = OsString::from("/select,");
            arg.push(&file);
            Invocation::new("explorer", [arg])
        }
        Os::Mac => Invocation::new("open", [OsString::from("-R"), file.into_os_string()]),
        Os::Linux => Invocation::new("xdg-open", [file.parent().unwrap_or(&file).as_os_str().to_os_string()]),
    }
}

/// The file manager on `dir`.
pub(crate) fn open_folder(os: Os, dir: &Path) -> Invocation {
    open_default(os, dir)
}

/// `file` (or a folder) opened with the program the OS associates with it.
pub(crate) fn open_default(os: Os, file: &Path) -> Invocation {
    let program = match os {
        Os::Windows => "explorer",
        Os::Mac => "open",
        Os::Linux => "xdg-open",
    };
    Invocation::new(program, [plain_path(file).into_os_string()])
}

/// `editor` (VS Code's `code` launcher) opening `file` at `line`: `code -g <file>:<line>`.
pub(crate) fn open_at_line(editor: &Path, file: &Path, line: u32) -> Invocation {
    let mut target = plain_path(file).into_os_string();
    target.push(format!(":{line}"));
    Invocation::new(editor, [OsString::from("-g"), target])
}

/// Extensions whose "default program" runs them (or changes the system): IMT never opens these with
/// the OS association, on any platform, as an opened log file may come from anywhere.
const RUNS_ON_OPEN: &[&str] = &[
    "exe",
    "com",
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "psd1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "msi",
    "msp",
    "mst",
    "scr",
    "hta",
    "cpl",
    "msc",
    "jar",
    "lnk",
    "url",
    "reg",
    "inf",
    "scf",
    "pif",
    "application",
    "appref-ms",
    "gadget",
    "settingcontent-ms",
    "library-ms",
    "app",
    "command",
    "tool",
    "pkg",
    "mpkg",
    "dmg",
    "workflow",
    "terminal",
    "webloc",
    "sh",
    "bash",
    "zsh",
    "csh",
    "ksh",
    "run",
    "desktop",
    "appimage",
    "deb",
    "rpm",
    "py",
    "pyw",
    "pl",
    "rb",
];

/// True when opening `file` with its default program could run it (see [`RUNS_ON_OPEN`]).
pub(crate) fn runs_on_open(file: &Path) -> bool {
    file.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| RUNS_ON_OPEN.contains(&e.to_ascii_lowercase().as_str()))
}

/// Where VS Code's command-line launcher is: on `PATH` (`code`, `code.cmd` on Windows), else its usual
/// install locations. `var` reads an environment variable and `is_file` checks a candidate (injected for tests).
pub(crate) fn find_vscode(os: Os, var: &dyn Fn(&str) -> Option<OsString>, is_file: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let name = if os == Os::Windows { "code.cmd" } else { "code" };
    let on_path = var("PATH")
        .map(|p| {
            // A relative PATH entry would depend on the working directory: never used.
            std::env::split_paths(&p)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join(name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let installed: Vec<PathBuf> = match os {
        Os::Windows => [
            ("LOCALAPPDATA", r"Programs\Microsoft VS Code\bin\code.cmd"),
            ("ProgramFiles", r"Microsoft VS Code\bin\code.cmd"),
            ("ProgramFiles(x86)", r"Microsoft VS Code\bin\code.cmd"),
        ]
        .iter()
        .filter_map(|(v, rel)| var(v).map(|base| PathBuf::from(base).join(rel)))
        .collect(),
        Os::Mac => {
            let app = "Visual Studio Code.app/Contents/Resources/app/bin/code";
            let mut v = vec![PathBuf::from("/Applications").join(app)];
            v.extend(var("HOME").map(|h| PathBuf::from(h).join("Applications").join(app)));
            v
        }
        Os::Linux => vec![
            PathBuf::from("/usr/bin/code"),
            PathBuf::from("/usr/share/code/bin/code"),
            PathBuf::from("/snap/bin/code"),
        ],
    };
    on_path.into_iter().chain(installed).find(|p| is_file(p))
}

/// VS Code's launcher on this machine, if installed.
pub(crate) fn installed_vscode() -> Option<PathBuf> {
    find_vscode(Os::CURRENT, &|v| std::env::var_os(v), &|p| p.is_file())
}

/// Opens a web address in the user's browser (the callers check the address first), as an `io` error
/// that says so when it fails.
pub(crate) fn open_url(url: &str) -> crate::CmdResult<()> {
    open::that_detached(url).map_err(|e| crate::CmdError::new(crate::ErrorCode::Io, format!("cannot open your browser: {e}")))
}

/// Starts `invocation` without waiting (in tests: builds it only, nothing is launched).
pub(crate) fn start(invocation: &Invocation) -> std::io::Result<()> {
    let cmd = invocation.command();
    if cfg!(test) {
        return Ok(());
    }
    spawn_detached(cmd, OPENER_TIMEOUT)
}

/// Starts `cmd` without blocking the caller. Its output is discarded; a background thread reaps it and
/// kills it if it is still running after `timeout`. Fails only when the process cannot be started.
pub(crate) fn spawn_detached(mut cmd: std::process::Command, timeout: Duration) -> std::io::Result<()> {
    use std::process::Stdio;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    let deadline = Instant::now() + timeout;
    std::thread::Builder::new().name("imt-opener".into()).spawn(move || loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    log::debug!(Status = status.code().unwrap_or(-1) as i64; "Opener helper exited with an error");
                }
                return;
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                log::warn!("Opener helper timed out; stopping it");
                let _ = child.kill();
                let _ = child.wait();
                return;
            }
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(i: &Invocation) -> Vec<String> {
        i.args.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn verbatim_windows_paths_lose_their_prefix() {
        assert_eq!(plain_path(Path::new(r"\\?\C:\logs\a.log")), PathBuf::from(r"C:\logs\a.log"));
        assert_eq!(
            plain_path(Path::new(r"\\?\UNC\srv\share\a.log")),
            PathBuf::from(r"\\srv\share\a.log")
        );
        assert_eq!(plain_path(Path::new(r"\\?\Volume{x}\a.log")), PathBuf::from(r"\\?\Volume{x}\a.log"));
        assert_eq!(plain_path(Path::new("/var/log/a.log")), PathBuf::from("/var/log/a.log"));
    }

    #[test]
    fn reveal_selects_the_file_per_platform() {
        let w = reveal(Os::Windows, Path::new(r"\\?\C:\logs\a b.log"));
        assert_eq!(
            (w.program.to_str(), args(&w)),
            (Some("explorer"), vec![r"/select,C:\logs\a b.log".to_string()])
        );
        let m = reveal(Os::Mac, Path::new("/logs/a.log"));
        assert_eq!(
            (m.program.to_str(), args(&m)),
            (Some("open"), vec!["-R".to_string(), "/logs/a.log".to_string()])
        );
        let l = reveal(Os::Linux, Path::new("/logs/a.log"));
        assert_eq!((l.program.to_str(), args(&l)), (Some("xdg-open"), vec!["/logs".to_string()]));
        assert_eq!(args(&reveal(Os::Linux, Path::new("/"))), vec!["/".to_string()]);
    }

    #[test]
    fn default_program_gets_the_path_as_one_argument() {
        // A hostile name stays one argument: no shell ever parses it.
        let name = r#"C:\logs\x" & calc & ".log"#;
        for (os, program) in [(Os::Windows, "explorer"), (Os::Mac, "open"), (Os::Linux, "xdg-open")] {
            let i = open_default(os, Path::new(name));
            assert_eq!((i.program.to_str(), args(&i)), (Some(program), vec![name.to_string()]));
            assert_eq!(open_folder(os, Path::new("/d")).program.to_str(), Some(program));
        }
        let cmd = open_default(Os::Linux, Path::new("/a.log")).command();
        assert_eq!(cmd.get_program(), "xdg-open");
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), vec![OsStr::new("/a.log")]);
    }

    #[test]
    fn the_editor_goes_to_the_line() {
        let i = open_at_line(Path::new(r"C:\VS Code\bin\code.cmd"), Path::new(r"\\?\C:\logs\a.log"), 42);
        assert_eq!(i.program, OsString::from(r"C:\VS Code\bin\code.cmd"));
        assert_eq!(args(&i), vec!["-g".to_string(), r"C:\logs\a.log:42".to_string()]);
    }

    #[test]
    fn executables_and_scripts_never_open_with_their_program() {
        for f in [
            "run.BAT",
            "x.exe",
            "a.ps1",
            "fix.reg",
            "s.sh",
            "t.command",
            "app.desktop",
            "Setup.MSI",
            "l.lnk",
        ] {
            assert!(runs_on_open(Path::new(f)), "{f}");
        }
        for f in [
            "app.log",
            "a.json",
            "x.csv",
            "System.evtx",
            "trace.etl",
            "noext",
            "a.log.gz",
            ".bashrc",
        ] {
            assert!(!runs_on_open(Path::new(f)), "{f}");
        }
    }

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: Vec<(String, OsString)> = pairs.iter().map(|(k, v)| (k.to_string(), OsString::from(v))).collect();
        move |k| map.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn finds_vscode_on_path_first() {
        let dir = std::env::temp_dir().join("imt-code-bin");
        let path = std::env::join_paths([Path::new("relative"), dir.as_path()]).unwrap();
        let path = path.to_string_lossy().into_owned();
        let launcher = dir.join(if Os::CURRENT == Os::Windows { "code.cmd" } else { "code" });
        let wanted = launcher.clone();
        let found = find_vscode(Os::CURRENT, &env(&[("PATH", &path)]), &move |p| p == wanted);
        assert_eq!(found, Some(launcher));
        // A relative PATH entry is never used (it would depend on the working directory).
        let rel = find_vscode(Os::CURRENT, &env(&[("PATH", "relative")]), &|_| true);
        assert!(rel.is_none_or(|p| !p.starts_with("relative")));
    }

    #[test]
    fn finds_vscode_where_its_installers_put_it() {
        let user = find_vscode(Os::Windows, &env(&[("LOCALAPPDATA", r"C:\Users\u\AppData\Local")]), &|p| {
            p.ends_with("code.cmd")
        });
        if cfg!(windows) {
            assert_eq!(
                user,
                Some(PathBuf::from(r"C:\Users\u\AppData\Local\Programs\Microsoft VS Code\bin\code.cmd"))
            );
        }
        let mac = find_vscode(Os::Mac, &env(&[]), &|p| p.starts_with("/Applications"));
        assert_eq!(
            mac,
            Some(PathBuf::from(
                "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"
            ))
        );
        let home = find_vscode(Os::Mac, &env(&[("HOME", "/Users/u")]), &|p| p.starts_with("/Users"));
        assert_eq!(
            home,
            Some(PathBuf::from(
                "/Users/u/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"
            ))
        );
        let linux = find_vscode(Os::Linux, &env(&[]), &|p| p == Path::new("/snap/bin/code"));
        assert_eq!(linux, Some(PathBuf::from("/snap/bin/code")));
        assert_eq!(find_vscode(Os::Linux, &env(&[("PATH", "/usr/bin")]), &|_| false), None);
        assert!(installed_vscode().is_none_or(|p| p.is_absolute()));
    }

    #[test]
    fn start_only_builds_the_command_in_tests() {
        assert!(start(&open_default(Os::CURRENT, Path::new("/nowhere/a.log"))).is_ok());
    }

    #[test]
    fn spawn_detached_reports_a_missing_program() {
        let cmd = imt_core::process::quiet_command("imt-definitely-not-a-program");
        assert!(spawn_detached(cmd, Duration::from_secs(1)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn spawn_detached_returns_at_once_and_stops_a_hung_helper() {
        let mut cmd = imt_core::process::quiet_command("sleep");
        cmd.arg("30");
        let started = Instant::now();
        spawn_detached(cmd, Duration::from_millis(200)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(2), "must not wait for the helper");
        let mut ok = imt_core::process::quiet_command("true");
        ok.arg("x");
        spawn_detached(ok, Duration::from_secs(5)).unwrap();
        let mut failing = imt_core::process::quiet_command("false");
        failing.arg("x");
        spawn_detached(failing, Duration::from_secs(5)).unwrap();
        // Give the reaper threads time to run their exit/timeout paths.
        std::thread::sleep(Duration::from_millis(600));
    }
}
