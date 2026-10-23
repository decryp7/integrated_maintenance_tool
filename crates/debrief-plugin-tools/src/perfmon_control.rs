//! Drives Performance Monitor Data Collector Sets through `logman`.
//!
//! [`PerfmonController`] imports a template ([`crate::perfmon_template`]), starts, stops,
//! deletes and queries a set, and lists the log files it produced. `logman` is reached
//! only through a [`crate::Runner`], so tests use a fake.
//!
//! Limitation: `logman query` prints localized text. Parsing relies on the English keys
//! `Status:`, `Root Path:`, `Output Location:` and `Segment:`; on other display languages
//! the status is [`RunState::Unknown`] and the paths are missing.

use crate::Runner;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Time allowed for one `logman` call unless configured otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// What `logman` printed and how it ended.
struct LogmanOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PerfmonError {
    /// Not running on Windows.
    Unsupported,
    /// `logman` refused: the user lacks rights.
    AccessDenied,
    /// The name or an argument was rejected before running anything.
    Invalid(String),
    /// `logman` could not be started or timed out.
    Io(String),
    /// `logman` exited with an error.
    Failed { code: Option<i32>, message: String },
}

impl fmt::Display for PerfmonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PerfmonError::Unsupported => write!(f, "Performance Monitor collectors can only be managed on Windows"),
            PerfmonError::AccessDenied => write!(
                f,
                "Access is denied: run Debrief elevated (as administrator) or add your account to the \"Performance Log Users\" group"
            ),
            PerfmonError::Invalid(m) => write!(f, "{m}"),
            PerfmonError::Io(m) => write!(f, "could not run logman: {m}"),
            PerfmonError::Failed { code, message } => match code {
                Some(c) => write!(f, "logman failed (exit code {c}): {message}"),
                None => write!(f, "logman failed: {message}"),
            },
        }
    }
}

impl std::error::Error for PerfmonError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RunState {
    Running,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectorStatus {
    pub name: String,
    pub status: RunState,
    pub root_path: Option<String>,
    /// Folder or file the (next) log is written to.
    pub output_location: Option<String>,
    /// The `Segment:` value (`On`/`Off`).
    pub segment: Option<String>,
}

const ACCESS_DENIED_HRESULT: i32 = 0x8007_0005_u32 as i32;

pub struct PerfmonController {
    runner: Runner,
    windows: bool,
    timeout: Duration,
}

impl PerfmonController {
    pub fn new(runner: Runner) -> Self {
        Self::with_platform(runner, cfg!(windows))
    }

