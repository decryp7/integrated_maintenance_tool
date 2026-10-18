//! Full-text index (Tantivy) shared by both persistent adapters. Indexing runs on its own thread
//! fed through a bounded channel, reports progress, can be cancelled, and commits every
//! [`COMMIT_EVERY`] documents with the last indexed id as the commit payload, so a reopened store
//! knows where to resume and the column store is queryable before the index is complete.

use anyhow::{Context, Result};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;
use std::thread::JoinHandle;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, FAST, INDEXED, TEXT};
use tantivy::{doc, Index, IndexWriter, Order};

/// Documents between commits (each commit is a resume point).
pub const COMMIT_EVERY: u64 = 2_000_000;
/// Indexing heap shared by the writer threads.
const WRITER_HEAP: usize = 256 << 20;
const WRITER_THREADS: usize = 2;

/// What is indexed of one event.
pub struct FtsDoc {
    pub id: u64,
    pub ts: i64,
    pub message: String,
    pub exception: Option<String>,
}

#[derive(Clone, Copy)]
struct Fields {
    id: Field,
    #[allow(dead_code)]
    ts: Field,
    message: Field,
    exception: Field,
}

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let id = b.add_u64_field("id", FAST);
    let ts = b.add_i64_field("ts", FAST | INDEXED);
    let message = b.add_text_field("message", TEXT);
    let exception = b.add_text_field("exception", TEXT);
    (
        b.build(),
        Fields {
            id,
            ts,
            message,
            exception,
        },
    )
}

fn fields(index: &Index) -> Result<Fields> {
    let s = index.schema();
    Ok(Fields {
        id: s.get_field("id")?,
        ts: s.get_field("ts")?,
        message: s.get_field("message")?,
        exception: s.get_field("exception")?,
    })
}

/// Result of a background indexing run.
#[derive(Debug, Clone, Copy)]
pub struct IndexReport {
    pub docs: u64,
    pub cancelled: bool,
}

/// When an ingest finished: the table (queryable from then on) and the full-text index.
#[derive(Debug, Clone, Copy)]
pub struct Finished {
    pub table_ms: u128,
    /// Documents the index had when the table was done: the rest is indexed in the background.
    pub indexed_at_table: u64,
    pub total_ms: u128,
    pub indexed: u64,
    pub cancelled: bool,
}

/// A running background indexer.
pub struct Indexer {
    tx: Option<SyncSender<Vec<FtsDoc>>>,
    handle: Option<JoinHandle<Result<IndexReport>>>,
    progress: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
}

impl Indexer {
    /// Creates (replacing) the index in `dir` and starts the indexing thread.
    pub fn start(dir: &Path) -> Result<Indexer> {
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        std::fs::create_dir_all(dir)?;
        let (schema, f) = schema();
        let index = Index::create_in_dir(dir, schema).context("creating the full-text index")?;
        let mut writer: IndexWriter = index.writer_with_num_threads(WRITER_THREADS, WRITER_HEAP)?;
        let (tx, rx) = sync_channel::<Vec<FtsDoc>>(4);
        let progress = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c) = (progress.clone(), cancel.clone());
        let handle = std::thread::Builder::new()
            .name("fts-indexer".into())
            .spawn(move || -> Result<IndexReport> {
                let (mut docs, mut since_commit, mut last_id) = (0u64, 0u64, None::<u64>);
                for batch in rx {
                    if c.load(Ordering::Relaxed) {
                        writer.rollback()?;
                        return Ok(IndexReport { docs: 0, cancelled: true });
                    }
                    for d in batch {
                        let mut doc = doc!(f.id => d.id, f.ts => d.ts, f.message => d.message);
                        if let Some(x) = d.exception {
                            doc.add_text(f.exception, x);
                        }
                        writer.add_document(doc)?;
                        last_id = Some(d.id);
                        docs += 1;
                        since_commit += 1;
                    }
                    p.store(docs, Ordering::Relaxed);
                    if since_commit >= COMMIT_EVERY {
                        commit(&mut writer, last_id)?;
                        since_commit = 0;
                    }
                }
                if c.load(Ordering::Relaxed) {
                    writer.rollback()?;
                    return Ok(IndexReport { docs: 0, cancelled: true });
                }
                commit(&mut writer, last_id)?;
                writer.wait_merging_threads()?;
                Ok(IndexReport { docs, cancelled: false })
            })?;
        Ok(Indexer {
            tx: Some(tx),
            handle: Some(handle),
            progress,
            cancel,
        })
    }

    /// Queues a batch (blocks while the indexer is four batches behind).
    pub fn send(&self, batch: Vec<FtsDoc>) -> Result<()> {
        self.tx
            .as_ref()
            .context("indexer already finished")?
            .send(batch)
            .map_err(|_| anyhow::anyhow!("the indexer stopped"))
    }

    /// Documents indexed so far.
    pub fn progress(&self) -> u64 {
        self.progress.load(Ordering::Relaxed)
    }

    /// Stops at the next batch and rolls back to the last commit (the UI's Cancel; exercised by
    /// the tests, the benchmark never cancels).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Waits for the queue to drain, commits and merges.
    pub fn finish(mut self) -> Result<IndexReport> {
        drop(self.tx.take());
        let h = self.handle.take().context("indexer already finished")?;
        h.join().map_err(|_| anyhow::anyhow!("the indexer thread panicked"))?
    }
}

