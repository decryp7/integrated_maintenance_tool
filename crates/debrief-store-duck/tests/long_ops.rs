//! Long store operations: saving the project (export), compacting and rebuilding the index report progress, can be
//! cancelled, and leave the store consistent.

mod common;

use common::{samples, store_with, TempDir};
use debrief_core::package::{part_path, ProjectManifest};
use debrief_core::ports::{Maintain, MaintenanceAction, Query, Search, StoreError};
use debrief_core::ql::QueryControl;
use debrief_core::Profile;
use std::time::Duration;

fn opened(tag: &str) -> (TempDir, debrief_store_duck::DuckStore, usize) {
    let dir = TempDir::new(tag);
    let (store, report) = store_with(&dir, Profile::default_profile(), &[samples().join("serilog-clef")]).unwrap();
    store.wait_for_index(Duration::from_secs(60)).unwrap();
    (dir, store, report.events)
}

#[test]
fn a_cancelled_save_leaves_no_partial_file_and_the_store_still_works() {
    let (dir, store, events) = opened("save-cancel");
    let out = dir.0.join("saved.debriefz");
    let control = QueryControl::new();
    control.cancel();
    let mut manifest = ProjectManifest::default();
    let err = control.scope(|| store.export_project(&mut manifest, &out)).unwrap_err();
    assert_eq!(err, StoreError::Cancelled);
    assert!(!out.exists() && !part_path(&out).exists(), "no partial file");
    assert!(
        !dir.0
            .join("tmp")
            .read_dir()
            .is_ok_and(|mut d| d.any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with("export-")))),
        "the working copy is gone"
    );
    assert_eq!(store.summary().unwrap().total, events);
    // A later save is not affected, reports its bytes and writes the archive.
    let control = QueryControl::new();
    let bytes = control.scope(|| store.export_project(&mut manifest, &out)).unwrap();
    assert!(bytes > 0 && out.exists() && !part_path(&out).exists());
    assert!(control.progress().done > 0, "the archive's bytes were counted");
}

#[test]
fn cancelled_maintenance_changes_nothing() {
    let (_dir, store, events) = opened("maintain-cancel");
    let control = QueryControl::new();
    control.cancel();
    for action in [MaintenanceAction::Compact, MaintenanceAction::RebuildIndex] {
        let err = control.scope(|| store.maintain(action)).unwrap_err();
        assert_eq!(err, StoreError::Cancelled, "{action:?}");
    }
    // The index was not emptied by the cancelled rebuild.
    let status = store.index_status().unwrap();
    assert!(status.ready && status.indexed as usize == events, "{status:?}");
    assert_eq!(store.summary().unwrap().total, events);
}

#[test]
fn a_rebuild_reports_progress_until_the_index_covers_every_event() {
    let (_dir, store, events) = opened("rebuild-progress");
    let r = store.maintain(MaintenanceAction::RebuildIndex).unwrap();
    assert!(r.message.contains("again"), "{r:?}");
    let mut last = 0;
    let started = std::time::Instant::now();
    loop {
        let s = store.index_status().unwrap();
        assert_eq!(s.total as usize, events);
        assert!(s.indexed <= s.total, "{s:?}");
        if s.ready {
            // Ready is only reported once every event is searchable.
            assert_eq!(s.indexed as usize, events, "{s:?}");
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(60), "indexing did not finish: {s:?}");
        last = last.max(s.indexed);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(last as usize <= events);
}
