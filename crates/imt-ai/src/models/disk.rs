//! Free disk space behind a trait, so the download's space check is testable.

use std::path::{Component, Path, PathBuf, Prefix};

pub trait DiskSpace: Send + Sync {
    /// Bytes available to the current user on the volume holding `dir`; `None` when unknown.
    fn available_bytes(&self, dir: &Path) -> Option<u64>;
}

pub struct SystemDisk;

impl DiskSpace for SystemDisk {
    fn available_bytes(&self, dir: &Path) -> Option<u64> {
        let dir = dir.canonicalize().ok()?;
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let mounts: Vec<(PathBuf, u64)> = disks
            .list()
            .iter()
            .map(|d| (d.mount_point().to_path_buf(), d.available_space()))
            .collect();
        volume_free(&dir, &mounts)
    }
}

/// Free bytes of the most specific mount point holding `path`.
fn volume_free(path: &Path, mounts: &[(PathBuf, u64)]) -> Option<u64> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_space_for_a_real_folder_and_none_for_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SystemDisk.available_bytes(dir.path()).is_some_and(|b| b > 0));
        assert!(SystemDisk.available_bytes(&dir.path().join("missing")).is_none());
    }

    #[test]
    fn verbatim_windows_paths_match_plain_mount_points() {
        // Regression: canonicalised (`\\?\C:\…`) paths matched no mount on Windows, so free space was never known.
        let mounts = vec![
            (PathBuf::from(r"C:\"), 10),
            (PathBuf::from(r"\\srv\share\"), 20),
            (PathBuf::from("/"), 1),
            (PathBuf::from("/data"), 2),
        ];
        if cfg!(windows) {
            assert_eq!(volume_free(Path::new(r"\\?\C:\Users\x"), &mounts), Some(10));
            assert_eq!(volume_free(Path::new(r"\\?\UNC\srv\share\runs"), &mounts), Some(20));
            assert_eq!(without_verbatim(Path::new(r"\\?\C:\Users\x")), PathBuf::from(r"C:\Users\x"));
            assert_eq!(
                without_verbatim(Path::new(r"\\?\UNC\srv\share\runs")),
                PathBuf::from(r"\\srv\share\runs")
            );
        } else {
            // The most specific mount wins.
            assert_eq!(volume_free(Path::new("/data/models"), &mounts), Some(2));
            assert_eq!(volume_free(Path::new("/home/x"), &mounts), Some(1));
        }
        for p in [r"\\.\pipe\x", r"C:\x", "/var/x", "relative"] {
            assert_eq!(without_verbatim(Path::new(p)), PathBuf::from(p));
        }
    }
}
