//! Choosing which log files to collect: globs, age and size caps.

use crate::plan::LogSpec;
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A file picked for the bundle.
#[derive(Debug, Clone, PartialEq)]
pub struct Selected {
    pub path: PathBuf,
    /// Path relative to the log source (just the file name for a single file).
    pub rel: String,
    pub size: u64,
    pub modified: SystemTime,
}

/// A file or folder that was looked at but not collected.
#[derive(Debug, Clone, PartialEq)]
pub struct Rejected {
    pub path: String,
    pub reason: String,
}

const MAX_DEPTH: usize = 16;

fn globs(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        // Plans are validated; a bad glob can only be skipped here.
        if let Ok(g) = Glob::new(p) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

fn walk(dir: &Path, base: &Path, depth: usize, out: &mut Vec<Selected>, rejected: &mut Vec<Rejected>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            rejected.push(Rejected {
                path: dir.display().to_string(),
                reason: format!("cannot read folder: {e}"),
            });
            return;
        }
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if depth < MAX_DEPTH {
                walk(&path, base, depth + 1, out, rejected);
            }
        } else if ft.is_file() {
            if let Ok(md) = e.metadata() {
                let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                out.push(Selected {
                    path,
                    rel,
                    size: md.len(),
                    modified: md.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                });
            }
        }
    }
}

/// Selects files for one log source. The result is newest first. Symbolic
/// links are not followed.
pub fn select_logs(spec: &LogSpec, now: SystemTime) -> (Vec<Selected>, Vec<Rejected>) {
    let root = Path::new(&spec.path);
    let mut rejected = Vec::new();
    let mut found = Vec::new();
    match std::fs::symlink_metadata(root) {
        Err(_) => {
            rejected.push(Rejected {
                path: spec.path.clone(),
                reason: "path does not exist".into(),
            });
            return (found, rejected);
        }
        Ok(md) if md.is_dir() => walk(root, root, 0, &mut found, &mut rejected),
        Ok(md) if md.is_file() => {
            let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            found.push(Selected {
                path: root.to_path_buf(),
                rel: name,
                size: md.len(),
                modified: md.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
        Ok(_) => {
            rejected.push(Rejected {
                path: spec.path.clone(),
                reason: "not a regular file or folder".into(),
            });
            return (found, rejected);
        }
    }

    let set = globs(&spec.include);
    let cutoff = spec.max_age_days.map(|d| {
        now.checked_sub(Duration::from_secs(u64::from(d) * 86_400))
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    let mut kept = Vec::new();
    for f in found {
        let name = f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !spec.include.is_empty() && !set.is_match(&f.rel) && !set.is_match(&name) {
            continue; // not interesting, not worth a note per file
        }
        if cutoff.is_some_and(|c| f.modified < c) {
            rejected.push(Rejected {
                path: f.path.display().to_string(),
                reason: format!("older than {} days", spec.max_age_days.unwrap_or(0)),
            });
            continue;
        }
        kept.push(f);
    }
    kept.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.rel.cmp(&b.rel)));

    if let Some(mb) = spec.max_total_mb {
        let cap = mb.saturating_mul(1 << 20);
        let mut total = 0u64;
        let mut within = Vec::new();
        for f in kept {
            if total.saturating_add(f.size) > cap {
                rejected.push(Rejected {
                    path: f.path.display().to_string(),
                    reason: format!("over the {mb} MB limit for this folder"),
                });
            } else {
                total += f.size;
                within.push(f);
            }
        }
        kept = within;
    }
    (kept, rejected)
}
