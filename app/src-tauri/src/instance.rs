//! One Debrief process per user: a second launch hands its arguments to the
//! running instance, which opens them per the `openFilesIn` setting and
//! focuses the window. `--new-instance` and safe mode start a separate
//! process. macOS "Open with" / double-click (`RunEvent::Opened`) follows the
//! same rules.
//!
//! Parsing and planning are pure functions; [`open_files`] applies a plan.

use crate::lifecycle::SAFE_MODE_FLAG;
use crate::window_manager::{ask_open, open_new, reveal, target};
use crate::window_prefs::{OpenFilesIn, SharedPrefs};
use crate::windows::{queue_open, Windows};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

/// Starts a separate process even when Debrief is already running.
pub const NEW_INSTANCE_FLAG: &str = "--new-instance";
/// Opens the given files (or nothing) in a new window of the running instance.
pub const NEW_WINDOW_FLAG: &str = "--new-window";
/// Command-line arguments are capped (a hostile caller cannot make Debrief open millions of paths).
const MAX_PATHS: usize = 256;
/// Ends the flags: every later argument is a path, even one that looks like a flag.
pub const END_OF_FLAGS: &str = "--";

/// The arguments before [`END_OF_FLAGS`]: the only ones read as flags (`--export-diagnostics`,
/// `--isolated`, `--safe-mode`, ...), so a path after it never switches on a mode.
pub fn flags<S: AsRef<str>>(args: &[S]) -> &[S] {
    let end = args.iter().position(|a| a.as_ref() == END_OF_FLAGS).unwrap_or(args.len());
    &args[..end]
}

/// What a command line asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchArgs {
    /// Absolute paths (relative ones resolved against the caller's folder), not yet checked.
    pub paths: Vec<PathBuf>,
    pub new_window: bool,
    pub new_instance: bool,
    pub safe_mode: bool,
}

/// Parses arguments (without the program name). Unknown flags are ignored.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I, cwd: Option<&Path>) -> LaunchArgs {
    let mut out = LaunchArgs::default();
    let mut paths_only = false;
    for arg in args {
        match arg.as_str() {
            a if paths_only => {
                if !a.is_empty() && !a.contains('\0') && out.paths.len() < MAX_PATHS {
                    out.paths.push(resolve(a, cwd));
                }
            }
            END_OF_FLAGS => paths_only = true,
            NEW_INSTANCE_FLAG => out.new_instance = true,
            NEW_WINDOW_FLAG => out.new_window = true,
            SAFE_MODE_FLAG => out.safe_mode = true,
            a if a.starts_with('-') || a.is_empty() || a.contains('\0') => {}
            a if out.paths.len() < MAX_PATHS => out.paths.push(resolve(a, cwd)),
            _ => {}
        }
    }
    out
}

fn resolve(arg: &str, cwd: Option<&Path>) -> PathBuf {
    let p = PathBuf::from(arg);
    match cwd {
        Some(dir) if p.is_relative() => dir.join(p),
        _ => p,
    }
}

/// Whether this process joins the single-instance group (and hands its
/// arguments over when another instance runs).
pub fn wants_single_instance(args: &LaunchArgs, safe_mode_env: bool) -> bool {
    !args.new_instance && !args.safe_mode && !safe_mode_env
}

/// Existing paths only, canonicalized (the edge between the OS and the app).
pub fn existing(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// The window files would go to: its label, and whether nothing is open there yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub label: String,
    pub pristine: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenPlan {
    /// Nothing to open: show and focus this window.
    Reveal(String),
    NewWindow(Vec<String>),
    OpenIn(String, Vec<String>),
    /// Ask in this window whether to open here or in a new window.
    Ask(String, Vec<String>),
}

impl OpenPlan {
    /// For logs (paths stay out of them).
    pub fn kind(&self) -> &'static str {
        match self {
            OpenPlan::Reveal(_) => "reveal",
            OpenPlan::NewWindow(_) => "new-window",
            OpenPlan::OpenIn(..) => "current-window",
            OpenPlan::Ask(..) => "ask",
        }
    }
}

