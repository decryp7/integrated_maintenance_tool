//! Word-search hits as a filter (smart search): the full-text index answers which events hold every word of
//! the search box, and SQL reads those ids, so every view (grid, timeline, charts, analyses) filters by words
//! without scanning the text.
//!
//! - Up to [`INLINE`] ids go into the statement as a list (`e.id IN (...)`), which lets the Parquet reader
//!   skip the row groups (segments are in id order) holding none of them; more go into a small Parquet file in
//!   the store's `tmp/hits/` folder (`e.id IN (SELECT id FROM read_parquet(...))`), emptied when the store
//!   opens and deleted when the last filter using it is dropped and it left the cache.
//! - The cache keeps the last [`KEEP`] searches, keyed by the words and the store's generation, so typing
//!   a search and paging through it computes the hits once.
//! - At most [`crate::fts::MAX_FILTER_HITS`] ids: beyond, the search is answered by a scan.

use anyhow::{Context, Result};
use duckdb::arrow::array::UInt64Array;
use duckdb::arrow::datatypes::{DataType, Field, Schema};
use duckdb::arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Searches whose hits are kept.
pub const KEEP: usize = 8;

/// Most ids written into a statement.
pub const INLINE: usize = 2_000;

/// A Parquet file of event ids (column `id`), deleted on drop.
#[derive(Debug)]
pub struct HitsFile {
    pub path: PathBuf,
}

impl Drop for HitsFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The hits of one search.
#[derive(Debug, Clone)]
pub enum Hits {
    /// Few: the ids themselves (sorted).
    Inline(Arc<[u64]>),
    /// Many: a file of them.
    File(Arc<HitsFile>),
}

impl Hits {
    /// The SQL condition on `e.id` and the file it reads (to keep while the statement may run).
    pub fn condition(&self) -> Option<(String, Option<Arc<HitsFile>>)> {
        match self {
            Hits::Inline(ids) if ids.is_empty() => Some(("FALSE".into(), None)),
            Hits::Inline(ids) => {
                let list: Vec<String> = ids.iter().map(u64::to_string).collect();
                Some((format!("e.id IN ({})", list.join(", ")), None))
            }
            Hits::File(f) => {
                let path = crate::sql::lit(&f.path.to_string_lossy().replace('\\', "/"))?;
                Some((format!("e.id IN (SELECT id FROM read_parquet({path}))"), Some(f.clone())))
            }
        }
    }
}

type Entry = (String, u64, Option<Hits>);

/// The hits of the last searches.
pub struct HitsCache {
    dir: PathBuf,
    next: AtomicU64,
    entries: Mutex<VecDeque<Entry>>,
}

impl HitsCache {
    /// A cache writing to `dir` (created, emptied of what an earlier run left).
    pub fn new(dir: &Path) -> HitsCache {
        let _ = std::fs::remove_dir_all(dir);
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::warn!("search hits folder {} unavailable: {e}", dir.display());
        }
        HitsCache {
            dir: dir.to_path_buf(),
            next: AtomicU64::new(0),
            entries: Mutex::new(VecDeque::new()),
        }
    }

    /// The hits of `key` at `version`, computing the ids (sorted; `None`: too many) when not cached.
    pub fn get_or_write(&self, key: &str, version: u64, ids: impl FnOnce() -> Result<Option<Vec<u64>>>) -> Result<Option<Hits>> {
        {
            let entries = crate::lock(&self.entries);
            if let Some((_, _, h)) = entries.iter().find(|(k, v, _)| k == key && *v == version) {
                return Ok(h.clone());
            }
        }
        let hits = match ids()? {
            Some(ids) if ids.len() <= INLINE => Some(Hits::Inline(ids.into())),
            Some(ids) => {
                let path = self.dir.join(format!("hits-{}.parquet", self.next.fetch_add(1, Ordering::Relaxed)));
                write(&path, ids).with_context(|| format!("writing search hits to {}", path.display()))?;
                Some(Hits::File(Arc::new(HitsFile { path })))
            }
            None => None,
        };
        let mut entries = crate::lock(&self.entries);
        entries.retain(|(k, _, _)| k != key);
        entries.push_back((key.to_string(), version, hits.clone()));
        while entries.len() > KEEP {
            entries.pop_front();
        }
        Ok(hits)
    }
}

impl HitsCache {
    /// A file of ids written as they come (not cached), for a selection SQL cannot express.
    pub fn writer(&self) -> Result<IdsWriter> {
        let path = self.dir.join(format!("ids-{}.parquet", self.next.fetch_add(1, Ordering::Relaxed)));
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::UInt64, false)]));
        let file = std::fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
        // Deleted when dropped unfinished.
        let target = HitsFile { path };
        let writer = ArrowWriter::try_new(file, schema.clone(), None)?;
        Ok(IdsWriter {
            target,
            schema,
            writer: Some(writer),
            pending: Vec::with_capacity(IDS_PER_GROUP),
            written: 0,
        })
    }
}

