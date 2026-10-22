//! Watched folders (ADR 0048): decide which files that appear in a folder are ready to import.
//!
//! The decision is a pure state machine ([`WatchState`]) fed with snapshots of the folder ([`scan_folder`] makes
//! them), so every awkward case is a unit test: a file still being written (its size or time keeps changing, or it
//! has a temporary name), an empty file, a file that vanished before it was read, one that is locked (retried with
//! backoff, then given up), and one too large to import automatically. Nothing here imports; the caller does that and
//! reports back with [`WatchState::imported`] or [`WatchState::failed`].

use crate::known_issues::{IssueMatch, IssueReport};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Quiet time a file needs (unchanged size and modification time) before it is imported.
pub const DEFAULT_DEBOUNCE_MS: u64 = 2_000;
/// Largest file imported automatically; bigger ones are reported once and left to the user.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 4 << 30;
/// Import attempts for a file that fails (locked, unreadable) before it is given up.
pub const MAX_ATTEMPTS: u32 = 4;
/// Files remembered as pending at once; more are ignored until some settle.
pub const MAX_PENDING: usize = 10_000;
/// Directory entries a scan reads at most.
pub const MAX_SCAN_ENTRIES: usize = 50_000;
/// Deepest folder a scan enters.
pub const MAX_SCAN_DEPTH: usize = 16;

/// How the watcher decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchConfig {
    pub debounce_ms: u64,
    pub max_file_bytes: u64,
}

impl Default for WatchConfig {
    fn default() -> Self {
        WatchConfig {
            debounce_ms: DEFAULT_DEBOUNCE_MS,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }
}

/// A file as the folder showed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    pub path: PathBuf,
    pub size: u64,
    /// Last modification (epoch ms).
    pub modified_ms: i64,
}

/// Why a file is not imported automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skipped {
    /// Bigger than [`WatchConfig::max_file_bytes`].
    TooLarge { size: u64 },
    /// Failed [`MAX_ATTEMPTS`] times; the last reason.
    GaveUp { reason: String },
}

/// What [`WatchState::observe`] found ready.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ready {
    /// Files not seen before: open them.
    pub new: Vec<PathBuf>,
    /// Files already imported that have grown or changed since: refresh them.
    pub changed: Vec<PathBuf>,
    /// Files reported for the first time as not imported.
    pub skipped: Vec<(PathBuf, Skipped)>,
}

impl Ready {
    pub fn is_empty(&self) -> bool {
        self.new.is_empty() && self.changed.is_empty() && self.skipped.is_empty()
    }
}

#[derive(Debug)]
struct Pending {
    stamp: FileStamp,
    /// Since when the stamp has not changed (ms on the caller's clock).
    stable_since: u64,
    attempts: u32,
    /// Not before this time after a failure.
    retry_at: u64,
    /// Whether it was imported before (a refresh, not an open).
    known: bool,
}

/// What has been imported, what is settling, and what was already reported.
#[derive(Debug, Default)]
pub struct WatchState {
    config: WatchConfig,
    imported: HashMap<PathBuf, (u64, i64)>,
    pending: HashMap<PathBuf, Pending>,
    reported: HashSet<PathBuf>,
    notified: HashMap<String, usize>,
}

/// Whether a name looks like a file that is still being copied or downloaded, or an editor's scratch file.
pub fn is_temporary_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.')
        || name.starts_with("~$")
        || name.ends_with('~')
        || [
            ".tmp",
            ".temp",
            ".part",
            ".partial",
            ".crdownload",
            ".download",
            ".filepart",
            ".swp",
            ".lock",
        ]
        .iter()
        .any(|s| lower.ends_with(s))
}

impl WatchState {
    pub fn new(config: WatchConfig) -> WatchState {
        WatchState {
            config,
            ..WatchState::default()
        }
    }

    /// Starts from files that are already in the store (their sizes and times), so a restart does not read them again.
    pub fn with_imported(mut self, files: impl IntoIterator<Item = FileStamp>) -> WatchState {
        for f in files {
            self.imported.insert(f.path, (f.size, f.modified_ms));
        }
        self
    }

