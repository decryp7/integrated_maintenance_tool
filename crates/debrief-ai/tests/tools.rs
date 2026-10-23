//! The read-only engine tools over the incident in /samples: output is bounded, errors are readable, and
//! nothing changes.

mod common;
use common::*;
use debrief_ai::assistant::*;
use serde_json::{json, Value};

fn tools(limits: ToolLimits) -> EngineTools<'static> {
    EngineTools::new(store(), limits)
}

fn call(name: &str, input: Value) -> ToolOutcome {
    tools(ToolLimits::for_context(8192)).call(name, &input)
}

fn parsed(o: &ToolOutcome) -> Value {
    serde_json::from_str(&o.text).unwrap_or_else(|e| panic!("{e}: {}", o.text))
}

#[test]
fn run_query_returns_ids_a_sample_and_evidence() {
    let o = call("run_query", json!({"ql": "level >= error", "limit": 5}));
    let v = parsed(&o);
    assert_eq!(v["kind"], "events");
    assert!(v["total"].as_u64().unwrap() > 5 && v["ids"].as_array().unwrap().len() <= 5);
    assert!(v["sample"][0]["message"].is_string() && v["sample"][0].get("template").is_none());
    assert_eq!(o.evidence.queries, ["level >= error"]);
    assert!(!o.evidence.record_ids.is_empty() && o.summary.ends_with("events"));
    let t = call(
        "run_query",
        json!({"ql": "level >= warning | stats count() as n by MachineName | sort n desc"}),
    );
    let v = parsed(&t);
    assert_eq!((v["kind"].as_str(), v["columns"][0].as_str()), (Some("table"), Some("MachineName")));
    assert!(t.summary.ends_with("rows"));
    let bad = call("run_query", json!({"ql": "level >"}));
    assert!(bad.is_error && bad.text.contains("query error") && bad.text.contains("at byte"));
    assert!(call("run_query", json!({"limit": 3})).text.contains("invalid arguments"));
}

#[test]
fn check_query_reports_validity() {
    assert_eq!(parsed(&call("check_query", json!({"ql": "level >= warning"})))["valid"], true);
    let v = parsed(&call("check_query", json!({"ql": "level >="})));
    assert_eq!(v["valid"], false);
    assert!(v["error"].is_string() && v["offset"].is_number());
}

#[test]
fn patterns_exceptions_overview_and_moment() {
    let p = call("patterns", json!({"ql": "MachineName = WEB02", "limit": 3}));
    let v = parsed(&p);
    assert!(v["patterns"].as_array().unwrap().len() <= 4 && v["total"].as_u64().unwrap() > 3);
    assert!(p.summary.contains("patterns") && p.evidence.queries == ["MachineName = WEB02"]);
    assert!(call("patterns", json!({})).evidence.queries.is_empty());
    assert!(call("patterns", json!({"ql": "(("})).is_error);
    let g = call("exception_groups", json!({}));
    assert!(parsed(&g)["groups"][0]["typeName"].is_string() && g.summary.contains("exception groups"));
    assert!(call("exception_groups", json!({"ql": "(("})).is_error);
    let o = call("overview", json!({}));
    assert!(!parsed(&o)["findings"].as_array().unwrap().is_empty() && o.summary.contains("findings"));
    let m = call("moment", json!({"from": ms(10, 30, 0), "to": ms(10, 50, 0)}));
    assert!(parsed(&m)["total"].as_u64().unwrap() > 0 && m.summary.contains("events in the window"));
    assert!(call("moment", json!({"from": "x"})).is_error);
}

#[test]
fn events_stats_and_neighbours() {
    let id = first_id("level = fatal");
    let e = call("get_events", json!({"ids": [id, id, 4_000_000]}));
    let v = parsed(&e);
    assert_eq!((v["requested"].as_u64(), v["returned"].as_u64()), (Some(2), Some(1)));
    assert_eq!(v["events"][0]["id"], id);
    assert_eq!(e.evidence.record_ids, [id]);
    // at most 50 ids are looked at, and the output is cut to the budget
    let many: Vec<u32> = (0..200).collect();
    let tiny = ToolLimits {
        max_chars: 1200,
        max_items: 3,
        max_str: 80,
    };
    let o = EngineTools::new(store(), tiny).call("get_events", &json!({"ids": many}));
    assert!(o.text.chars().count() <= 1200 + 15, "{}", o.text.len());
    assert!(
        parsed(&call("get_events", json!({"ids": (0..80).collect::<Vec<u32>>()})))["requested"]
            .as_u64()
            .unwrap()
            <= 50
    );
    assert!(call("get_events", json!({"ids": "no"})).is_error);

    let s = call("field_stats", json!({"field": "MachineName", "ql": "level >= error"}));
    assert!(parsed(&s)["top"][0]["value"].is_string() || parsed(&s)["top"][0].is_object());
    assert!(s.summary.contains("MachineName"));
    assert!(call("field_stats", json!({"field": "x", "ql": "(("})).is_error);

    let n = call("neighbours", json!({"record_id": id, "before": 2, "after": 50}));
    assert!(parsed(&n)["lines"].as_array().unwrap().len() >= 3);
    assert_eq!(n.evidence.record_ids, [id]);
    assert!(call("neighbours", json!({"record_id": 4_000_000})).is_error);
}

