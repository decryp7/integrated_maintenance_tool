//! Span rule inference on the store: example start/end events picked in the grid become a rule that is
//! evaluated on the data, applied as a profile override and then shows in the Runs table.

mod common;

use common::TempDir;
use imt_core::batch::EventId;
use imt_core::ports::{Analysis, Filter, Ingest, PageQuery, Profiles, Query};
use imt_core::spans::infer::{SpanExample, SpanHints, SpanRuleCheck, SpanRuleRequest};
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::DuckStore;

fn line(minute: u32, msg: &str) -> String {
    format!("2026-10-14 10:{minute:02}:00.000 +00:00 [INF] {msg} {{\"MachineName\": \"UI01\", \"ProcessId\": 7}}\n")
}

fn id_of(store: &DuckStore, text: &str) -> EventId {
    let page = store
        .page(&PageQuery {
            query: Filter {
                text: text.into(),
                case_sensitive: true,
                ..Filter::default()
            },
            limit: 1,
            ..PageQuery::default()
        })
        .unwrap();
    page.rows.first().unwrap_or_else(|| panic!("no event \"{text}\"")).id
}

#[test]
fn a_rule_is_proposed_from_examples_evaluated_and_applied() {
    let dir = TempDir::new("spans");
    let logs = dir.0.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let text: String = [
        line(0, "View XXXX1 loaded"),
        line(1, "View XXXX2 loaded"),
        line(2, "rendering"),
        line(3, "View XXXX1 closed"),
        line(4, "View XXXX3 loaded"),
        line(5, "View XXXX2 closed"),
        line(6, "View XXXX3 closed"),
        line(7, "View XXXX9 closed"),
        line(8, "View XXXX4 loaded"),
    ]
    .concat();
    std::fs::write(logs.join("ui.log"), text).unwrap();
    let store = DuckStore::create(&dir.0.join("store"), Profile::default_profile(), PluginRegistry::new()).unwrap();
    store.open(&[logs]).unwrap().wait().unwrap();
    let req = SpanRuleRequest {
        examples: vec![SpanExample {
            start: id_of(&store, "XXXX1 loaded"),
            end: id_of(&store, "XXXX1 closed"),
        }],
        hints: SpanHints {
            name: Some("view".into()),
            key_field: None,
        },
    };
    let proposal = store.propose_span_rule(&req).unwrap();
    let best = &proposal.candidates[0];
    assert_eq!(best.candidate.example_keys, vec!["XXXX1"], "{proposal:#?}");
    let e = &best.evaluation;
    assert_eq!((e.spans, e.unmatched_ends, e.open, e.distinct_keys), (3, 1, 1, 4), "{e:#?}");
    assert_eq!(e.max_concurrent, 2);
    assert_eq!(e.longest_ms, Some(4 * 60_000)); // XXXX2
                                                // An AI-proposed rule is checked the same way.
    let check = SpanRuleCheck {
        profile_override: best.candidate.profile_override.clone(),
        kind: "view".into(),
    };
    let again = store.evaluate_span_rule(&check).unwrap();
    assert_eq!(&again.result, e);
    assert!(again.coverage.complete());
    let wrong = SpanRuleCheck {
        profile_override: "[[span]]\nkind = \"x\"\nstart = \"nope\"\nstop = \"nope\"".into(),
        kind: "x".into(),
    };
    assert!(store.evaluate_span_rule(&wrong).is_err());
    // Applied through the profile-override path, the runs show in the Runs table.
    let profile = store
        .active_profile()
        .unwrap()
        .with_override(&best.candidate.profile_override)
        .unwrap();
    store.set_profile(profile).unwrap().wait().unwrap();
    let runs = store
        .runs(&serde_json::from_value(serde_json::json!({"kind": "view"})).unwrap())
        .unwrap();
    assert_eq!(runs.total, 5, "{:#?}", runs.runs); // 3 closed, 1 without a start, 1 open
    assert!(runs.summaries.iter().any(|s| s.kind == "view"));
}