fn commit(writer: &mut IndexWriter, last_id: Option<u64>) -> Result<()> {
    let mut prepared = writer.prepare_commit()?;
    if let Some(id) = last_id {
        prepared.set_payload(&format!("indexed_to={id}"));
    }
    prepared.commit()?;
    Ok(())
}

/// An opened index for searching.
pub struct Fts {
    index: Index,
    reader: tantivy::IndexReader,
    f: Fields,
}

impl Fts {
    pub fn open(dir: &Path) -> Result<Fts> {
        let index = Index::open_in_dir(dir).with_context(|| format!("opening the full-text index in {}", dir.display()))?;
        let f = fields(&index)?;
        let reader = index.reader()?;
        Ok(Fts { index, reader, f })
    }

    /// The last id a commit covered (where indexing resumes after a crash or cancel).
    pub fn indexed_to(&self) -> Result<Option<u64>> {
        let meta = self.index.load_metas()?;
        Ok(meta
            .payload
            .and_then(|p| p.strip_prefix("indexed_to=").and_then(|n| n.parse().ok())))
    }

    pub fn docs(&self) -> u64 {
        self.reader.searcher().num_docs()
    }

    /// Hits of a user query over message and exception (all terms required) and the ids of the
    /// first `limit` hits in time order.
    pub fn search(&self, text: &str, limit: usize) -> Result<(u64, Vec<u64>)> {
        let searcher = self.reader.searcher();
        let mut qp = QueryParser::for_index(&self.index, vec![self.f.message, self.f.exception]);
        qp.set_conjunction_by_default();
        // Lenient parsing: user text never fails as a query syntax error.
        let (query, _errors) = qp.parse_query_lenient(text);
        let top = TopDocs::with_limit(limit.max(1)).order_by_fast_field::<i64>("ts", Order::Asc);
        let (count, hits) = searcher.search(&query, &(Count, top))?;
        let mut ids = Vec::with_capacity(hits.len());
        for (_ts, addr) in hits {
            let col = searcher.segment_reader(addr.segment_ord).fast_fields().u64("id")?;
            if let Some(id) = col.first(addr.doc_id) {
                ids.push(id);
            }
        }
        ids.sort_unstable();
        Ok((count as u64, ids))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn docs(from: u64, n: u64) -> Vec<FtsDoc> {
        (from..from + n)
            .map(|i| FtsDoc {
                id: i,
                ts: 1_000 + i as i64,
                message: if i % 10 == 0 {
                    format!("request {i} timed out")
                } else {
                    format!("request {i} ok")
                },
                exception: (i % 25 == 0).then(|| "System.TimeoutException: deadline".to_string()),
            })
            .collect()
    }

    #[test]
    fn indexes_in_the_background_and_searches_in_time_order() {
        let dir = std::env::temp_dir().join(format!("imt-spike-fts-{}", std::process::id()));
        let ix = Indexer::start(&dir).unwrap();
        ix.send(docs(0, 500)).unwrap();
        ix.send(docs(500, 500)).unwrap();
        let report = ix.finish().unwrap();
        assert_eq!(report.docs, 1000);
        assert!(!report.cancelled);
        let fts = Fts::open(&dir).unwrap();
        assert_eq!(fts.docs(), 1000);
        assert_eq!(fts.indexed_to().unwrap(), Some(999));
        let (hits, ids) = fts.search("timed out", 3).unwrap();
        assert_eq!(hits, 100);
        assert_eq!(ids, vec![0, 10, 20]);
        let (hits, _) = fts.search("timeoutexception", 5).unwrap();
        assert_eq!(hits, 40, "`System.TimeoutException` is the tokens `system` and `timeoutexception`");
        let (hits, _) = fts.search("timeout", 5).unwrap();
        assert_eq!(hits, 0, "tokens, not substrings: `timeout` is not inside `timeoutexception`");
        let (hits, _) = fts.search("deadline", 5).unwrap();
        assert_eq!(hits, 40);
        let (hits, _) = fts.search("unbalanced \"quote AND (", 5).unwrap();
        assert!(hits <= 1000, "lenient parsing never fails");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_rolls_back_to_the_last_commit() {
        let dir = std::env::temp_dir().join(format!("imt-spike-fts-cancel-{}", std::process::id()));
        let ix = Indexer::start(&dir).unwrap();
        ix.send(docs(0, 100)).unwrap();
        ix.cancel();
        ix.send(docs(100, 100)).unwrap_or(());
        let report = ix.finish().unwrap();
        assert!(report.cancelled);
        let fts = Fts::open(&dir).unwrap();
        assert_eq!(fts.docs(), 0);
        assert_eq!(fts.indexed_to().unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
