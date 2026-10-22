//! The segment store: positions, rows by id, compaction, removal and recovery after a crash.

mod common;

use common::{log_samples, store_with, TempDir};
use imt_core::batch::{EventId, SourceId};
use imt_core::ports::{Ingest, PageQuery, Positions, Query, StoreError};
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::segments::SEGMENTS_DIR;
use imt_store_duck::{DuckStore, StoreOptions};

fn files(dir: &TempDir) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir.0.join(SEGMENTS_DIR))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn positions_follow_time_order_and_rows_come_by_id() {
    let dir = TempDir::new("pos");
    let (store, report) = store_with(&dir, Profile::default_profile(), &log_samples()).unwrap();
    let total = store.positioned().unwrap();
    assert_eq!(total as usize, report.events);
    // The unfiltered page in time order equals the ids at positions 0.. and the rows fetched by id.
    let page = store
        .page(&PageQuery {
            limit: 300,
            offset: 1000,
            ..Default::default()
        })
        .unwrap();
    let ids = store.ids_at(1000, 300).unwrap();
    assert_eq!(ids, page.rows.iter().map(|r| r.id).collect::<Vec<_>>());
    assert_eq!(store.position_of(ids[7]).unwrap(), Some(1007));
    assert_eq!(store.position_of(EventId(u64::MAX)).unwrap(), None);
    let rows: Vec<EventId> = store
        .rows(&ids)
        .unwrap()
        .iter()
        .flat_map(|b| b.rows().map(|r| r.id()).collect::<Vec<_>>())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(rows, sorted, "every id once, in id order");
    assert!(store.rows(&[]).unwrap().is_empty());
    assert!(store.ids_at(total, 10).unwrap().is_empty());
    assert_eq!(store.ids_at(0, 1_000_000).unwrap().len(), 5000, "at most a page");
}

#[test]
fn removal_hides_at_once_and_compaction_reclaims() {
    let logs = TempDir::new("compact-logs");
    for i in 0..40 {
        std::fs::write(
            logs.0.join(format!("f{i:02}.log")),
            format!(
                "2026-10-14 10:00:{:02} INFO file {i} line 1\n2026-10-14 10:00:{:02} WARN file {i} line 2\n",
                i % 60,
                i % 60
            ),
        )
        .unwrap();
    }
    let dir = TempDir::new("compact");
    // One parse thread and a time-limited segment age would give one segment; force many small ones by
    // importing file by file.
    let store = DuckStore::create_with(
        &dir.0,
        Profile::default_profile(),
        PluginRegistry::new(),
        StoreOptions {
            parse_threads: 1,
            ..StoreOptions::default()
        },
    )
    .unwrap();
    for i in 0..40 {
        store.open(&[logs.0.join(format!("f{i:02}.log"))]).unwrap().wait().unwrap();
    }
    assert_eq!(store.summary().unwrap().total, 80);
    // 40 small segments (one per import) were merged once there were more than 32.
    let n = files(&dir).len();
    assert!(n < 40, "{n} segment files");
    // Removing a source hides its events at once; a segment that is mostly garbage is rewritten (here:
    // the last file's own segment, dropped).
    let before = files(&dir).len();
    let all = store.summary().unwrap().sources;
    let src = all
        .iter()
        .find(|s| s.name.ends_with("f39.log"))
        .cloned()
        .unwrap_or_else(|| panic!("{:?}", all.iter().map(|s| &s.name).collect::<Vec<_>>()));
    store.remove_source(SourceId(src.id)).unwrap();
    assert_eq!(store.summary().unwrap().total, 78);
    assert_eq!(store.positioned().unwrap(), 78);
    assert_eq!(files(&dir).len(), before - 1);
    // A source that is a small part of a merged segment is rewritten out of it (exact catalog and histogram).
    let src = store
        .summary()
        .unwrap()
        .sources
        .into_iter()
        .find(|s| s.name.ends_with("f07.log"))
        .unwrap();
    store.remove_source(SourceId(src.id)).unwrap();
    assert_eq!(store.summary().unwrap().total, 76);
    assert_eq!(
        store.compact().unwrap(),
        imt_store_duck::Compaction::default(),
        "nothing left to do"
    );
    let glob = dir.0.join(SEGMENTS_DIR).join("*.parquet").to_string_lossy().replace('\\', "/");
    assert_eq!(
        store
            .bench_sql(&format!("SELECT 1 FROM read_parquet('{glob}') WHERE source = {}", src.id))
            .unwrap(),
        0
    );
    assert_eq!(
        store
            .bench_sql(&format!("SELECT 1 FROM histogram WHERE source = {}", src.id))
            .unwrap(),
        0
    );
    assert_eq!(store.bench_sql("SELECT sum(n) FROM histogram HAVING sum(n) = 76").unwrap(), 1);
    // Removing everything leaves no file behind.
    store.clear().unwrap();
    assert_eq!(store.summary().unwrap().total, 0);
    assert!(files(&dir).is_empty());
    // And it can import again.
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    assert_eq!(store.summary().unwrap().total, 80);
}

