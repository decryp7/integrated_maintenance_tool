//! The parallel import (ADR 0014 §2): files are parsed on a bounded rayon pool, many at a time, and a large
//! uncompressed text or JSON-lines file is split into chunks of about [`CHUNK_TARGET`] bytes that are parsed
//! in parallel too. The store's [`SharedSink`] receives batches from every parse thread.
//!
//! Chunks come from the file plan: one sequential newline scan ([`scan_chunk_starts`]) gives each chunk its
//! first line number, and each start is then moved forward to the next line that begins an event, so
//! continuation lines (stack traces) never separate from their event and every chunk but the first starts
//! with an event. Files whose reading carries state across lines (time-only timestamps, JSON arrays, other
//! importers, archives) are read whole by one thread.

use super::batcher::{BatchedSource, RecordBatcher};
use super::chunk::{LineReader, ReaderChunks};
use super::plan::{scan_chunk_starts, ChunkStart, FilePlan, JsonShape, PlanKind, CHUNK_TARGET};
use super::sink::{BatchSink, NewSource, SourceReport};
use super::source::{import_path_with, ImportEnv, Outcome};
use super::{json, text};
use crate::api::UnrecognizedFile;
use crate::batch::{PendingBatch, SourceId};
use crate::importers::ImportCtx;
use crate::workspace::{ImportProgress, NOT_IMPORTED};
use anyhow::{anyhow, Result};
use rayon::prelude::*;
use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Parse threads by default: one core is left for the writer and the UI, and at most
/// [`MAX_DEFAULT_THREADS`] so the worker stays within its memory budget (each parse thread holds a chunk
/// buffer, a batch and its store writer's buffers). Settings can raise it.
pub fn default_threads() -> usize {
    crate::threads::threads(crate::threads::WorkKind::Cpu, None).clamp(1, MAX_DEFAULT_THREADS)
}

/// Upper bound of [`default_threads`].
pub const MAX_DEFAULT_THREADS: usize = 8;

/// How the parallel import runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParallelOptions {
    /// Parse threads (at least 1).
    pub threads: usize,
    /// Target size of a chunk of a large file; files under twice this size are read whole.
    pub chunk_target: u64,
}

impl Default for ParallelOptions {
    fn default() -> Self {
        ParallelOptions {
            threads: default_threads(),
            chunk_target: CHUNK_TARGET,
        }
    }
}

/// The store side of a parallel import: like [`BatchSink`], but shared by every parse thread. Batches of
/// one source may arrive from several threads (its chunks); `finish_source` is called once they all have.
pub trait SharedSink: Sync {
    /// Sets aside `n` consecutive source ids (handed to [`SharedSink::begin_source`] later) and returns the
    /// first, so sources are numbered in the order of the files whatever thread reads them first.
    fn reserve_sources(&self, n: u32) -> Result<SourceId>;
    /// Begins a source, with a reserved id or (`None`) the next free one.
    fn begin_source(&self, source: &NewSource, id: Option<SourceId>) -> Result<SourceId>;
    fn write(&self, batch: PendingBatch) -> Result<()>;
    fn finish_source(&self, report: &SourceReport) -> Result<()>;
    fn abort_source(&self, id: SourceId) -> Result<()>;
}

/// A [`SharedSink`] used where a [`BatchSink`] is expected.
pub struct SharedRef<'a>(pub &'a dyn SharedSink);

impl BatchSink for SharedRef<'_> {
    fn begin_source(&mut self, source: &NewSource) -> Result<SourceId> {
        self.0.begin_source(source, None)
    }
    fn write(&mut self, batch: PendingBatch) -> Result<()> {
        self.0.write(batch)
    }
    fn finish_source(&mut self, report: &SourceReport) -> Result<()> {
        self.0.finish_source(report)
    }
    fn abort_source(&mut self, id: SourceId) -> Result<()> {
        self.0.abort_source(id)
    }
}

/// The sink of one file: its sources take the ids reserved for the file, in order.
struct Ordered<'a> {
    sink: &'a dyn SharedSink,
    next: u32,
    end: u32,
}

impl BatchSink for Ordered<'_> {
    fn begin_source(&mut self, source: &NewSource) -> Result<SourceId> {
        let id = (self.next < self.end).then(|| {
            self.next += 1;
            SourceId(self.next - 1)
        });
        self.sink.begin_source(source, id)
    }
    fn write(&mut self, batch: PendingBatch) -> Result<()> {
        self.sink.write(batch)
    }
    fn finish_source(&mut self, report: &SourceReport) -> Result<()> {
        self.sink.finish_source(report)
    }
    fn abort_source(&mut self, id: SourceId) -> Result<()> {
        self.sink.abort_source(id)
    }
}

