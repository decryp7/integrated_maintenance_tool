//! Finding the files of an import, as a stream in sorted order.
//!
//! The walk lists many folders at once (listing and `stat` wait on the disk or the network, so more walkers
//! than cores help): when a folder's listing is consumed, the listings of its next sub-folders are already being
//! made on the walkers. The files are *emitted* in one deterministic order, the sorted order of their paths
//! (a depth-first walk that visits each folder's entries by name, which is exactly how `PathBuf` sorts), whatever
//! the walkers did. That order is what source ids follow (ADR 0014, ADR 0051), so the import can start reading the
//! first file while the walk goes on: no list has to be complete first. [`discover`] collects the stream into the
//! sorted list for callers that want one.
//!
//! Several opened roots stream one after another when none contains another (every file of one root then sorts
//! before every file of the next); roots inside one another are merged and sorted before they are emitted.

use crate::profile::CompiledProfile;
use crate::workspace::ImportProgress;
use rayon::prelude::*;
use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Entries of one folder counted into the progress at once, so a single huge folder still shows a growing number.
const FLUSH_EVERY: usize = 256;

/// Folders listed ahead of the one being emitted (bounds what the walkers hold in memory).
const LOOKAHEAD: usize = 64;

/// Files of one folder probed in parallel above this count.
const PROBE_PARALLEL: usize = 64;

/// A file the walk found, with what the import needs to number its sources before reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub size: u64,
    /// Sources the file can become (see the probe given to [`discover_stream`]).
    pub slots: u32,
}

enum Entry {
    File(Found),
    Dir(PathBuf),
}

/// The listing of one folder, made on a walker and consumed by the emitter.
#[derive(Default)]
struct Slot {
    cell: Mutex<Option<Vec<Entry>>>,
    ready: Condvar,
}

impl Slot {
    fn set(&self, entries: Vec<Entry>) {
        *self.cell.lock().unwrap_or_else(|p| p.into_inner()) = Some(entries);
        self.ready.notify_all();
    }

    /// The listing once it is made; `None` when the import was cancelled meanwhile.
    fn wait(&self, progress: &ImportProgress) -> Option<Vec<Entry>> {
        let mut cell = self.cell.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if let Some(entries) = cell.take() {
                return Some(entries);
            }
            if progress.is_cancelled() {
                return None;
            }
            cell = self
                .ready
                .wait_timeout(cell, Duration::from_millis(25))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}

struct Walk<'a> {
    root: &'a Path,
    profile: &'a CompiledProfile,
    removed: &'a HashSet<PathBuf>,
    known: &'a HashSet<String>,
    probe: &'a (dyn Fn(&Path) -> u32 + Sync),
    progress: &'a ImportProgress,
}

type Emit<'e> = &'e mut (dyn FnMut(Found) -> bool + Send);