/// Where files from another app go. An empty window is always reused.
pub fn plan(paths: Vec<String>, force_new_window: bool, setting: OpenFilesIn, target: Option<Target>) -> OpenPlan {
    let Some(t) = target else {
        return OpenPlan::NewWindow(paths);
    };
    if force_new_window {
        return OpenPlan::NewWindow(paths);
    }
    if paths.is_empty() {
        return OpenPlan::Reveal(t.label);
    }
    if t.pristine {
        return OpenPlan::OpenIn(t.label, paths);
    }
    match setting {
        OpenFilesIn::NewWindow => OpenPlan::NewWindow(paths),
        OpenFilesIn::CurrentWindow => OpenPlan::OpenIn(t.label, paths),
        OpenFilesIn::Ask => OpenPlan::Ask(t.label, paths),
    }
}

/// Opens files that came from outside (second launch, the OS, the tray) per the setting.
pub fn open_files<R: Runtime>(app: &AppHandle<R>, paths: Vec<String>, force_new_window: bool) {
    let setting = app.try_state::<SharedPrefs>().map(|p| p.get().open_files_in).unwrap_or_default();
    let target = target(app).map(|label| Target {
        pristine: app
            .try_state::<Windows>()
            .and_then(|w| w.get(&label))
            .is_some_and(|s| s.is_pristine()),
        label,
    });
    let plan = plan(paths, force_new_window, setting, target);
    log::info!(Plan = plan.kind(); "Opening files from another app");
    let result = match plan {
        OpenPlan::Reveal(label) => reveal(app, &label),
        OpenPlan::NewWindow(paths) => open_new(app, paths).map(|_| ()).map_err(|e| crate::CmdError::from(crate::err(e))),
        OpenPlan::OpenIn(label, paths) => {
            queue_open(app, &label, paths);
            reveal(app, &label)
        }
        OpenPlan::Ask(label, paths) => {
            let shown = reveal(app, &label);
            ask_open(app, &label, paths);
            shown
        }
    };
    if let Err(e) = result {
        log::warn!("could not open files from another app: {e}");
    }
}

/// The single-instance callback: another launch passed `argv` (program name first) from `cwd`.
pub fn on_second_instance<R: Runtime>(app: &AppHandle<R>, argv: Vec<String>, cwd: String) {
    let cwd = (!cwd.is_empty()).then(|| PathBuf::from(&cwd));
    let args = parse_args(argv.into_iter().skip(1), cwd.as_deref());
    let paths = existing(&args.paths);
    // Counts only: paths stay out of the logs.
    log::info!(Paths = paths.len() as u64, Requested = args.paths.len() as u64, NewWindow = args.new_window; "Second launch forwarded to the running instance");
    open_files(app, paths, args.new_window);
}

/// Whether this process holds the single-instance lock (managed state).
pub struct SingleInstance(pub bool);

