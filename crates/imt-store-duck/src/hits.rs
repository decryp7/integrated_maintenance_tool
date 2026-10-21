//! Word-search hits as a filter (smart search): the full-text index answers which events hold every word of
//! the search box, and SQL reads those ids from a small Parquet file (`e.id IN (SELECT id FROM
//! read_parquet(...))`), so every view (grid, timeline, charts, analyses) filters by words without
//! scanning the text.
//!
//! - The files live in the store's `tmp/hits/` folder, emptied when the store opens; a file is deleted when
//!   the last filter using it is dropped and it left the cache.
//! - The cache keeps the last [`KEEP`] searches, keyed by the words and the store's generation, so typing
//!   a search and paging through it computes the hits once.
//! - At most [`crate::fts::MAX_FILTER_HITS`] ids: beyond, the search is answered as substrings of its words.

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

type Entry = (String, u64, Option<Arc<HitsFile>>);

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

    /// The hits file of `key` at `version`, computing the ids (sorted; `None`: too many) when not cached.
    pub fn get_or_write(&self, key: &str, version: u64, ids: impl FnOnce() -> Result<Option<Vec<u64>>>) -> Result<Option<Arc<HitsFile>>> {
        {
            let entries = crate::lock(&self.entries);
            if let Some((_, _, f)) = entries.iter().find(|(k, v, _)| k == key && *v == version) {
                return Ok(f.clone());
            }
        }
        let file = match ids()? {
            Some(ids) => {
                let path = self.dir.join(format!("hits-{}.parquet", self.next.fetch_add(1, Ordering::Relaxed)));
                write(&path, ids).with_context(|| format!("writing search hits to {}", path.display()))?;
                Some(Arc::new(HitsFile { path }))
            }
            None => None,
        };
        let mut entries = crate::lock(&self.entries);
        entries.retain(|(k, _, _)| k != key);
        entries.push_back((key.to_string(), version, file.clone()));
        while entries.len() > KEEP {
            entries.pop_front();
        }
        Ok(file)
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

    #[test]
    fn hits_are_cached_by_key_and_version_and_deleted_when_evicted() {
        let dir = std::env::temp_dir().join(format!("imt-hits-{}", std::process::id()));
        let cache = HitsCache::new(&dir);
        let a = cache.get_or_write("a", 1, || Ok(Some(vec![1, 2, 3]))).unwrap().unwrap();
        assert!(a.path.exists());
        let again = cache.get_or_write("a", 1, || panic!("cached")).unwrap().unwrap();
        assert_eq!(again.path, a.path);
        assert!(cache.get_or_write("many", 1, || Ok(None)).unwrap().is_none(), "too many hits");
        assert!(cache.get_or_write("many", 1, || panic!("cached")).unwrap().is_none());
        let path = a.path.clone();
        drop((a, again));
        for i in 0..KEEP {
            cache.get_or_write(&format!("k{i}"), 1, || Ok(Some(vec![]))).unwrap();
        }
        assert!(!path.exists(), "evicted and unused: deleted");
        assert!(cache.get_or_write("x", 1, || anyhow::bail!("index broken")).is_err());
        drop(cache);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