    /// Takes a snapshot of the folder at time `now` (ms) and returns what is ready. Files missing from the snapshot
    /// are forgotten (they vanished); files whose stamp changed start their quiet time again.
    pub fn observe(&mut self, scan: &[FileStamp], now: u64) -> Ready {
        let present: HashSet<&Path> = scan.iter().map(|f| f.path.as_path()).collect();
        self.pending.retain(|p, _| present.contains(p.as_path()));
        self.reported.retain(|p| present.contains(p.as_path()));
        let mut ready = Ready::default();
        for f in scan {
            if f.path.file_name().and_then(|n| n.to_str()).is_none_or(is_temporary_name) || f.size == 0 {
                self.pending.remove(&f.path);
                continue;
            }
            let known = self.imported.get(&f.path).copied();
            if known == Some((f.size, f.modified_ms)) {
                self.pending.remove(&f.path);
                continue;
            }
            if f.size > self.config.max_file_bytes {
                if self.reported.insert(f.path.clone()) {
                    ready.skipped.push((f.path.clone(), Skipped::TooLarge { size: f.size }));
                }
                continue;
            }
            if self.reported.contains(&f.path) {
                // Given up on this very stamp; a changed file is tried again.
                if self.pending.get(&f.path).is_none_or(|p| p.stamp == *f) {
                    continue;
                }
                self.reported.remove(&f.path);
            }
            if !self.pending.contains_key(&f.path) && self.pending.len() >= MAX_PENDING {
                continue;
            }
            let entry = self.pending.entry(f.path.clone()).or_insert_with(|| Pending {
                stamp: f.clone(),
                stable_since: now,
                attempts: 0,
                retry_at: 0,
                known: known.is_some(),
            });
            if entry.stamp != *f {
                entry.stamp = f.clone();
                entry.stable_since = now;
                entry.attempts = 0;
                entry.retry_at = 0;
                entry.known = known.is_some();
            }
            if now.saturating_sub(entry.stable_since) >= self.config.debounce_ms && now >= entry.retry_at {
                if entry.known {
                    ready.changed.push(f.path.clone());
                } else {
                    ready.new.push(f.path.clone());
                }
            }
        }
        ready.new.sort();
        ready.changed.sort();
        ready.skipped.sort_by(|a, b| a.0.cmp(&b.0));
        ready
    }

    /// The file was imported as `stamp` (what was seen when it was read).
    pub fn imported(&mut self, stamp: &FileStamp) {
        self.imported.insert(stamp.path.clone(), (stamp.size, stamp.modified_ms));
        self.pending.remove(&stamp.path);
    }

    /// Reading the file failed (locked, vanished, unreadable). Returns `Some` when it is given up on; otherwise it is
    /// tried again after a growing delay.
    pub fn failed(&mut self, path: &Path, reason: &str, now: u64) -> Option<Skipped> {
        let p = self.pending.get_mut(path)?;
        p.attempts += 1;
        if p.attempts >= MAX_ATTEMPTS {
            self.reported.insert(path.to_path_buf());
            return Some(Skipped::GaveUp {
                reason: reason.to_string(),
            });
        }
        p.retry_at = now + self.config.debounce_ms.saturating_mul(1 << p.attempts);
        None
    }

    /// Files waiting for their quiet time or a retry.
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// The known-issue matches worth telling the user about: issues not reported before, or reported with fewer events.
    pub fn fresh_matches(&mut self, report: &IssueReport) -> Vec<IssueMatch> {
        let mut out = Vec::new();
        for m in &report.matches {
            let seen = self.notified.get(&m.issue.id).copied();
            if seen.is_none_or(|n| m.count > n) {
                self.notified.insert(m.issue.id.clone(), m.count);
                out.push(m.clone());
            }
        }
        out
    }
}