/// Most source ids set aside for one archive.
const MAX_RESERVED: u32 = 10_000;

/// Sources a file can become: the entries of a zip archive, otherwise one.
fn source_slots(path: &Path) -> u32 {
    let Ok(mut f) = std::fs::File::open(path) else { return 1 };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() || !crate::archive::is_zip(&magic) {
        return 1;
    }
    match f.seek(SeekFrom::Start(0)).map(|_| zip::ZipArchive::new(std::io::BufReader::new(f))) {
        Ok(Ok(z)) => (z.len() as u32).clamp(1, MAX_RESERVED),
        _ => 1,
    }
}

/// Splitting of large files, offered to the reading of one file.
#[derive(Clone, Copy)]
pub(super) struct Split<'a> {
    pub sink: &'a dyn SharedSink,
    pub target: u64,
}

impl Split<'_> {
    /// Whether a planned file of `size` bytes is read in chunks.
    pub fn applies(&self, plan: &FilePlan, size: u64) -> bool {
        plan.splittable
            && matches!(
                plan.kind,
                PlanKind::Text { time_only: false, .. } | PlanKind::Json(JsonShape::Lines)
            )
            && size >= self.target.saturating_mul(2)
    }
}

/// Reads `files` into `sink` on a pool of `opts.threads` threads, largest files first; outcomes come back
/// in the order of `files`. After a cancel the files not started are listed as not imported.
pub fn import_files_parallel(
    env: &ImportEnv,
    files: &[(PathBuf, String)],
    removed: &HashSet<PathBuf>,
    progress: &ImportProgress,
    opts: &ParallelOptions,
    sink: &dyn SharedSink,
) -> Vec<Outcome> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(opts.threads.max(1))
        .thread_name(|i| format!("debrief-parse-{i}"))
        .build();
    // Every file's size and archive entries are needed before the first is read (source ids follow the file
    // order): looked at on the pool, counted into the progress ("detecting").
    progress.detecting(files.len());
    let look = |(p, _): &(PathBuf, String)| -> (u64, u32) {
        let looked = if progress.is_cancelled() {
            (0, 1)
        } else {
            (std::fs::metadata(p).map_or(0, |m| m.len()), source_slots(p))
        };
        progress.detected(1);
        looked
    };
    let looked: Vec<(u64, u32)> = match &pool {
        Ok(pool) => pool.install(|| files.par_iter().map(look).collect()),
        Err(_) => files.iter().map(look).collect(),
    };
    let sizes: Vec<u64> = looked.iter().map(|l| l.0).collect();
    let slots: Vec<u32> = looked.iter().map(|l| l.1).collect();
    progress.start(files.len(), sizes.iter().sum());
    let split = Split {
        sink,
        target: opts.chunk_target.max(1),
    };
    // Source ids in file order: each file gets its slots before any is read.
    let total: u32 = slots.iter().fold(0u32, |a, s| a.saturating_add(*s));
    let base = match sink.reserve_sources(total) {
        Ok(b) => Some(b.0),
        Err(e) => {
            log::warn!("could not reserve source ids: {e:#}");
            None
        }
    };
    let mut starts = Vec::with_capacity(files.len());
    let mut acc = 0u32;
    for s in &slots {
        starts.push(acc);
        acc = acc.saturating_add(*s);
    }
    let one = |i: usize| -> Vec<Outcome> {
        let (path, name) = &files[i];
        let out = if progress.is_cancelled() {
            vec![Outcome::Unrecognized(UnrecognizedFile {
                path: path.to_string_lossy().into_owned(),
                name: name.clone(),
                reason: NOT_IMPORTED.into(),
                crashed: false,
            })]
        } else {
            progress.reading(path);
            let (next, end) = base.map_or((0, 0), |b| (b + starts[i], b + starts[i] + slots[i]));
            import_path_with(env, path, name, removed, &mut Ordered { sink, next, end }, Some(split))
        };
        progress.file_done(sizes[i]);
        out
    };
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(sizes[i]));
    let mut results: Vec<(usize, Vec<Outcome>)> = match pool {
        Ok(pool) => pool.install(|| order.par_iter().map(|&i| (i, one(i))).collect()),
        // No threads to be had: read one file after another.
        Err(e) => {
            log::warn!("parallel import unavailable ({e}); reading files one by one");
            order.iter().map(|&i| (i, one(i))).collect()
        }
    };
    results.sort_by_key(|(i, _)| *i);
    progress.finish();
    results.into_iter().flat_map(|(_, o)| o).collect()
}