#[test]
fn unknown_tools_and_helpers() {
    assert!(call("rm_rf", json!({})).text.contains("unknown tool"));
    let specs = EngineTools::tool_specs();
    assert_eq!(specs.len(), 11);
    assert!(specs
        .iter()
        .all(|s| s.input_schema["type"] == "object" && !s.description.is_empty()));
    assert!(tools(ToolLimits::for_context(8192)).specs().len() == 11);
    let l = ToolLimits::for_context(4096);
    assert!(l.max_chars < ToolLimits::for_context(32768).max_chars && l.max_items >= 3);
    assert_eq!(ToolLimits::for_context(1).max_chars, 1200);
    assert_eq!(truncate_chars("abcdef", 3), "abc…[truncated]");
    assert_eq!(truncate_chars("abc", 3), "abc");
    let clipped = clip_json(
        &json!({"a": (0..10).collect::<Vec<_>>(), "s": "x".repeat(500), "n": 5, "deep": [[[[[[[[[1]]]]]]]]]}),
        &ToolLimits {
            max_chars: 1000,
            max_items: 2,
            max_str: 20,
        },
        0,
    );
    assert_eq!(clipped["a"].as_array().unwrap().len(), 3);
    assert_eq!(clipped["a"][2]["_more"], 8);
    assert!(clipped["s"].as_str().unwrap().ends_with("…[truncated]") && clipped["n"] == 5);
    assert!(clipped["deep"].to_string().contains("items") || clipped["deep"].to_string().contains('1'));
    let mut ids = Vec::new();
    collect_ids(
        &json!({"rows": [{"id": 3}, {"sampleIds": [4, 5]}], "firstError": 9, "other": {"recordId": 7}}),
        &mut ids,
    );
    ids.sort();
    assert_eq!(ids, [3, 4, 5, 7, 9]);
    let mut e = Evidence::default();
    e.merge(&Evidence {
        record_ids: vec![1, 2],
        queries: vec!["q".into()],
        sql: vec![],
    });
    e.merge(&Evidence {
        record_ids: vec![2, 3],
        queries: vec!["q".into(), "r".into()],
        sql: vec![],
    });
    assert_eq!((e.record_ids, e.queries), (vec![1, 2, 3], vec!["q".to_string(), "r".to_string()]));
    assert_eq!(
        Proposal::Query(QueryProposal {
            ql: "x".into(),
            explanation: String::new(),
            estimated_matches: None,
            repaired: false
        })
        .title(),
        "Query"
    );
}

/// A store that can be swapped between tool calls, like a workspace that live tail updates.
struct Swappable(std::sync::Mutex<Box<dyn debrief_core::ports::Store>>);
impl debrief_ai::StoreAccess for Swappable {
    fn visit(&self, f: &mut dyn FnMut(&dyn debrief_core::ports::Store)) {
        f(self.0.lock().unwrap().as_ref())
    }
}

fn workspace(dir: &str) -> Box<dyn debrief_core::ports::Store> {
    let mut ws = debrief_core::Workspace::new(debrief_core::Profile::default_profile()).unwrap();
    ws.open(&[samples().join(dir)]).unwrap();
    Box::new(debrief_core::workspace::WorkspaceStore::new(ws))
}

#[test]
fn citations_are_rechecked_by_content_after_the_store_changed() {
    use debrief_ai::assistant::Citation;
    use debrief_ai::assistant::ToolHost;
    let access = Swappable(std::sync::Mutex::new(workspace("serilog-text")));
    let mut t = EngineTools::new(&access as &dyn debrief_ai::StoreAccess, ToolLimits::for_context(8192));
    let o = t.call("get_events", &json!({"ids": [0, 1]}));
    assert_eq!(o.evidence.record_ids, [0, 1]);
    let cite = |ids: &[u32]| -> Vec<Citation> { ids.iter().map(|id| Citation::Record { id: *id, verified: true }).collect() };
    // Nothing changed: still verified.
    let mut c = cite(&[0, 1]);
    assert_eq!(t.recheck_citations(&mut c), 0);
    assert!(c.iter().all(|c| matches!(c, Citation::Record { verified: true, .. })));
    // The workspace now holds other files: id 0 names a different event, a large id does not exist.
    *access.0.lock().unwrap() = workspace("csv");
    let mut c = cite(&[0, 999_999]);
    c.push(Citation::Query {
        ql: "level >= error".into(),
    });
    assert_eq!(t.recheck_citations(&mut c), 2);
    assert!(c[..2].iter().all(|c| matches!(c, Citation::Record { verified: false, .. })));
    assert!(matches!(c[2], Citation::Query { .. }));
}
