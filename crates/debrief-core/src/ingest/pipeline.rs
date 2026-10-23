//! The streaming import (ADR 0050): finding the files and reading them run at the same time.
//!
//! Discovery ([`super::discover_stream`]) walks the folders on a few I/O-bound walkers and emits the files in sorted
//! order into a bounded queue. The dispatcher (this thread) takes them in that order, sets aside the source ids each
//! file needs (so ids follow the sorted order of the paths, exactly as when the whole list was gathered first) and
//! hands the file to a CPU-bound parse pool, keeping at most [`StreamOptions::threads`]` * 2` files in flight. Both
//! ends are bounded: a full queue stops the walk, a full pool stops the dispatcher. A cancel stops both sides at
//! their next entry; files already found but not started are listed as not imported.

use super::discover::{discover_stream, Found};
use super::fingerprint::fingerprint;
use super::parallel::{source_slots, Ordered, SharedSink, Split};
use super::source::{import_path_with, ImportEnv, Outcome};
use crate::api::UnrecognizedFile;
use crate::profile::CompiledProfile;
use crate::workspace::{ImportProgress, NOT_IMPORTED};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::sync_channel;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Files the walk may run ahead of the import (paths only: a few hundred KB at most).
pub const DEFAULT_QUEUE: usize = 8192;

/// What to read: the opened paths and how the profile selects files.
pub struct StreamSpec<'a> {
    pub paths: &'a [PathBuf],
    /// The opened folders, for the display names.
    pub roots: &'a [PathBuf],
    pub profile: &'a CompiledProfile,
    pub removed: &'a HashSet<PathBuf>,
    /// Files already read.
    pub known: &'a HashSet<String>,
}

/// How the streaming import runs.
#[derive(Debug, Clone, Copy)]
pub struct StreamOptions {
    /// Parse threads (CPU bound).
    pub threads: usize,
    /// Folder walkers (I/O bound).
    pub walkers: usize,
    /// Target size of a chunk of a large file.
    pub chunk_target: u64,
    /// Capacity of the queue between the walk and the import.
    pub queue: usize,
    /// Tests: sleeps for a pseudo-random time (derived from this seed and the file) while listing and while
    /// reading each file, so listings and reads finish in another order each seed. Results must not change.
    #[doc(hidden)]
    pub jitter: Option<u64>,
}

/// Sleeps up to 2 ms, derived from `seed` and `key` (see [`StreamOptions::jitter`]).
fn nap(seed: Option<u64>, key: &Path) {
    if let Some(seed) = seed {
        let mut h = seed ^ 0x9e37_79b9_7f4a_7c15;
        for b in key.to_string_lossy().bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        std::thread::sleep(Duration::from_micros(h % 2000));
    }
}

impl Default for StreamOptions {
    fn default() -> Self {
        let threads = super::default_threads();
        StreamOptions {
            threads,
            walkers: (threads * 2).clamp(4, 16),
            chunk_target: super::plan::CHUNK_TARGET,
            queue: DEFAULT_QUEUE,
            jitter: None,
        }
    }
}

/// Counts the files in flight; `acquire` waits for room (or a cancel).
struct Gate {
    busy: Mutex<usize>,
    freed: Condvar,
    limit: usize,
}

impl Gate {
    fn acquire(&self, progress: &ImportProgress) -> bool {
        let mut busy = self.busy.lock().unwrap_or_else(|p| p.into_inner());
        while *busy >= self.limit {
            if progress.is_cancelled() {
                return false;
            }
            busy = self
                .freed
                .wait_timeout(busy, Duration::from_millis(25))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        *busy += 1;
        true
    }

    fn release(&self) {
        *self.busy.lock().unwrap_or_else(|p| p.into_inner()) -= 1;
        self.freed.notify_one();
    }
}

fn not_imported(path: &Path, name: &str) -> Vec<Outcome> {
    vec![Outcome::Unrecognized(UnrecognizedFile {
        path: path.to_string_lossy().into_owned(),
        name: name.to_string(),
        reason: NOT_IMPORTED.into(),
        crashed: false,
    })]
}

struct Ctx<'a> {
    env: &'a ImportEnv<'a>,
    roots: &'a [PathBuf],
    removed: &'a HashSet<PathBuf>,
    progress: &'a ImportProgress,
    sink: &'a dyn SharedSink,
    split: Split<'a>,
    gate: &'a Gate,
    results: &'a Mutex<Vec<(usize, Vec<Outcome>)>>,
    jitter: Option<u64>,
}

