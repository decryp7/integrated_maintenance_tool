//! The browser commands through the real IPC path (`tauri::test`): the guard that keeps every command from
//! a page, argument checks, a chat's read-along and agent switches with Stop and Continue, approvals and the
//! engine-built search for an event. The page itself (script evaluation) needs a real webview; its logic is
//! tested in `imt_ai::browser`.

use super::hub::{BrowserHub, BrowserSettings};
use super::BrowserState;
use crate::test_support::{main_window, samples, test_app_with_commands, window};
use crate::tools::tests::ipc;
use serde_json::{json, Value};
use tauri::Manager;

fn err(r: Result<Value, Value>) -> String {
    r.unwrap_err().to_string()
}

#[test]
fn a_page_in_the_browser_window_gets_no_command_at_all() {
    let app = test_app_with_commands(None);
    let browser = window(&app, "browser-main");
    for cmd in [
        "browser_state",
        "list_windows",
        "open_paths",
        "browser_agent_respond",
        "no_such_command",
    ] {
        let e = err(ipc(&browser, cmd, json!({})));
        assert!(e.contains("not available to web pages"), "{cmd}: {e}");
    }
    // The app's own window is unaffected.
    assert!(ipc(&window(&app, "main"), "browser_state", json!({})).is_ok());
}

#[test]
fn the_panel_reads_the_browser_state_and_only_known_addresses_open_externally() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let s = ipc(&main, "browser_state", json!({})).unwrap();
    assert_eq!(
        (s["page"]["open"].as_bool(), s["engine"].as_str()),
        (Some(false), Some("duckDuckGo"))
    );
    assert_eq!(s["customEngine"], false);
    assert!(err(ipc(&main, "browser_open_external", json!({"url": "https://evil.example/"}))).contains("refused"));
    assert!(err(ipc(&main, "browser_open_external", json!({"url": "file:///etc/passwd"}))).contains("http and https"));
    assert_eq!(ipc(&main, "browser_close", json!({})).unwrap(), false, "no browser was open");
    assert!(err(ipc(&main, "browser_nav", json!({"command": "back"}))).contains("not open"));
    assert!(ipc(&main, "browser_nav", json!({"command": "sideways"})).is_err());
    assert!(err(ipc(
        &main,
        "browser_open",
        json!({"target": {"kind": "address", "text": "javascript:alert(1)"}})
    ))
    .contains("http and https"));
    assert!(err(ipc(&main, "browser_open", json!({"target": {"kind": "search", "query": " "}}))).contains("empty"));
    let long = "a".repeat(3000);
    assert!(err(ipc(&main, "browser_open", json!({"target": {"kind": "address", "text": long}}))).contains("longer"));
}

#[test]
fn a_chat_switches_read_along_and_agent_mode_and_stop_ends_it() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let chat = main_window(&app).ai.chats.create().unwrap();
    let call = |cmd: &str, extra: Value| {
        let mut body = json!({"sessionId": chat});
        if let (Some(b), Some(e)) = (body.as_object_mut(), extra.as_object()) {
            b.extend(e.clone());
        }
        ipc(&main, cmd, body)
    };
    let s = call("browser_chat_status", json!({})).unwrap();
    assert_eq!(
        (s["readAlong"].as_bool(), s["agent"].as_bool(), s["stepLimit"].as_u64()),
        (Some(false), Some(false), Some(25))
    );
    assert_eq!(s["sessionId"], chat.as_str());
    assert!(call("browser_set_read_along", json!({"on": true})).unwrap()["readAlong"]
        .as_bool()
        .unwrap());
    assert!(err(call("browser_agent_continue", json!({}))).contains("agent mode is off"));
    let s = call("browser_set_agent", json!({"on": true})).unwrap();
    assert!(s["agent"].as_bool().unwrap());
    assert_eq!(
        call("browser_agent_continue", json!({})).unwrap()["stepLimit"],
        25,
        "nothing used yet: the same budget"
    );
    assert_eq!(
        call("browser_agent_respond", json!({"requestId": 1, "decision": "allow"})).unwrap(),
        false,
        "nothing waits"
    );
    assert!(call("browser_agent_respond", json!({"requestId": 1, "decision": "yes"})).is_err());
    let s = call("browser_agent_stop", json!({})).unwrap();
    assert!(!s["agent"].as_bool().unwrap() && s["readAlong"].as_bool().unwrap());
    assert!(!call("browser_set_agent", json!({"on": false})).unwrap()["agent"].as_bool().unwrap());
    // Pages and links of the chat.
    assert_eq!(call("browser_sources", json!({})).unwrap(), json!([]));
    assert_eq!(call("browser_discard_sources", json!({})).unwrap(), json!([]));
    assert!(err(call("browser_capture", json!({}))).contains("not open"));
    assert!(err(call("browser_open_suggested", json!({"url": "https://e.org/"}))).contains("not suggested"));
    // Unknown chats are refused everywhere.
    for cmd in ["browser_chat_status", "browser_set_agent", "browser_capture", "browser_agent_stop"] {
        let e = err(ipc(&main, cmd, json!({"sessionId": "chat-404", "on": true})));
        assert!(e.contains("no longer exists"), "{cmd}: {e}");
    }
    // Deleting the chat drops its access.
    let hub = app.state::<BrowserState>().0.clone();
    hub.chat("main", &chat).set_read_along(true);
    super::forget_chat(app.handle(), "main", &chat);
    assert!(!hub.chat("main", &chat).read_along());
}

