//! The local alias model (Debrief's own assistant model, here a scripted fake) and the assistant's name tools with the
//! name book attached: the "graphic builder" case through the layers.

use debrief_ai::assistant::{EngineTools, ToolHost, ToolLimits};
use debrief_ai::llm::ScriptedProvider;
use debrief_ai::names_model::LocalAliasModel;
use debrief_core::names::alias_job::AliasJobConfig;
use debrief_core::names::Names;
use debrief_core::ports::Store;
use debrief_core::workspace::WorkspaceStore;
use debrief_core::{Profile, Workspace};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn store() -> WorkspaceStore {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "debrief-ai-names-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rows = [
        "Started CompanyA.Graphic.Builder.exe pid 4242",
        "Render queue drained by graphicbuilder.exe",
        "Loaded graphicbuilder.exe again",
        "Started CompanyA.Tray.Agent.exe",
    ];
    let text: String = rows
        .iter()
        .enumerate()
        .map(|(i, m)| format!("{{\"@t\":\"2026-10-14T09:0{i}:00.000Z\",\"@m\":\"{m}\"}}\n"))
        .collect();
    std::fs::write(dir.join("host-agent.log"), text).unwrap();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    WorkspaceStore::new(ws)
}

fn call(s: &WorkspaceStore, names: &Arc<Names>, name: &str, input: Value) -> Value {
    let store: &dyn Store = s;
    let mut t = EngineTools::new(store, ToolLimits::for_context(8192)).with_names(names.clone());
    let o = t.call(name, &input);
    assert!(!o.is_error, "{}", o.text);
    serde_json::from_str(&o.text).unwrap()
}

#[test]
fn graphic_builder_through_the_assistant_tools_with_each_layer() {
    let s = store();
    let names = Arc::new(Names::default());
    names.refresh(&s as &dyn Store).unwrap();
    // Layer 1: run-together and spaced words reach the same entity, with the reason and evidence.
    for q in ["graphic builder", "graphicbuilder", "grafic bilder"] {
        let v = call(&s, &names, "find_values", json!({"text": q}));
        let top = &v["values"][0];
        assert_eq!(top["group"], "executable|graphic builder", "{q}: {v}");
        assert!(
            top["why"].as_str().is_some_and(|w| !w.is_empty()) && !top["evidence"].as_array().unwrap().is_empty(),
            "{v}"
        );
        assert!(top["spellings"].as_array().unwrap().len() >= 2);
    }
    // Layer 2: the local model suggests "image editor"; nothing is asked at search time.
    let provider = Arc::new(ScriptedProvider::text(&[
        "{\"items\":[{\"id\":1,\"name\":\"Graphic Builder\",\"aliases\":[\"image editor\"]},{\"id\":2,\"name\":\"Tray Agent\",\"aliases\":[]}]}",
    ]));
    let before = call(&s, &names, "find_values", json!({"text": "image editor"}));
    assert!(before["values"].as_array().unwrap().is_empty());
    let model = LocalAliasModel::new(provider.clone(), Arc::default());
    let cfg = AliasJobConfig {
        min_count: 1,
        pause: std::time::Duration::ZERO,
        ..AliasJobConfig::default()
    };
    let rep = names.run_alias_job(Some(&model), &cfg, &|| false);
    assert!(rep.stored >= 1, "{rep:?}");
    let asked = provider.requests().len();
    let v = call(&s, &names, "find_values", json!({"text": "image editor"}));
    assert_eq!(v["values"][0]["via"], "alias", "{v}");
    assert!(v["values"][0]["why"].as_str().unwrap().starts_with("AI alias"));
    assert_eq!(provider.requests().len(), asked, "no model call at search time");
    // find_events reports the match and where to look.
    let f = call(&s, &names, "find_events", json!({"text": "image editor"}));
    assert!(f["note"].as_str().unwrap().contains("Matched"), "{f}");
}

#[test]
fn a_model_that_cannot_answer_stops_the_job_without_aliases() {
    let s = store();
    let names = Arc::new(Names::default());
    names.refresh(&s as &dyn Store).unwrap();
    let provider = Arc::new(ScriptedProvider::text(&["not json at all"]));
    let model = LocalAliasModel::new(provider, Arc::default());
    let cfg = AliasJobConfig {
        min_count: 1,
        pause: std::time::Duration::ZERO,
        ..AliasJobConfig::default()
    };
    let rep = names.run_alias_job(Some(&model), &cfg, &|| false);
    assert_eq!(rep.stored, 0, "{rep:?}");
    let v = call(&s, &names, "find_values", json!({"text": "image editor"}));
    assert!(v["values"].as_array().unwrap().is_empty());
}