type Results = Mutex<Vec<(usize, Vec<Outcome>)>>;

/// Takes the files in order, numbers their sources and hands them to the pool.
fn dispatch<'s>(ctx: &'s Ctx<'s>, scope: Option<&rayon::Scope<'s>>, rx: std::sync::mpsc::Receiver<Found>) {
    let Ctx {
        env,
        progress,
        sink,
        split,
        gate,
        results,
        ..
    } = *ctx;
    for (i, found) in rx.iter().enumerate() {
        let name = crate::workspace::display_name(&found.path, ctx.roots);
        if progress.is_cancelled() || !gate.acquire(progress) {
            progress.file_done(found.size);
            results
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((i, not_imported(&found.path, &name)));
            continue;
        }
        let need = sink.new_slots(&found.path, found.slots);
        let base = if need == 0 {
            None
        } else {
            match sink.reserve_sources(need) {
                Ok(b) => Some(b.0),
                Err(e) => {
                    log::warn!("could not reserve source ids: {e:#}");
                    None
                }
            }
        };
        let (removed, jitter) = (ctx.removed, ctx.jitter);
        let work = move || {
            progress.reading(&found.path);
            nap(jitter, &found.path);
            let (next, end) = base.map_or((0, 0), |b| (b, b + need));
            let out = import_path_with(env, &found.path, &name, removed, &mut Ordered { sink, next, end }, Some(split));
            if found.slots == 1 {
                if let [Outcome::Imported(r)] = out.as_slice() {
                    if r.path == found.path.to_string_lossy() {
                        if let Some(fp) = fingerprint(&found.path) {
                            sink.note_fingerprint(r.id, fp);
                        }
                    }
                }
            }
            progress.file_done(found.size);
            results.lock().unwrap_or_else(|p| p.into_inner()).push((i, out));
            gate.release();
        };
        match scope {
            Some(s) => s.spawn(move |_| work()),
            None => work(),
        }
    }
}

