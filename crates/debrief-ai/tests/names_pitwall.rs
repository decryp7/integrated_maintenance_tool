//! The Pitwall adapter against a fake server on a free local port (never a real Pitwall), and the assistant's name
//! tools with the name book attached: the "graphic builder" case through the layers.

use debrief_ai::assistant::{EngineTools, ToolHost, ToolLimits};
use debrief_ai::names_model::PitwallModel;
use debrief_core::names::alias_job::{AliasJobConfig, AliasModel};
use debrief_core::names::Names;
use debrief_core::ports::Store;
use debrief_core::workspace::WorkspaceStore;
use debrief_core::{Profile, Workspace};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A one-purpose HTTP server: `/v1/models` lists one model, `/v1/chat/completions` answers `reply` and counts calls.
fn fake_pitwall(reply: &'static str) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = vec![0u8; 16 * 1024];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = if req.starts_with("GET /v1/models") {
                json!({"data": [{"id": "qwen-fake"}]}).to_string()
            } else {
                c.fetch_add(1, Ordering::SeqCst);
                json!({"choices": [{"message": {"content": reply}}]}).to_string()
            };
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, calls)
}

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
fn pitwall_answers_batches_and_is_local_only() {
    let (url, calls) = fake_pitwall("{\"items\":[]}");
    let m = PitwallModel::connect(&url, None).unwrap();
    assert_eq!(m.version(), "pitwall:qwen-fake");
    assert_eq!(m.complete("hello").unwrap(), "{\"items\":[]}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(PitwallModel::connect("http://example.com:80", None).is_err());
    // Nothing listens on a closed port: the feature is off, not an error for the user.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", closed.local_addr().unwrap().port());
    drop(closed);
    assert!(PitwallModel::connect(&url, Some("m")).unwrap().complete("x").is_err());
    assert!(PitwallModel::connect(&url, None).is_err());
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
    // Layer 2: a fake Pitwall suggests "image editor"; nothing is asked at search time.
    let (url, calls) = fake_pitwall("{\"items\":[{\"id\":1,\"name\":\"Graphic Builder\",\"aliases\":[\"image editor\"]},{\"id\":2,\"name\":\"Tray Agent\",\"aliases\":[]}]}");
    let before = call(&s, &names, "find_values", json!({"text": "image editor"}));
    assert!(before["values"].as_array().unwrap().is_empty());
    let model = PitwallModel::connect(&url, None).unwrap();
    let cfg = AliasJobConfig {
        min_count: 1,
        pause: std::time::Duration::ZERO,
        ..AliasJobConfig::default()
    };
    let rep = names.run_alias_job(Some(&model), &cfg, &|| false);
    assert!(rep.stored >= 1, "{rep:?}");
    let jobs = calls.load(Ordering::SeqCst);
    let v = call(&s, &names, "find_values", json!({"text": "image editor"}));
    assert_eq!(v["values"][0]["via"], "alias", "{v}");
    assert!(v["values"][0]["why"].as_str().unwrap().starts_with("AI alias"));
    assert_eq!(calls.load(Ordering::SeqCst), jobs, "no model call at search time");
    // find_events reports the match and where to look.
    let f = call(&s, &names, "find_events", json!({"text": "image editor"}));
    assert!(f["note"].as_str().unwrap().contains("Matched"), "{f}");
}
