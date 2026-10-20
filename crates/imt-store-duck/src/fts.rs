//! Full-text index (ADR 0014 §1 `Search`): Tantivy in the store's `index/` folder, a rebuildable cache.
//!
//! - Words: Tantivy's default tokenizer splits on everything that is not a letter or digit and lower-cases,
//!   so `graphic builder` finds `GRAPHIC.BUILDER` and `System.TimeoutException` is `system` + `timeoutexception`.
//!   An event's text is its raw text, plus the message and exception where raw does not contain them.
//! - Indexing runs on a background thread from the registered segments of ready sources, one segment at a
//!   time under the store's write lock (imports and compaction wait for at most one segment), so the grid is
//!   usable at once and import speed does not depend on indexing speed. A segment is indexed by deleting
//!   the documents of its key, adding its rows and committing, then marking it `indexed` in the catalog: a
//!   crash in between is repaired by indexing it again, never by duplicates.
//! - The writer's heap is [`WRITER_HEAP`] (64 MB, the ADR budget) and only exists while indexing.
//! - The commit payload names the catalog's `fts_epoch`; an index that is missing, damaged or from another
//!   epoch is recreated and every segment indexed again.

use crate::segments::SEGMENTS_DIR;
use crate::writer::{meta_u64, set_meta};
use anyhow::{Context, Result};
use duckdb::{params, Connection};
use imt_core::batch::EventId;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ProjectionMask;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, FAST, INDEXED, TEXT};
use tantivy::{doc, Index, IndexReader, IndexWriter, Order, ReloadPolicy, Term};

/// Folder of the index in a store folder.
pub const INDEX_DIR: &str = "index";

/// Heap of the index writer (ADR 0014 memory budget), shared by its threads.
pub const WRITER_HEAP: usize = 64 << 20;
const WRITER_THREADS: usize = 2;

/// Hits returned by default and at most.
pub const DEFAULT_HITS: usize = 1000;
pub const MAX_HITS: usize = 10_000;

#[derive(Clone, Copy)]
struct Fields {
    id: Field,
    source: Field,
    seg: Field,
    text: Field,
}

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let id = b.add_u64_field("id", FAST);
    let source = b.add_u64_field("source", INDEXED);
    let seg = b.add_u64_field("seg", INDEXED);
    b.add_i64_field("ts", FAST);
    let text = b.add_text_field("text", TEXT);
    (b.build(), Fields { id, source, seg, text })
}

fn fields(index: &Index) -> Result<Fields> {
    let s = index.schema();
    Ok(Fields {
        id: s.get_field("id")?,
        source: s.get_field("source")?,
        seg: s.get_field("seg")?,
        text: s.get_field("text")?,
    })
}

