//! Housekeeping for the store folders (`<data>/stores/<window>-<pid>-<n>`): what each one weighs, when it was last
//! used and whether anything still needs it, and removing the ones nothing needs.
//!
//! A folder is
//! - `open`: a window has it open (never removed),
//! - `session`: an autosaved session reopens it after a crash (kept, so the session can be restored),
//! - `running`: another Debrief process (its pid is in the name) is still using it,
//! - `orphaned`: nothing references it; typically the extraction of a project that was closed, or a store a busy
//!   file system could not remove when its window closed.
//!
//! Only orphaned folders are ever removed, and the state is decided again at the moment of removal, so a store opened
//! (or a process started) since the list was shown is skipped with a reason. Ids are folder names, checked to be one
//! plain path component of the stores folder; links are never followed ([`super::walk`]). A missing stores folder is an
//! empty list, a locked file is a skipped item with its reason.

use super::model::{CategoryClearResult, CategoryId};
use super::walk::{self, Budget};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

/// Why a store folder is kept or may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StoreState {
    Open,
    Session,
    Running,
    Orphaned,
}

/// One store folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreFolder {
    /// The folder's name (what `storage_stores_clear` takes).
    pub id: String,
    pub bytes: u64,
    pub files: u64,
    /// When something in it last changed (ms since the epoch); `None` when the file system does not say.
    pub last_used_ms: Option<i64>,
    pub state: StoreState,
}

/// Every store folder, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreFolders {
    pub folders: Vec<StoreFolder>,
    pub total_bytes: u64,
    /// What removing every orphaned folder would free.
    pub orphaned_bytes: u64,
    /// The scan stopped at its limits: sizes are lower bounds.
    pub capped: bool,
}

/// A folder that stayed, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreSkipped {
    pub id: String,
    pub reason: String,
}

/// What removing store folders did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreClearResult {
    pub freed_bytes: u64,
    pub removed: Vec<String>,
    pub skipped: Vec<StoreSkipped>,
    pub cancelled: bool,
}

/// The automatic clean-up: orphaned store folders unused for this many days are removed when Debrief starts. 0 = off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoragePolicy {
    pub auto_clean_days: u32,
}

/// Longest automatic clean-up age accepted (a year).
pub const MAX_AUTO_CLEAN_DAYS: u32 = 365;

impl StoragePolicy {
    /// Refuses an age past [`MAX_AUTO_CLEAN_DAYS`] with what to enter.
    pub fn checked(self) -> Result<StoragePolicy, String> {
        if self.auto_clean_days > MAX_AUTO_CLEAN_DAYS {
            return Err(format!(
                "the automatic clean-up age is at most {MAX_AUTO_CLEAN_DAYS} days; use 0 to switch it off"
            ));
        }
        Ok(self)
    }
}

/// What decides whether a folder is needed.
pub struct Needs<'a> {
    /// The stores folder.
    pub root: &'a Path,
    /// Store folders windows have open.
    pub open: &'a [PathBuf],
    /// Store folders autosaved sessions reopen.
    pub kept: &'a [PathBuf],
    /// Whether a Debrief process with this id is running.
    pub alive: &'a dyn Fn(u32) -> bool,
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn state_of(needs: &Needs, dir: &Path) -> StoreState {
    let dir = canonical(dir);
    let same = |list: &[PathBuf]| list.iter().any(|p| canonical(p) == dir);
    if same(needs.open) {
        StoreState::Open
    } else if same(needs.kept) {
        StoreState::Session
    } else if crate::engine::scratch_pid(&dir).is_some_and(|pid| (needs.alive)(pid)) {
        StoreState::Running
    } else {
        StoreState::Orphaned
    }
}

/// The newest change among the folder and its direct entries (cheap: one level), in ms.
fn last_used(dir: &Path) -> Option<i64> {
    let mut newest = std::fs::metadata(dir).and_then(|m| m.modified()).ok();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for m in entries.flatten().filter_map(|e| e.metadata().ok()) {
            if let Ok(t) = m.modified() {
                newest = Some(newest.map_or(t, |n| n.max(t)));
            }
        }
    }
    newest
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
}

