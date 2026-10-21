//! The import writer (ADR 0014 R2b notes): every parse thread writes its own Parquet segment (no single
//! writer to wait for), and one catalog connection records sources, diagnostics and finished segments.
//!
//! - Ids: a segment reserves a block of [`SEGMENT_ROWS`] ids when it opens; the next free id is kept in
//!   `meta` with every listing (and as soon as a block is reserved when batches are observed, since the
//!   observer sees ids before they are listed), so listed ids are never reused, even after a crash.
//! - Visibility: a source becomes ready (visible through the `events` view) when it is finished and every
//!   segment holding its rows has been listed. A segment is closed when it is full, when it has been open
//!   for [`SEGMENT_AGE`](crate::segments::SEGMENT_AGE), or at the end of the import; closed segments are
//!   listed together at most every [`LIST_EVERY`] (rebinding the view reads every segment's footer, so
//!   listing each segment at once made imports quadratic in the number of segments), by whichever parse
//!   thread finds the catalog free (the others go on parsing), and all at the end of the import.
//! - An aborted source is deleted from `sources` at once (its rows stay in segments as garbage, hidden by
//!   the view, until compaction rewrites those segments).
//! - Every batch with its final ids is also handed to `observe` (the full-text indexer).

use crate::lock;
use crate::schema::events_view;
use crate::segments::{Segment, SegmentWriter, SEGMENTS_DIR, SEGMENT_ROWS};
use anyhow::{Context, Result};
use duckdb::{params, Connection};
use imt_core::batch::{EventBatch, EventId, EventSchema, PendingBatch, SourceId};
use imt_core::ingest::{NewSource, SharedSink, SourceReport};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant};

/// Closed segments are listed (and the view rebound) at most this often while importing.
pub const LIST_EVERY: Duration = Duration::from_secs(1);