/// Whether `line` begins an event of the planned format.
fn begins_event(plan: &FilePlan, ctx: &ImportCtx, line: &str) -> bool {
    match plan.kind {
        PlanKind::Text { format, .. } => crate::importers::lines::begins_event(ctx.profile, format, line),
        // Top-level objects start at the first column; nested lines of pretty-printed objects are indented.
        PlanKind::Json(_) => line.starts_with('{'),
        PlanKind::Binary | PlanKind::Legacy => false,
    }
}

/// The first line at or after `start` (and before `end`) that begins an event, if any.
fn event_start(path: &Path, plan: &FilePlan, ctx: &ImportCtx, start: ChunkStart, end: u64) -> Result<Option<ChunkStart>> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start.byte_offset))?;
    let mut chunks = ReaderChunks::new(file.take(end - start.byte_offset), plan.encoding, start.byte_offset).starting_at_line(start.line);
    let mut lines = LineReader::new(&mut chunks);
    while let Some(l) = lines.next_line()? {
        if !l.too_long && begins_event(plan, ctx, l.text) {
            return Ok(Some(ChunkStart {
                byte_offset: l.byte_offset,
                line: l.number,
            }));
        }
    }
    Ok(None)
}

/// Chunk boundaries of a file: `(start, end)` byte ranges, each but the first starting with an event.
pub(super) fn plan_chunks(path: &Path, plan: &FilePlan, ctx: &ImportCtx, size: u64, target: u64) -> Result<Vec<(ChunkStart, u64)>> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(plan.data_start))?;
    let starts = scan_chunk_starts(
        std::io::BufReader::with_capacity(1 << 20, file),
        plan.encoding,
        plan.data_start,
        target,
    )?;
    let mut aligned = vec![starts[0]];
    for (i, s) in starts.iter().enumerate().skip(1) {
        let end = starts.get(i + 1).map_or(size, |n| n.byte_offset);
        // No event begins in this stretch: it stays with the chunk before.
        if let Some(found) = event_start(path, plan, ctx, *s, end)? {
            aligned.push(found);
        }
    }
    Ok(aligned
        .iter()
        .enumerate()
        .map(|(i, s)| (*s, aligned.get(i + 1).map_or(size, |n| n.byte_offset)))
        .collect())
}

/// Reads one chunk of a source into its own batcher.
#[allow(clippy::too_many_arguments)]
fn read_chunk(
    env: &ImportEnv,
    ctx: &ImportCtx,
    plan: &FilePlan,
    path: &Path,
    (start, end): (ChunkStart, u64),
    id: SourceId,
    name: &str,
    sink: &dyn SharedSink,
) -> Result<(BatchedSource, usize)> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start.byte_offset))?;
    let len = end - start.byte_offset;
    let mut chunks = ReaderChunks::new(file.take(len), plan.encoding, start.byte_offset).starting_at_line(start.line);
    let mut lines = LineReader::new(&mut chunks);
    let mut shared = SharedRef(sink);
    let mut batcher = RecordBatcher::new(ctx.profile, env.schema.clone(), id, name, &mut shared, env.cancel).with_window(env.window);
    let skipped = match plan.kind {
        PlanKind::Text { format, day_first, .. } => text::import_text(ctx, format, day_first, &mut lines, &mut batcher)?,
        PlanKind::Json(shape) => json::import_json(ctx, shape, len, &mut lines, &mut batcher)?,
        PlanKind::Binary | PlanKind::Legacy => return Err(anyhow!("{} files are not read in chunks", plan.importer)),
    };
    Ok((batcher.finish()?, skipped))
}

