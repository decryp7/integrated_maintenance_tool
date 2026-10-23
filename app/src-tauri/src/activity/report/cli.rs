//! `debrief-app --export-diagnostics [<path.zip>] [--force]`: writes the same report as Help ▸ Report a problem
//! without the UI — no window, no plugins, no workers — and exits 0 (printing where it went) or 1 (with
//! what to fix). Without a path it writes `debrief-diagnostics-<timestamp>.zip` into Documents (else Downloads,
//! else the home folder). A path must end in `.zip` and its folder must exist; an existing file is replaced
//! only with `--force`.
//!
//! The report has no description; the steps come from the activity entries of Debrief's log (the last
//! session's breadcrumbs), and settings files that cannot be read are listed instead of failing the export.
//! Debrief's folders are found as the app finds them (`DEBRIEF_DATA_DIR` in debug builds, see `dev_overrides`).

use super::{breadcrumbs_from_log, ReportInputs, REPORT_BREADCRUMBS};
use crate::app_dirs::{under, AppDir};
use crate::snapshots::{Limits, SnapshotStore};
use chrono::{DateTime, Local, Utc};
use std::path::{Path, PathBuf};

/// The flag.
pub const FLAG: &str = "--export-diagnostics";
/// Replace an existing file.
pub const FORCE: &str = "--force";
/// The app's identifier (`tauri.conf.json`), which names its folders.
pub const IDENTIFIER: &str = "net.decryptology.debrief";

/// Where the report goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    Path(PathBuf),
    /// A timestamped file in the user's Documents (or Downloads, or home) folder.
    Default,
}

/// The parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportArgs {
    pub destination: Destination,
    pub force: bool,
}

/// `None` when the command line does not ask for an export (the app starts normally). Only the flags
/// before `--` count: a path after it (a relaunch reopening a session) never triggers an export.
pub fn parse<S: AsRef<str>>(args: &[S]) -> Option<Result<ExportArgs, String>> {
    let args: Vec<&str> = crate::instance::flags(args).iter().map(AsRef::as_ref).collect();
    let at = args.iter().position(|a| *a == FLAG || a.starts_with(&format!("{FLAG}=")))?;
    let force = args.contains(&FORCE);
    let inline = args[at].strip_prefix(&format!("{FLAG}=")).map(str::to_string);
    let next = args.get(at + 1).filter(|a| !a.starts_with("--")).map(|a| a.to_string());
    let path = inline.or(next);
    Some(match path {
        Some(p) if p.trim().is_empty() => Err(format!("{FLAG} needs a file name ending in .zip")),
        Some(p) => Ok(ExportArgs {
            destination: Destination::Path(PathBuf::from(p)),
            force,
        }),
        None => Ok(ExportArgs {
            destination: Destination::Default,
            force,
        }),
    })
}

/// `debrief-diagnostics-20261014-091503.zip` (local time).
pub fn default_file_name(now: &DateTime<Local>) -> String {
    format!("debrief-diagnostics-{}.zip", now.format("%Y%m%d-%H%M%S"))
}

/// The absolute path to write: relative paths are taken from `cwd`; the folder must exist (it is
/// canonicalised); the name must end in `.zip`; an existing file is refused unless `force`.
pub fn resolve(args: &ExportArgs, cwd: &Path, default_dirs: &[PathBuf], now: &DateTime<Local>) -> Result<PathBuf, String> {
    let path = match &args.destination {
        Destination::Path(p) if p.is_absolute() => p.clone(),
        Destination::Path(p) => cwd.join(p),
        Destination::Default => default_dirs
            .iter()
            .find(|d| d.is_dir())
            .ok_or("found no Documents, Downloads or home folder; give the file to write: --export-diagnostics <path.zip>")?
            .join(default_file_name(now)),
    };
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        return Err(format!("{} must end in .zip", path.display()));
    }
    let name = path.file_name().ok_or_else(|| format!("{} has no file name", path.display()))?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(cwd);
    let parent = parent
        .canonicalize()
        .map_err(|_| format!("the folder {} does not exist", parent.display()))?;
    let dest = parent.join(name);
    if dest.is_dir() {
        return Err(format!("{} is a folder", dest.display()));
    }
    if dest.exists() && !args.force {
        return Err(format!("{} already exists; add {FORCE} to replace it", dest.display()));
    }
    Ok(dest)
}

/// Debrief's folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    pub data: PathBuf,
    pub config: PathBuf,
    pub log: PathBuf,
}

