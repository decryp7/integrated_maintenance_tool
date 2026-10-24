//! "tell me about graphic builder": the log holds `CompanyA.Graphic.Builder.exe` (invented data), the user's words
//! have a space. A scripted model issues the calls a real one would; the tools must return the events.

use debrief_ai::assistant::{Assistant, AssistantConfig, EngineTools, ToolHost, ToolLimits};
use debrief_ai::chat::*;
use debrief_ai::llm::{Reply, ScriptedProvider};
use debrief_core::ports::Store;
use debrief_core::workspace::WorkspaceStore;
use debrief_core::{Profile, Workspace};
use serde_json::{json, Value};

fn store() -> WorkspaceStore {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("debrief-ai-find-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rows = [
        ("Started CompanyA.Graphic.Builder.exe pid 4242", "agent.exe"),
        ("Render queue drained", "CompanyA.Graphic.Builder.exe"),
        ("Render queue drained", "CompanyA.Graphic.Builder.exe"),
        ("Heartbeat", "agent.exe"),
    ];
    let text: String = rows
        .iter()
        .enumerate()
        .map(|(i, (m, p))| format!("{{\"@t\":\"2026-10-14T09:0{i}:00.000Z\",\"@m\":\"{m}\",\"ProcessName\":\"{p}\"}}\n"))
        .collect();
    std::fs::write(dir.join("host-agent.log"), text).unwrap();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    WorkspaceStore::new(ws)
}

fn call(s: &WorkspaceStore, name: &str, input: Value) -> Value {
    let store: &dyn Store = s;
    let mut t = EngineTools::new(store, ToolLimits::for_context(8192));
    let o = t.call(name, &input);
    assert!(!o.is_error, "{}", o.text);
    serde_json::from_str(&o.text).unwrap()
}

#[test]
fn the_phrase_the_user_typed_finds_the_dotted_name() {
    let v = call(&store(), "find_events", json!({"text": "graphic builder"}));
    assert_eq!((v["step"].as_str(), v["total"].as_u64()), (Some("phrase"), Some(1)), "{v}");
    assert!(
        v["note"]
            .as_str()
            .unwrap()
            .contains("ProcessName = CompanyA.Graphic.Builder.exe (2 events)"),
        "{v}"
    );
}

#[test]
fn the_old_contains_query_misses_which_is_why_find_events_exists() {
    let s = store();
    let v = call(&s, "run_query", json!({"ql": "message contains \"graphic builder\""}));
    assert_eq!(v["total"], 0, "substring cannot see through the dots");
    let tools = EngineTools::tool_specs();
    let find = tools.iter().find(|t| t.name == "find_events").unwrap();
    assert!(find.description.contains("Prefer this over `contains`"));
}

#[test]
fn find_values_resolves_the_words_to_the_process_value() {
    let v = call(&store(), "find_values", json!({"text": "graphic builder"}));
    let first = &v["values"][0];
    assert_eq!(
        (first["field"].as_str(), first["value"].as_str()),
        (Some("ProcessName"), Some("CompanyA.Graphic.Builder.exe"))
    );
    assert_eq!((first["count"].as_u64(), first["via"].as_str()), (Some(2), Some("parts")));
}

#[test]
fn a_model_resolves_the_name_then_queries_the_exact_value() {
    let s = store();
    let a = Assistant::new(
        ScriptedProvider::new(vec![
            Reply::Tools(String::new(), vec![("find_values".into(), json!({"text": "graphic builder"}))]),
            Reply::Tools(
                String::new(),
                vec![("run_query".into(), json!({"ql": "ProcessName = \"CompanyA.Graphic.Builder.exe\""}))],
            ),
            Reply::Text("CompanyA.Graphic.Builder.exe appears in 2 events.".into()),
        ]),
        AssistantConfig::default(),
    );
    let store: &dyn Store = &s;
    let env = ChatEnv::new(&a, store.into());
    let mut session = ChatSession::new("c1");
    let mut events = Vec::new();
    session
        .send(&env, "tell me about graphic builder", &ChatContext::default(), &mut |e| {
            events.push(e.clone())
        })
        .unwrap();
    let summaries: Vec<String> = events
        .iter()
        .filter_map(|e| match &e.event {
            ChatEvent::ToolFinished { summary, error: false, .. } => Some(summary.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(summaries, ["1 matching values", "2 events"]);
}

#[test]
fn find_values_returns_the_kind_and_where_a_name_in_message_text_was_seen() {
    // "Started CompanyA.Graphic.Builder.exe ..." is message text only; a typo still resolves it.
    let v = call(&store(), "find_values", json!({"text": "grafic bilder", "limit": 5}));
    let values = v["values"].as_array().unwrap();
    let hit = values
        .iter()
        .find(|m| m["value"] == "CompanyA.Graphic.Builder.exe" && m["field"] == "@entity")
        .unwrap_or_else(|| panic!("{v}"));
    assert_eq!(hit["kind"], "executable", "{hit}");
    assert_eq!(hit["seenIn"][0], "host-agent.log", "{hit}");
    assert!(hit["firstSeen"].is_i64() && hit["lastSeen"].is_i64(), "{hit}");
}