/// Reads a large file in parallel chunks into `split.sink`; `None` when it has a single chunk (read it the
/// usual way). A failure (or crash) of any chunk fails the source.
#[allow(clippy::too_many_arguments)]
pub(super) fn read_split(
    env: &ImportEnv,
    ctx: &ImportCtx,
    plan: &FilePlan,
    path: &Path,
    id: SourceId,
    name: &str,
    split: Split<'_>,
) -> Result<Option<(BatchedSource, usize)>> {
    let size = std::fs::metadata(path)?.len();
    let chunks = plan_chunks(path, plan, ctx, size, split.target)?;
    if chunks.len() < 2 {
        return Ok(None);
    }
    let parts: Vec<Result<(BatchedSource, usize)>> = chunks
        .par_iter()
        .map(|c| crate::guard::catch("Debrief", || read_chunk(env, ctx, plan, path, *c, id, name, split.sink)))
        .collect();
    let mut total = BatchedSource::default();
    let mut skipped = 0;
    for p in parts {
        let (b, s) = p?;
        total.merge(b);
        skipped += s;
    }
    Ok(Some((total, skipped)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{EventBatch, EventId, EventSchema};
    use crate::importers::Importers;
    use crate::ingest::plan::plan;
    use crate::parser::ParserSet;
    use crate::profile::{CompiledProfile, Profile};
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    /// Collects batches from any thread.
    #[derive(Default)]
    struct Shared {
        batches: Mutex<Vec<EventBatch>>,
        next_id: AtomicU64,
        next_source: AtomicU32,
        finished: Mutex<Vec<SourceReport>>,
        aborted: Mutex<Vec<SourceId>>,
        /// Cancelled as soon as a source begins (for the cancel test).
        cancel_on_begin: Option<Arc<ImportProgress>>,
    }

    impl SharedSink for Shared {
        fn reserve_sources(&self, n: u32) -> Result<SourceId> {
            Ok(SourceId(self.next_source.fetch_add(n, Ordering::SeqCst)))
        }
        fn begin_source(&self, _s: &NewSource, id: Option<SourceId>) -> Result<SourceId> {
            if let Some(progress) = &self.cancel_on_begin {
                progress.cancel();
            }
            Ok(id.unwrap_or_else(|| SourceId(self.next_source.fetch_add(1, Ordering::SeqCst))))
        }
        fn write(&self, batch: PendingBatch) -> Result<()> {
            let first = self.next_id.fetch_add(batch.len() as u64, Ordering::SeqCst);
            self.batches.lock().unwrap().push(batch.assign_ids(EventId(first))?);
            Ok(())
        }
        fn finish_source(&self, r: &SourceReport) -> Result<()> {
            self.finished.lock().unwrap().push(r.clone());
            Ok(())
        }
        fn abort_source(&self, id: SourceId) -> Result<()> {
            self.aborted.lock().unwrap().push(id);
            Ok(())
        }
    }

    fn temp() -> PathBuf {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        let d = std::env::temp_dir().join(format!("debrief-parallel-{}-{:x}", std::process::id(), u64::from_le_bytes(b)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A text log whose events have stack traces, so chunk starts must move to event starts.
    fn log(events: usize) -> String {
        let mut s = String::from("banner before the first event\n");
        for i in 0..events {
            s.push_str(&format!(
                "2026-10-14 10:{:02}:{:02}.{:03} +00:00 [ERR] event {i}\n",
                (i / 60) % 60,
                i % 60,
                i % 1000
            ));
            if i % 3 == 0 {
                s.push_str("System.Exception: boom\n   at A.B()\n   at C.D()\n");
            }
        }
        s
    }

    fn run(files: &[(PathBuf, String)], target: u64, threads: usize) -> (Shared, Vec<Outcome>) {
        let importers = Importers::default();
        let profile = ParserSet::of_compiled(CompiledProfile::compile(Profile::default_profile()).unwrap());
        let cancel = AtomicBool::new(false);
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &profile,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
            window: None,
        };
        let shared = Shared::default();
        let progress = ImportProgress::default();
        let opts = ParallelOptions {
            threads,
            chunk_target: target,
        };
        let out = import_files_parallel(&env, files, &HashSet::new(), &progress, &opts, &shared);
        assert_eq!(progress.snapshot().done, files.len());
        (shared, out)
    }

    /// (line, message, raw) of every event, in line order.
    fn rows(shared: &Shared) -> Vec<(u32, String, String)> {
        let mut v: Vec<_> = shared
            .batches
            .lock()
            .unwrap()
            .iter()
            .flat_map(|b| {
                b.rows()
                    .map(|r| (r.line(), r.message().to_string(), r.raw().to_string()))
                    .collect::<Vec<_>>()
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn chunked_reading_equals_reading_whole() {
        let dir = temp();
        let text = log(3000);
        let p = dir.join("big.log");
        std::fs::write(&p, &text).unwrap();
        let files = vec![(p.clone(), "big.log".to_string())];
        let (whole, out_whole) = run(&files, 1 << 30, 1);
        let (chunked, out_chunked) = run(&files, 4096, 4);
        let (Outcome::Imported(a), Outcome::Imported(b)) = (&out_whole[0], &out_chunked[0]) else {
            panic!("{out_whole:?} {out_chunked:?}")
        };
        assert_eq!((a.events, a.skipped, a.from, a.to), (b.events, b.skipped, b.from, b.to));
        assert_eq!(a.events, 3001, "the banner is one untimed event");
        assert_eq!(rows(&whole), rows(&chunked));
        assert!(chunked.batches.lock().unwrap().len() > 3, "read in several chunks");
        // Byte offsets point at each event's first line.
        for b in chunked.batches.lock().unwrap().iter() {
            for r in b.rows() {
                let at = r.byte_offset() as usize;
                assert!(text[at..].starts_with(r.raw().lines().next().unwrap()), "{}", r.line());
            }
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn chunks_start_at_events_and_carry_line_numbers() {
        let dir = temp();
        let p = dir.join("a.log");
        std::fs::write(&p, log(500)).unwrap();
        let importers = Importers::default();
        let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &profile,
            path: &p,
            file_date: None,
        };
        let head = std::fs::read(&p).unwrap();
        let fp = plan(&importers, &ctx, &head, true).unwrap();
        let chunks = plan_chunks(&p, &fp, &ctx, head.len() as u64, 2000).unwrap();
        assert!(chunks.len() > 5);
        let text = String::from_utf8(head).unwrap();
        for (i, (s, e)) in chunks.iter().enumerate() {
            let at = s.byte_offset as usize;
            let line_no = text[..at].matches('\n').count() as u32 + 1;
            assert_eq!(s.line, line_no);
            if i > 0 {
                assert!(text[at..].starts_with("2026-10-14"), "chunk {i} starts with an event");
                assert_eq!(chunks[i - 1].1, s.byte_offset);
            }
            assert!(e > &s.byte_offset);
        }
        assert_eq!(chunks.last().unwrap().1, text.len() as u64);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn json_lines_split_and_other_files_read_whole() {
        let dir = temp();
        let mut json = String::new();
        for i in 0..2000 {
            json.push_str(&format!("{{\"@t\":\"2026-10-14T10:00:{:02}Z\",\"@m\":\"m{i}\"}}\n", i % 60));
        }
        let j = dir.join("a.clef");
        std::fs::write(&j, &json).unwrap();
        let t = dir.join("small.log");
        std::fs::write(&t, "2026-10-14 10:00:00 INFO a\n").unwrap();
        let bad = dir.join("bad.bin");
        std::fs::write(&bad, [0u8, 1, 2, 3, 0, 0, 9, 9, 0, 1]).unwrap();
        let files = vec![
            (j.clone(), "a.clef".to_string()),
            (t.clone(), "small.log".to_string()),
            (bad.clone(), "bad.bin".to_string()),
        ];
        let (shared, out) = run(&files, 4096, 3);
        let Outcome::Imported(a) = &out[0] else { panic!("{out:?}") };
        assert_eq!(a.events, 2000);
        // Sources are numbered in file order although the big file was started first.
        let Outcome::Imported(b) = &out[1] else { panic!("{out:?}") };
        assert_eq!((a.id, b.id), (SourceId(0), SourceId(1)));
        assert!(matches!(&out[1], Outcome::Imported(r) if r.events == 1));
        assert!(matches!(&out[2], Outcome::Unrecognized(u) if u.reason == "binary file"));
        let lines: Vec<u32> = rows(&shared).iter().filter(|r| r.1.starts_with('m')).map(|r| r.0).collect();
        assert_eq!(lines, (1..=2000).collect::<Vec<_>>());
        assert_eq!(shared.finished.lock().unwrap().len(), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_failing_chunk_fails_only_its_source() {
        struct FailSome(Shared);
        impl SharedSink for FailSome {
            fn reserve_sources(&self, _n: u32) -> Result<SourceId> {
                anyhow::bail!("no reservations here")
            }
            fn begin_source(&self, s: &NewSource, id: Option<SourceId>) -> Result<SourceId> {
                assert_eq!(id, None, "nothing was reserved");
                self.0.begin_source(s, id)
            }
            fn write(&self, batch: PendingBatch) -> Result<()> {
                // A crash in the middle of the file.
                if self.0.batches.lock().unwrap().len() == 2 {
                    panic!("disk on fire");
                }
                self.0.write(batch)
            }
            fn finish_source(&self, r: &SourceReport) -> Result<()> {
                self.0.finish_source(r)
            }
            fn abort_source(&self, id: SourceId) -> Result<()> {
                self.0.abort_source(id)
            }
        }
        let dir = temp();
        let p = dir.join("big.log");
        std::fs::write(&p, log(20000)).unwrap();
        let importers = Importers::default();
        let profile = ParserSet::of_compiled(CompiledProfile::compile(Profile::default_profile()).unwrap());
        let cancel = AtomicBool::new(false);
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &profile,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
            window: None,
        };
        let sink = FailSome(Shared::default());
        let progress = ImportProgress::default();
        let opts = ParallelOptions {
            threads: 2,
            chunk_target: 50_000,
        };
        let out = import_files_parallel(&env, &[(p, "big.log".into())], &HashSet::new(), &progress, &opts, &sink);
        let Outcome::Unrecognized(u) = &out[0] else { panic!("{out:?}") };
        assert!(u.reason.contains("disk on fire"), "{}", u.reason);
        assert_eq!(sink.0.aborted.lock().unwrap().len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    fn env_parts() -> (Importers, ParserSet, AtomicBool) {
        (
            Importers::default(),
            ParserSet::of_compiled(CompiledProfile::compile(Profile::default_profile()).unwrap()),
            AtomicBool::new(false),
        )
    }

    #[test]
    fn a_shared_sink_stands_in_for_a_batch_sink_and_passes_everything_through() {
        // A real report to pass through.
        let dir = temp();
        let p = dir.join("a.log");
        std::fs::write(&p, log(3)).unwrap();
        let (done, _) = run(&[(p, "a.log".to_string())], 1 << 30, 1);
        let report = done.finished.lock().unwrap()[0].clone();
        std::fs::remove_dir_all(dir).ok();
        let shared = Shared::default();
        let mut sink = SharedRef(&shared);
        let source = NewSource {
            path: "a.log".into(),
            name: "a.log".into(),
        };
        let id = sink.begin_source(&source).unwrap();
        let other = sink.begin_source(&source).unwrap();
        assert_ne!(id, other, "each source gets the next free id");
        sink.finish_source(&report).unwrap();
        sink.abort_source(other).unwrap();
        assert_eq!(shared.finished.lock().unwrap().len(), 1);
        assert_eq!(*shared.aborted.lock().unwrap(), vec![other]);
    }

    #[test]
    fn zip_archives_reserve_one_source_per_entry_and_anything_else_one() {
        use std::io::Write;
        let dir = temp();
        let zip_path = dir.join("logs.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        for n in ["a.log", "b.log", "c.log"] {
            w.start_file(n, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b"2026-10-14 10:00:00 INFO x\n").unwrap();
        }
        w.finish().unwrap();
        assert_eq!(source_slots(&zip_path), 3);
        // A file that starts like a zip but is not one, a short file, a plain log and a missing file: one slot.
        let fake = dir.join("fake.zip");
        std::fs::write(&fake, b"PK\x03\x04 and then nothing a zip reader accepts").unwrap();
        assert_eq!(source_slots(&fake), 1);
        let short = dir.join("short");
        std::fs::write(&short, b"PK").unwrap();
        assert_eq!(source_slots(&short), 1);
        let plain = dir.join("plain.log");
        std::fs::write(&plain, b"2026-10-14 10:00:00 INFO x\n").unwrap();
        assert_eq!(source_slots(&plain), 1);
        assert_eq!(source_slots(&dir.join("missing")), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn after_a_cancel_the_files_not_started_are_listed_as_not_imported() {
        let dir = temp();
        // The bigger file is read first (one thread), and cancels; the smaller one is never started.
        let paths: Vec<(PathBuf, String)> = [("a.log", 50), ("b.log", 5)]
            .iter()
            .map(|(n, events)| {
                let p = dir.join(n);
                std::fs::write(&p, log(*events)).unwrap();
                (p, n.to_string())
            })
            .collect();
        let (importers, profile, cancel) = env_parts();
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &profile,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
            window: None,
        };
        let progress = Arc::new(ImportProgress::default());
        let sink = Shared {
            cancel_on_begin: Some(progress.clone()),
            ..Shared::default()
        };
        let opts = ParallelOptions {
            threads: 1,
            chunk_target: 1 << 20,
        };
        let out = import_files_parallel(&env, &paths, &HashSet::new(), &progress, &opts, &sink);
        assert_eq!(out.len(), 2);
        let Outcome::Unrecognized(u) = &out[1] else {
            panic!("{:?}", out[1])
        };
        assert_eq!((u.reason.as_str(), u.name.as_str()), (NOT_IMPORTED, "b.log"));
        assert!(progress.snapshot().cancelled);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn only_big_splittable_event_text_is_read_in_chunks() {
        let dir = temp();
        let p = dir.join("a.log");
        std::fs::write(&p, log(200)).unwrap();
        let (importers, _, _) = env_parts();
        let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &profile,
            path: &p,
            file_date: None,
        };
        let head = std::fs::read(&p).unwrap();
        let mut fp = plan(&importers, &ctx, &head, true).unwrap();
        let shared = Shared::default();
        let split = Split {
            sink: &shared,
            target: 1000,
        };
        let size = head.len() as u64;
        assert!(split.applies(&fp, size));
        assert!(!split.applies(&fp, 1999), "a file under two chunks is read whole");
        fp.splittable = false;
        assert!(!split.applies(&fp, size));
        fp.splittable = true;
        fp.kind = PlanKind::Binary;
        assert!(!split.applies(&fp, size), "binary files are never chunked");
        assert!(!begins_event(&fp, &ctx, "2026-10-14 10:00:00 INFO a"));
        fp.kind = PlanKind::Legacy;
        assert!(!begins_event(&fp, &ctx, "2026-10-14 10:00:00 INFO a"));
        // JSON events start at the first column; nested lines are indented.
        fp.kind = PlanKind::Json(JsonShape::Lines);
        assert!(begins_event(&fp, &ctx, "{\"@t\":1}") && !begins_event(&fp, &ctx, "  {\"a\":1}"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_stretch_without_an_event_start_stays_with_the_chunk_before_and_small_files_are_not_split() {
        let dir = temp();
        let p = dir.join("one-event.log");
        // One event followed by a long continuation: no chunk start after the first can begin an event.
        let mut text = String::from("2026-10-14 10:00:00.000 +00:00 [ERR] the only event\n");
        for i in 0..400 {
            text.push_str(&format!("   at Frame{i}.Method()\n"));
        }
        std::fs::write(&p, &text).unwrap();
        let (importers, profile_set, cancel) = env_parts();
        let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &profile,
            path: &p,
            file_date: None,
        };
        let head = std::fs::read(&p).unwrap();
        let fp = plan(&importers, &ctx, &head, true).unwrap();
        let chunks = plan_chunks(&p, &fp, &ctx, head.len() as u64, 500).unwrap();
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        assert_eq!(chunks[0].1, head.len() as u64);

        // read_split reports "read it the usual way" when there is a single chunk.
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &profile_set,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
            window: None,
        };
        let shared = Shared::default();
        let split = Split {
            sink: &shared,
            target: 500,
        };
        assert!(read_split(&env, &ctx, &fp, &p, SourceId(0), "one-event.log", split)
            .unwrap()
            .is_none());
        assert!(shared.batches.lock().unwrap().is_empty());
        // A plan that cannot be chunked fails the source with the importer's name.
        let mut binary = fp;
        binary.kind = PlanKind::Binary;
        let big = dir.join("big.log");
        std::fs::write(&big, log(300)).unwrap();
        let bctx = ImportCtx {
            profile: &profile,
            path: &big,
            file_date: None,
        };
        let size = std::fs::metadata(&big).unwrap().len();
        let chunks = plan_chunks(&big, &binary, &bctx, size, 500).unwrap();
        let e = read_chunk(
            &env,
            &bctx,
            &binary,
            &big,
            (chunks[0].0, chunks[0].1),
            SourceId(0),
            "big.log",
            &shared,
        )
        .err()
        .unwrap();
        assert!(e.to_string().contains("not read in chunks"), "{e}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn defaults_are_bounded() {
        let o = ParallelOptions::default();
        assert!((1..=MAX_DEFAULT_THREADS).contains(&o.threads));
        assert_eq!(o.chunk_target, CHUNK_TARGET);
    }
}