pub(crate) fn meta_u64(conn: &Connection, key: &str) -> u64 {
    conn.query_row("SELECT value FROM meta WHERE key = ?", params![key], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

pub(crate) fn set_meta(conn: &Connection, key: &str, value: u64) -> Result<()> {
    conn.execute("INSERT OR REPLACE INTO meta VALUES (?, ?)", params![key, value.to_string()])?;
    Ok(())
}

/// Absolute paths of the listed segments, in id order.
pub(crate) fn segment_files(conn: &Connection, dir: &Path) -> Result<Vec<String>> {
    let seg_dir = dir.join(SEGMENTS_DIR);
    let mut stmt = conn.prepare("SELECT file FROM segments ORDER BY first_id")?;
    let files = stmt.query_map([], |r| r.get::<_, String>(0))?;
    files
        .map(|f| Ok(seg_dir.join(f?).to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>>>()
}

/// Recreates the `events` and `events_wide` views over the listed segments.
pub(crate) fn refresh_view(conn: &Connection, dir: &Path, schema: &EventSchema) -> Result<()> {
    let files = segment_files(conn, dir)?;
    conn.execute_batch(&events_view(schema, &files)).context("updating the events view")
}

/// Lists a closed segment with its field catalog and histogram (inside the caller's transaction).
pub(crate) fn insert_segment(conn: &Connection, seg: &Segment, live: u64, indexed: bool) -> Result<()> {
    conn.execute(
        "INSERT INTO segments (file, first_id, end_id, rows, live, indexed, promoted) VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![seg.file, seg.first_id, seg.end_id, seg.rows, live, indexed, seg.promotion.encode()],
    )?;
    let mut app = conn.appender("segment_fields")?;
    for f in &seg.fields {
        app.append_row(params![seg.file, f.key, f.rows, f.numeric_rows, f.distinct, f.top])?;
    }
    app.flush()?;
    let mut app = conn.appender("histogram")?;
    for (b, n) in &seg.histogram {
        app.append_row(params![seg.file, b.minute, b.level, b.source, n])?;
    }
    app.flush()?;
    Ok(())
}

/// Unlists a segment and its catalog rows (inside the caller's transaction).
pub(crate) fn delete_segment(conn: &Connection, file: &str) -> Result<()> {
    for sql in [
        "DELETE FROM segments WHERE file = ?",
        "DELETE FROM segment_fields WHERE file = ?",
        "DELETE FROM histogram WHERE file = ?",
    ] {
        conn.execute(sql, params![file])?;
    }
    Ok(())
}

/// Lists a finished source as ready, with its diagnostics.
fn write_ready(conn: &Connection, r: &SourceReport) -> Result<()> {
    conn.execute(
        "INSERT INTO sources (id, path, name, importer, file_kind, events, skipped, dropped, ts_from, ts_to, bytes, ready) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, TRUE)",
        params![
            r.id.0,
            r.path,
            r.name,
            r.importer,
            format!("{:?}", r.file_kind),
            r.events as u64,
            r.skipped as u64,
            r.dropped as u64,
            r.from,
            r.to,
            r.bytes,
        ],
    )?;
    let mut app = conn.appender("diagnostics")?;
    for p in &r.problems {
        let samples = serde_json::to_string(&p.sample_lines)?;
        app.append_row(params![r.id.0, p.kind.id(), format!("{:?}", p.kind.severity()), p.count, samples])?;
    }
    app.flush()?;
    Ok(())
}

/// Counts `segs` as listed and takes the reports of the finished sources whose rows are now all listed.
fn completed(sources: &mut HashMap<SourceId, SourceState>, segs: &[Segment]) -> Vec<SourceReport> {
    for seg in segs {
        for s in &seg.sources {
            if let Some(st) = sources.get_mut(&SourceId(*s)) {
                st.open = st.open.saturating_sub(1);
            }
        }
    }
    let mut ready: Vec<SourceReport> = sources
        .values_mut()
        .filter(|st| st.open == 0 && !st.aborted)
        .filter_map(|st| st.finished.take())
        .collect();
    ready.sort_by_key(|r| r.id.0);
    ready
}

/// A source of the running import.
#[derive(Default)]
struct SourceState {
    /// Segments holding its rows that are not listed yet.
    open: usize,
    /// Its report, once it is finished, until it is written.
    finished: Option<SourceReport>,
    aborted: bool,
}

/// Next free ids, persisted in `meta`.
struct Ids {
    event: u64,
    source: u32,
}

/// Receives each batch once it has its ids (the full-text indexer).
pub type Observe = Box<dyn Fn(&EventBatch) + Send + Sync>;

/// The writer of one import; see the module docs. Call [`SegmentSink::finish`] at the end.
pub struct SegmentSink {
    dir: PathBuf,
    seg_dir: PathBuf,
    schema: Arc<EventSchema>,
    catalog: Mutex<Connection>,
    ids: Mutex<Ids>,
    /// One open segment per parse thread (the last slot serves threads outside the pool).
    slots: Vec<Mutex<Option<SegmentWriter>>>,
    sources: Mutex<HashMap<SourceId, SourceState>>,
    /// Closed segments waiting to be listed, and when segments were last listed.
    pending: Mutex<(Vec<Segment>, Option<Instant>)>,
    /// [`LIST_EVERY`] (tests change it).
    list_every: Duration,
    generation: Arc<AtomicU64>,
    /// Distinguishes the files of rewritten segments.
    file_generation: u64,
    observe: Option<Observe>,
}

impl SegmentSink {
    /// A writer into the store in `dir` for `threads` parse threads, continuing after the ids in use.
    pub fn new(
        conn: Connection,
        dir: &Path,
        schema: Arc<EventSchema>,
        threads: usize,
        generation: Arc<AtomicU64>,
        observe: Option<Observe>,
    ) -> Result<SegmentSink> {
        let seg_dir = dir.join(SEGMENTS_DIR);
        std::fs::create_dir_all(&seg_dir)?;
        let (max_end, max_source) = conn.query_row(
            "SELECT (SELECT coalesce(max(end_id), 0) FROM segments), (SELECT coalesce(max(id) + 1, 0) FROM sources)",
            [],
            |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u32>(1)?)),
        )?;
        let ids = Ids {
            event: meta_u64(&conn, "next_id").max(max_end),
            source: u32::try_from(meta_u64(&conn, "next_source")).unwrap_or(u32::MAX).max(max_source),
        };
        let file_generation = meta_u64(&conn, "file_generation") + 1;
        set_meta(&conn, "file_generation", file_generation)?;
        Ok(SegmentSink {
            dir: dir.to_path_buf(),
            seg_dir,
            schema,
            catalog: Mutex::new(conn),
            ids: Mutex::new(ids),
            slots: (0..threads.max(1) + 1).map(|_| Mutex::new(None)).collect(),
            sources: Mutex::new(HashMap::new()),
            pending: Mutex::new((Vec::new(), None)),
            list_every: LIST_EVERY,
            generation,
            file_generation,
            observe,
        })
    }

    fn slot(&self) -> &Mutex<Option<SegmentWriter>> {
        let pool = self.slots.len() - 1;
        let i = rayon::current_thread_index().map_or(pool, |i| i % pool);
        &self.slots[i]
    }

    /// A new segment with a fresh id block of at least `rows` ids.
    fn open_segment(&self, rows: u64) -> Result<SegmentWriter> {
        let mut ids = lock(&self.ids);
        let first = ids.event;
        let end = first + SEGMENT_ROWS.max(rows);
        if self.observe.is_some() {
            set_meta(&lock(&self.catalog), "next_id", end)?;
        }
        ids.event = end;
        drop(ids);
        SegmentWriter::create(&self.seg_dir, &self.schema, first, end, self.file_generation)
    }

    /// Closes a segment and lists the closed segments when they are due and the catalog is free.
    fn register(&self, w: SegmentWriter) -> Result<()> {
        let seg = w.close()?;
        lock(&self.pending).0.push(seg);
        let conn = match self.catalog.try_lock() {
            Ok(c) => c,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            // Another thread is listing: this segment goes with a later listing.
            Err(TryLockError::WouldBlock) => return Ok(()),
        };
        let segs = {
            let mut pending = lock(&self.pending);
            if pending.1.is_some_and(|t| t.elapsed() < self.list_every) {
                return Ok(());
            }
            pending.1 = Some(Instant::now());
            std::mem::take(&mut pending.0)
        };
        self.list(&conn, segs, false)
    }

    /// Lists closed segments and makes the sources they complete ready, in one catalog transaction (a
    /// commit costs tens of milliseconds); `last` also persists the next source id.
    fn list(&self, conn: &Connection, segs: Vec<Segment>, last: bool) -> Result<()> {
        let ready = completed(&mut lock(&self.sources), &segs);
        conn.execute_batch("BEGIN TRANSACTION")?;
        let done = (|| -> Result<()> {
            for seg in &segs {
                insert_segment(conn, seg, seg.rows, false)?;
            }
            for r in &ready {
                write_ready(conn, r)?;
            }
            let (event, source) = {
                let ids = lock(&self.ids);
                (ids.event, ids.source)
            };
            set_meta(conn, "next_id", event)?;
            if last {
                set_meta(conn, "next_source", source as u64)?;
            }
            refresh_view(conn, &self.dir, &self.schema)
        })();
        match done {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                for seg in &segs {
                    let _ = std::fs::remove_file(self.seg_dir.join(&seg.file));
                }
                return Err(e);
            }
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Registers every open segment and persists the next ids. Sources finished in this import are
    /// ready afterwards.
    pub fn finish(self) -> Result<()> {
        let mut first_err = None;
        // Closing a segment encodes and compresses its last row group: in parallel.
        let open: Vec<SegmentWriter> = self.slots.iter().filter_map(|s| lock(s).take()).collect();
        let closed: Vec<Result<Segment>> = std::thread::scope(|s| {
            let handles: Vec<_> = open.into_iter().map(|w| s.spawn(move || w.close())).collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|p| {
                        Err(anyhow::anyhow!(
                            "closing a segment crashed: {}",
                            imt_core::guard::panic_message(p.as_ref())
                        ))
                    })
                })
                .collect()
        });
        let mut segs = std::mem::take(&mut lock(&self.pending).0);
        for seg in closed {
            match seg {
                Ok(s) => segs.push(s),
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        if let Err(e) = self.list(&lock(&self.catalog), segs, true) {
            first_err.get_or_insert(e);
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl SharedSink for SegmentSink {
    fn reserve_sources(&self, n: u32) -> Result<SourceId> {
        let mut ids = lock(&self.ids);
        let first = ids.source;
        let end = first.checked_add(n).context("out of source ids")?;
        set_meta(&lock(&self.catalog), "next_source", end as u64)?;
        ids.source = end;
        Ok(SourceId(first))
    }

    /// Sources are listed when they become ready (one catalog commit costs tens of milliseconds, so
    /// nothing is written per source while reading).
    fn begin_source(&self, _source: &NewSource, reserved: Option<SourceId>) -> Result<SourceId> {
        let id = match reserved {
            Some(id) => id,
            None => self.reserve_sources(1)?,
        };
        lock(&self.sources).insert(id, SourceState::default());
        Ok(id)
    }

    fn write(&self, batch: PendingBatch) -> Result<()> {
        let n = batch.len() as u64;
        if n == 0 {
            return Ok(());
        }
        let mut slot = lock(self.slot());
        if slot.as_ref().is_some_and(|w| w.remaining() < n || w.is_old()) {
            if let Some(w) = slot.take() {
                self.register(w)?;
            }
        }
        let w = match slot.as_mut() {
            Some(w) => w,
            None => slot.insert(self.open_segment(n)?),
        };
        let batch = batch.assign_ids(EventId(w.next_id()))?;
        let new = w.write(&batch)?;
        if !new.is_empty() {
            let mut sources = lock(&self.sources);
            for s in new {
                sources.entry(SourceId(s)).or_default().open += 1;
            }
        }
        drop(slot);
        if let Some(observe) = &self.observe {
            observe(&batch);
        }
        Ok(())
    }

    /// The source is listed with the next segment registered (or at the end of the import).
    fn finish_source(&self, r: &SourceReport) -> Result<()> {
        lock(&self.sources).entry(r.id).or_default().finished = Some(r.clone());
        Ok(())
    }

    /// Never listed; its rows in segments are garbage the view hides.
    fn abort_source(&self, id: SourceId) -> Result<()> {
        let mut sources = lock(&self.sources);
        let st = sources.entry(id).or_default();
        st.aborted = true;
        st.finished = None;
        Ok(())
    }
}

impl Drop for SegmentSink {
    fn drop(&mut self) {
        // Dropped without `finish` (the import failed or crashed): unfinished segments are discarded.
        for slot in &self.slots {
            if let Some(w) = lock(slot).take() {
                w.discard();
            }
        }
        // Closed but never listed: no reader knows them (opening the store would sweep them too).
        for seg in std::mem::take(&mut lock(&self.pending).0) {
            let _ = std::fs::remove_file(self.seg_dir.join(&seg.file));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::batch::{EventBatchBuilder, EventTime, LevelCode, NewEvent};

    fn store() -> (PathBuf, Connection) {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        let dir = std::env::temp_dir().join(format!("imt-writer-{}-{:x}", std::process::id(), u64::from_le_bytes(b)));
        std::fs::create_dir_all(&dir).unwrap();
        let c = Connection::open(dir.join("data.duckdb")).unwrap();
        c.execute_batch(&crate::schema::ddl(&EventSchema::plain())).unwrap();
        refresh_view(&c, &dir, &EventSchema::plain()).unwrap();
        (dir, c)
    }

    fn batch(source: SourceId, n: usize) -> PendingBatch {
        let mut b = EventBatchBuilder::new(Arc::new(EventSchema::plain()));
        for i in 0..n {
            let ev = NewEvent {
                time: EventTime::Parsed(i as i64),
                level: LevelCode::new(2).unwrap(),
                source,
                line: i as u32 + 1,
                byte_offset: 0,
                message: "m",
                template: None,
                exception: None,
                markers: 0,
                raw: "raw m",
            };
            b.push(&ev, [("K", imt_core::batch::FieldValue::Text("v"))]).unwrap();
        }
        b.finish()
    }

    fn report(id: SourceId, events: usize) -> SourceReport {
        SourceReport {
            id,
            path: "p".into(),
            name: "n".into(),
            importer: "text".into(),
            file_kind: imt_core::api::FileKind::Text,
            events,
            skipped: 0,
            dropped: 0,
            step_stats: Vec::new(),
            from: Some(0),
            to: Some(1),
            bytes: 10,
            problems: vec![imt_core::ingest::Problem {
                kind: imt_core::ingest::ProblemKind::LineTooLong,
                count: 1,
                sample_lines: vec![3],
            }],
        }
    }

    fn count(c: &Connection, sql: &str) -> u64 {
        c.query_row(sql, [], |r| r.get::<_, u64>(0)).unwrap()
    }

    fn sink(c: &Connection, dir: &Path, seen: Option<Arc<AtomicU64>>) -> SegmentSink {
        let observe: Option<Observe> = seen.map(|s| -> Observe {
            Box::new(move |b: &EventBatch| {
                s.fetch_add(b.num_rows() as u64, Ordering::SeqCst);
            })
        });
        SegmentSink::new(
            c.try_clone().unwrap(),
            dir,
            Arc::new(EventSchema::plain()),
            2,
            Arc::new(AtomicU64::new(0)),
            observe,
        )
        .unwrap()
    }

    #[test]
    fn sources_become_visible_when_their_segments_are_registered() {
        let (dir, c) = store();
        let seen = Arc::new(AtomicU64::new(0));
        let w = sink(&c, &dir, Some(seen.clone()));
        let src = NewSource {
            path: "a".into(),
            name: "a".into(),
        };
        let a = w.begin_source(&src, None).unwrap();
        // More than a segment: the first one is registered while the source is still being read.
        let per = 8192;
        let batches = (SEGMENT_ROWS as usize / per) + 2;
        for _ in 0..batches {
            w.write(batch(a, per)).unwrap();
        }
        assert_eq!(count(&c, "SELECT count(*) FROM segments"), 1);
        assert_eq!(count(&c, "SELECT count(*) FROM events"), 0, "not ready before it is finished");
        w.finish_source(&report(a, per * batches)).unwrap();
        assert_eq!(count(&c, "SELECT count(*) FROM events"), 0, "a segment of it is still open");
        // An aborted source leaves garbage that the view hides.
        let b = w.begin_source(&src, None).unwrap();
        w.write(batch(b, 5)).unwrap();
        w.abort_source(b).unwrap();
        w.finish().unwrap();
        assert_eq!(count(&c, "SELECT count(*) FROM events"), (per * batches) as u64);
        assert_eq!(count(&c, "SELECT count(*) FROM diagnostics"), 1);
        assert_eq!(count(&c, "SELECT count(*) FROM sources"), 1);
        assert_eq!(seen.load(Ordering::SeqCst), (per * batches + 5) as u64, "every batch was observed");
        // Ids are unique and never reused by a later import.
        assert_eq!(
            count(&c, "SELECT count(DISTINCT id) FROM events"),
            count(&c, "SELECT count(*) FROM events")
        );
        let max = count(&c, "SELECT max(end_id) FROM segments");
        let w = sink(&c, &dir, None);
        let next = w.begin_source(&src, None).unwrap();
        assert!(next.0 > b.0);
        w.write(batch(next, 1)).unwrap();
        w.finish_source(&report(next, 1)).unwrap();
        w.finish().unwrap();
        assert!(count(&c, &format!("SELECT id FROM events WHERE source = {}", next.0)) >= max);
        // A dropped writer discards its open segment.
        let w = sink(&c, &dir, None);
        let d = w.begin_source(&src, None).unwrap();
        w.write(batch(d, 3)).unwrap();
        drop(w);
        assert_eq!(count(&c, &format!("SELECT count(*) FROM events WHERE source = {}", d.0)), 0);
        let tmp = std::fs::read_dir(dir.join(SEGMENTS_DIR))
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "tmp"))
            .count();
        assert_eq!(tmp, 0);
        drop(c);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn closed_segments_wait_for_the_next_listing() {
        let (dir, c) = store();
        let mut w = sink(&c, &dir, None);
        w.list_every = Duration::from_secs(3600);
        let src = NewSource {
            path: "a".into(),
            name: "a".into(),
        };
        let a = w.begin_source(&src, None).unwrap();
        let per = 8192;
        let full = SEGMENT_ROWS as usize / per;
        // The first closed segment is listed at once, the second waits (listed less than an hour ago).
        for _ in 0..2 * full + 1 {
            w.write(batch(a, per)).unwrap();
        }
        assert_eq!(count(&c, "SELECT count(*) FROM segments"), 1);
        let files = || std::fs::read_dir(dir.join(SEGMENTS_DIR)).unwrap().count();
        assert_eq!(files(), 3, "two closed segments and an open one");
        w.finish_source(&report(a, per * (2 * full + 1))).unwrap();
        w.finish().unwrap();
        assert_eq!(count(&c, "SELECT count(*) FROM segments"), 3, "the rest is listed at the end");
        assert_eq!(count(&c, "SELECT count(*) FROM events"), (per * (2 * full + 1)) as u64);
        assert_eq!(
            count(&c, "SELECT CAST(value AS BIGINT) FROM meta WHERE key = 'next_id'"),
            count(&c, "SELECT max(end_id) FROM segments")
        );
        // A writer dropped with a closed segment that was never listed deletes it.
        let mut w = sink(&c, &dir, None);
        w.list_every = Duration::from_secs(3600);
        let b = w.begin_source(&src, None).unwrap();
        for _ in 0..2 * full + 1 {
            w.write(batch(b, per)).unwrap();
        }
        assert_eq!(files(), 6);
        drop(w);
        assert_eq!(files(), 4, "the listed segment stays until compaction");
        assert_eq!(count(&c, &format!("SELECT count(*) FROM events WHERE source = {}", b.0)), 0);
        drop(c);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn parallel_writers_each_fill_their_own_segment() {
        let (dir, c) = store();
        let w = sink(&c, &dir, None);
        let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
        let ids: Vec<SourceId> = (0..6)
            .map(|i| {
                w.begin_source(
                    &NewSource {
                        path: format!("f{i}"),
                        name: format!("f{i}"),
                    },
                    None,
                )
                .unwrap()
            })
            .collect();
        pool.install(|| {
            use rayon::prelude::*;
            ids.par_iter().for_each(|id| {
                for _ in 0..3 {
                    w.write(batch(*id, 1000)).unwrap();
                }
                w.finish_source(&report(*id, 3000)).unwrap();
            })
        });
        w.finish().unwrap();
        assert_eq!(count(&c, "SELECT count(*) FROM events"), 18_000);
        assert_eq!(count(&c, "SELECT count(*) FROM sources WHERE ready"), 6);
        assert!(
            count(&c, "SELECT count(*) FROM segments") <= 3,
            "one segment per thread (plus the outside slot)"
        );
        drop(c);
        std::fs::remove_dir_all(dir).ok();
    }
}
