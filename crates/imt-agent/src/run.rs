//! Run folders: `<outputDir>/<yyyyMMdd-HHmmss>-<schedule>/` with a versioned
//! `manifest.json` describing why the run happened and what each action
//! produced. The manifest is written when the run starts (status `running`)
//! and again when it ends, atomically, so a crash leaves a readable record.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const RUN_SCHEMA: u32 = 1;
pub const MANIFEST: &str = "manifest.json";
/// Most run folders listed or considered by retention.
pub const MAX_RUNS_SCANNED: usize = 10_000;
/// Deepest folder level counted when sizing a run.
const MAX_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Running,
    /// Every action succeeded.
    Ok,
    /// Some actions failed or were skipped.
    Partial,
    Failed,
    /// Stopped by a shutdown before finishing.
    Stopped,
    /// The folder has no readable manifest (e.g. the agent was killed).
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RunReason {
    /// A cron or interval timer.
    Schedule {
        timer: String,
    },
    Trigger {
        detail: String,
    },
    /// `imt-agent once`.
    Manual,
}

impl RunReason {
    pub fn label(&self) -> String {
        match self {
            RunReason::Schedule { timer } => format!("scheduled ({timer})"),
            RunReason::Trigger { detail } => format!("triggered: {detail}"),
            RunReason::Manual => "manual".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionStatus {
    Ok,
    Failed,
    /// Not applicable here (e.g. PerfMon on macOS) or nothing to do.
    Skipped,
    /// Ended early by a shutdown.
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRecord {
    pub kind: String,
    pub status: ActionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Files and folders produced, relative to the run folder ('/' separated).
    #[serde(default)]
    pub outputs: Vec<String>,
    pub bytes: u64,
    pub ms: u64,
}

impl ActionRecord {
    pub fn new(kind: &str, status: ActionStatus, message: Option<String>) -> ActionRecord {
        ActionRecord {
            kind: kind.to_string(),
            status,
            message,
            outputs: Vec::new(),
            bytes: 0,
            ms: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunManifest {
    pub schema: u32,
    pub id: String,
    pub schedule: String,
    pub reason: RunReason,
    pub host: String,
    pub agent_version: String,
    pub started_at: String,
    pub started_ms: i64,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
    pub status: RunStatus,
    #[serde(default)]
    pub actions: Vec<ActionRecord>,
    #[serde(default)]
    pub total_bytes: u64,
    #[serde(default)]
    pub errors: Vec<String>,
}

impl RunManifest {
    pub fn start(id: &str, schedule: &str, reason: RunReason, now: DateTime<Utc>) -> RunManifest {
        RunManifest {
            schema: RUN_SCHEMA,
            id: id.to_string(),
            schedule: schedule.to_string(),
            reason,
            host: host_name(),
            agent_version: env!("CARGO_PKG_VERSION").into(),
            started_at: now.to_rfc3339_opts(SecondsFormat::Secs, true),
            started_ms: now.timestamp_millis(),
            finished_at: None,
            duration_ms: 0,
            status: RunStatus::Running,
            actions: Vec::new(),
            total_bytes: 0,
            errors: Vec::new(),
        }
    }

    /// Completes the manifest: status from the actions, end time, size.
    pub fn finish(&mut self, now: DateTime<Utc>, stopped: bool, total_bytes: u64) {
        self.finished_at = Some(now.to_rfc3339_opts(SecondsFormat::Secs, true));
        self.duration_ms = (now.timestamp_millis() - self.started_ms).max(0) as u64;
        self.total_bytes = total_bytes;
        self.status = overall(&self.actions, stopped, !self.errors.is_empty());
    }
}

/// The run's status from its actions.
pub fn overall(actions: &[ActionRecord], stopped: bool, errors: bool) -> RunStatus {
    let ok = actions.iter().filter(|a| a.status == ActionStatus::Ok).count();
    if stopped || actions.iter().any(|a| a.status == ActionStatus::Stopped) {
        RunStatus::Stopped
    } else if ok == actions.len() && !errors && ok > 0 {
        RunStatus::Ok
    } else if ok > 0 {
        RunStatus::Partial
    } else {
        RunStatus::Failed
    }
}

fn host_name() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "unknown-host".into())
}

/// Creates a new, uniquely named run folder.
pub fn create_run_dir(output: &Path, schedule: &str, now: DateTime<Utc>) -> std::io::Result<(String, PathBuf)> {
    std::fs::create_dir_all(output)?;
    let stem = format!("{}-{}", now.format("%Y%m%d-%H%M%S"), imt_collect::safe_segment(schedule));
    for n in 0..100 {
        let id = if n == 0 { stem.clone() } else { format!("{stem}-{n}") };
        let dir = output.join(&id);
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok((id, dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other(format!("too many runs named {stem}")))
}

pub fn write_manifest(dir: &Path, m: &RunManifest) -> std::io::Result<()> {
    let tmp = dir.join(format!("{MANIFEST}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(m).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, dir.join(MANIFEST))
}

/// Reads a run's manifest; refuses newer schemas and oversized files.
pub fn read_manifest(dir: &Path) -> Result<RunManifest, String> {
    let path = dir.join(MANIFEST);
    let len = std::fs::metadata(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .len();
    if len > 8 << 20 {
        return Err(format!("{} is too large", path.display()));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let m: RunManifest = serde_json::from_str(&text).map_err(|e| format!("{} is not a run manifest: {e}", path.display()))?;
    if m.schema > RUN_SCHEMA {
        return Err(format!("{} was written by a newer imt-agent (schema {})", path.display(), m.schema));
    }
    Ok(m)
}

/// Total size of the files under `dir` (bounded depth, links not followed).
pub fn dir_size(dir: &Path) -> u64 {
    fn walk(dir: &Path, depth: usize) -> u64 {
        let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
        entries
            .flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() && depth < MAX_DEPTH => walk(&e.path(), depth + 1),
                Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => 0,
            })
            .sum()
    }
    walk(dir, 0)
}

/// One run, as listed in the Runs browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: String,
    pub dir: String,
    pub schedule: String,
    pub status: RunStatus,
    pub reason: String,
    pub started_ms: i64,
    pub duration_ms: u64,
    pub total_bytes: u64,
    pub actions: usize,
    #[serde(default)]
    pub error: Option<String>,
}

/// Whether a folder name looks like a run (`yyyyMMdd-HHmmss-...`).
pub fn is_run_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() > 16 && b[..8].iter().all(u8::is_ascii_digit) && b[8] == b'-' && b[9..15].iter().all(u8::is_ascii_digit) && b[15] == b'-'
}

/// Start time encoded in a run folder name (UTC).
fn started_from_name(name: &str) -> i64 {
    chrono::NaiveDateTime::parse_from_str(&name[..15], "%Y%m%d-%H%M%S")
        .map(|t| t.and_utc().timestamp_millis())
        .unwrap_or(0)
}

/// The runs in `output`, newest first (at most [`MAX_RUNS_SCANNED`]).
pub fn list_runs(output: &Path) -> Vec<RunSummary> {
    let Ok(entries) = std::fs::read_dir(output) else {
        return Vec::new();
    };
    let mut runs: Vec<RunSummary> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            is_run_name(&name).then(|| (name, e.path()))
        })
        .take(MAX_RUNS_SCANNED)
        .map(|(name, dir)| summarise(&name, &dir))
        .collect();
    runs.sort_by(|a, b| b.started_ms.cmp(&a.started_ms).then_with(|| b.id.cmp(&a.id)));
    runs
}

fn summarise(name: &str, dir: &Path) -> RunSummary {
    let dir_s = dir.to_string_lossy().into_owned();
    match read_manifest(dir) {
        Ok(m) => RunSummary {
            id: name.to_string(),
            dir: dir_s,
            schedule: m.schedule,
            status: m.status,
            reason: m.reason.label(),
            started_ms: m.started_ms,
            duration_ms: m.duration_ms,
            total_bytes: if m.status == RunStatus::Running {
                dir_size(dir)
            } else {
                m.total_bytes
            },
            actions: m.actions.len(),
            error: m.errors.first().cloned(),
        },
        Err(e) => RunSummary {
            id: name.to_string(),
            schedule: name.get(16..).unwrap_or_default().to_string(),
            status: RunStatus::Incomplete,
            reason: String::new(),
            started_ms: started_from_name(name),
            duration_ms: 0,
            total_bytes: dir_size(dir),
            actions: 0,
            error: Some(e),
            dir: dir_s,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 17, h, 0, 0).unwrap()
    }

    #[test]
    fn folders_are_unique_and_manifests_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let (id, run) = create_run_dir(dir.path(), "night ly/x", t(2)).unwrap();
        assert_eq!(id, "20261017-020000-night ly_x");
        let (id2, _) = create_run_dir(dir.path(), "night ly/x", t(2)).unwrap();
        assert_eq!(id2, "20261017-020000-night ly_x-1");
        let mut m = RunManifest::start(&id, "night ly/x", RunReason::Manual, t(2));
        write_manifest(&run, &m).unwrap();
        assert_eq!(read_manifest(&run).unwrap().status, RunStatus::Running);
        m.actions.push(ActionRecord::new("collect", ActionStatus::Ok, None));
        m.finish(t(3), false, 42);
        assert_eq!((m.status, m.duration_ms, m.total_bytes), (RunStatus::Ok, 3_600_000, 42));
        write_manifest(&run, &m).unwrap();
        assert_eq!(read_manifest(&run).unwrap(), m);
        assert!(!run.join("manifest.json.tmp").exists());
    }

    #[test]
    fn overall_status() {
        let a = |s| ActionRecord::new("x", s, None);
        assert_eq!(overall(&[a(ActionStatus::Ok)], false, false), RunStatus::Ok);
        assert_eq!(overall(&[a(ActionStatus::Ok)], false, true), RunStatus::Partial);
        assert_eq!(
            overall(&[a(ActionStatus::Ok), a(ActionStatus::Skipped)], false, false),
            RunStatus::Partial
        );
        assert_eq!(overall(&[a(ActionStatus::Failed)], false, false), RunStatus::Failed);
        assert_eq!(overall(&[], false, false), RunStatus::Failed);
        assert_eq!(overall(&[a(ActionStatus::Ok)], true, false), RunStatus::Stopped);
        assert_eq!(overall(&[a(ActionStatus::Stopped)], false, false), RunStatus::Stopped);
    }

    #[test]
    fn bad_manifests() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_manifest(dir.path()).unwrap_err().contains("cannot read"));
        std::fs::write(dir.path().join(MANIFEST), "{}").unwrap();
        assert!(read_manifest(dir.path()).unwrap_err().contains("not a run manifest"));
        let mut m = RunManifest::start("x", "s", RunReason::Manual, t(1));
        m.schema = 9;
        write_manifest(dir.path(), &m).unwrap();
        assert!(read_manifest(dir.path()).unwrap_err().contains("newer imt-agent"));
        std::fs::write(dir.path().join(MANIFEST), vec![b' '; (8 << 20) + 1]).unwrap();
        assert!(read_manifest(dir.path()).unwrap_err().contains("too large"));
        // A run folder cannot be created inside a file.
        let file = dir.path().join("f");
        std::fs::write(&file, "x").unwrap();
        assert!(create_run_dir(&file, "s", t(1)).is_err());
    }

    #[test]
    fn listing_newest_first_with_incomplete_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (a, ra) = create_run_dir(dir.path(), "a", t(1)).unwrap();
        let mut m = RunManifest::start(&a, "a", RunReason::Schedule { timer: "every 1h".into() }, t(1));
        m.actions
            .push(ActionRecord::new("copyLogs", ActionStatus::Failed, Some("boom".into())));
        m.errors.push("disk".into());
        m.finish(t(1), false, 7);
        write_manifest(&ra, &m).unwrap();
        let (_b, rb) = create_run_dir(dir.path(), "b", t(3)).unwrap();
        std::fs::create_dir_all(rb.join("logs/deep")).unwrap();
        std::fs::write(rb.join("logs/deep/x.log"), "12345").unwrap();
        let (c, rc) = create_run_dir(dir.path(), "c", t(2)).unwrap();
        write_manifest(&rc, &RunManifest::start(&c, "c", RunReason::Trigger { detail: "cpu".into() }, t(2))).unwrap();
        std::fs::create_dir(dir.path().join("not-a-run")).unwrap();
        std::fs::write(dir.path().join("20261017-010000-file"), "x").unwrap();

        let runs = list_runs(dir.path());
        assert_eq!(runs.iter().map(|r| r.schedule.as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
        assert_eq!(runs[0].status, RunStatus::Incomplete);
        assert_eq!(runs[0].total_bytes, 5);
        assert!(runs[0].error.as_deref().unwrap().contains("cannot read"));
        assert_eq!(runs[1].status, RunStatus::Running);
        assert_eq!(runs[1].reason, "triggered: cpu");
        assert_eq!(runs[2].status, RunStatus::Failed);
        assert_eq!(runs[2].reason, "scheduled (every 1h)");
        assert_eq!(runs[2].error.as_deref(), Some("disk"));
        assert!(list_runs(&dir.path().join("missing")).is_empty());
        assert_eq!(RunReason::Manual.label(), "manual");
    }

    #[test]
    fn run_names() {
        assert!(is_run_name("20261017-020000-x"));
        assert!(!is_run_name("20261017-020000-"));
        assert!(!is_run_name("2026101x-020000-x"));
        assert!(!is_run_name(".imt-agent"));
        assert_eq!(started_from_name("20261017-020000-x"), t(2).timestamp_millis());
        assert_eq!(started_from_name("99999999-999999-x"), 0);
    }
}
