//! Time-window import (ADR 0047): only events inside the window reach the store; old files are not opened.

use crate::common;

use common::{store_with, TempDir};
use debrief_core::ingest::ImportWindow;
use debrief_core::ports::{Ingest, Query, StoreError};
use debrief_core::PluginRegistry;
use debrief_core::Profile;
use debrief_store_duck::DuckStore;
use std::path::PathBuf;

const HOUR: i64 = 3_600_000;

fn log(dir: &TempDir, name: &str) -> PathBuf {
    let mut text = String::new();
    for h in 0..6 {
        for m in 0..10 {
            text.push_str(&format!("2026-10-14 {h:02}:{:02}:00.000 +00:00 [INF] tick {h}:{m}\n", m * 5));
        }
    }
    let p = dir.0.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

fn total(store: &DuckStore) -> usize {
    store.summary().unwrap().total
}

#[test]
fn only_events_inside_the_window_are_imported() {
    let data = TempDir::new("win-data");
    let file = log(&data, "a.log");
    let dir = TempDir::new("win-store");
    let store = DuckStore::create(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    let from = DAY_OF_LOG + 2 * HOUR;
    let to = DAY_OF_LOG + 3 * HOUR + 30 * 60_000;
    store
        .set_import_window(&ImportWindow {
            from: Some(from),
            to: Some(to),
        })
        .unwrap();
    let report = store.open(std::slice::from_ref(&file)).unwrap().wait().unwrap();
    // 02:00 .. 03:30 inclusive at five-minute steps: 02:00-02:45 (10) + 03:00-03:30 (7).
    assert_eq!(report.events, 17);
    assert_eq!(total(&store), 17);
    let s = store.summary().unwrap();
    assert_eq!((s.from, s.to), (Some(from), Some(to)));
    assert_eq!(s.untimed, 0, "an event without a time is outside any window");
}

#[test]
fn an_open_ended_window_and_clearing_it() {
    let data = TempDir::new("win-data2");
    let file = log(&data, "b.log");
    let dir = TempDir::new("win-store2");
    let (store, _) = store_with(&dir, Profile::default_profile(), &[]).unwrap();
    store
        .set_import_window(&ImportWindow {
            from: Some(DAY_OF_LOG + 5 * HOUR),
            to: None,
        })
        .unwrap();
    assert_eq!(store.open(std::slice::from_ref(&file)).unwrap().wait().unwrap().events, 10);
    store.clear().unwrap();
    store.set_import_window(&ImportWindow::default()).unwrap();
    assert_eq!(store.open(&[file]).unwrap().wait().unwrap().events, 60);
}

#[test]
fn a_backwards_window_is_refused() {
    let dir = TempDir::new("win-store3");
    let (store, _) = store_with(&dir, Profile::default_profile(), &[]).unwrap();
    let e = store
        .set_import_window(&ImportWindow {
            from: Some(5),
            to: Some(1),
        })
        .unwrap_err();
    assert!(matches!(&e, StoreError::Invalid(m) if m.contains("before")), "{e:?}");
}

#[test]
fn files_that_stopped_growing_before_the_window_are_not_read() {
    let data = TempDir::new("win-data4");
    let old = log(&data, "old.log");
    let fresh = log(&data, "fresh.log");
    let long_ago = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(long_ago)
        .unwrap();
    let dir = TempDir::new("win-store4");
    let (store, _) = store_with(&dir, Profile::default_profile(), &[]).unwrap();
    store
        .set_import_window(&ImportWindow {
            from: Some(DAY_OF_LOG),
            to: None,
        })
        .unwrap();
    let report = store.open(&[old, fresh]).unwrap().wait().unwrap();
    assert_eq!(
        report.sources.len(),
        1,
        "{:?}",
        report.sources.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    assert!(report.sources[0].name.ends_with("fresh.log"));
    assert!(report.unrecognized.is_empty(), "skipping by age is not a failure");
}

/// 2026-10-14 00:00:00 UTC in epoch ms.
const DAY_OF_LOG: i64 = 1_791_936_000_000;