#[test]
fn reopening_recovers_from_a_crash_and_refuses_damage() {
    let dir = TempDir::new("recover");
    let (store, report) = store_with(&dir, Profile::default_profile(), &log_samples()).unwrap();
    drop(store);
    let seg_dir = dir.0.join(SEGMENTS_DIR);
    // What a crash leaves: a half-written segment and a finished one that was never listed.
    std::fs::write(seg_dir.join("seg-00000000ffffffff-0001.tmp"), b"half").unwrap();
    std::fs::write(seg_dir.join("seg-00000000fffffffe-0001.parquet"), b"unlisted").unwrap();
    let store = DuckStore::open(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    assert_eq!(store.summary().unwrap().total, report.events);
    assert!(files(&dir).iter().all(|f| !f.contains("ffffff")), "{:?}", files(&dir));
    // Moved elsewhere (a shared project): the view follows the folder. The index is built in the background after
    // opening and writes into the folder, so it is finished before the folder is copied.
    store.wait_for_index(std::time::Duration::from_secs(60)).unwrap();
    drop(store);
    let moved = TempDir::new("recover-moved");
    std::fs::remove_dir_all(&moved.0).unwrap();
    copy(&dir.0, &moved.0);
    drop(dir);
    let store = DuckStore::open(&moved.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    assert_eq!(store.summary().unwrap().total, report.events);
    drop(store);
    // A missing segment is refused with a message.
    let first = files(&moved).into_iter().next().unwrap();
    std::fs::remove_file(moved.0.join(SEGMENTS_DIR).join(&first)).unwrap();
    let Err(StoreError::Damaged(e)) = DuckStore::open(&moved.0, Profile::default_profile(), PluginRegistry::new()) else {
        panic!("a missing segment must be refused")
    };
    assert!(e.contains("missing"), "{e}");
}

#[test]
fn a_damaged_segment_is_a_typed_error_not_a_crash() {
    let dir = TempDir::new("damaged");
    let (store, _) = store_with(&dir, Profile::default_profile(), &log_samples()).unwrap();
    drop(store);
    let first = files(&dir).into_iter().next().unwrap();
    std::fs::write(dir.0.join(SEGMENTS_DIR).join(&first), b"not a parquet file at all").unwrap();
    // Opening may already notice; otherwise the first read of the events does.
    let failure = match DuckStore::open(&dir.0, Profile::default_profile(), PluginRegistry::new()) {
        Err(e) => e,
        Ok(store) => store
            .page(&imt_core::ports::PageQuery::default())
            .err()
            .or_else(|| store.summary().err())
            .expect("reading a damaged segment fails"),
    };
    assert!(matches!(&failure, StoreError::Damaged(m) if m.contains("damaged")), "{failure:?}");
    assert!(!failure.is_retryable());
}

fn copy(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

#[test]
fn refresh_rereads_changed_files_and_reads_new_ones() {
    let logs = TempDir::new("refresh-logs");
    let a = logs.0.join("a.log");
    let b = logs.0.join("b.log");
    std::fs::write(&a, "2026-10-14 10:00:00 INFO a1\n").unwrap();
    std::fs::write(&b, "2026-10-14 10:00:00 INFO b1\n2026-10-14 10:00:01 INFO b2\n").unwrap();
    let dir = TempDir::new("refresh");
    let store = DuckStore::create(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    assert_eq!(store.summary().unwrap().total, 3);
    // a grows, b is deleted, c is new.
    std::fs::write(&a, "2026-10-14 10:00:00 INFO a1\n2026-10-14 10:00:02 WARN a2\n").unwrap();
    std::fs::remove_file(&b).unwrap();
    std::fs::write(logs.0.join("c.log"), "2026-10-14 10:00:03 INFO c1\n").unwrap();
    store.refresh(&[a.clone(), b.clone()]).unwrap().wait().unwrap();
    let s = store.summary().unwrap();
    assert_eq!(s.total, 3, "a1, a2, c1");
    let mut names: Vec<String> = s.sources.iter().map(|x| x.name.rsplit('/').next().unwrap().to_string()).collect();
    names.sort();
    assert_eq!(names, ["a.log", "c.log"]);
    assert_eq!(store.positioned().unwrap(), 3);
}