/// The files under `root` that `accept` takes (given the path relative to `root`), without following symbolic links, at
/// most [`MAX_SCAN_ENTRIES`] entries and [`MAX_SCAN_DEPTH`] levels deep. The flag says a limit cut the scan. Unreadable
/// folders and files that vanish during the scan are skipped.
pub fn scan_folder(root: &Path, accept: &dyn Fn(&Path) -> bool) -> (Vec<FileStamp>, bool) {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut entries = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            entries += 1;
            if entries > MAX_SCAN_ENTRIES {
                return (out, true);
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                if depth < MAX_SCAN_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if kind.is_file() {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                if !accept(rel) {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                let modified_ms = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|d| i64::try_from(d.as_millis()).ok())
                    .unwrap_or(0);
                out.push(FileStamp {
                    path,
                    size: meta.len(),
                    modified_ms,
                });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::known_issues::{KnownIssue, Matcher, Origin};

    fn stamp(name: &str, size: u64, mtime: i64) -> FileStamp {
        FileStamp {
            path: PathBuf::from(format!("/w/{name}")),
            size,
            modified_ms: mtime,
        }
    }

    fn state() -> WatchState {
        WatchState::new(WatchConfig {
            debounce_ms: 1_000,
            max_file_bytes: 1_000,
        })
    }

    #[test]
    fn a_file_is_ready_only_after_it_stopped_changing() {
        let mut s = state();
        assert!(
            s.observe(&[stamp("a.log", 10, 1)], 0).is_empty(),
            "first sight starts the quiet time"
        );
        assert!(s.observe(&[stamp("a.log", 10, 1)], 900).is_empty());
        let r = s.observe(&[stamp("a.log", 10, 1)], 1_000);
        assert_eq!(r.new, [PathBuf::from("/w/a.log")]);
    }

    #[test]
    fn a_file_still_being_written_keeps_waiting() {
        let mut s = state();
        s.observe(&[stamp("a.log", 10, 1)], 0);
        assert!(
            s.observe(&[stamp("a.log", 20, 2)], 800).is_empty(),
            "it grew: the quiet time starts again"
        );
        assert!(s.observe(&[stamp("a.log", 20, 2)], 1_700).is_empty());
        assert_eq!(s.observe(&[stamp("a.log", 20, 2)], 1_800).new.len(), 1);
    }

    #[test]
    fn temporary_empty_and_hidden_files_are_ignored() {
        let mut s = state();
        let scan = [
            stamp("a.log.part", 5, 1),
            stamp("b.tmp", 5, 1),
            stamp(".hidden", 5, 1),
            stamp("c.log", 0, 1),
            stamp("~$d.docx", 5, 1),
            stamp("e.log~", 5, 1),
        ];
        s.observe(&scan, 0);
        assert!(s.observe(&scan, 5_000).is_empty());
        assert_eq!(s.pending_len(), 0);
        assert!(is_temporary_name("x.CRDOWNLOAD") && !is_temporary_name("app.log"));
    }

    #[test]
    fn a_vanished_file_is_forgotten() {
        let mut s = state();
        s.observe(&[stamp("a.log", 10, 1)], 0);
        assert_eq!(s.pending_len(), 1);
        assert!(s.observe(&[], 500).is_empty());
        assert_eq!(s.pending_len(), 0);
        // It comes back: the quiet time starts from zero.
        assert!(s.observe(&[stamp("a.log", 10, 1)], 5_000).is_empty());
    }

    #[test]
    fn imported_files_are_not_read_again_until_they_change() {
        let mut s = state();
        let a = stamp("a.log", 10, 1);
        s.observe(std::slice::from_ref(&a), 0);
        assert_eq!(s.observe(std::slice::from_ref(&a), 1_000).new.len(), 1);
        s.imported(&a);
        assert!(s.observe(std::slice::from_ref(&a), 9_000).is_empty());
        let grown = stamp("a.log", 30, 5);
        assert!(s.observe(std::slice::from_ref(&grown), 10_000).is_empty());
        let r = s.observe(std::slice::from_ref(&grown), 11_000);
        assert_eq!(
            (r.new.len(), r.changed.len()),
            (0, 1),
            "a known file that grew is refreshed, not opened"
        );
    }

    #[test]
    fn a_restart_does_not_read_imported_files_again() {
        let a = stamp("a.log", 10, 1);
        let mut s = state().with_imported([a.clone()]);
        s.observe(std::slice::from_ref(&a), 0);
        assert!(s.observe(std::slice::from_ref(&a), 10_000).is_empty());
    }

    #[test]
    fn huge_files_are_reported_once() {
        let mut s = state();
        let big = stamp("big.log", 5_000, 1);
        let r = s.observe(std::slice::from_ref(&big), 0);
        assert_eq!(r.skipped, [(big.path.clone(), Skipped::TooLarge { size: 5_000 })]);
        assert!(s.observe(std::slice::from_ref(&big), 10_000).is_empty(), "reported once");
        assert_eq!(s.pending_len(), 0);
    }

    #[test]
    fn a_locked_file_is_retried_with_backoff_and_then_given_up() {
        let mut s = state();
        let a = stamp("a.log", 10, 1);
        s.observe(std::slice::from_ref(&a), 0);
        assert_eq!(s.observe(std::slice::from_ref(&a), 1_000).new.len(), 1);
        assert_eq!(s.failed(&a.path, "locked", 1_000), None);
        assert!(s.observe(std::slice::from_ref(&a), 2_500).is_empty(), "waiting for the retry time");
        assert_eq!(s.observe(std::slice::from_ref(&a), 3_000).new.len(), 1);
        assert_eq!(s.failed(&a.path, "locked", 3_000), None);
        assert_eq!(s.failed(&a.path, "locked", 8_000), None);
        let gave_up = s.failed(&a.path, "locked", 20_000);
        assert_eq!(gave_up, Some(Skipped::GaveUp { reason: "locked".into() }));
        assert!(s.observe(std::slice::from_ref(&a), 99_000).is_empty(), "given up on this very file");
        // Changing the file is a new chance.
        let fixed = stamp("a.log", 12, 9);
        s.observe(std::slice::from_ref(&fixed), 100_000);
        assert_eq!(s.observe(std::slice::from_ref(&fixed), 101_000).new.len(), 1);
        assert_eq!(s.failed(Path::new("/w/unknown"), "x", 0), None);
    }

    #[test]
    fn the_pending_list_is_bounded() {
        let mut s = state();
        let scan: Vec<FileStamp> = (0..MAX_PENDING + 50).map(|i| stamp(&format!("f{i}.log"), 10, 1)).collect();
        s.observe(&scan, 0);
        assert_eq!(s.pending_len(), MAX_PENDING);
    }

    fn matched(id: &str, count: usize) -> IssueMatch {
        IssueMatch {
            issue: KnownIssue {
                id: id.into(),
                title: id.into(),
                fix: "f".into(),
                matcher: Matcher::Query { ql: "x".into() },
                file: "k.md".into(),
            },
            origin: Origin::Rule,
            query: "x".into(),
            count,
            sample_ids: vec![],
        }
    }

    #[test]
    fn a_known_issue_is_announced_when_new_or_when_it_grows() {
        let mut s = state();
        let report = |ms: Vec<IssueMatch>| IssueReport {
            matches: ms,
            ..IssueReport::default()
        };
        assert_eq!(s.fresh_matches(&report(vec![matched("pool", 3)])).len(), 1);
        assert!(s.fresh_matches(&report(vec![matched("pool", 3)])).is_empty());
        let again = s.fresh_matches(&report(vec![matched("pool", 9), matched("disk", 1)]));
        assert_eq!(again.iter().map(|m| m.issue.id.as_str()).collect::<Vec<_>>(), ["pool", "disk"]);
    }

    #[test]
    fn scanning_filters_skips_links_and_stops_at_the_limits() {
        let dir = std::env::temp_dir().join(format!("imt-watch-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.log"), "x").unwrap();
        std::fs::write(dir.join("sub/b.log"), "y").unwrap();
        std::fs::write(dir.join("c.bin"), "z").unwrap();
        let (files, cut) = scan_folder(&dir, &|rel| rel.extension().is_some_and(|e| e == "log"));
        assert!(!cut);
        let names: Vec<_> = files
            .iter()
            .map(|f| f.path.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(names, ["a.log", "sub/b.log"]);
        assert!(files.iter().all(|f| f.size == 1 && f.modified_ms > 0));
        assert_eq!(
            scan_folder(&dir.join("missing"), &|_| true),
            (Vec::new(), false),
            "a missing folder is empty, not an error"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
