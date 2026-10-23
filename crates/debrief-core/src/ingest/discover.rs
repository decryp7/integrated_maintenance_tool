//! Finding the files of an import: a parallel walk of the folders that reports what it has found as it goes.
//!
//! The walk lists many folders at once (listing and `stat` wait on the disk or the network, so more walkers
//! than cores help), counts files and bytes into the [`ImportProgress`] so the UI shows a growing number from
//! the first moments, and stops at the next folder after a cancel. The result is sorted and de-duplicated, so
//! it is the same list whatever the threads did: source ids follow this order (ADR 0014), which is why the
//! import needs the whole list before it reads the first file.

use crate::profile::CompiledProfile;
use crate::workspace::ImportProgress;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Entries of one folder counted into the progress at once, so a single huge folder still shows a growing number.
const FLUSH_EVERY: usize = 256;

struct Walk<'a> {
    root: &'a Path,
    profile: &'a CompiledProfile,
    removed: &'a HashSet<PathBuf>,
    known: &'a HashSet<String>,
    progress: &'a ImportProgress,
    out: Mutex<Vec<PathBuf>>,
}

impl Walk<'_> {
    fn add(&self, files: &mut Vec<PathBuf>, bytes: &mut u64, dir: &Path) {
        if files.is_empty() {
            return;
        }
        self.progress.found(files.len(), std::mem::take(bytes), dir);
        self.out.lock().unwrap_or_else(|p| p.into_inner()).append(files);
    }

    fn folder<'s>(&'s self, dir: PathBuf, scope: &rayon::Scope<'s>) {
        if self.progress.is_cancelled() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        let (mut files, mut bytes) = (Vec::new(), 0u64);
        for entry in entries.flatten() {
            let p = entry.path();
            let rel = p.strip_prefix(self.root).unwrap_or(&p);
            if self.profile.exclude.is_match(rel) || self.removed.contains(&p) {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => scope.spawn(move |s| self.folder(p, s)),
                Ok(t) if t.is_file() && self.profile.include.is_match(rel) && !self.known.contains(p.to_string_lossy().as_ref()) => {
                    bytes += entry.metadata().map_or(0, |m| m.len());
                    files.push(p);
                }
                _ => {}
            }
            if files.len() >= FLUSH_EVERY {
                self.add(&mut files, &mut bytes, &dir);
                if self.progress.is_cancelled() {
                    return;
                }
            }
        }
        self.add(&mut files, &mut bytes, &dir);
    }
}

/// The files under `paths` (a file counts itself) that the profile's include/exclude globs select (relative to
/// the folder), minus `removed` and `known` ones, sorted and without duplicates. Counts go to `progress` while
/// walking on `threads` walkers; after a cancel the list holds what was found until then.
pub fn discover(
    paths: &[PathBuf],
    profile: &CompiledProfile,
    removed: &HashSet<PathBuf>,
    known: &HashSet<String>,
    progress: &ImportProgress,
    threads: usize,
) -> Vec<PathBuf> {
    let mut all = Vec::new();
    for root in paths {
        if root.is_file() {
            let size = std::fs::metadata(root).map_or(0, |m| m.len());
            if !known.contains(root.to_string_lossy().as_ref()) {
                progress.found(1, size, root.parent().unwrap_or(root));
                all.push(root.clone());
            }
            continue;
        }
        let walk = Walk {
            root,
            profile,
            removed,
            known,
            progress,
            out: Mutex::new(Vec::new()),
        };
        let run = || rayon::scope(|s| walk.folder(root.clone(), s));
        match rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .thread_name(|i| format!("debrief-walk-{i}"))
            .build()
        {
            Ok(pool) => pool.install(run),
            Err(e) => {
                log::warn!("parallel folder walk unavailable ({e}); walking on the shared threads");
                run()
            }
        }
        all.append(&mut walk.out.into_inner().unwrap_or_else(|p| p.into_inner()));
    }
    all.sort();
    all.dedup();
    all
}