/// The folders the app uses, found without starting it: under the override root when there is one, else
/// where Tauri puts them for [`IDENTIFIER`] (data and config in the roaming/app data folders; logs in
/// `~/Library/Logs` on macOS, else the local data folder's `logs`).
pub fn folders(root: Option<&Path>) -> Option<Folders> {
    if let Some(root) = root {
        return Some(Folders {
            data: under(root, AppDir::Data),
            config: under(root, AppDir::Config),
            log: under(root, AppDir::Log),
        });
    }
    let log = if cfg!(target_os = "macos") {
        dirs::home_dir()?.join("Library").join("Logs").join(IDENTIFIER)
    } else {
        dirs::data_local_dir()?.join(IDENTIFIER).join("logs")
    };
    Some(Folders {
        data: dirs::data_dir()?.join(IDENTIFIER),
        config: dirs::config_dir()?.join(IDENTIFIER),
        log,
    })
}

/// The inputs of a report made from the folders alone (no running app).
pub fn inputs(f: &Folders, now: DateTime<Utc>) -> ReportInputs {
    let logs = super::log_window(&f.log);
    let breadcrumbs = breadcrumbs_from_log(&super::activity_lines(&logs), REPORT_BREADCRUMBS);
    ReportInputs {
        log_dir: Some(f.log.clone()),
        config_dir: Some(f.config.clone()),
        data_dir: Some(f.data.clone()),
        snapshots: Some(SnapshotStore::new(f.log.join("snapshots"), None, Limits::default())),
        breadcrumbs,
        created_at: now,
        ..ReportInputs::empty()
    }
}

/// What an export did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    pub path: PathBuf,
    pub files: usize,
    /// Settings files that could not be read ("settings.json is not valid JSON ...").
    pub unreadable: Vec<String>,
}

/// Writes the report for `args`. Pure but for the file system: folders, working directory and time are given.
pub fn export(args: &ExportArgs, f: &Folders, cwd: &Path, default_dirs: &[PathBuf], now: DateTime<Utc>) -> Result<Exported, String> {
    let dest = resolve(args, cwd, default_dirs, &now.with_timezone(&Local))?;
    let inputs = inputs(f, now);
    let unreadable = super::unreadable_settings(&super::system_info(&inputs));
    if dest.exists() {
        std::fs::remove_file(&dest).map_err(|e| format!("cannot replace {}: {e}", dest.display()))?;
    }
    let files = super::write(&inputs, "", &dest).map_err(|e| format!("{e:#}"))?;
    Ok(Exported {
        path: dest,
        files,
        unreadable,
    })
}

