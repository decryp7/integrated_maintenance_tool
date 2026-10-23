//! The tail of an import (`Inner::maintain`): a cancelled import skips compaction but still orders the events, and
//! the store stays consistent and queryable. Compaction's own Cancel and progress are covered here too.

use super::*;
use debrief_core::ports::{Ingest, PageQuery, Positions, Query};
use debrief_core::ql::{QueryControl, QueryStage};
use debrief_core::{PluginRegistry, Profile};

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("debrief-maintain-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn samples() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples");
    vec![root.join("serilog-text"), root.join("serilog-clef")]
}

fn segment_files(conn: &Connection) -> Vec<String> {
    let mut stmt = conn.prepare("SELECT file FROM segments ORDER BY file").unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().map(Result::unwrap).collect()
}

/// A store with the sample logs, and one of its sources dropped from the catalog the way `remove_source` does it, but
/// without the compaction that follows: its rows are garbage the next compaction would rewrite.
fn with_garbage(dir: &Dir) -> DuckStore {
    let store = DuckStore::create(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    store.open(&samples()).unwrap().wait().unwrap();
    {
        let conn = store.inner.conn().unwrap();
        let sources: u32 = conn.query_row("SELECT count(*) FROM sources", [], |r| r.get(0)).unwrap();
        assert!(sources > 1, "the fixture has several sources");
        let victim: u32 = conn.query_row("SELECT min(id) FROM sources", [], |r| r.get(0)).unwrap();
        conn.execute("DELETE FROM sources WHERE id = ?", params![victim]).unwrap();
    }
    store
}

#[test]
fn a_cancelled_import_skips_compaction_but_still_orders_the_events() {
    let dir = Dir::new("cancelled-tail");
    let store = with_garbage(&dir);
    let conn = store.inner.conn().unwrap();
    let files_before = segment_files(&conn);
    // The positions are stale (as when events were just added): maintain must renumber them.
    conn.execute_batch("DELETE FROM ord").unwrap();
    assert_eq!(store.positioned().unwrap(), 0);
    let generation = store.inner.generation.load(Ordering::SeqCst);

    let progress = ImportProgress::default();
    progress.cancel();
    store.inner.maintain(&conn, Some(&progress));

    assert_eq!(segment_files(&conn), files_before, "compaction was skipped");
    let events = store.summary().unwrap().total as u64;
    assert!(events > 0);
    assert_eq!(store.positioned().unwrap(), events, "every visible event has a position");
    let ids = store.ids_at(0, 10_000).unwrap();
    assert_eq!(ids.len() as u64, events.min(debrief_core::ports::MAX_PAGE_ROWS as u64));
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(store.position_of(*id).unwrap(), Some(i as u64));
    }
    assert!(
        store.inner.generation.load(Ordering::SeqCst) > generation,
        "readers are told the data changed"
    );
    let page = store
        .page(&PageQuery {
            limit: 5,
            ..PageQuery::default()
        })
        .unwrap();
    assert_eq!(page.rows.len(), 5, "the store stays queryable");

    // The next change (here: an import that is not cancelled) retries the compaction.
    store.inner.maintain(&conn, Some(&ImportProgress::default()));
    assert_ne!(segment_files(&conn), files_before, "the garbage segments were rewritten");
    assert_eq!(store.positioned().unwrap(), store.summary().unwrap().total as u64);
}

#[test]
fn compaction_reports_progress_and_stops_between_groups_when_cancelled() {
    let dir = Dir::new("compact-cancel");
    let store = with_garbage(&dir);
    let conn = store.inner.conn().unwrap();
    let files_before = segment_files(&conn);

    // Cancelled before it starts: nothing is rewritten, and the error says so.
    let control = QueryControl::new();
    control.cancel();
    let err = control
        .scope(|| compact::compact_with(&conn, &store.inner.dir, &store.inner.schema, true))
        .unwrap_err();
    assert!(err.is::<compact::CompactCancelled>(), "{err:#}");
    assert_eq!(segment_files(&conn), files_before);
    assert!(store.summary().unwrap().total > 0, "the store still reads");

    // Not cancelled: the groups are counted until done equals total.
    let control = QueryControl::new();
    let done = control
        .scope(|| compact::compact_with(&conn, &store.inner.dir, &store.inner.schema, true))
        .unwrap();
    assert!(done.replaced > 0);
    let p = control.progress();
    assert_eq!(p.stage, QueryStage::Evaluating);
    assert!(p.total > 0 && p.done == p.total, "{p:?}");
    assert_ne!(segment_files(&conn), files_before);
}