    /// `windows` is injectable so both platforms are testable everywhere.
    pub fn with_platform(runner: Runner, windows: bool) -> Self {
        PerfmonController {
            runner,
            windows,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// How long one `logman` call may take before it is stopped.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn logman(&self, args: &[&str]) -> Result<LogmanOutput, PerfmonError> {
        if !self.windows {
            return Err(PerfmonError::Unsupported);
        }
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let out = self
            .runner
            .run("logman", &args, self.timeout)
            .map_err(|e| PerfmonError::Io(e.to_string()))?;
        let out = LogmanOutput {
            code: out.exit_code,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: out.stderr,
        };
        if out.code == Some(0) {
            return Ok(out);
        }
        let message = format!("{}\n{}", out.stderr.trim(), out.stdout.trim()).trim().to_string();
        let lower = message.to_ascii_lowercase();
        if lower.contains("access is denied") || lower.contains("0x80070005") || out.code == Some(ACCESS_DENIED_HRESULT) {
            return Err(PerfmonError::AccessDenied);
        }
        Err(PerfmonError::Failed { code: out.code, message })
    }

    /// Imports a template XML as `name`. With `replace`, an existing set of that name is stopped and deleted first.
    pub fn import(&self, name: &str, xml_path: &Path, replace: bool) -> Result<(), PerfmonError> {
        check_name(name)?;
        if replace && self.exists(name)? {
            self.delete(name)?;
        }
        let xml = xml_path.to_string_lossy();
        self.logman(&["import", "-n", name, "-xml", &xml]).map(|_| ())
    }

    pub fn start(&self, name: &str) -> Result<(), PerfmonError> {
        check_name(name)?;
        self.logman(&["start", "-n", name]).map(|_| ())
    }

    pub fn stop(&self, name: &str) -> Result<(), PerfmonError> {
        check_name(name)?;
        self.logman(&["stop", "-n", name]).map(|_| ())
    }

    /// Deletes the set, stopping it first when it is running.
    pub fn delete(&self, name: &str) -> Result<(), PerfmonError> {
        check_name(name)?;
        if let Ok(s) = self.query(name) {
            if s.status == RunState::Running {
                self.stop(name)?;
            }
        }
        self.logman(&["delete", "-n", name]).map(|_| ())
    }

    pub fn query(&self, name: &str) -> Result<CollectorStatus, PerfmonError> {
        check_name(name)?;
        let out = self.logman(&["query", name])?;
        Ok(parse_query(name, &out.stdout))
    }

    fn exists(&self, name: &str) -> Result<bool, PerfmonError> {
        match self.query(name) {
            Ok(_) => Ok(true),
            Err(PerfmonError::Failed { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

fn check_name(name: &str) -> Result<(), PerfmonError> {
    let bad = name.trim().is_empty() || name.starts_with('-') || name.chars().any(|c| c.is_control() || "\"/:*?<>|".contains(c));
    if bad {
        return Err(PerfmonError::Invalid(format!("invalid collector name \"{name}\"")));
    }
    Ok(())
}

/// Parses the English `logman query <name>` listing. The first occurrence of each key wins
/// (the set's own section precedes its data collectors).
pub fn parse_query(name: &str, text: &str) -> CollectorStatus {
    let mut s = CollectorStatus {
        name: name.to_string(),
        status: RunState::Unknown,
        root_path: None,
        output_location: None,
        segment: None,
    };
    let mut seen_status = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "Status" if !seen_status => {
                seen_status = true;
                s.status = match value.to_ascii_lowercase().as_str() {
                    "running" => RunState::Running,
                    "stopped" => RunState::Stopped,
                    _ => RunState::Unknown,
                };
            }
            "Root Path" if s.root_path.is_none() => s.root_path = Some(value.to_string()),
            "Output Location" if s.output_location.is_none() => s.output_location = Some(value.to_string()),
            "Segment" if s.segment.is_none() => s.segment = Some(value.to_string()),
            _ => {}
        }
    }
    s
}

/// Replaces `%NAME%` with the environment variable's value when it is set.
fn expand_env(path: &str) -> String {
    let mut out = String::new();
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => match std::env::var(&after[..end]) {
                Ok(v) if !after[..end].is_empty() => {
                    out.push_str(&v);
                    rest = &after[end + 1..];
                }
                _ => {
                    out.push('%');
                    rest = after;
                }
            },
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

const MAX_DEPTH: usize = 4;
const LOG_EXTENSIONS: [&str; 3] = ["blg", "csv", "tsv"];

fn walk(dir: &Path, depth: usize, out: &mut Vec<(SystemTime, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() && depth < MAX_DEPTH {
            walk(&path, depth + 1, out);
        } else if kind.is_file() {
            let wanted = path
                .extension()
                .is_some_and(|e| LOG_EXTENSIONS.iter().any(|x| e.to_string_lossy().eq_ignore_ascii_case(x)));
            if wanted {
                let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                out.push((modified, path));
            }
        }
    }
}

/// The `.blg`/`.csv`/`.tsv` files under the set's root path (and output location), newest first.
pub fn output_files(status: &CollectorStatus) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for p in [&status.root_path, &status.output_location].into_iter().flatten() {
        let path = PathBuf::from(expand_env(p));
        let dir = if path.extension().is_some() {
            path.parent().map(Path::to_path_buf).unwrap_or(path)
        } else {
            path
        };
        if !dirs.iter().any(|d| dir.starts_with(d)) {
            dirs.retain(|d| !d.starts_with(&dir));
            dirs.push(dir);
        }
    }
    let mut found = Vec::new();
    for d in dirs {
        walk(&d, 0, &mut found);
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.dedup_by(|a, b| a.1 == b.1);
    found.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{fail, ok, FakeRunner};
    use debrief_collect::{CommandError, CommandOutput};
    use std::sync::{Arc, Mutex};

    const QUERY: &str = "\
Name:                 Debrief
Status:               Running
Root Path:            %DEBRIEF_TEST_ROOT%
Segment:              Off
Schedules:            On
Run as:               SYSTEM

Name:                 Debrief\\PerfCounters
Type:                 Counter
Output Location:      %DEBRIEF_TEST_ROOT%\\20261014-000001\\PerfCounters.csv
Append:               Off
Status:               Ignored
Counters:             \\Process(*)\\Private Bytes
";

    fn controller(fake: &Arc<FakeRunner>) -> PerfmonController {
        PerfmonController::with_platform(fake.clone(), true)
    }

    #[test]
    fn parses_the_query_listing() {
        let s = parse_query("Debrief", QUERY);
        assert_eq!(s.status, RunState::Running);
        assert_eq!(s.root_path.as_deref(), Some("%DEBRIEF_TEST_ROOT%"));
        assert_eq!(
            s.output_location.as_deref(),
            Some("%DEBRIEF_TEST_ROOT%\\20261014-000001\\PerfCounters.csv")
        );
        assert_eq!(s.segment.as_deref(), Some("Off"));
        assert_eq!(parse_query("x", "Status: Stopped\n").status, RunState::Stopped);
        // Localized output is not understood.
        let de = parse_query("x", "Status:  Wird ausgeführt\nStammpfad: C:\\x\n");
        assert_eq!((de.status, de.root_path), (RunState::Unknown, None));
        assert_eq!(parse_query("x", "no keys here\nStatus:\n").status, RunState::Unknown);
    }

    #[test]
    fn start_stop_import_use_the_right_command_lines() {
        let fake = Arc::new(FakeRunner::new(|_, _| ok("The command completed successfully.")));
        let c = controller(&fake);
        c.import("Debrief", Path::new("C:\\t\\debrief.xml"), false).unwrap();
        c.start("Debrief").unwrap();
        c.stop("Debrief").unwrap();
        assert_eq!(
            fake.calls(),
            [
                "logman import -n Debrief -xml C:\\t\\debrief.xml",
                "logman start -n Debrief",
                "logman stop -n Debrief"
            ]
        );
    }

    #[test]
    fn replace_deletes_an_existing_set_first_and_stops_it_if_running() {
        let state = Arc::new(Mutex::new("Running"));
        let s2 = state.clone();
        let fake = Arc::new(FakeRunner::new(move |_, args| match args[0].as_str() {
            "query" => ok(&format!("Name: Debrief\nStatus: {}\n", s2.lock().unwrap())),
            "stop" => {
                *s2.lock().unwrap() = "Stopped";
                ok("")
            }
            _ => ok(""),
        }));
        controller(&fake).import("Debrief", Path::new("x.xml"), true).unwrap();
        assert_eq!(
            fake.calls(),
            [
                "logman query Debrief",
                "logman query Debrief",
                "logman stop -n Debrief",
                "logman delete -n Debrief",
                "logman import -n Debrief -xml x.xml"
            ]
        );
        // Not existing: straight to import.
        let fake = Arc::new(FakeRunner::new(|_, args| {
            if args[0] == "query" {
                fail(1, "", "Data Collector Set was not found.")
            } else {
                ok("")
            }
        }));
        controller(&fake).import("Debrief", Path::new("x.xml"), true).unwrap();
        assert_eq!(fake.calls().len(), 2);
        // A permission error while probing surfaces.
        let fake = Arc::new(FakeRunner::new(|_, _| fail(5, "", "Access is denied.")));
        assert_eq!(
            controller(&fake).import("Debrief", Path::new("x.xml"), true),
            Err(PerfmonError::AccessDenied)
        );
    }

    #[test]
    fn maps_errors() {
        let denied = |code: i32, out: &'static str, err: &'static str| {
            let fake = Arc::new(FakeRunner::new(move |_, _| fail(code, out, err)));
            controller(&fake).start("Debrief")
        };
        assert_eq!(denied(5, "", "Error:\n Access is denied."), Err(PerfmonError::AccessDenied));
        assert_eq!(denied(1, "Error: 0x80070005", ""), Err(PerfmonError::AccessDenied));
        assert_eq!(denied(0x8007_0005_u32 as i32, "", ""), Err(PerfmonError::AccessDenied));
        let e = denied(3, "", "boom").unwrap_err();
        assert_eq!(
            e,
            PerfmonError::Failed {
                code: Some(3),
                message: "boom".into()
            }
        );
        assert_eq!(e.to_string(), "logman failed (exit code 3): boom");
        let killed = Arc::new(FakeRunner::new(|_, _| Ok(CommandOutput::default())));
        assert_eq!(controller(&killed).stop("Debrief").unwrap_err().to_string(), "logman failed: ");
        let io = Arc::new(FakeRunner::new(|_, _| Err(CommandError::NotFound("logman".into()))));
        assert_eq!(
            controller(&io).stop("Debrief").unwrap_err().to_string(),
            "could not run logman: logman was not found on this machine"
        );
        assert!(PerfmonError::AccessDenied.to_string().contains("Performance Log Users"));
        assert!(PerfmonError::Invalid("x".into()).to_string() == "x");
    }

    #[test]
    fn unsupported_off_windows_and_names_are_checked() {
        let fake = Arc::new(FakeRunner::new(|_, _| ok("")));
        let c = PerfmonController::with_platform(fake.clone(), false);
        assert_eq!(c.start("Debrief"), Err(PerfmonError::Unsupported));
        assert_eq!(c.query("Debrief").unwrap_err(), PerfmonError::Unsupported);
        assert!(PerfmonError::Unsupported.to_string().contains("only be managed on Windows"));
        assert!(fake.calls().is_empty());
        let c = controller(&fake);
        for bad in ["", "-x", "a\"b", "a\nb", "a/b"] {
            assert!(matches!(c.start(bad), Err(PerfmonError::Invalid(_))), "{bad}");
        }
        assert!(c.stop("").is_err() && c.delete("").is_err() && c.query("").is_err() && c.import("", Path::new("x"), false).is_err());
        let default = PerfmonController::new(fake.clone()).timeout(Duration::from_secs(5));
        assert_eq!((default.windows, default.timeout), (cfg!(windows), Duration::from_secs(5)));
        // The configured timeout reaches the runner.
        let _ = controller(&fake).timeout(Duration::from_secs(7)).start("Debrief");
        assert_eq!(fake.timeouts().last(), Some(&Duration::from_secs(7)));
    }

    #[test]
    fn delete_stops_running_sets_only() {
        let fake = Arc::new(FakeRunner::new(
            |_, args| if args[0] == "query" { ok("Status: Stopped\n") } else { ok("") },
        ));
        controller(&fake).delete("Debrief").unwrap();
        assert_eq!(fake.calls(), ["logman query Debrief", "logman delete -n Debrief"]);
    }

    #[test]
    fn finds_output_files_newest_first() {
        // Nested one level down: on Unix the backslashes of the Windows-style output location do not separate folders,
        // so its "folder" is the parent of the root, which must hold nothing but this test's files (not the shared /tmp).
        let root = std::env::temp_dir()
            .join(format!("debrief-perfmon-test-{}", std::process::id()))
            .join("root");
        let sub = root.join("20261014-000001");
        std::fs::create_dir_all(&sub).unwrap();
        let old = sub.join("a.csv");
        let new = sub.join("b.BLG");
        std::fs::write(&old, "x").unwrap();
        std::fs::write(sub.join("notes.txt"), "x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        std::fs::write(&new, "x").unwrap();
        // Deeper than MAX_DEPTH is ignored.
        let deep = root.join("1/2/3/4/5");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep.csv"), "x").unwrap();
        std::env::set_var("DEBRIEF_TEST_ROOT", &root);
        let s = parse_query("Debrief", QUERY);
        assert_eq!(output_files(&s), [new, old]);
        // The output location alone (a file) falls back to its folder.
        let only = CollectorStatus {
            root_path: None,
            ..s.clone()
        };
        assert_eq!(output_files(&only).len(), 2);
        let none = CollectorStatus {
            root_path: None,
            output_location: None,
            ..s
        };
        assert!(output_files(&none).is_empty());
        let _ = std::fs::remove_dir_all(root.parent().unwrap_or(&root));
        assert_eq!(
            expand_env("%DEBRIEF_TEST_ROOT_UNSET%\\x 100% %"),
            "%DEBRIEF_TEST_ROOT_UNSET%\\x 100% %"
        );
        assert_eq!(expand_env("a%%b"), "a%%b");
    }
}