/// The store folders under `needs.root` (real folders only; files and links in it are not stores).
pub fn list(needs: &Needs, budget: &mut Budget) -> StoreFolders {
    let mut out = StoreFolders::default();
    let Ok(entries) = std::fs::read_dir(needs.root) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        let is_dir = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir());
        let Some(id) = e.file_name().to_str().map(String::from) else {
            continue;
        };
        if !is_dir {
            continue;
        }
        let size = walk::size_of(&path, budget);
        let folder = StoreFolder {
            id,
            bytes: size.bytes,
            files: size.files,
            last_used_ms: last_used(&path),
            state: state_of(needs, &path),
        };
        out.capped |= size.capped;
        out.total_bytes += folder.bytes;
        if folder.state == StoreState::Orphaned {
            out.orphaned_bytes += folder.bytes;
        }
        out.folders.push(folder);
    }
    out.folders
        .sort_by(|a, b| b.last_used_ms.cmp(&a.last_used_ms).then_with(|| a.id.cmp(&b.id)));
    out
}

/// `id` as a path under the stores folder, if it is one plain folder name.
fn folder_of(root: &Path, id: &str) -> Option<PathBuf> {
    let mut parts = Path::new(id).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(name)), None) if name == std::ffi::OsStr::new(id) => Some(root.join(id)),
        _ => None,
    }
}

fn why(state: StoreState) -> &'static str {
    match state {
        StoreState::Open => "a window has this store open",
        StoreState::Session => "a saved session reopens this store; restore or discard the session first",
        StoreState::Running => "another Debrief process is using this store",
        StoreState::Orphaned => "",
    }
}

/// Removes the orphaned folders among `ids`; every other id stays with the reason.
pub fn clear(needs: &Needs, ids: &[String], cancel: &AtomicBool) -> StoreClearResult {
    clear_if(needs, ids, cancel, |_| true)
}

fn clear_if(needs: &Needs, ids: &[String], cancel: &AtomicBool, wanted: impl Fn(&Path) -> bool) -> StoreClearResult {
    let mut result = StoreClearResult::default();
    let skip = |result: &mut StoreClearResult, id: &str, reason: String| {
        result.skipped.push(StoreSkipped {
            id: id.to_string(),
            reason,
        });
    };
    for id in ids {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            result.cancelled = true;
            break;
        }
        let Some(dir) = folder_of(needs.root, id) else {
            skip(&mut result, id, "not a store folder name".into());
            continue;
        };
        match std::fs::symlink_metadata(&dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                skip(&mut result, id, "already gone".into());
                continue;
            }
            Ok(m) if m.is_dir() => {}
            _ => {
                skip(&mut result, id, "not a store folder".into());
                continue;
            }
        }
        let state = state_of(needs, &dir);
        if state != StoreState::Orphaned {
            skip(&mut result, id, why(state).into());
            continue;
        }
        if !wanted(&dir) {
            continue;
        }
        let before = walk::size_of(&dir, &mut Budget::standard()).bytes;
        let mut part = CategoryClearResult::new(CategoryId::Stores);
        if walk::remove(&dir, cancel, &mut part) {
            result.freed_bytes += before;
            result.removed.push(id.clone());
        } else {
            let reason = part
                .skipped
                .first()
                .map_or_else(|| "could not be removed".to_string(), |s| s.reason.clone());
            skip(&mut result, id, reason);
        }
    }
    result
}

/// Removes orphaned folders not used for `days` days (the automatic policy, run at start-up).
pub fn clear_unused_for(needs: &Needs, days: u32, now_ms: i64, cancel: &AtomicBool) -> StoreClearResult {
    let ids: Vec<String> = std::fs::read_dir(needs.root)
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    let limit = i64::from(days) * 86_400_000;
    clear_if(needs, &ids, cancel, |dir| {
        last_used(dir).is_some_and(|t| now_ms.saturating_sub(t) >= limit)
    })
    .without_skips()
}