/// Finds the files of `spec` and reads them into `sink` while the walk goes on. Outcomes come back in the sorted
/// order of the files, whatever the order the threads finished in. Counts go to `progress` (see
/// [`ImportProgress::snapshot`]: "finding" until the first file is read, then "importing" of the files found so far
/// with `finding` set until the walk ends). The caller ends the progress (its `begin` guard).
pub fn import_stream(
    env: &ImportEnv,
    spec: &StreamSpec,
    removed_for_read: &HashSet<PathBuf>,
    progress: &ImportProgress,
    opts: &StreamOptions,
    sink: &dyn SharedSink,
) -> Vec<Outcome> {
    progress.stream_begin();
    let threads = opts.threads.max(1);
    let split = Split {
        sink,
        target: opts.chunk_target.max(1),
    };
    let (tx, rx) = sync_channel::<Found>(opts.queue.max(1));
    let results: Results = Mutex::new(Vec::new());
    let gate = Gate {
        busy: Mutex::new(0),
        freed: Condvar::new(),
        limit: threads * 2,
    };
    // One thread more than the parse threads: the dispatcher runs on the pool too and mostly waits.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads + 1)
        .thread_name(|i| format!("debrief-parse-{i}"))
        .build();
    let ctx = Ctx {
        env,
        roots: spec.roots,
        removed: removed_for_read,
        progress,
        sink,
        split,
        gate: &gate,
        results: &results,
        jitter: opts.jitter,
    };
    std::thread::scope(|ts| {
        let walk = ts.spawn(move || {
            let tx = tx;
            discover_stream(
                spec.paths,
                spec.profile,
                spec.removed,
                spec.known,
                progress,
                opts.walkers,
                &|p| {
                    nap(opts.jitter, p);
                    source_slots(p)
                },
                &mut |f| {
                    progress.discovered(1, f.size, f.path.parent().unwrap_or(&f.path));
                    tx.send(f).is_ok()
                },
            );
            // The total is final from here, while the files found last are still being read.
            progress.discovery_done();
        });
        match &pool {
            Ok(pool) => pool.scope(|s| dispatch(&ctx, Some(s), rx)),
            Err(e) => {
                log::warn!("parallel import unavailable ({e}); reading files one by one");
                dispatch(&ctx, None, rx)
            }
        }
        let _ = walk.join();
    });
    let mut results = results.into_inner().unwrap_or_else(|p| p.into_inner());
    results.sort_by_key(|(i, _)| *i);
    results.into_iter().flat_map(|(_, o)| o).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{EventId, EventSchema, PendingBatch, SourceId};
    use crate::importers::Importers;
    use crate::ingest::sink::{NewSource, SourceReport};
    use crate::profile::Profile;
    use crate::workspace::ImportPhase;
    use anyhow::Result;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    /// Records the ids handed out per path; reading a source can be made slow.
    #[derive(Default)]
    struct Sink {
        next_source: AtomicU32,
        next_id: AtomicU64,
        began: Mutex<Vec<(u32, String)>>,
        slow: Duration,
    }

    impl SharedSink for Sink {
        fn reserve_sources(&self, n: u32) -> Result<SourceId> {
            Ok(SourceId(self.next_source.fetch_add(n, Ordering::SeqCst)))
        }
        fn begin_source(&self, s: &NewSource, id: Option<SourceId>) -> Result<SourceId> {
            std::thread::sleep(self.slow);
            let id = id.unwrap_or_else(|| SourceId(self.next_source.fetch_add(1, Ordering::SeqCst)));
            self.began.lock().unwrap().push((id.0, s.path.clone()));
            Ok(id)
        }
        fn write(&self, batch: PendingBatch) -> Result<()> {
            let first = self.next_id.fetch_add(batch.len() as u64, Ordering::SeqCst);
            batch.assign_ids(EventId(first))?;
            Ok(())
        }
        fn finish_source(&self, _: &SourceReport) -> Result<()> {
            Ok(())
        }
        fn abort_source(&self, _: SourceId) -> Result<()> {
            Ok(())
        }
    }

    struct Tree(PathBuf);

    impl Tree {
        fn new(files: usize) -> Tree {
            let mut b = [0u8; 8];
            getrandom::fill(&mut b).unwrap();
            let root = std::env::temp_dir().join(format!("debrief-stream-{}-{:x}", std::process::id(), u64::from_le_bytes(b)));
            for i in 0..files {
                let d = root.join(format!("d{}/e{}", i % 6, (i / 6) % 4));
                std::fs::create_dir_all(&d).unwrap();
                std::fs::write(d.join(format!("f{i:04}.log")), format!("2026-10-14 10:00:00 INFO file {i}\n")).unwrap();
            }
            Tree(std::fs::canonicalize(root).unwrap())
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn run(tree: &Tree, sink: &Sink, progress: &ImportProgress, opts: &StreamOptions) -> Vec<Outcome> {
        let importers = Importers::default();
        let parsers = crate::parser::ParserSet::of_compiled(CompiledProfile::compile(Profile::default_profile()).unwrap());
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &parsers,
            schema: Arc::new(EventSchema::plain()),
            cancel: progress.cancel_flag(),
            window: None,
        };
        let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let (removed, known) = (HashSet::new(), HashSet::new());
        let roots = vec![tree.0.clone()];
        let spec = StreamSpec {
            paths: &roots,
            roots: &roots,
            profile: &profile,
            removed: &removed,
            known: &known,
        };
        let _run = progress.begin();
        import_stream(&env, &spec, &removed, progress, opts, sink)
    }

    fn opts(threads: usize, queue: usize) -> StreamOptions {
        StreamOptions {
            threads,
            walkers: 4,
            queue,
            ..StreamOptions::default()
        }
    }

    #[test]
    fn source_ids_follow_the_sorted_paths_whatever_the_workers_or_the_delays() {
        let tree = Tree::new(120);
        let mut seen: Vec<Vec<(u32, String)>> = Vec::new();
        for (threads, jitter) in [
            (1, None),
            (2, None),
            (8, None),
            (16, None),
            (8, Some(3)),
            (4, Some(11)),
            (16, Some(77)),
        ] {
            let sink = Sink::default();
            let out = run(
                &tree,
                &sink,
                &ImportProgress::default(),
                &StreamOptions {
                    jitter,
                    ..opts(threads, 16)
                },
            );
            assert_eq!(out.len(), 120);
            let mut began = sink.began.into_inner().unwrap();
            began.sort();
            seen.push(began);
        }
        let first = &seen[0];
        assert_eq!(first.iter().map(|x| x.0).collect::<Vec<_>>(), (0..120).collect::<Vec<u32>>());
        assert!(
            first.windows(2).all(|w| Path::new(&w[0].1) < Path::new(&w[1].1)),
            "ids in path order"
        );
        for run in &seen[1..] {
            assert_eq!(run, first);
        }
    }

    #[test]
    fn the_walk_is_held_back_by_the_queue_and_finding_overlaps_importing() {
        let tree = Tree::new(150);
        let sink = Sink {
            slow: Duration::from_millis(8),
            ..Sink::default()
        };
        let progress = ImportProgress::default();
        let mut max_found_early = 0;
        let mut overlap = false;
        let mut phases = Vec::new();
        std::thread::scope(|s| {
            let job = s.spawn(|| run(&tree, &sink, &progress, &opts(1, 4)));
            while !job.is_finished() {
                let p = progress.snapshot();
                if p.phase == ImportPhase::Importing && p.finding && p.done > 0 {
                    overlap = true;
                    assert!(p.total >= p.done, "{p:?}");
                    // Imported + in flight (2) + queued (4) + the entry the walk holds + the one being dispatched.
                    max_found_early = max_found_early.max(p.total - p.done);
                }
                if phases.last() != Some(&p.phase) {
                    phases.push(p.phase);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(job.join().unwrap().len(), 150);
        });
        assert!(overlap, "never saw importing while still finding: {phases:?}");
        assert!(max_found_early <= 12, "the walk ran {max_found_early} files ahead of a queue of 4");
        let end = progress.snapshot();
        assert_eq!((end.done, end.total, end.finding), (150, 150, false));
    }

    #[test]
    fn cancel_stops_the_walk_and_the_import_and_lists_the_rest_as_not_imported() {
        let tree = Tree::new(400);
        let sink = Sink {
            slow: Duration::from_millis(10),
            ..Sink::default()
        };
        let progress = ImportProgress::default();
        let started = Instant::now();
        let out = std::thread::scope(|s| {
            let job = s.spawn(|| run(&tree, &sink, &progress, &opts(2, 8)));
            while sink.began.lock().unwrap().is_empty() {
                std::thread::yield_now();
            }
            progress.cancel();
            job.join().unwrap()
        });
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        let read = sink.began.lock().unwrap().len();
        let skipped = out
            .iter()
            .filter(|o| matches!(o, Outcome::Unrecognized(u) if u.reason == NOT_IMPORTED))
            .count();
        assert!(read < 400 && skipped > 0, "read {read}, skipped {skipped}");
        assert!(read + skipped <= 400);
    }
}
