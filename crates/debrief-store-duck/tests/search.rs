//! Full-text search (ADR 0014): words across punctuation, case-insensitive; the index is a cache that is
//! rebuilt when missing and follows removals.

mod common;

use common::TempDir;
use debrief_core::ports::{Ingest, Query, Search, SearchRequest};
use debrief_core::{PluginRegistry, Profile};
use debrief_store_duck::DuckStore;
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
    assert_eq!(search(&store, "\"graphic builder\""), 11, "a phrase ignores separators: GRAPHIC.BUILDER, Graphic-Builder");
    assert_eq!(search(&store, "failed builder"), 10, "words in any order");
    assert_eq!(search(&store, "\"failed builder\""), 0, "a phrase is checked on the rows");
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
    store.remove_source(debrief_core::batch::SourceId(b.id)).unwrap();
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

#[test]
fn smart_search_filters_by_words_and_counts_partial_matches() {
    use debrief_core::ports::{Filter, PageQuery};
    use debrief_core::query::TextMode;
    let logs = TempDir::new("smart-logs");
    std::fs::write(
        logs.0.join("a.log"),
        [
            "2026-10-14 10:00:00 INFO time out waiting",
            "2026-10-14 10:00:01 ERROR runtime error in Graphic.Builder",
            "2026-10-14 10:00:02 ERROR System.TimeoutException at Foo",
            "2026-10-14 10:00:03 INFO timeouts are counted",
        ]
        .join("\n"),
    )
    .unwrap();
    let dir = TempDir::new("smart-store");
    let store = DuckStore::create(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    store.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    assert!(store.wait_for_index(Duration::from_secs(60)).unwrap().ready);
    let filter = |text: &str, text_mode| Filter {
        text: text.into(),
        text_mode,
        ..Filter::default()
    };
    let page = |f: Filter| {
        store
            .page(&PageQuery {
                query: f,
                ..PageQuery::default()
            })
            .unwrap()
    };
    let messages = |f: Filter| page(f).rows.into_iter().map(|r| r.message).collect::<Vec<_>>();
    // The last word is a prefix; whole words only, so "runtime" is not "time".
    assert_eq!(
        messages(filter("time", TextMode::Words)),
        ["time out waiting", "System.TimeoutException at Foo", "timeouts are counted"]
    );
    assert_eq!(
        messages(filter("timeout exception", TextMode::Words)),
        ["System.TimeoutException at Foo"]
    );
    assert_eq!(
        messages(filter("graphic builder", TextMode::Words)),
        ["runtime error in Graphic.Builder"]
    );
    // Partial matches: "untime" is only a substring.
    assert!(page(filter("untime", TextMode::Words)).rows.is_empty());
    let partial = store.partial_matches(&filter("untime", TextMode::Words)).unwrap();
    assert_eq!((partial.added, partial.complete), (1, true));
    let rows = page(filter("time", TextMode::WordsOrSubstring)).rows;
    let kinds: Vec<_> = rows.iter().map(|r| r.search_match).collect();
    use debrief_core::ports::SearchMatch::{Partial, Words};
    assert_eq!(
        kinds,
        [Some(Words), Some(Partial), Some(Words), Some(Words)],
        "the runtime line is a partial match"
    );
    assert_eq!(store.partial_matches(&filter("time", TextMode::Words)).unwrap().added, 1);
    assert_eq!(store.partial_matches(&filter("", TextMode::Words)).unwrap().added, 0);
    // A plain substring search is unmarked.
    assert!(page(filter("time", TextMode::Substring))
        .rows
        .iter()
        .all(|r| r.search_match.is_none()));
    // Cancelled: stops with a typed error.
    let control = debrief_core::ql::QueryControl::new();
    control.cancel();
    let cancelled = control.scope(|| store.partial_matches(&filter("time", TextMode::Words)));
    assert!(
        matches!(cancelled, Err(debrief_core::ports::StoreError::Cancelled)),
        "{cancelled:?}"
    );
}