impl StoreClearResult {
    /// The automatic run reports only what it removed and what it could not remove, not the folders still needed.
    fn without_skips(mut self) -> StoreClearResult {
        self.skipped
            .retain(|s| !s.reason.starts_with("a window") && !s.reason.starts_with("a saved") && !s.reason.starts_with("another"));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::walk::tests::write;
    use crate::test_support::TempDir;

    fn no_one(_: u32) -> bool {
        false
    }

    fn needs<'a>(root: &'a Path, open: &'a [PathBuf], kept: &'a [PathBuf], alive: &'a dyn Fn(u32) -> bool) -> Needs<'a> {
        Needs { root, open, kept, alive }
    }

    fn make(root: &Path, name: &str, bytes: usize) -> PathBuf {
        let dir = root.join(name);
        write(&dir.join("data.duckdb"), bytes);
        dir
    }

    #[test]
    fn folders_are_sized_classified_and_listed_newest_first() {
        let d = TempDir::new("stores-list");
        let open = make(&d.0, "main-1-1", 100);
        let kept = make(&d.0, "w-2-1", 200);
        make(&d.0, "w-3-4", 300);
        make(&d.0, "w-777-1", 400);
        write(&d.0.join("stray.txt"), 5);
        let (open_l, kept_l) = ([open], [kept]);
        let alive = |pid: u32| pid == 777;
        let l = list(&needs(&d.0, &open_l, &kept_l, &alive), &mut Budget::standard());
        let by = |id: &str| l.folders.iter().find(|f| f.id == id).unwrap();
        assert_eq!(l.folders.len(), 4, "the stray file is not a store");
        assert_eq!((by("main-1-1").state, by("main-1-1").bytes), (StoreState::Open, 100));
        assert_eq!(by("w-2-1").state, StoreState::Session);
        assert_eq!(by("w-3-4").state, StoreState::Orphaned);
        assert_eq!(by("w-777-1").state, StoreState::Running);
        assert_eq!((l.total_bytes, l.orphaned_bytes), (1000, 300));
        assert!(by("w-3-4").last_used_ms.is_some());
        assert!(!l.capped);
    }

    #[test]
    fn a_missing_stores_folder_is_an_empty_list_and_nothing_to_clear() {
        let d = TempDir::new("stores-missing");
        let root = d.0.join("nope");
        let l = list(&needs(&root, &[], &[], &no_one), &mut Budget::standard());
        assert_eq!(l, StoreFolders::default());
        let r = clear(&needs(&root, &[], &[], &no_one), &["w-1-1".into()], &AtomicBool::new(false));
        assert_eq!((r.removed.len(), r.skipped[0].reason.as_str()), (0, "already gone"));
    }

    #[test]
    fn only_orphaned_folders_are_removed_and_the_others_say_why() {
        let d = TempDir::new("stores-clear");
        let open = make(&d.0, "main-1-1", 10);
        let kept = make(&d.0, "w-2-1", 10);
        let gone = make(&d.0, "w-3-4", 50);
        make(&d.0, "w-777-1", 10);
        let (open_l, kept_l) = ([open.clone()], [kept.clone()]);
        let alive = |pid: u32| pid == 777;
        let ids: Vec<String> = ["main-1-1", "w-2-1", "w-3-4", "w-777-1", "../escape", "a/b", "", ".."]
            .map(String::from)
            .to_vec();
        let r = clear(&needs(&d.0, &open_l, &kept_l, &alive), &ids, &AtomicBool::new(false));
        assert_eq!(r.removed, ["w-3-4"]);
        assert_eq!(r.freed_bytes, 50);
        assert!(!gone.exists() && open.exists() && kept.exists() && d.0.join("w-777-1").exists());
        let reasons: std::collections::HashMap<&str, &str> = r.skipped.iter().map(|s| (s.id.as_str(), s.reason.as_str())).collect();
        assert!(reasons["main-1-1"].contains("open"));
        assert!(reasons["w-2-1"].contains("session"));
        assert!(reasons["w-777-1"].contains("another Debrief"));
        for bad in ["../escape", "a/b", "", ".."] {
            assert_eq!(reasons[bad], "not a store folder name", "{bad:?}");
        }
    }

    #[test]
    fn a_store_opened_after_the_list_was_shown_is_not_removed() {
        let d = TempDir::new("stores-race");
        let dir = make(&d.0, "w-3-4", 10);
        let shown = list(&needs(&d.0, &[], &[], &no_one), &mut Budget::standard());
        assert_eq!(shown.folders[0].state, StoreState::Orphaned);
        // A window opens it before the user confirms.
        let open = [dir.clone()];
        let r = clear(&needs(&d.0, &open, &[], &no_one), &["w-3-4".into()], &AtomicBool::new(false));
        assert!(r.removed.is_empty() && dir.exists());
    }

    #[test]
    fn a_plain_file_or_a_link_is_not_a_store_and_a_link_target_stays() {
        let d = TempDir::new("stores-links");
        let target = d.0.join("elsewhere");
        write(&target.join("keep.txt"), 5);
        let root = d.0.join("stores");
        std::fs::create_dir_all(&root).unwrap();
        write(&root.join("file.txt"), 5);
        if walk::tests::link_dir(&target, &root.join("w-9-9")) {
            let r = clear(
                &needs(&root, &[], &[], &no_one),
                &["w-9-9".into(), "file.txt".into()],
                &AtomicBool::new(false),
            );
            assert!(r.removed.is_empty());
            assert!(target.join("keep.txt").exists());
            assert!(list(&needs(&root, &[], &[], &no_one), &mut Budget::standard()).folders.is_empty());
        }
    }

    #[test]
    fn a_locked_folder_stays_with_its_reason_and_the_others_still_go() {
        let d = TempDir::new("stores-locked");
        let locked = make(&d.0, "w-3-1", 10);
        let free = make(&d.0, "w-3-2", 10);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // A folder that cannot be modified: its entries cannot be deleted.
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
            let r = clear(
                &needs(&d.0, &[], &[], &no_one),
                &["w-3-1".into(), "w-3-2".into()],
                &AtomicBool::new(false),
            );
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
            // Root may delete anyway (CI containers): the contract is that nothing panics and the free one goes.
            assert!(r.removed.contains(&"w-3-2".to_string()));
            if locked.exists() {
                assert_eq!(r.skipped.len(), 1);
                assert!(!r.skipped[0].reason.is_empty());
            }
            assert!(!free.exists());
        }
        #[cfg(not(unix))]
        let _ = (locked, free);
    }