impl<'a> Walk<'a> {
    /// The entries of `dir` that the profile selects, sorted by name (files probed on the calling walker).
    fn list(&self, dir: &Path) -> Vec<Entry> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut named: Vec<(OsString, Entry)> = Vec::new();
        let mut files: Vec<(OsString, PathBuf, u64)> = Vec::new();
        for entry in entries.flatten() {
            let p = entry.path();
            let rel = p.strip_prefix(self.root).unwrap_or(&p);
            if self.profile.exclude.is_match(rel) || self.removed.contains(&p) {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => named.push((entry.file_name(), Entry::Dir(p))),
                Ok(t) if t.is_file() && self.profile.include.is_match(rel) && !self.known.contains(p.to_string_lossy().as_ref()) => {
                    files.push((entry.file_name(), p, entry.metadata().map_or(0, |m| m.len())));
                }
                _ => {}
            }
        }
        let probe = |(name, path, size): (OsString, PathBuf, u64)| {
            let slots = (self.probe)(&path);
            (name, Entry::File(Found { path, size, slots }))
        };
        if files.len() > PROBE_PARALLEL {
            named.par_extend(files.into_par_iter().map(probe));
        } else {
            named.extend(files.into_iter().map(probe));
        }
        named.sort_by(|a, b| a.0.cmp(&b.0));
        named.into_iter().map(|(_, e)| e).collect()
    }

    fn spawn_list<'s>(&'s self, scope: &rayon::Scope<'s>, dir: PathBuf) -> Arc<Slot> {
        let slot = Arc::new(Slot::default());
        let mine = slot.clone();
        scope.spawn(move |_| {
            mine.set(if self.progress.is_cancelled() {
                Vec::new()
            } else {
                self.list(&dir)
            });
        });
        slot
    }

    /// Emits the entries of a listed folder in order, descending into sub-folders at their place. `false` once the
    /// consumer stopped or the import was cancelled.
    fn visit<'s>(&'s self, scope: &rayon::Scope<'s>, slot: &Slot, emit: Emit<'_>) -> bool {
        let Some(entries) = slot.wait(self.progress) else { return false };
        let dirs: Vec<PathBuf> = entries
            .iter()
            .filter_map(|e| if let Entry::Dir(d) = e { Some(d.clone()) } else { None })
            .collect();
        let mut next = 0;
        let mut ahead: VecDeque<Arc<Slot>> = VecDeque::new();
        let mut top_up = |ahead: &mut VecDeque<Arc<Slot>>| {
            while ahead.len() < LOOKAHEAD && next < dirs.len() {
                ahead.push_back(self.spawn_list(scope, dirs[next].clone()));
                next += 1;
            }
        };
        top_up(&mut ahead);
        for entry in entries {
            if self.progress.is_cancelled() {
                return false;
            }
            match entry {
                Entry::File(f) => {
                    if !emit(f) {
                        return false;
                    }
                }
                Entry::Dir(_) => {
                    let Some(child) = ahead.pop_front() else { return true };
                    top_up(&mut ahead);
                    if !self.visit(scope, &child, emit) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Whether some root of `roots` (sorted) lies inside another, so their files interleave in sorted order.
fn nested(roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .enumerate()
        .any(|(i, a)| roots.iter().enumerate().any(|(j, b)| i != j && a.starts_with(b)))
}

/// Streams the files under `paths` (a file counts itself) that the profile's include/exclude globs select (relative
/// to the folder), minus `removed` and `known` ones, to `emit` in sorted order and without duplicates, while `threads`
/// walkers list folders ahead of it. `probe` tells how many sources a file can become (run on the walkers). `emit`
/// returns `false` to stop; a cancel in `progress` stops the walk at the next entry.
#[allow(clippy::too_many_arguments)] // the walk's inputs, as for `discover`, plus the probe and the sink
pub fn discover_stream(
    paths: &[PathBuf],
    profile: &CompiledProfile,
    removed: &HashSet<PathBuf>,
    known: &HashSet<String>,
    progress: &ImportProgress,
    threads: usize,
    probe: &(dyn Fn(&Path) -> u32 + Sync),
    emit: Emit<'_>,
) {
    let mut roots: Vec<PathBuf> = paths.to_vec();
    roots.sort();
    roots.dedup();
    if nested(&roots) {
        // Files of one root sort among those of another: gather, sort, then emit.
        let mut all: Vec<Found> = Vec::new();
        for root in &roots {
            stream_root(root, profile, removed, known, progress, threads, probe, &mut |f| {
                all.push(f);
                true
            });
        }
        all.sort_by(|a, b| a.path.cmp(&b.path));
        all.dedup_by(|a, b| a.path == b.path);
        for f in all {
            if progress.is_cancelled() || !emit(f) {
                return;
            }
        }
        return;
    }
    for root in &roots {
        let mut go = true;
        stream_root(root, profile, removed, known, progress, threads, probe, &mut |f| {
            go = emit(f);
            go
        });
        if !go || progress.is_cancelled() {
            return;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn stream_root(
    root: &Path,
    profile: &CompiledProfile,
    removed: &HashSet<PathBuf>,
    known: &HashSet<String>,
    progress: &ImportProgress,
    threads: usize,
    probe: &(dyn Fn(&Path) -> u32 + Sync),
    emit: Emit<'_>,
) {
    if root.is_file() {
        if !known.contains(root.to_string_lossy().as_ref()) {
            let size = std::fs::metadata(root).map_or(0, |m| m.len());
            emit(Found {
                path: root.to_path_buf(),
                size,
                slots: probe(root),
            });
        }
        return;
    }
    let walk = Walk {
        root,
        profile,
        removed,
        known,
        probe,
        progress,
    };
    // One thread more than the walkers: the emitter runs on the pool too and mostly waits for listings.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.max(1) + 1)
        .thread_name(|i| format!("debrief-walk-{i}"))
        .build();
    let run = |emit: Emit<'_>| {
        rayon::scope(|s| {
            let first = walk.spawn_list(s, root.to_path_buf());
            walk.visit(s, &first, emit);
        })
    };
    match pool {
        Ok(pool) => pool.install(|| run(emit)),
        Err(e) => {
            log::warn!("parallel folder walk unavailable ({e}); walking on the shared threads");
            run(emit)
        }
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
    let (mut files, mut bytes) = (0usize, 0u64);
    discover_stream(paths, profile, removed, known, progress, threads, &|_| 1, &mut |f| {
        files += 1;
        bytes += f.size;
        if files >= FLUSH_EVERY {
            progress.found(
                std::mem::take(&mut files),
                std::mem::take(&mut bytes),
                f.path.parent().unwrap_or(&f.path),
            );
        }
        all.push(f.path);
        true
    });
    if files > 0 {
        progress.found(files, bytes, all.last().and_then(|p| p.parent()).unwrap_or(Path::new("")));
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;
    use crate::test_dirs::scratch_dir;

    fn tree() -> PathBuf {
        let dir = std::fs::canonicalize(scratch_dir("discover")).unwrap();
        for f in ["a.log", "sub/b.log", "sub/deep/c.log", "z.log"] {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        dir
    }

    fn all(paths: &[PathBuf]) -> Vec<PathBuf> {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        discover(paths, &p, &HashSet::new(), &HashSet::new(), &ImportProgress::default(), 2)
    }

    #[test]
    fn a_root_inside_another_is_merged_sorted_and_listed_once() {
        let dir = tree();
        let found = all(&[dir.join("sub"), dir.clone()]);
        let want: Vec<PathBuf> = ["a.log", "sub/b.log", "sub/deep/c.log", "z.log"]
            .iter()
            .map(|f| dir.join(f))
            .collect();
        assert_eq!(found, want);
    }

    #[test]
    fn the_consumer_can_stop_merged_roots_after_the_first_file() {
        let dir = tree();
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let mut got = Vec::new();
        discover_stream(
            &[dir.join("sub"), dir.clone()],
            &p,
            &HashSet::new(),
            &HashSet::new(),
            &ImportProgress::default(),
            2,
            &|_| 1,
            &mut |f| {
                got.push(f.path);
                false
            },
        );
        assert_eq!(got, [dir.join("a.log")]);
    }

    #[test]
    fn a_cancelled_import_lists_nothing_from_merged_roots() {
        let dir = tree();
        let progress = ImportProgress::default();
        progress.cancel();
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let found = discover(&[dir.join("sub"), dir], &p, &HashSet::new(), &HashSet::new(), &progress, 2);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn the_consumer_can_stop_the_walk_inside_a_folder() {
        let dir = tree();
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let mut got = Vec::new();
        discover_stream(
            std::slice::from_ref(&dir),
            &p,
            &HashSet::new(),
            &HashSet::new(),
            &ImportProgress::default(),
            2,
            &|_| 1,
            &mut |f| {
                got.push(f.path);
                false
            },
        );
        assert_eq!(got, [dir.join("a.log")]);
    }
}
