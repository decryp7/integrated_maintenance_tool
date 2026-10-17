//! Free disk space behind a trait, so the download's space check is testable.

use std::path::Path;

pub trait DiskSpace: Send + Sync {
    /// Bytes available to the current user on the volume holding `dir`; `None` when unknown.
    fn available_bytes(&self, dir: &Path) -> Option<u64>;
}

pub struct SystemDisk;

impl DiskSpace for SystemDisk {
    fn available_bytes(&self, dir: &Path) -> Option<u64> {
        let dir = dir.canonicalize().ok()?;
        let disks = sysinfo::Disks::new_with_refreshed_list();
        disks
            .list()
            .iter()
            .filter(|d| dir.starts_with(d.mount_point()))
            .max_by_key(|d| d.mount_point().as_os_str().len())
            .map(|d| d.available_space())
    }
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
}
