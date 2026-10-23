//! Bounded sizing and confined removal of files and folders. Plain `std::fs`, no Tauri: unit-tested with
//! temporary folders.
//!
//! - Symbolic links (and Windows junctions) are never followed: sizing counts them as nothing, removal
//!   removes the link itself and leaves its target alone.
//! - Sizing stops at an entry budget and a deadline shared by the whole scan; the result says so
//!   (`capped`), and the UI shows the sizes as lower bounds.
//! - Removal only touches paths whose parent resolves inside a category's canonical root
//!   ([`confined`]), and reports what it could not delete (in use, read-only) instead of failing.

use super::model::CategoryClearResult;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Entries one usage scan inspects at most (all categories together).
pub const MAX_ENTRIES: u64 = 500_000;
/// How long one usage scan may take.
pub const MAX_SCAN_TIME: Duration = Duration::from_secs(10);
/// Folder nesting below which nothing is sized or removed.
pub const MAX_DEPTH: usize = 64;

/// What a scan may still inspect.
#[derive(Debug)]
pub struct Budget {
    left: u64,
    deadline: Instant,
}

impl Budget {
    pub fn new(entries: u64, time: Duration) -> Budget {
        Budget {
            left: entries,
            deadline: Instant::now() + time,
        }
    }

    /// The limits of a usage scan ([`MAX_ENTRIES`], [`MAX_SCAN_TIME`]).
    pub fn standard() -> Budget {
        Budget::new(MAX_ENTRIES, MAX_SCAN_TIME)
    }

    /// Spends one entry; false once the budget or the time is used up.
    fn take(&mut self) -> bool {
        if self.left == 0 || Instant::now() >= self.deadline {
            return false;
        }
        self.left -= 1;
        true
    }
}

/// Bytes and files found; `capped` when the scan stopped early (the numbers are then lower bounds).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub bytes: u64,
    pub files: u64,
    pub capped: bool,
}

/// The size of a file or a folder (recursively), without following links. Missing paths are empty.
pub fn size_of(path: &Path, budget: &mut Budget) -> Size {
    let mut size = Size::default();
    if !budget.take() {
        size.capped = true;
        return size;
    }
    let mut stack = vec![(path.to_path_buf(), 0usize)];
    while let Some((p, depth)) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&p) else { continue };
        let ft = meta.file_type();
        if ft.is_symlink() {
            continue;
        }
        if ft.is_file() {
            size.bytes += meta.len();
            size.files += 1;
            continue;
        }
        if !ft.is_dir() {
            continue;
        }
        if depth >= MAX_DEPTH {
            size.capped = true;
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&p) else { continue };
        for entry in entries.flatten() {
            if !budget.take() {
                size.capped = true;
                return size;
            }
            stack.push((entry.path(), depth + 1));
        }
    }
    size
}

/// Removal confined to a root: the shell's one path validator module owns it.
pub use crate::paths::confined;

/// Why something could not be deleted, in words for the UI.
pub fn reason(e: &std::io::Error) -> String {
    // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION: another process (or Debrief itself) has it open.
    if cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)) {
        return "in use by a running program".into();
    }
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        return "in use, read-only or not permitted".into();
    }
    e.to_string()
}

/// Removes a file, a link (not its target) or a folder with everything in it. Whatever cannot be
/// deleted is recorded in `out` and left in place, together with the folders that contain it. Returns
/// whether `path` is gone. Stops early (returning false) once `cancel` is set.
pub fn remove(path: &Path, cancel: &AtomicBool, out: &mut CategoryClearResult) -> bool {
    remove_at(path, cancel, out, 0)
}