/// Lets a release build (a GUI program on Windows) print to the console it was started from.
fn attach_console() {
    #[cfg(windows)]
    // SAFETY: plain Win32 call without pointers; failing (no parent console, or one already attached) is fine.
    unsafe {
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}

/// Runs the export when the command line asks for it and returns the exit code; `None` otherwise (the app
/// starts). Call first thing in `run`, after `dev_overrides::init`.
pub fn run_if_requested() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = parse(&args)?;
    attach_console();
    let defaults: Vec<PathBuf> = [dirs::document_dir(), dirs::download_dir(), dirs::home_dir()]
        .into_iter()
        .flatten()
        .collect();
    let result = parsed.and_then(|a| {
        let f = folders(crate::dev_overrides::current().data_dir.as_deref()).ok_or("cannot find Debrief's folders (no home folder)")?;
        let cwd = std::env::current_dir().map_err(|e| format!("cannot read the current folder: {e}"))?;
        export(&a, &f, &cwd, &defaults, Utc::now())
    });
    Some(match result {
        Ok(done) => {
            for u in &done.unreadable {
                eprintln!("Debrief diagnostics: could not read {u}");
            }
            println!("{}", done.path.display());
            eprintln!("Saved Debrief diagnostics ({} files) to {}", done.files, done.path.display());
            0
        }
        Err(e) => {
            eprintln!("Debrief could not export diagnostics: {e}");
            1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 14, 9, 15, 3).unwrap()
    }

    #[test]
    fn arguments_are_parsed() {
        assert_eq!(parse(&["--safe-mode", "a.log"]), None);
        assert_eq!(
            parse(&["--", "--export-diagnostics", "/x.zip", "--force"]),
            None,
            "a path, not a flag"
        );
        assert_eq!(
            parse(&["--export-diagnostics", "out.zip"]),
            Some(Ok(ExportArgs {
                destination: Destination::Path("out.zip".into()),
                force: false
            }))
        );
        assert_eq!(
            parse(&["--export-diagnostics=out.zip", "--force"]).unwrap().unwrap().destination,
            Destination::Path("out.zip".into())
        );
        let d = parse(&["--export-diagnostics", "--force"]).unwrap().unwrap();
        assert_eq!((d.destination, d.force), (Destination::Default, true));
        assert_eq!(parse(&["--export-diagnostics"]).unwrap().unwrap().destination, Destination::Default);
        assert!(parse(&["--export-diagnostics="]).unwrap().is_err());
    }

    #[test]
    fn destinations_are_validated() {
        let d = crate::activity::report::tests::temp("cli-dest");
        let local = now().with_timezone(&Local);
        let args = |p: &str, force: bool| ExportArgs {
            destination: Destination::Path(p.into()),
            force,
        };
        // Relative to the working folder, canonicalised.
        let got = resolve(&args("r.zip", false), &d, &[], &local).unwrap();
        assert_eq!(got, d.canonicalize().unwrap().join("r.zip"));
        assert!(got.is_absolute());
        assert!(resolve(&args("r.txt", false), &d, &[], &local)
            .unwrap_err()
            .contains("must end in .zip"));
        assert!(resolve(&args("missing/r.zip", false), &d, &[], &local)
            .unwrap_err()
            .contains("does not exist"));
        std::fs::write(d.join("r.zip"), "x").unwrap();
        assert!(resolve(&args("r.zip", false), &d, &[], &local).unwrap_err().contains("--force"));
        assert!(resolve(&args("r.zip", true), &d, &[], &local).is_ok());
        std::fs::create_dir_all(d.join("dir.zip")).unwrap();
        assert!(resolve(&args("dir.zip", true), &d, &[], &local)
            .unwrap_err()
            .contains("is a folder"));
        // The default: the first folder that exists, a timestamped name.
        let def = ExportArgs {
            destination: Destination::Default,
            force: false,
        };
        let got = resolve(&def, &d, &[d.join("nope"), d.clone()], &local).unwrap();
        assert_eq!(got.file_name().unwrap().to_string_lossy(), default_file_name(&local));
        assert!(default_file_name(&local).starts_with("debrief-diagnostics-2026101"));
        assert!(resolve(&def, &d, &[d.join("nope")], &local).unwrap_err().contains("Documents"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn folders_follow_the_override_root_and_the_identifier() {
        let root = std::env::temp_dir().join("debrief-cli-root");
        let f = folders(Some(&root)).unwrap();
        assert_eq!(
            (f.data, f.config, f.log),
            (root.join("data"), root.join("config"), root.join("logs"))
        );
        if let Some(f) = folders(None) {
            assert!(f.data.ends_with(IDENTIFIER) && f.config.ends_with(IDENTIFIER));
        }
        // The identifier is the app's.
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], IDENTIFIER);
    }

    #[test]
    fn the_export_writes_the_report_even_with_corrupt_settings() {
        let d = crate::activity::report::tests::temp("cli-export");
        let f = folders(Some(&d)).unwrap();
        std::fs::create_dir_all(&f.log).unwrap();
        std::fs::create_dir_all(&f.config).unwrap();
        std::fs::create_dir_all(&f.data).unwrap();
        std::fs::write(
            f.log.join("debrief.2026-10-14.log"),
            "{\"@t\":\"2026-10-14T09:00:00Z\",\"SourceContext\":\"Debrief.Activity\",\"Operation\":\"file.open\",\"Trigger\":\"menu\",\"Window\":\"main\"}\n",
        )
        .unwrap();
        std::fs::write(f.config.join("settings.json"), "{ corrupt").unwrap();
        std::fs::write(f.data.join("running.marker"), r#"{"version":1,"pid":4000000000,"startedAt":5}"#).unwrap();
        let args = ExportArgs {
            destination: Destination::Path("out.zip".into()),
            force: false,
        };
        let done = export(&args, &f, &d, &[], now()).unwrap();
        assert_eq!(done.unreadable.len(), 1, "{:?}", done.unreadable);
        assert!(done.files > 5);
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&done.path).unwrap()).unwrap();
        let steps = crate::activity::report::tests::zip_text(&mut zip, "steps-to-reproduce.txt");
        assert!(steps.starts_with("1. Opened files from the menu"), "{steps}");
        let rec = crate::activity::report::tests::zip_text(&mut zip, "session-recovery.json");
        assert!(rec.contains("\"processAlive\": false"), "{rec}");
        // Again: refused without --force, replaced with it.
        assert!(export(&args, &f, &d, &[], now()).unwrap_err().contains("--force"));
        let forced = ExportArgs { force: true, ..args };
        assert!(export(&forced, &f, &d, &[], now()).is_ok());
        std::fs::remove_dir_all(d).unwrap();
    }
}