/// Releases the single-instance lock before Debrief relaunches itself, so the new
/// process becomes the running instance instead of handing over to this one.
pub fn release<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<SingleInstance>().is_some_and(|s| s.0) {
        #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
        tauri_plugin_single_instance::destroy(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{main_window, samples, test_app, window};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_flags_and_resolves_relative_paths() {
        let a = parse_args(
            args(&["logs", "/abs/x.debriefproj", "--new-window", "--verbose", ""]),
            Some(Path::new("/home/u")),
        );
        assert_eq!(a.paths, [PathBuf::from("/home/u/logs"), PathBuf::from("/abs/x.debriefproj")]);
        assert!(a.new_window && !a.new_instance && !a.safe_mode);
        assert!(wants_single_instance(&a, false));
        let b = parse_args(args(&["--new-instance", "rel"]), None);
        assert_eq!(b.paths, [PathBuf::from("rel")]);
        assert!(!wants_single_instance(&b, false));
        let c = parse_args(args(&[SAFE_MODE_FLAG]), None);
        assert!(c.safe_mode && !wants_single_instance(&c, false));
        assert!(!wants_single_instance(&LaunchArgs::default(), true), "DEBRIEF_SAFE_MODE");
        let many: Vec<String> = (0..1000).map(|i| format!("/p{i}")).collect();
        assert_eq!(parse_args(many, None).paths.len(), MAX_PATHS);
        assert!(parse_args(args(&["a\0b"]), None).paths.is_empty());
    }

    #[test]
    fn everything_after_the_end_of_flags_is_a_path() {
        let a = parse_args(args(&["--new-window", "--", "--safe-mode", "/x.log", "", "a\0b"]), None);
        assert!(a.new_window && !a.safe_mode);
        assert_eq!(a.paths, [PathBuf::from("--safe-mode"), PathBuf::from("/x.log")]);
        assert_eq!(flags(&["--isolated", "--", "--export-diagnostics"]), ["--isolated"]);
        assert_eq!(flags(&["a", "b"]), ["a", "b"]);
        assert!(flags::<&str>(&[]).is_empty());
    }

    #[test]
    fn existing_keeps_real_paths_only() {
        let found = existing(&[samples().join("csv"), PathBuf::from("/does/not/exist")]);
        assert_eq!(found.len(), 1);
        assert!(Path::new(&found[0]).is_absolute());
    }

    #[test]
    fn plans_per_setting() {
        use OpenFilesIn::*;
        let busy = || {
            Some(Target {
                label: "w-2".into(),
                pristine: false,
            })
        };
        let empty = || {
            Some(Target {
                label: "main".into(),
                pristine: true,
            })
        };
        let p = || vec!["/a".to_string()];
        assert_eq!(plan(p(), false, NewWindow, busy()), OpenPlan::NewWindow(p()));
        assert_eq!(plan(p(), false, CurrentWindow, busy()), OpenPlan::OpenIn("w-2".into(), p()));
        assert_eq!(plan(p(), false, Ask, busy()), OpenPlan::Ask("w-2".into(), p()));
        assert_eq!(
            plan(p(), false, NewWindow, empty()),
            OpenPlan::OpenIn("main".into(), p()),
            "an empty window is reused"
        );
        assert_eq!(plan(p(), true, CurrentWindow, empty()), OpenPlan::NewWindow(p()), "--new-window");
        assert_eq!(plan(vec![], false, Ask, busy()), OpenPlan::Reveal("w-2".into()));
        assert_eq!(plan(vec![], true, Ask, busy()), OpenPlan::NewWindow(vec![]));
        assert_eq!(
            plan(p(), false, CurrentWindow, None),
            OpenPlan::NewWindow(p()),
            "no window left (tray)"
        );
    }

    #[test]
    fn second_launch_opens_files_in_the_running_instance() {
        let app = test_app(None);
        let h = app.handle().clone();
        window(&app, "main");
        let csv = samples().join("csv").to_string_lossy().into_owned();
        // The empty main window takes the files.
        on_second_instance(&h, args(&["debrief-app", &csv, "/does/not/exist"]), String::new());
        assert_eq!(main_window(&app).take_pending().len(), 1);
        // Relative to the caller's folder.
        let dir = samples().to_string_lossy().into_owned();
        on_second_instance(&h, args(&["debrief-app", "--new-window", "csv"]), dir);
        assert_eq!(crate::windows::CurrentWindow::of(&h, "w-2").unwrap().take_pending().len(), 1);
        // No paths: the window is just shown.
        on_second_instance(&h, args(&["debrief-app"]), String::new());
        assert_eq!(app.webview_windows().len(), 2);
        // Releasing without holding the lock does nothing.
        app.manage(SingleInstance(false));
        release(&h);
    }
}