/// The key of a segment file in the index (FNV-1a of its name).
pub fn segment_key(file: &str) -> u64 {
    file.bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn payload(epoch: u64) -> String {
    format!("fts_epoch={epoch}")
}

/// An opened index.
pub struct Fts {
    index: Index,
    reader: IndexReader,
    f: Fields,
}

impl Fts {
    /// Opens the index in `dir` for `epoch`, or recreates it (empty) when it is missing, damaged or of
    /// another epoch; the flag says whether it was recreated.
    pub fn open_or_create(dir: &Path, epoch: u64) -> Result<(Fts, bool)> {
        if let Ok(index) = Index::open_in_dir(dir) {
            let current = index.load_metas().ok().and_then(|m| m.payload).is_some_and(|p| p == payload(epoch));
            if current {
                if let Ok(f) = fields(&index) {
                    let reader = index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into()?;
                    return Ok((Fts { index, reader, f }, false));
                }
            }
        }
        if dir.exists() {
            std::fs::remove_dir_all(dir).with_context(|| format!("removing the old index in {}", dir.display()))?;
        }
        std::fs::create_dir_all(dir)?;
        let (schema, f) = schema();
        let index = Index::create_in_dir(dir, schema).context("creating the full-text index")?;
        {
            let mut w: IndexWriter = index.writer_with_num_threads(1, 15 << 20)?;
            let mut prepared = w.prepare_commit()?;
            prepared.set_payload(&payload(epoch));
            prepared.commit()?;
        }
        let reader = index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into()?;
        Ok((Fts { index, reader, f }, true))
    }

    fn writer(&self) -> Result<IndexWriter> {
        Ok(self.index.writer_with_num_threads(WRITER_THREADS, WRITER_HEAP)?)
    }

    fn commit(&self, w: &mut IndexWriter, epoch: u64) -> Result<()> {
        let mut prepared = w.prepare_commit()?;
        prepared.set_payload(&payload(epoch));
        prepared.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Documents in the index.
    pub fn docs(&self) -> u64 {
        self.reader.searcher().num_docs()
    }

    /// Hits of `text` (every word required, lenient syntax) and the ids of the first `limit`, in time order.
    pub fn search(&self, text: &str, limit: usize) -> Result<(u64, Vec<EventId>)> {
        let searcher = self.reader.searcher();
        let mut qp = QueryParser::for_index(&self.index, vec![self.f.text]);
        qp.set_conjunction_by_default();
        let (query, _errors) = qp.parse_query_lenient(text);
        let top = TopDocs::with_limit(limit.clamp(1, MAX_HITS)).order_by_fast_field::<i64>("ts", Order::Asc);
        let (count, hits) = searcher.search(&query, &(Count, top))?;
        let mut ids = Vec::with_capacity(hits.len());
        for (_ts, addr) in hits {
            let col = searcher.segment_reader(addr.segment_ord).fast_fields().u64("id")?;
            if let Some(id) = col.first(addr.doc_id) {
                ids.push(EventId(id));
            }
        }
        Ok((count as u64, ids))
    }

    /// Indexes the rows of ready sources in one segment file.
    fn index_segment(&self, w: &mut IndexWriter, path: &Path, file: &str, ready: &HashSet<u32>) -> Result<u64> {
        let key = segment_key(file);
        w.delete_term(Term::from_field_u64(self.f.seg, key));
        let builder = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(path)?)?;
        let names = ["id", "ts", "source", "message", "exception", "raw"];
        let schema = builder.parquet_schema();
        let roots: Vec<usize> = names
            .iter()
            .map(|n| {
                builder
                    .schema()
                    .index_of(n)
                    .with_context(|| format!("segment {file} has no column {n}"))
            })
            .collect::<Result<_>>()?;
        let mask = ProjectionMask::roots(schema, roots);
        let reader = builder.with_projection(mask).with_batch_size(8192).build()?;
        let ts_field = self.index.schema().get_field("ts")?;
        let mut n = 0;
        for rb in reader {
            let rb = rb?;
            use duckdb::arrow::array::{Array, AsArray};
            use duckdb::arrow::datatypes::{Int64Type, UInt32Type, UInt64Type};
            let col = |name: &str| rb.column_by_name(name).with_context(|| format!("column {name}"));
            let ids = col("id")?.as_primitive_opt::<UInt64Type>().context("id type")?;
            let ts = col("ts")?.as_primitive_opt::<Int64Type>().context("ts type")?;
            let src = col("source")?.as_primitive_opt::<UInt32Type>().context("source type")?;
            let msg = col("message")?.as_string_opt::<i32>().context("message type")?;
            let exc = col("exception")?.as_string_opt::<i32>().context("exception type")?;
            let raw = col("raw")?.as_string_opt::<i32>().context("raw type")?;
            for i in 0..rb.num_rows() {
                if !ready.contains(&src.value(i)) {
                    continue;
                }
                let r = raw.value(i);
                let mut d = doc!(
                    self.f.id => ids.value(i),
                    self.f.source => src.value(i) as u64,
                    self.f.seg => key,
                    ts_field => if ts.is_null(i) { i64::MAX } else { ts.value(i) },
                    self.f.text => r,
                );
                let m = msg.value(i);
                if !r.contains(m) {
                    d.add_text(self.f.text, m);
                }
                if !exc.is_null(i) && !r.contains(exc.value(i)) {
                    d.add_text(self.f.text, exc.value(i));
                }
                w.add_document(d)?;
                n += 1;
            }
        }
        Ok(n)
    }
}

/// State shared by the store and its indexing thread.
pub struct FtsShared {
    pub fts: Fts,
    /// The one writer (Tantivy allows one per index), while there is work.
    writer: Mutex<Option<IndexWriter>>,
    stop: AtomicBool,
    wake: (Mutex<bool>, Condvar),
    /// Segments not indexed yet, as last seen.
    pub pending: AtomicU64,
    /// The last indexing failure.
    pub failure: Mutex<Option<String>>,
}

/// The indexing thread of a store; stopped and joined on drop.
pub struct FtsService {
    pub shared: Arc<FtsShared>,
    epoch: u64,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl FtsService {
    /// Opens (or recreates) the index of the store in `dir` and starts indexing what is missing.
    pub fn start(dir: &Path, conn: Connection, write_lock: Arc<Mutex<()>>) -> Result<FtsService> {
        let epoch = match meta_u64(&conn, "fts_epoch") {
            0 => {
                set_meta(&conn, "fts_epoch", 1)?;
                1
            }
            e => e,
        };
        let (fts, recreated) = Fts::open_or_create(&dir.join(INDEX_DIR), epoch)?;
        if recreated {
            conn.execute_batch("UPDATE segments SET indexed = FALSE")?;
        }
        let shared = Arc::new(FtsShared {
            fts,
            writer: Mutex::new(None),
            stop: AtomicBool::new(false),
            wake: (Mutex::new(true), Condvar::new()),
            pending: AtomicU64::new(0),
            failure: Mutex::new(None),
        });
        let s = shared.clone();
        let store_dir = dir.to_path_buf();
        let handle = std::thread::Builder::new()
            .name("imt-fts".into())
            .spawn(move || run(&s, &store_dir, &conn, &write_lock, epoch))?;
        Ok(FtsService {
            shared,
            epoch,
            handle: Some(handle),
        })
    }

    /// Asks the thread to look for new segments.
    pub fn wake(&self) {
        *crate::lock(&self.shared.wake.0) = true;
        self.shared.wake.1.notify_all();
    }

    /// Runs `f` with the writer and commits.
    fn with_writer(&self, f: impl FnOnce(&mut IndexWriter) -> Result<()>) -> Result<()> {
        let fts = &self.shared.fts;
        let mut slot = crate::lock(&self.shared.writer);
        let w = match slot.as_mut() {
            Some(w) => w,
            None => slot.insert(fts.writer()?),
        };
        f(w)?;
        fts.commit(w, self.epoch)
    }

    /// Deletes a source's documents (it was removed).
    pub fn delete_source(&self, source: u32) -> Result<()> {
        let field = self.shared.fts.f.source;
        self.with_writer(|w| {
            w.delete_term(Term::from_field_u64(field, source as u64));
            Ok(())
        })
    }

    /// Deletes every document (the store was cleared).
    pub fn clear(&self) -> Result<()> {
        self.with_writer(|w| {
            w.delete_all_documents()?;
            Ok(())
        })
    }
}

impl Drop for FtsService {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.wake();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Segments to index and the ready sources, read under the write lock.
fn todo(conn: &Connection) -> Result<(Vec<String>, HashSet<u32>)> {
    let mut stmt = conn.prepare("SELECT file FROM segments WHERE NOT indexed ORDER BY first_id")?;
    let files = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<duckdb::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare("SELECT id FROM sources WHERE ready")?;
    let ready = stmt
        .query_map([], |r| r.get::<_, u32>(0))?
        .collect::<duckdb::Result<HashSet<_>>>()?;
    Ok((files, ready))
}

/// Indexes the next segment, if the write lock is free and there is one; true when it did.
fn step(s: &FtsShared, seg_dir: &Path, conn: &Connection, write_lock: &Mutex<()>, epoch: u64) -> Result<bool> {
    let Ok(_writing) = write_lock.try_lock() else { return Ok(false) };
    let (files, ready) = todo(conn)?;
    s.pending.store(files.len() as u64, Ordering::SeqCst);
    let Some(file) = files.first() else {
        return Ok(false);
    };
    let mut slot = crate::lock(&s.writer);
    let w = match slot.as_mut() {
        Some(w) => w,
        None => slot.insert(s.fts.writer()?),
    };
    if let Err(e) = s.fts.index_segment(w, &seg_dir.join(file), file, &ready) {
        // Not retried in a loop: the segment stays unsearchable and the failure is reported.
        let _ = w.rollback();
        conn.execute("UPDATE segments SET indexed = TRUE WHERE file = ?", params![file])?;
        return Err(e.context(format!("indexing {file}")));
    }
    s.fts.commit(w, epoch)?;
    conn.execute("UPDATE segments SET indexed = TRUE WHERE file = ?", params![file])?;
    s.pending.store(files.len() as u64 - 1, Ordering::SeqCst);
    Ok(true)
}

fn run(s: &FtsShared, dir: &Path, conn: &Connection, write_lock: &Mutex<()>, epoch: u64) {
    let seg_dir = dir.join(SEGMENTS_DIR);
    while !s.stop.load(Ordering::SeqCst) {
        // One segment per turn, under the write lock (imports and compaction wait for at most one).
        match step(s, &seg_dir, conn, write_lock, epoch) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => {
                log::warn!("full-text indexing failed: {e:#}");
                *crate::lock(&s.failure) = Some(format!("{e:#}"));
                *crate::lock(&s.writer) = None;
            }
        }
        // Idle: free the writer's heap and wait for news (or retry a busy lock soon).
        if s.pending.load(Ordering::SeqCst) == 0 {
            *crate::lock(&s.writer) = None;
        }
        let mut woken = crate::lock(&s.wake.0);
        if !*woken {
            woken = s
                .wake
                .1
                .wait_timeout(woken, Duration::from_millis(500))
                .map(|r| r.0)
                .unwrap_or_else(|p| p.into_inner().0);
        }
        *woken = false;
    }
}
