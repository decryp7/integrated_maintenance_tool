//! The symbol cache's size bound: the least recently used PDBs are removed when the cache grows past what the user
//! allows. The cache is in symstore layout (`<pdb>/<key>/<pdb>`); a PDB's modification time is its last use
//! ([`touch`] on every hit). Leftover temporary files of interrupted downloads go first.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Most files looked at in one pass (a cache this large is pruned in several passes).
pub const MAX_FILES: usize = 100_000;

/// What [`prune`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pruned {
    pub removed: usize,
    /// Bytes left in the cache.
    pub kept_bytes: u64,
}

/// Marks a cached PDB as just used (best effort: a read-only cache is pruned by age instead).
pub fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// Removes the least recently used PDBs from `cache` until it holds at most `max_bytes`, and the temporary files of
/// interrupted downloads. Folders emptied on the way are removed. Errors on single files are skipped.
pub fn prune(cache: &Path, max_bytes: u64) -> Pruned {
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    let mut out = Pruned::default();
    'walk: for name in entries(cache).into_iter().filter(|p| p.is_dir()) {
        for key in entries(&name).into_iter().filter(|p| p.is_dir()) {
            for file in entries(&key) {
                if files.len() >= MAX_FILES {
                    break 'walk;
                }
                let Ok(meta) = std::fs::symlink_metadata(&file) else { continue };
                if !meta.is_file() {
                    continue;
                }
                let partial = file.file_name().is_some_and(|n| n.to_string_lossy().contains(".part-"));
                let stale = meta
                    .modified()
                    .ok()
                    .and_then(|m| m.elapsed().ok())
                    .is_some_and(|d| d.as_secs() > 3600);
                if partial {
                    // A download in progress is younger than an hour (its timeout is far shorter).
                    if stale && std::fs::remove_file(&file).is_ok() {
                        out.removed += 1;
                    }
                    continue;
                }
                files.push((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), meta.len(), file));
            }
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    let mut kept = 0u64;
    for (_, len, path) in files {
        if kept.saturating_add(len) <= max_bytes {
            kept += len;
        } else if std::fs::remove_file(&path).is_ok() {
            out.removed += 1;
            for dir in path.ancestors().skip(1).take(2) {
                // Only empty folders go; a folder with files stays.
                if std::fs::remove_dir(dir).is_err() {
                    break;
                }
            }
        } else {
            kept += len;
        }
    }
    out.kept_bytes = kept;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn put(cache: &Path, name: &str, len: usize, age_s: u64) -> PathBuf {
        let p = cache.join(name).join("KEY1").join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, vec![0u8; len]).unwrap();
        let f = std::fs::File::options().write(true).open(&p).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(age_s)).unwrap();
        p
    }

    #[test]
    fn the_least_recently_used_pdbs_go_first() {
        let dir = tempfile::tempdir().unwrap();
        let c = dir.path();
        let old = put(c, "old.pdb", 400, 300);
        let mid = put(c, "mid.pdb", 400, 200);
        let new = put(c, "new.pdb", 400, 100);
        // Used just now: it becomes the newest.
        touch(&old);
        let r = prune(c, 900);
        assert_eq!(
            r,
            Pruned {
                removed: 1,
                kept_bytes: 800
            }
        );
        assert!(old.is_file() && new.is_file() && !mid.exists());
        assert!(!c.join("mid.pdb").exists(), "its emptied folders are removed");
        assert_eq!(prune(c, 900).removed, 0);
        assert_eq!(prune(c, 0), Pruned { removed: 2, kept_bytes: 0 });
        assert!(entries(c).is_empty());
        // A missing cache is an empty one.
        assert_eq!(prune(&c.join("nope"), 0), Pruned::default());
        touch(&c.join("nope.pdb"));
    }

    #[test]
    fn stale_temporary_files_are_removed_and_fresh_ones_kept() {
        let dir = tempfile::tempdir().unwrap();
        let c = dir.path();
        let stale = put(c, "a.pdb.part-1-0", 10, 7200);
        let fresh = put(c, "b.pdb.part-1-1", 10, 10);
        std::fs::write(c.join("stray.txt"), b"x").unwrap();
        let r = prune(c, 1 << 20);
        assert_eq!(r.removed, 1);
        assert!(!stale.exists() && fresh.exists() && c.join("stray.txt").exists());
    }
}
