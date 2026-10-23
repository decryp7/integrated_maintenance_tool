//! The in-memory workspace behind the store ports, part two: clock skew, live tail, partial search matches,
//! redaction summaries, cancelled queries and the operations it leaves to a real store. Data is invented per test.

use debrief_core::api::QueryRequest;
use debrief_core::ports::{
    Diagnostics, Ingest, PartialMatches, Profiles, Projects, Query as QueryPort, QuickFix, Redaction, Search, Skew, StoreError, Tail,
    TailRead,
};
use debrief_core::ql::QueryControl;
use debrief_core::query::{Query, TextMode};
use debrief_core::skew::{ClockOffsets, OffsetSpec, SetOffsetRequest, SuggestRequest};
use debrief_core::workspace::WorkspaceStore;
use debrief_core::{Profile, Workspace};
use std::path::{Path, PathBuf};

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("debrief-wsports-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(std::fs::canonicalize(p).unwrap())
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, text).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const BASE: i64 = 1_791_972_000_000; // 2026-10-14T10:00:00Z

fn clef(ms: i64, msg: &str, corr: &str) -> String {
    let t = chrono::DateTime::from_timestamp_millis(BASE + ms)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ");
    format!("{{\"@t\":\"{t}\",\"@m\":\"{msg}\",\"CorrelationId\":\"{corr}\"}}\n")
}

fn fresh() -> WorkspaceStore {
    WorkspaceStore::new(Workspace::new(Profile::default_profile()).unwrap())
}

fn two_servers(d: &Dir) -> (PathBuf, PathBuf) {
    // Server B's clock runs 4 s behind A's.
    let a: String = (0..10).map(|i| clef(i * 1000, &format!("A {i}"), &format!("c{i}"))).collect();
    let b: String = (0..10)
        .map(|i| clef(i * 1000 + 20 - 4_000, &format!("B {i}"), &format!("c{i}")))
        .collect();
    (d.write("a.log", &a), d.write("b.log", &b))
}

fn source(store: &WorkspaceStore, name: &str) -> u32 {
    store.summary().unwrap().sources.iter().find(|s| s.name.ends_with(name)).unwrap().id
}

#[test]
fn clock_offsets_are_suggested_set_kept_and_refused_through_the_ports() {
    let d = Dir::new("skew");
    let (a, b) = two_servers(&d);
    let store = fresh();
    store.open(&[a, b]).unwrap().wait().unwrap();
    let (sa, sb) = (source(&store, "a.log"), source(&store, "b.log"));
    assert!(store.clock_offsets().unwrap().is_empty());

    let suggestion = store.suggest_offsets(&SuggestRequest::default()).unwrap();
    let for_b = suggestion.suggestions.iter().find(|s| s.source == sb).expect("a suggestion for B");
    assert_eq!(for_b.suggested_ms, 3_980);
    assert_eq!(suggestion.reference, Some(sa));

    let change = store
        .set_source_offset(&SetOffsetRequest {
            spec: OffsetSpec::Manual {
                source: sb,
                offset_ms: for_b.suggested_ms,
            },
        })
        .unwrap();
    assert_eq!((change.source, change.offset_ms, change.previous_ms), (sb, 3_980, 0));
    let offsets = store.clock_offsets().unwrap();
    assert_eq!(offsets.iter().count(), 1);

    // An offset beyond the limit is refused with a message, and changes nothing.
    let err = store
        .set_source_offset(&SetOffsetRequest {
            spec: OffsetSpec::Manual {
                source: sb,
                offset_ms: i64::MAX / 4,
            },
        })
        .unwrap_err();
    assert!(matches!(err, StoreError::Invalid(ref m) if m.contains("too large")), "{err:?}");
    assert_eq!(store.clock_offsets().unwrap(), offsets);

    // Replacing the offsets (a project's) moves the events back.
    store.set_clock_offsets(&ClockOffsets::default()).unwrap();
    assert!(store.clock_offsets().unwrap().is_empty());
    let after = store.suggest_offsets(&SuggestRequest::default()).unwrap();
    assert_eq!(
        after
            .suggestions
            .iter()
            .find(|s| s.source == source(&store, "b.log"))
            .unwrap()
            .current_ms,
        0
    );
}

#[test]
fn opening_at_the_end_follows_only_what_is_written_next() {
    let d = Dir::new("tail");
    let path = d.write(
        "live.log",
        &(0..5).map(|i| clef(i * 1000, &format!("old {i}"), "c")).collect::<String>(),
    );
    let store = fresh();
    store.open_at_end(std::slice::from_ref(&path)).unwrap().wait().unwrap();
    assert_eq!(store.summary().unwrap().total, 0, "nothing before the end is read");
    let files = store.tail_files().unwrap();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].read, TailRead::Append);
    assert_eq!(files[0].size, std::fs::metadata(&path).unwrap().len());
    assert!(files[0].name.ends_with("live.log"));

    // A line written afterwards is the only event.
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(&clef(9_000, "new", "c"));
    std::fs::write(&path, text).unwrap();
    store.append_tail(&[path]).unwrap();
    assert_eq!(store.summary().unwrap().total, 1);

    // Opening a folder that does not exist is an error with a message, not a panic.
    let err = store
        .open_at_end(&[PathBuf::from("/no/such/dir/debrief")])
        .err()
        .expect("a missing path is an error");
    assert!(!err.to_string().is_empty());
}

