//! "show me errors from graphic builder", end to end over a real store with invented events: a scripted model calls
//! `find_values` (the user's words, then the joined spelling `graphicbuilder.exe`), then `run_query` with the resolved
//! value and the level filter. The answer cites the returned event ids and the chat events carry the exact query,
//! which the UI shows as an editable chip (Open in Events, Edit).

use crate::duck_store;

use debrief_ai::assistant::{Assistant, AssistantConfig};
use debrief_ai::chat::*;
use debrief_ai::llm::{Reply, ScriptedProvider};
use duck_store::{clef, open, Fixture};
use serde_json::{json, Value};

const QL: &str = "level >= error and ProcessName = \"CompanyA.Graphic.Builder.exe\"";

fn fixture() -> Fixture {
    let rows = [
        ("2026-10-14T09:00:00.000Z", "Information", "Started", "agent.exe"),
        (
            "2026-10-14T09:00:01.000Z",
            "Error",
            "Render failed: out of texture memory",
            "CompanyA.Graphic.Builder.exe",
        ),
        (
            "2026-10-14T09:00:02.000Z",
            "Information",
            "Render queue drained",
            "CompanyA.Graphic.Builder.exe",
        ),
        (
            "2026-10-14T09:00:05.000Z",
            "Fatal",
            "Builder crashed",
            "CompanyA.Graphic.Builder.exe",
        ),
        ("2026-10-14T09:00:09.000Z", "Error", "Disk full", "agent.exe"),
        (
            "2026-10-14T09:00:11.000Z",
            "Warning",
            "Launched graphicbuilder.exe from the shortcut",
            "agent.exe",
        ),
    ];
    open(&[("host.log", clef(&rows))])
}

fn turn(f: &Fixture, script: Vec<Reply>, question: &str) -> Vec<ChatEvent> {
    let a = Assistant::new(ScriptedProvider::new(script), AssistantConfig::default());
    let env = ChatEnv::new(&a, f.store().into());
    let mut session = ChatSession::new("t");
    let mut events = Vec::new();
    session
        .send(&env, question, &ChatContext::default(), &mut |e| events.push(e.event.clone()))
        .unwrap();
    events
}

#[test]
fn errors_from_graphic_builder_resolve_the_name_run_the_query_and_cite_the_events() {
    let f = fixture();
    let events = turn(
        &f,
        vec![
            Reply::Tools(
                String::new(),
                vec![
                    ("find_values".into(), json!({"text": "graphic builder"})),
                    ("find_values".into(), json!({"text": "graphicbuilder.exe"})),
                ],
            ),
            Reply::Tools(String::new(), vec![("run_query".into(), json!({"ql": QL}))]),
            Reply::Text("Graphic Builder logged 2 errors. See [#1] and [#3].".into()),
        ],
        "show me errors from graphic builder",
    );
    // The ids the query returned are what the answer cites.
    let finished: Vec<(&str, bool, &[String], &[u32])> = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::ToolFinished { name, error, evidence, .. } => {
                Some((name.as_str(), *error, &evidence.queries[..], &evidence.record_ids[..]))
            }
            _ => None,
        })
        .collect();
    let names: Vec<&str> = finished.iter().map(|f| f.0).collect();
    assert_eq!(names, ["find_values", "find_values", "run_query"]);
    assert!(finished.iter().all(|f| !f.1), "{finished:?}");
    let (_, _, queries, ids) = finished[2];
    // The chip: the exact query that ran, in the finished event.
    assert_eq!(queries, [QL.to_string()]);
    assert_eq!(ids.len(), 2, "the two errors of the builder: {ids:?}");
    let ChatEvent::Done { citations, .. } = events.last().unwrap() else {
        panic!("{events:?}")
    };
    let cited: Vec<u32> = citations
        .iter()
        .filter_map(|c| match c {
            debrief_ai::assistant::Citation::Record { id, verified: true } => Some(*id),
            _ => None,
        })
        .collect();
    assert!(
        !cited.is_empty() && cited.iter().all(|c| ids.contains(c)),
        "cited {cited:?} of {ids:?}"
    );
}

#[test]
fn both_spellings_resolve_to_values_the_query_can_use() {
    // What the model sees from find_values for the user's words and for the joined spelling.
    let f = fixture();
    let mut tools = debrief_ai::assistant::EngineTools::new(f.store(), debrief_ai::assistant::ToolLimits::for_context(8192));
    use debrief_ai::assistant::ToolHost;
    for text in ["graphic builder", "graphicbuilder.exe"] {
        let o = tools.call("find_values", &json!({"text": text}));
        assert!(!o.is_error, "{text}: {}", o.text);
        let v: Value = serde_json::from_str(&o.text).unwrap();
        let hit = v["values"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["value"] == "CompanyA.Graphic.Builder.exe");
        assert!(hit, "{text}: {v}");
    }
}