#[test]
fn the_engine_builds_the_search_for_an_event() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    ipc(
        &main,
        "open_paths",
        json!({ "paths": [samples().join("serilog-clef").to_string_lossy()] }),
    )
    .unwrap();
    let page = ipc(
        &main,
        "query_page",
        json!({"request": {"query": {"text": "timed out"}, "offset": 0, "limit": 1, "descending": false}}),
    )
    .unwrap();
    let id = page["rows"][0]["id"].as_u64().unwrap();
    let q = ipc(&main, "browser_event_query", json!({"recordId": id})).unwrap();
    let text = q["query"].as_str().unwrap();
    assert!(!text.is_empty() && !text.contains("WEB0"), "{q}");
    assert!(err(ipc(&main, "browser_event_query", json!({"recordId": u32::MAX}))).contains("nothing to search for"));
}

#[test]
fn settings_choose_the_engine_and_the_agent_limits() {
    use imt_core::settings::{sanitize, Configurable};
    let hub = std::sync::Arc::new(BrowserHub::default());
    let section = super::settings_section::BrowserSection { hub: hub.clone() };
    let schema = section.settings_section();
    let values = |v: Value| sanitize(&schema, &v).0;
    section
        .apply_settings(&values(
            json!({"searchEngine": "google", "agentStepLimit": 40, "agentActionsPerMinute": 6}),
        ))
        .unwrap();
    let s = hub.settings();
    assert_eq!(
        (s.search.engine.id(), s.agent.step_limit, s.agent.actions_per_minute),
        ("google", 40, 6)
    );
    let e = section.apply_settings(&values(json!({"searchEngine": "custom"}))).unwrap_err();
    assert!(e.to_string().contains("custom search address"), "{e}");
    assert!(section
        .apply_settings(&values(
            json!({"searchEngine": "custom", "customSearchUrl": "https://s.example/?q="})
        ))
        .is_err());
    section
        .apply_settings(&values(
            json!({"searchEngine": "custom", "customSearchUrl": "https://s.example/?q={q}"}),
        ))
        .unwrap();
    assert!(hub.settings().search.custom.is_some());
    section.apply_settings(&values(json!({}))).unwrap();
    assert_eq!(hub.settings(), BrowserSettings::default(), "defaults when nothing is set");
}

#[test]
fn agent_actions_reach_the_activity_log_without_page_or_typed_text() {
    use imt_ai::browser::{ActionEntry, ActionLog, ActionOutcome};
    let _g = crate::activity::test_capture();
    super::ActivityLog { window: "main".into() }.record(&ActionEntry {
        chat: "chat-1".into(),
        action: "type",
        site: Some("learn.example.org".into()),
        outcome: ActionOutcome::Approved,
        step: 3,
    });
    let logged: Vec<_> = crate::activity::captured()
        .into_iter()
        .filter(|c| c.props.get("Operation").map(String::as_str) == Some("browser.agent"))
        .collect();
    assert_eq!(logged.len(), 1, "{logged:?}");
    let p = &logged[0].props;
    assert_eq!(
        (p["Action"].as_str(), p["Site"].as_str(), p["Outcome"].as_str(), p["Step"].as_str()),
        ("type", "learn.example.org", "approved", "3")
    );
    assert_eq!(p["SourceContext"], crate::activity::SOURCE_CONTEXT);
}
