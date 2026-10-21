//! Live tail and restore on the store: appended lines are read again into new segments (and searchable),
//! new files under an opened folder are picked up, a truncated file is read from its start, and a saved
//! workspace state imports its roots without the files it removed.

mod common;

use common::{store_with, TempDir};
use imt_core::api::WorkspaceState;
use imt_core::ports::{Ingest, Query, Search, SearchRequest};
use imt_core::Profile;
use std::io::Write;
use std::time::Duration;

fn append(path: &std::path::Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

#[test]
fn appended_lines_new_files_and_truncation_are_followed() {
    let logs = TempDir::new("tail-logs");
    let a = logs.0.join("a.log");
    std::fs::write(&a, "2026-10-14 10:00:00 INFO first\n").unwrap();
    let dir = TempDir::new("tail-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    assert_eq!(store.summary().unwrap().total, 1);

    append(
        &a,
        "2026-10-14 10:00:01 ERROR second with zebrafish\n2026-10-14 10:00:02 INFO third\n",
    );
    assert_eq!(store.append_tail(std::slice::from_ref(&a)).unwrap(), 2);
    assert_eq!(store.summary().unwrap().total, 3);
    store.wait_for_index(Duration::from_secs(60)).unwrap();
    let hits = store
        .search(&SearchRequest {
            text: "zebrafish".into(),
            limit: 5,
        })
        .unwrap();
    assert_eq!(hits.total, 1, "appended events are searchable");

    // A new file in the opened folder.
    let b = logs.0.join("b.log");
    std::fs::write(&b, "2026-10-14 10:00:03 WARN fourth\n").unwrap();
    assert_eq!(store.append_tail(std::slice::from_ref(&b)).unwrap(), 1);
    assert_eq!(store.summary().unwrap().sources.len(), 2);

    // Truncated (copy-truncate rotation): read again from its start.
    std::fs::write(&a, "2026-10-14 11:00:00 INFO fresh\n").unwrap();
    store.append_tail(std::slice::from_ref(&a)).unwrap();
    assert_eq!(store.summary().unwrap().total, 2, "a's three events were replaced by its one");

    // Nothing changed: nothing added.
    assert_eq!(store.append_tail(&[]).unwrap(), 0);
}

#[test]
fn a_saved_state_is_restored_without_its_removed_files() {
    let logs = TempDir::new("restore-logs");
    std::fs::write(logs.0.join("a.log"), "2026-10-14 10:00:00 INFO a\n").unwrap();
    std::fs::write(logs.0.join("b.log"), "2026-10-14 10:00:00 INFO b\n2026-10-14 10:00:01 INFO b2\n").unwrap();
    let dir = TempDir::new("restore-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), &[]).unwrap();
    let removed = std::fs::canonicalize(logs.0.join("b.log")).unwrap();
    let state = WorkspaceState {
        roots: vec![logs.0.to_string_lossy().into_owned()],
        removed: vec![removed.to_string_lossy().into_owned()],
        offsets: Default::default(),
    };
    store.restore(&state).unwrap().wait().unwrap();
    let s = store.summary().unwrap();
    assert_eq!((s.total, s.sources.len()), (1, 1));
}
