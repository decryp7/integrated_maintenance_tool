//! Pruning old runs by count, age and total size. Planning is pure; deleting
//! is a thin step that only ever removes run folders (names checked) inside
//! the output folder, never the run that just finished.

use crate::config::RetentionPolicy;
use crate::run::{is_run_name, list_runs, RunStatus};
use std::path::Path;

/// What retention knows about a run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunEntry {
    pub id: String,
    pub started_ms: i64,
    pub bytes: u64,
    /// Running runs (or the current one) are never deleted.
    pub protected: bool,
}

/// Ids to delete, oldest first: runs beyond `max_runs`, runs older than
/// `max_age`, then the oldest until the rest fit in `max_total_bytes`.
pub fn plan(runs: &[RunEntry], policy: &RetentionPolicy, now_ms: i64) -> Vec<String> {
    let mut sorted: Vec<&RunEntry> = runs.iter().collect();
    sorted.sort_by(|a, b| b.started_ms.cmp(&a.started_ms).then_with(|| b.id.cmp(&a.id)));
    let mut doomed = vec![false; sorted.len()];
    if let Some(max) = policy.max_runs {
        for d in doomed.iter_mut().skip(max) {
            *d = true;
        }
    }
    if let Some(age) = policy.max_age {
        let cutoff = now_ms.saturating_sub(age.as_millis() as i64);
        for (i, r) in sorted.iter().enumerate() {
            if r.started_ms < cutoff {
                doomed[i] = true;
            }
        }
    }
    if let Some(cap) = policy.max_total_bytes {
        let mut total: u64 = sorted
            .iter()
            .zip(&doomed)
            .filter(|(r, d)| !**d || r.protected)
            .map(|(r, _)| r.bytes)
            .sum();
        for i in (0..sorted.len()).rev() {
            if total <= cap {
                break;
            }
            if !doomed[i] && !sorted[i].protected {
                doomed[i] = true;
                total = total.saturating_sub(sorted[i].bytes);
            }
        }
    }
    let mut out: Vec<String> = sorted
        .iter()
        .zip(doomed)
        .filter(|(r, d)| *d && !r.protected)
        .map(|(r, _)| r.id.clone())
        .collect();
    out.reverse();
    out
}

/// Applies the policy to `output`, keeping `current`. Returns the deleted ids
/// and the errors met (a folder that cannot be deleted does not stop the rest).
pub fn apply(output: &Path, policy: &RetentionPolicy, now_ms: i64, current: &str) -> (Vec<String>, Vec<String>) {
    let entries: Vec<RunEntry> = list_runs(output)
        .into_iter()
        .map(|r| RunEntry {
            protected: r.id == current || r.status == RunStatus::Running,
            id: r.id,
            started_ms: r.started_ms,
            bytes: r.total_bytes,
        })
        .collect();
    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    for id in plan(&entries, policy, now_ms) {
        if !is_run_name(&id) || id.contains(['/', '\\']) || id.contains("..") {
            continue;
        }
        match std::fs::remove_dir_all(output.join(&id)) {
            Ok(()) => deleted.push(id),
            Err(e) => errors.push(format!("cannot delete run {id}: {e}")),
        }
    }
    (deleted, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{create_run_dir, write_manifest, ActionRecord, ActionStatus, RunManifest, RunReason};
    use chrono::{TimeZone, Utc};
    use std::time::Duration;

    const H: i64 = 3_600_000;

    fn entry(id: &str, hours_ago: i64, bytes: u64) -> RunEntry {
        RunEntry {
            id: id.into(),
            started_ms: 100 * H - hours_ago * H,
            bytes,
            protected: false,
        }
    }

    fn policy(runs: Option<usize>, bytes: Option<u64>, hours: Option<u64>) -> RetentionPolicy {
        RetentionPolicy {
            max_runs: runs,
            max_total_bytes: bytes,
            max_age: hours.map(|h| Duration::from_secs(h * 3600)),
        }
    }

    #[test]
    fn by_count() {
        let runs = [entry("a", 3, 1), entry("b", 2, 1), entry("c", 1, 1), entry("d", 0, 1)];
        assert_eq!(plan(&runs, &policy(Some(2), None, None), 100 * H), ["a", "b"]);
        assert!(plan(&runs, &policy(Some(10), None, None), 100 * H).is_empty());
        assert!(plan(&runs, &RetentionPolicy::default(), 100 * H).is_empty());
    }

    #[test]
    fn by_age() {
        let runs = [entry("old", 50, 1), entry("mid", 10, 1), entry("new", 1, 1)];
        assert_eq!(plan(&runs, &policy(None, None, Some(24)), 100 * H), ["old"]);
    }

    #[test]
    fn by_size_oldest_first_and_protected_runs_kept() {
        let mut runs = vec![entry("a", 4, 50), entry("b", 3, 50), entry("c", 2, 50), entry("d", 1, 50)];
        assert_eq!(plan(&runs, &policy(None, Some(100), None), 100 * H), ["a", "b"]);
        // The oldest is running: it stays and counts, so another goes instead.
        runs[0].protected = true;
        assert_eq!(plan(&runs, &policy(None, Some(100), None), 100 * H), ["b", "c"]);
        // Count and size together.
        runs[0].protected = false;
        assert_eq!(plan(&runs, &policy(Some(3), Some(60), None), 100 * H), ["a", "b", "c"]);
    }

    #[test]
    fn applies_to_disk_and_keeps_the_current_run() {
        let dir = tempfile::tempdir().unwrap();
        let t = |h| Utc.with_ymd_and_hms(2026, 1, 1, h, 0, 0).unwrap();
        let mut ids = Vec::new();
        for h in 0..4 {
            let (id, run) = create_run_dir(dir.path(), "s", t(h)).unwrap();
            let mut m = RunManifest::start(&id, "s", RunReason::Manual, t(h));
            m.actions.push(ActionRecord::new("collect", ActionStatus::Ok, None));
            m.finish(t(h), false, 10);
            write_manifest(&run, &m).unwrap();
            ids.push(id);
        }
        std::fs::create_dir(dir.path().join("keep-me")).unwrap();
        // max 1 run, but the current one is the oldest: it is kept along with the newest.
        let (deleted, errors) = apply(dir.path(), &policy(Some(1), None, None), t(5).timestamp_millis(), &ids[0]);
        assert_eq!(deleted, [ids[1].clone(), ids[2].clone()]);
        assert!(errors.is_empty());
        assert!(dir.path().join(&ids[0]).exists() && dir.path().join(&ids[3]).exists());
        assert!(dir.path().join("keep-me").exists());
    }
}
