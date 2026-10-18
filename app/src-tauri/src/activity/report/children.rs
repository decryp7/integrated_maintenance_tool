//! The child processes' part of a report: the tails of their logs (`<logs>/children/*.log`, bounded each
//! and in total) and the `processes.json` summary (live state for the children this session runs, else
//! what their logs say).

use crate::snapshots::tail_of;
use imt_ipc::child_log::{self, ProcessSummary};
use std::path::{Path, PathBuf};

/// Bytes of one child log in a report (its newest part, from a line start).
pub const CHILD_LOG_TAIL: u64 = 256 * 1024;
/// Bytes of all child logs in a report.
pub const CHILD_LOGS_TOTAL: u64 = 2 * 1024 * 1024;
/// Most child log files considered.
pub const MAX_CHILD_FILES: usize = 64;

/// One child log as it goes in the zip.
pub struct ChildFile {
    pub file: String,
    pub data: Vec<u8>,
    pub summary: String,
}

/// The children's logs and their summary.
pub struct Children {
    pub logs: Vec<ChildFile>,
    pub processes: Vec<ProcessSummary>,
}

/// `<name>.log` → (name, 0); `<name>.<n>.log` → (name, n): which session a file is (0 = this one).
fn session_of(file: &str) -> Option<(String, u32)> {
    let stem = file.strip_suffix(".log")?;
    match stem.rsplit_once('.') {
        Some((name, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => Some((name.to_string(), n.parse().ok()?)),
        _ => Some((stem.to_string(), 0)),
    }
}

/// The child log files in `dir`: this session's first (by name), then older sessions'.
fn files(dir: &Path) -> Vec<(PathBuf, String, String, u32)> {
    let mut out: Vec<(PathBuf, String, String, u32)> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter_map(|e| {
                    let file = e.file_name().to_str()?.to_string();
                    let (name, session) = session_of(&file)?;
                    (child_log::log_name(&name) == name).then_some((e.path(), file, name, session))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.3.cmp(&b.3).then_with(|| a.2.cmp(&b.2)));
    out.truncate(MAX_CHILD_FILES);
    out
}

/// Collects the logs in `dir` (none without one) and the processes: `live` (this session's children as
/// supervised now) wins over what a log says.
pub fn collect(dir: Option<&Path>, live: &[ProcessSummary]) -> Children {
    let mut logs = Vec::new();
    let mut processes: Vec<ProcessSummary> = Vec::new();
    let mut left = CHILD_LOGS_TOTAL;
    for (path, file, name, session) in dir.map(files).unwrap_or_default() {
        let data = if left == 0 {
            None
        } else {
            tail_of(&path, CHILD_LOG_TAIL.min(left))
        };
        let Some(data) = data else { continue };
        left = left.saturating_sub(data.len() as u64);
        let summary = if session == 0 {
            let p = live
                .iter()
                .find(|p| p.name == name)
                .cloned()
                // The whole session's file (bounded by the child log's cap), not just the tail: the start lines.
                .or_else(|| {
                    let all = tail_of(&path, 2 * child_log::MAX_FILE_BYTES).unwrap_or_default();
                    child_log::summarize(&name, &String::from_utf8_lossy(&all))
                });
            let text = p
                .as_ref()
                .map(ProcessSummary::describe)
                .unwrap_or_else(|| "Child process log".into());
            processes.extend(p);
            text
        } else {
            format!("{name}: previous session ({session} before this one)")
        };
        logs.push(ChildFile { file, data, summary });
    }
    // Children running now without a log file (no folder yet): their state still counts.
    for p in live {
        if !processes.iter().any(|q| q.name == p.name) {
            processes.push(p.clone());
        }
    }
    processes.sort_by(|a, b| a.name.cmp(&b.name));
    Children { logs, processes }
}

/// "3 child processes, 2 restarts, 1 quarantined".
pub fn processes_summary(processes: &[ProcessSummary]) -> String {
    let restarts: u32 = processes.iter().map(|p| p.restarts).sum();
    let quarantined = processes.iter().filter(|p| p.quarantined).count();
    let n = processes.len();
    format!(
        "{n} child process{}, {restarts} restart{}, {quarantined} quarantined",
        if n == 1 { "" } else { "es" },
        if restarts == 1 { "" } else { "s" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_are_read_from_file_names() {
        assert_eq!(session_of("ai-worker.log"), Some(("ai-worker".into(), 0)));
        assert_eq!(session_of("ai-worker.2.log"), Some(("ai-worker".into(), 2)));
        assert_eq!(
            session_of("plugin-host-net.decryptology.web.log"),
            Some(("plugin-host-net.decryptology.web".into(), 0))
        );
        assert_eq!(
            session_of("plugin-host-net.decryptology.web.1.log"),
            Some(("plugin-host-net.decryptology.web".into(), 1))
        );
        assert_eq!(session_of("x.txt"), None);
    }

    #[test]
    fn summaries_count_restarts_and_quarantines() {
        assert_eq!(processes_summary(&[]), "0 child processes, 0 restarts, 0 quarantined");
        let mut p = child_log::summarize("a", "{\"Event\":\"start\",\"ChildKind\":\"plugin\"}\n{\"Event\":\"start\"}\n").unwrap();
        p.quarantined = true;
        assert_eq!(processes_summary(&[p]), "1 child process, 1 restart, 1 quarantined");
        assert!(collect(None, &[]).logs.is_empty());
    }
}
