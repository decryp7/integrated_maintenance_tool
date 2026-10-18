//! Free disk space before a run. Uses `sysinfo`'s disk list (already a
//! dependency of the collector and monitor, all platforms, no unsafe code
//! here): the volume holding a path is the mounted disk with the longest
//! mount point that is a prefix of the path.

use std::path::{Component, Path, PathBuf, Prefix};

/// Reports the free space of the volume holding a path.
pub trait DiskProbe: Send + Sync {
    /// Free bytes for `path`, or `None` when unknown.
    fn free_bytes(&self, path: &Path) -> Option<u64>;
}

pub struct SystemDisks;

impl DiskProbe for SystemDisks {
    fn free_bytes(&self, path: &Path) -> Option<u64> {
        let path = existing_ancestor(path)?;
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let mounts: Vec<(PathBuf, u64)> = disks
            .list()
            .iter()
            .map(|d| (d.mount_point().to_path_buf(), d.available_space()))
            .collect();
        volume_free(&path, &mounts)
    }
}

/// The nearest existing ancestor, canonicalised (the output folder may not exist yet).
pub fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|p| std::fs::canonicalize(p).ok())
}

/// Free space of the mount with the longest mount point containing `path`.
/// Verbatim Windows paths (`\\?\C:\…`, what `canonicalize` returns) match plain mount points (`C:\`).
pub fn volume_free(path: &Path, mounts: &[(PathBuf, u64)]) -> Option<u64> {
    let path = without_verbatim(path);
    mounts
        .iter()
        .map(|(m, free)| (without_verbatim(m), *free))
        .filter(|(m, _)| path.starts_with(m))
        .max_by_key(|(m, _)| m.components().count())
        .map(|(_, free)| free)
}

/// `\\?\C:\x` as `C:\x` and `\\?\UNC\server\share\x` as `\\server\share\x`; other paths unchanged.
/// Only Windows paths have a prefix, so this is the identity elsewhere.
fn without_verbatim(path: &Path) -> PathBuf {
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return path.to_path_buf();
    };
    let head = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => format!("{}:", drive as char),
        Prefix::VerbatimUNC(server, share) => format!(r"\\{}\{}", server.to_string_lossy(), share.to_string_lossy()),
        _ => return path.to_path_buf(),
    };
    let mut plain = PathBuf::from(head);
    plain.extend(parts);
    plain
}

/// `Err` with a message telling the user what to do when less than `min` is free.
/// Unknown free space does not block a run.
pub fn check_free(probe: &dyn DiskProbe, path: &Path, min: u64) -> Result<(), String> {
    match probe.free_bytes(path) {
        Some(free) if free < min => Err(format!(
            "only {} MB free on the volume of {} (at least {} MB required by agent.minFreeMb); free some space or lower the limit",
            free >> 20,
            path.display(),
            min >> 20
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Option<u64>);
    impl DiskProbe for Fixed {
        fn free_bytes(&self, _: &Path) -> Option<u64> {
            self.0
        }
    }

    #[test]
    fn longest_mount_point_wins() {
        let mounts = vec![(PathBuf::from("/"), 10), (PathBuf::from("/data"), 99), (PathBuf::from("/data2"), 5)];
        assert_eq!(volume_free(Path::new("/data/runs"), &mounts), Some(99));
        assert_eq!(volume_free(Path::new("/var/x"), &mounts), Some(10));
        assert_eq!(volume_free(Path::new("relative"), &mounts), None);
    }

    #[test]
    fn verbatim_windows_paths_match_plain_mount_points() {
        // Regression: canonicalised (`\\?\C:\…`) paths matched no mount on Windows, so free space was never known.
        let mounts = vec![(PathBuf::from(r"C:\"), 10), (PathBuf::from(r"\\srv\share\"), 20)];
        let (disk, unc) = (Path::new(r"\\?\C:\Users\x"), Path::new(r"\\?\UNC\srv\share\runs"));
        if cfg!(windows) {
            assert_eq!(volume_free(disk, &mounts), Some(10));
            assert_eq!(volume_free(unc, &mounts), Some(20));
            assert_eq!(without_verbatim(disk), PathBuf::from(r"C:\Users\x"));
            assert_eq!(without_verbatim(unc), PathBuf::from(r"\\srv\share\runs"));
        }
        // Other paths, and every path elsewhere, are unchanged.
        for p in [r"\\.\pipe\x", r"C:\x", "/var/x", "relative"] {
            assert_eq!(without_verbatim(Path::new(p)), PathBuf::from(p));
        }
    }

    #[test]
    fn checks_the_minimum() {
        assert!(check_free(&Fixed(Some(2 << 20)), Path::new("/x"), 1 << 20).is_ok());
        let e = check_free(&Fixed(Some(1 << 20)), Path::new("/x"), 5 << 20).unwrap_err();
        assert!(e.contains("only 1 MB free") && e.contains("at least 5 MB"), "{e}");
        assert!(check_free(&Fixed(None), Path::new("/x"), u64::MAX).is_ok());
    }

    #[test]
    fn real_disks_report_space_for_missing_subfolders() {
        let dir = tempfile::tempdir().unwrap();
        let free = SystemDisks.free_bytes(&dir.path().join("not/yet/created"));
        assert!(free.is_some_and(|f| f > 0), "{free:?}");
        assert_eq!(
            existing_ancestor(&dir.path().join("a/b")),
            Some(std::fs::canonicalize(dir.path()).unwrap())
        );
    }
}