fn remove_at(path: &Path, cancel: &AtomicBool, out: &mut CategoryClearResult, depth: usize) -> bool {
    if cancel.load(Ordering::Relaxed) {
        return false;
    }
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
        Err(e) => {
            out.skip(path, reason(&e));
            return false;
        }
    };
    let ft = meta.file_type();
    let gone = |r: std::io::Result<()>, out: &mut CategoryClearResult| match r {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            out.skip(path, reason(&e));
            false
        }
    };
    if ft.is_symlink() {
        // Never what it points to. A link to a folder is a directory entry on Windows (`remove_dir`).
        let r = std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path));
        return gone(r, out);
    }
    if ft.is_dir() {
        if depth >= MAX_DEPTH {
            out.skip(path, "nested too deeply");
            return false;
        }
        let entries = match std::fs::read_dir(path) {
            Ok(it) => it,
            Err(e) => {
                out.skip(path, reason(&e));
                return false;
            }
        };
        let mut all = true;
        for entry in entries {
            match entry {
                Ok(entry) => all &= remove_at(&entry.path(), cancel, out, depth + 1),
                Err(e) => {
                    out.skip(path, reason(&e));
                    all = false;
                }
            }
        }
        // A folder that still holds something it could not delete (or a cancelled one) stays.
        return all && gone(std::fs::remove_dir(path), out);
    }
    let removed = gone(std::fs::remove_file(path), out);
    if removed {
        out.freed_bytes += meta.len();
        out.removed_files += 1;
    }
    removed
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::storage::model::CategoryId;
    use crate::test_support::TempDir;
    use std::path::PathBuf;

    pub fn write(path: &Path, bytes: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    /// Links `link` to the folder `target`: a symbolic link on Unix, a junction-free directory symlink or
    /// (without the right to create one) a junction on Windows. `false` when neither is possible.
    pub fn link_dir(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(target, link).is_ok()
                || debrief_core::process::quiet_command("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(link)
                    .arg(target)
                    .output()
                    .is_ok_and(|o| o.status.success())
        }
    }

    #[test]
    fn sizes_files_and_folders_recursively() {
        let d = TempDir::new("storage-size");
        write(&d.0.join("a.bin"), 10);
        write(&d.0.join("sub/b.bin"), 20);
        write(&d.0.join("sub/deeper/c.bin"), 30);
        let mut budget = Budget::standard();
        assert_eq!(
            size_of(&d.0, &mut budget),
            Size {
                bytes: 60,
                files: 3,
                capped: false
            }
        );
        assert_eq!(size_of(&d.0.join("a.bin"), &mut budget).bytes, 10);
        assert_eq!(size_of(&d.0.join("missing"), &mut budget), Size::default());
    }

    #[test]
    fn sizing_stops_at_the_entry_budget_and_the_deadline_and_says_so() {
        let d = TempDir::new("storage-cap");
        for i in 0..20 {
            write(&d.0.join(format!("f{i}")), 1);
        }
        let s = size_of(&d.0, &mut Budget::new(5, Duration::from_secs(60)));
        assert!(s.capped);
        assert!(s.files < 20);
        let s = size_of(&d.0, &mut Budget::new(1_000, Duration::ZERO));
        assert!(s.capped);
        assert_eq!(s.files, 0);
        // A spent budget caps every later category too.
        let mut b = Budget::new(0, Duration::from_secs(60));
        assert!(size_of(&d.0, &mut b).capped);
    }

    #[test]
    fn sizing_does_not_follow_links() {
        let d = TempDir::new("storage-size-link");
        let outside = d.0.join("outside");
        write(&outside.join("big.bin"), 1000);
        let root = d.0.join("root");
        write(&root.join("small.bin"), 1);
        if !link_dir(&outside, &root.join("link")) {
            eprintln!("cannot create links here; skipped");
            return;
        }
        assert_eq!(size_of(&root, &mut Budget::standard()).bytes, 1);
    }

    #[test]
    fn removal_deletes_everything_and_counts_what_it_freed() {
        let d = TempDir::new("storage-remove");
        write(&d.0.join("x/a.bin"), 5);
        write(&d.0.join("x/y/b.bin"), 7);
        let mut out = CategoryClearResult::new(CategoryId::Logs);
        assert!(remove(&d.0.join("x"), &AtomicBool::new(false), &mut out));
        assert!(!d.0.join("x").exists());
        assert_eq!((out.freed_bytes, out.removed_files, out.skipped_count), (12, 2, 0));
        // Already gone counts as removed.
        assert!(remove(&d.0.join("x"), &AtomicBool::new(false), &mut out));
    }

    #[test]
    fn removal_removes_a_link_but_never_its_target() {
        let d = TempDir::new("storage-remove-link");
        let outside = d.0.join("outside");
        write(&outside.join("keep.bin"), 3);
        let root = d.0.join("root");
        std::fs::create_dir_all(&root).unwrap();
        if !link_dir(&outside, &root.join("link")) {
            eprintln!("cannot create links here; skipped");
            return;
        }
        let mut out = CategoryClearResult::new(CategoryId::WebviewCache);
        assert!(remove(&root, &AtomicBool::new(false), &mut out));
        assert!(!root.exists());
        assert!(outside.join("keep.bin").is_file(), "the link's target must survive");
        assert_eq!(out.freed_bytes, 0);
    }

    #[test]
    fn a_cancelled_removal_stops_and_keeps_the_rest() {
        let d = TempDir::new("storage-cancel");
        write(&d.0.join("x/a.bin"), 5);
        let mut out = CategoryClearResult::new(CategoryId::Logs);
        assert!(!remove(&d.0.join("x"), &AtomicBool::new(true), &mut out));
        assert!(d.0.join("x/a.bin").is_file());
    }

    #[test]
    fn files_in_use_are_skipped_and_reported_with_their_folder_kept() {
        let d = TempDir::new("storage-in-use");
        let busy = d.0.join("x").join("locked").join("busy.bin");
        write(&busy, 4);
        write(&d.0.join("x/free.bin"), 6);
        let Some(_hold) = hold(&busy) else {
            eprintln!("cannot make a file undeletable here (running as root?); skipped");
            return;
        };
        let mut out = CategoryClearResult::new(CategoryId::WebviewCache);
        let gone = remove(&d.0.join("x"), &AtomicBool::new(false), &mut out);
        release(&busy);
        assert!(!gone);
        assert!(busy.is_file(), "the file in use stays");
        assert!(!d.0.join("x/free.bin").exists(), "the rest is removed");
        assert_eq!((out.freed_bytes, out.removed_files), (6, 1));
        assert!(out.skipped_count >= 1);
        assert_eq!(std::path::Path::new(&out.skipped[0].path), busy);
        assert!(out.skipped[0].reason.contains("in use"), "{:?}", out.skipped[0]);
    }

    /// Makes `file` undeletable the way a running program does: on Windows an open handle without
    /// delete sharing; on Unix (where open files can be deleted) a read-only parent folder.
    #[cfg(windows)]
    pub fn hold(file: &Path) -> Option<std::fs::File> {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).share_mode(0).open(file).ok()
    }

    #[cfg(windows)]
    pub fn release(_: &Path) {}

    #[cfg(unix)]
    pub fn hold(file: &Path) -> Option<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = file.parent().unwrap();
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root ignores permissions: then nothing can be held.
        let probe = dir.join(".probe");
        if std::fs::write(&probe, b"").is_ok() {
            let _ = std::fs::remove_file(probe);
            release(file);
            return None;
        }
        Some(())
    }

    #[cfg(unix)]
    pub fn release(file: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(file.parent().unwrap(), std::fs::Permissions::from_mode(0o755));
    }

    #[test]
    fn confinement_refuses_paths_outside_the_root() {
        let d = TempDir::new("storage-confine");
        let root = d.0.join("root");
        write(&root.join("inside.bin"), 1);
        write(&d.0.join("outside.bin"), 1);
        let canon = std::fs::canonicalize(&root).unwrap();
        assert_eq!(confined(&canon, &root.join("inside.bin")).unwrap(), canon.join("inside.bin"));
        for bad in [
            d.0.join("outside.bin"),
            root.join("..").join("outside.bin"),
            root.join("."),
            root.join(".."),
            PathBuf::from("/"),
            root.join("missing-dir").join("x"),
        ] {
            assert!(confined(&canon, &bad).is_err(), "{}", bad.display());
        }
        assert!(d.0.join("outside.bin").is_file());
    }

    #[test]
    fn confinement_resolves_a_linked_parent_and_refuses_it() {
        let d = TempDir::new("storage-confine-link");
        let outside = d.0.join("outside");
        write(&outside.join("victim.bin"), 1);
        let root = d.0.join("root");
        std::fs::create_dir_all(&root).unwrap();
        if !link_dir(&outside, &root.join("link")) {
            eprintln!("cannot create links here; skipped");
            return;
        }
        let canon = std::fs::canonicalize(&root).unwrap();
        assert!(confined(&canon, &root.join("link").join("victim.bin")).is_err());
        // The link itself is inside and may be removed (without touching its target).
        assert!(confined(&canon, &root.join("link")).is_ok());
    }

    #[test]
    fn reasons_name_files_in_use() {
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(reason(&denied).contains("in use"));
        if cfg!(windows) {
            assert_eq!(reason(&std::io::Error::from_raw_os_error(32)), "in use by a running program");
        }
        assert!(!reason(&std::io::Error::other("boom")).is_empty());
    }
}
