//! Full-text search (ADR 0014): words across punctuation, case-insensitive; the index is a cache that is
//! rebuilt when missing and follows removals.

mod common;

use common::TempDir;
use imt_core::ports::{Ingest, Query, Search, SearchRequest};
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::DuckStore;
use std::time::Duration;

fn search(store: &DuckStore, text: &str) -> u64 {
    store
        .search(&SearchRequest {
            text: text.into(),
            limit: 0,
        })
        .unwrap()
        .total
}

#[test]
fn finds_words_across_punctuation_and_follows_the_data() {
    let logs = TempDir::new("search-logs");
    let mut a = String::new();
    for i in 0..500 {
        a.push_str(&format!("2026-10-14 10:00:{:02} INFO request {i} handled\n", i % 60));
        if i % 50 == 0 {
            a.push_str(&format!(
                "2026-10-14 10:01:{:02} ERROR GRAPHIC.BUILDER failed for job {i}\n",
                i % 60
            ));
        }
    }
    std::fs::write(logs.0.join("a.log"), &a).unwrap();
    std::fs::write(
        logs.0.join("b.clef"),
        "{\"@t\":\"2026-10-14T10:02:00Z\",\"@mt\":\"Timeout in {Service}\",\"Service\":\"Graphic-Builder\"}\n",
    )
    .unwrap();
    let dir = TempDir::new("search-store");
    let store = DuckStore::create(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    let status = store.wait_for_index(Duration::from_secs(60)).unwrap();
    assert!(status.ready, "{status:?}");
    assert_eq!(status.indexed, status.total);
    assert_eq!(search(&store, "graphic builder"), 11, "10 text events and the JSON one");
    assert_eq!(search(&store, "GRAPHIC.BUILDER failed"), 10);
    assert_eq!(search(&store, "\"builder failed\""), 10, "a phrase");
    assert_eq!(search(&store, "handled"), 500);
    assert_eq!(search(&store, "nothing-like-this"), 0);
    assert!(
        search(&store, "unbalanced \"quote AND (") <= status.total,
        "lenient syntax never fails"
    );
    // Hits come in time order, bounded by the limit.
    let r = store
        .search(&SearchRequest {
            text: "handled".into(),
            limit: 3,
        })
        .unwrap();
    assert_eq!((r.total, r.ids.len()), (500, 3));
    // A removed source leaves the index.
    let b = store.summary().unwrap().sources.into_iter().find(|s| s.name == "b.clef").unwrap();
    store.remove_source(imt_core::batch::SourceId(b.id)).unwrap();
    assert_eq!(search(&store, "graphic builder"), 10);
    drop(store);
    // Reopened: the index is current and kept.
    let store = DuckStore::open(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    assert_eq!(search(&store, "graphic builder"), 10);
    drop(store);
    // Deleted (a shared project has no index): rebuilt in the background.
    std::fs::remove_dir_all(dir.0.join("index")).unwrap();
    let store = DuckStore::open(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    assert!(store.wait_for_index(Duration::from_secs(60)).unwrap().ready);
    assert_eq!(search(&store, "graphic builder"), 10);
    assert_eq!(search(&store, "handled"), 500);
    store.clear().unwrap();
    assert_eq!(search(&store, "handled"), 0);
}