/// Ids buffered before they are written.
const IDS_PER_GROUP: usize = 65_536;

/// Writes event ids to a Parquet file a group at a time (bounded memory, any number of ids).
pub struct IdsWriter {
    target: HitsFile,
    schema: Arc<Schema>,
    writer: Option<ArrowWriter<std::fs::File>>,
    pending: Vec<u64>,
    written: u64,
}

impl IdsWriter {
    pub fn push(&mut self, id: u64) -> Result<()> {
        self.pending.push(id);
        if self.pending.len() >= IDS_PER_GROUP {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let ids = std::mem::replace(&mut self.pending, Vec::with_capacity(IDS_PER_GROUP));
        self.written += ids.len() as u64;
        let batch = RecordBatch::try_new(self.schema.clone(), vec![Arc::new(UInt64Array::from(ids))])?;
        if let Some(w) = &mut self.writer {
            w.write(&batch)?;
        }
        Ok(())
    }

    /// The ids written, as hits (none: an empty list).
    pub fn finish(mut self) -> Result<Hits> {
        self.flush()?;
        if let Some(w) = self.writer.take() {
            w.close()?;
        }
        Ok(if self.written == 0 {
            Hits::Inline(Arc::new([]))
        } else {
            Hits::File(Arc::new(self.target))
        })
    }
}

fn write(path: &Path, ids: Vec<u64>) -> Result<()> {
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::UInt64, false)]));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(UInt64Array::from(ids))])?;
    let mut w = ArrowWriter::try_new(std::fs::File::create(path)?, schema, None)?;
    w.write(&batch)?;
    w.close()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(h: Option<Hits>) -> Arc<HitsFile> {
        match h {
            Some(Hits::File(f)) => f,
            other => panic!("a file: {other:?}"),
        }
    }

    #[test]
    fn hits_are_cached_by_key_and_version_and_deleted_when_evicted() {
        let dir = std::env::temp_dir().join(format!("debrief-hits-{}", std::process::id()));
        let cache = HitsCache::new(&dir);
        let many: Vec<u64> = (0..=INLINE as u64).collect();
        let a = file(cache.get_or_write("a", 1, || Ok(Some(many.clone()))).unwrap());
        assert!(a.path.exists());
        let again = file(cache.get_or_write("a", 1, || panic!("cached")).unwrap());
        assert_eq!(again.path, a.path);
        let (sql, kept) = Hits::File(a.clone()).condition().unwrap();
        assert!(sql.contains("read_parquet") && kept.is_some(), "{sql}");
        drop(kept);
        assert!(cache.get_or_write("many", 1, || Ok(None)).unwrap().is_none(), "too many hits");
        assert!(cache.get_or_write("many", 1, || panic!("cached")).unwrap().is_none());
        // Few hits go into the statement.
        let few = cache.get_or_write("few", 1, || Ok(Some(vec![3, 7]))).unwrap().unwrap();
        let (sql, kept) = few.condition().unwrap();
        assert_eq!((sql.as_str(), kept.is_none()), ("e.id IN (3, 7)", true));
        let none = cache.get_or_write("none", 1, || Ok(Some(vec![]))).unwrap().unwrap();
        assert_eq!(none.condition().unwrap().0, "FALSE");
        let path = a.path.clone();
        drop((a, again));
        for i in 0..KEEP {
            cache.get_or_write(&format!("k{i}"), 1, || Ok(Some(vec![]))).unwrap();
        }
        assert!(!path.exists(), "evicted and unused: deleted");
        assert!(cache.get_or_write("x", 1, || anyhow::bail!("index broken")).is_err());
        // Ids written a group at a time: a file of all of them, or an empty list.
        let mut w = cache.writer().unwrap();
        for id in 0..IDS_PER_GROUP as u64 + 10 {
            w.push(id).unwrap();
        }
        let written = file(Some(w.finish().unwrap()));
        let rows: i64 = duckdb::Connection::open_in_memory()
            .unwrap()
            .query_row(
                &format!(
                    "SELECT count(*) FROM read_parquet('{}')",
                    written.path.to_string_lossy().replace('\\', "/")
                ),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, IDS_PER_GROUP as i64 + 10);
        let empty = cache.writer().unwrap();
        assert_eq!(empty.finish().unwrap().condition().unwrap().0, "FALSE");
        let path = written.path.clone();
        drop(written);
        assert!(!path.exists(), "deleted when no longer used");
        drop(cache);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