#[test]
fn a_file_that_cannot_be_followed_from_the_middle_is_listed_as_read_whole() {
    let d = Dir::new("tail-whole");
    // One JSON document is read in full whenever it changes.
    let path = d.write(
        "doc.json",
        "[{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"one\"},{\"@t\":\"2026-10-14T10:00:01Z\",\"@m\":\"two\"}]",
    );
    let store = fresh();
    store.open_at_end(std::slice::from_ref(&path)).unwrap().wait().unwrap();
    let files = store.tail_files().unwrap();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].read, TailRead::Whole);
    assert_eq!(store.summary().unwrap().total, 2, "the file was read in full");
}

#[test]
fn partial_matches_count_substring_hits_that_a_word_search_misses() {
    let d = Dir::new("partial");
    let text: String = [("the abc ready", 1), ("abcdef done", 2), ("xabcx noise", 3), ("nothing", 4)]
        .iter()
        .map(|(m, i)| clef(*i * 1000, m, "c"))
        .collect();
    let store = fresh();
    store.open(&[d.write("w.log", &text)]).unwrap().wait().unwrap();
    let filter = Query {
        text: "abc".into(),
        text_mode: TextMode::Words,
        ..Query::default()
    };
    let m: PartialMatches = store.partial_matches(&filter).unwrap();
    assert_eq!(
        (m.added, m.complete),
        (1, true),
        "'xabcx' holds 'abc' only inside a word; 'abcdef' starts with it"
    );
    let none = Query {
        text: "zzz".into(),
        ..Query::default()
    };
    assert_eq!(store.partial_matches(&none).unwrap().added, 0);
    // A bad regex is reported, not panicked on.
    let bad = Query {
        text: "(".into(),
        regex: true,
        ..Query::default()
    };
    assert!(matches!(store.partial_matches(&bad), Err(StoreError::Invalid(_))));
}

#[test]
fn a_cancelled_run_reports_cancelled_and_the_store_stays_usable() {
    let d = Dir::new("cancel");
    let store = fresh();
    store
        .open(&[d.write("c.log", &(0..20).map(|i| clef(i * 1000, "m", "c")).collect::<String>())])
        .unwrap()
        .wait()
        .unwrap();
    let req = QueryRequest {
        query: Query {
            ql: "level >= information | stats count()".into(),
            ..Query::default()
        },
    };
    let control = QueryControl::new();
    control.cancel();
    assert_eq!(store.run_query_with(&req, &control).unwrap_err(), StoreError::Cancelled);
    assert!(store.run_query(&req).is_ok());
}

#[test]
fn redaction_summaries_and_unsupported_operations_through_the_ports() {
    let d = Dir::new("redact");
    let profile = Profile::from_toml("name = 'r'\n[redact]\ndetectors = ['email']").unwrap();
    let store = WorkspaceStore::new(Workspace::new(profile).unwrap());
    let text: String = [("mail ann@example.org", 1), ("quiet", 2)]
        .iter()
        .map(|(m, i)| clef(*i * 1000, m, "c"))
        .collect();
    store.open(&[d.write("r.log", &text)]).unwrap().wait().unwrap();
    let req = serde_json::from_value(serde_json::json!({"samples": 3})).unwrap();
    let s = store.redaction_summary(&req).unwrap();
    assert_eq!((s.scanned, s.changed), (2, 1));
    assert_eq!(s.samples[0].after, "mail <email>");

    // What only a real store can do is Unsupported, with the feature named.
    let unsupported = |e: StoreError| assert!(matches!(e, StoreError::Unsupported { .. }), "{e}");
    unsupported(
        store
            .propose_span_rule(&serde_json::from_value(serde_json::json!({"examples": [], "hints": {}})).unwrap())
            .unwrap_err(),
    );
    unsupported(
        store
            .evaluate_span_rule(&serde_json::from_value(serde_json::json!({"profileOverride": "", "kind": "run"})).unwrap())
            .unwrap_err(),
    );
    unsupported(store.export_project(&Default::default(), Path::new("x.debriefz")).unwrap_err());
    unsupported(
        store
            .evaluate_fix(&serde_json::from_value::<QuickFix>(serde_json::json!({"parserOverride": ""})).unwrap())
            .unwrap_err(),
    );
    store.close();
}

#[test]
fn run_rule_drafts_are_previewed_through_the_profiles_port() {
    let d = Dir::new("preview");
    let store = fresh();
    store.open(&[d.write("p.log", &clef(0, "begin", "c"))]).unwrap().wait().unwrap();
    // A draft that cannot compile is answered with its problems, not with an error.
    let draft = serde_json::from_value(serde_json::json!({
        "draft": {"kind": "", "start": {"type": "markers", "kinds": ["start"]}, "stop": {"type": "markers", "kinds": ["stop"]}}
    }))
    .unwrap();
    let preview = debrief_core::ports::Profiles::preview_spans(&store, &draft).unwrap();
    assert!(!preview.errors.is_empty(), "{preview:?}");
    assert!(preview.runs.is_empty());
}