    #[test]
    fn cancelling_stops_before_the_next_folder() {
        let d = TempDir::new("stores-cancel");
        make(&d.0, "w-3-1", 10);
        let r = clear(&needs(&d.0, &[], &[], &no_one), &["w-3-1".into()], &AtomicBool::new(true));
        assert!(r.cancelled && r.removed.is_empty());
        assert!(d.0.join("w-3-1").exists());
    }

    #[test]
    fn the_automatic_policy_removes_only_old_orphans() {
        let d = TempDir::new("stores-auto");
        make(&d.0, "w-3-1", 10);
        let kept = make(&d.0, "w-4-1", 10);
        let kept_l = [kept];
        let day = 86_400_000i64;
        let now = last_used(&d.0.join("w-3-1")).unwrap();
        // Two days later, a 7-day policy keeps everything; a 1-day policy removes the orphan, never the session's.
        let young = clear_unused_for(&needs(&d.0, &[], &kept_l, &no_one), 7, now + 2 * day, &AtomicBool::new(false));
        assert!(young.removed.is_empty() && young.skipped.is_empty());
        let old = clear_unused_for(&needs(&d.0, &[], &kept_l, &no_one), 1, now + 2 * day, &AtomicBool::new(false));
        assert_eq!(old.removed, ["w-3-1"]);
        assert!(d.0.join("w-4-1").exists());
        assert!(old.skipped.is_empty(), "folders still needed are not reported by the automatic run");
    }

    #[test]
    fn the_policy_is_bounded() {
        assert!(StoragePolicy { auto_clean_days: 30 }.checked().is_ok());
        assert!(StoragePolicy { auto_clean_days: 0 }.checked().is_ok());
        assert!(StoragePolicy { auto_clean_days: 366 }.checked().unwrap_err().contains("365"));
    }
}
