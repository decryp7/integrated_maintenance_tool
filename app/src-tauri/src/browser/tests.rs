//! The browser commands through the real IPC path (`tauri::test`): argument checks, a chat's read-along and agent switches with Stop and Continue, approvals and the
//! engine-built search for an event. The renderer needs a real browser; it is tested in `chromium_driver` and `renderer`.

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
    assert!(err(call("browser_capture", json!({}))).contains("no web page is open"));
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
    assert_eq!(q["source"], "engine", "no model: the engine builds it");
    assert!(err(ipc(&main, "browser_event_query", json!({"recordId": u32::MAX}))).contains("nothing to search for"));
}

#[test]
fn settings_choose_the_engine_and_the_agent_limits() {
    use debrief_core::settings::{sanitize, Configurable};
    let hub = std::sync::Arc::new(BrowserHub::default());
    let section = super::settings_section::BrowserSection {
        hub: hub.clone(),
        renderers: None,
    };
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
    use debrief_ai::browser::{ActionEntry, ActionLog, ActionOutcome};
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

#[test]
fn research_settings_default_to_automatic_with_ten_results_and_are_validated() {
    use debrief_core::settings::{sanitize, Configurable};
    let hub = std::sync::Arc::new(BrowserHub::default());
    let section = super::settings_section::BrowserSection {
        hub: hub.clone(),
        renderers: None,
    };
    let schema = section.settings_section();
    let values = |v: Value| sanitize(&schema, &v).0;
    section.apply_settings(&values(json!({}))).unwrap();
    let r = hub.settings().research;
    assert_eq!((r.auto, r.max_results), (true, 10));
    section
        .apply_settings(&values(json!({"autoResearch": false, "researchResults": "30"})))
        .unwrap();
    let r = hub.settings().research;
    assert_eq!((r.auto, r.max_results), (false, 30));
    // Not one of the choices: the settings service refuses the value and the default applies.
    section.apply_settings(&values(json!({"researchResults": "3"}))).unwrap();
    assert_eq!(hub.settings().research.max_results, 10);
    let field = schema.fields.iter().find(|f| f.key == "researchResults").expect("the field");
    let debrief_core::settings::FieldKind::Enum { options } = &field.kind else {
        panic!("a choice: {:?}", field.kind)
    };
    assert_eq!(options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), ["10", "20", "30"]);
}

#[test]
fn research_stop_and_retry_need_a_research_to_act_on() {
    let app = test_app_with_commands(None);
    let _main = window(&app, "main");
    let spec = crate::jobs::JobSpec::new(crate::jobs::JobKind::Research, "Research: x")
        .window("main")
        .retry_with(json!({"op": "research"}));
    // A request without a question cannot run again.
    let job = crate::jobs::model::Job::finished_for_test(spec, crate::jobs::JobState::Failed);
    assert!(super::research::retry(app.handle(), &job)
        .unwrap_err()
        .contains("cannot be run again"));
    let mut closed = job.clone();
    closed.source.window = Some("w-9".into());
    assert!(super::research::retry(app.handle(), &closed).unwrap_err().contains("closed"));
    assert_eq!(super::research::settings(app.handle()).max_results, 10);
}

#[test]
fn google_is_the_default_engine_of_settings_and_listed_first() {
    use debrief_core::settings::{sanitize, Configurable, FieldKind};
    let hub = std::sync::Arc::new(BrowserHub::default());
    let section = super::settings_section::BrowserSection {
        hub: hub.clone(),
        renderers: None,
    };
    let schema = section.settings_section();
    let field = schema.fields.iter().find(|f| f.key == "searchEngine").expect("the field");
    assert_eq!(field.default, json!("google"));
    let FieldKind::Enum { options } = &field.kind else {
        panic!("an enum")
    };
    assert_eq!(options[0].value, "google");
    section.apply_settings(&sanitize(&schema, &json!({})).0).unwrap();
    assert_eq!(hub.settings().search.engine, debrief_ai::browser::SearchEngine::Google);
}

#[test]
fn announcements_of_who_drives_reach_the_window_once_per_change() {
    use std::sync::{Arc, Mutex};
    use tauri::Listener;
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let chat = main_window(&app).ai.chats.create().unwrap();
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let s = seen.clone();
    main.listen(super::BROWSER_EVENT, move |e| {
        if let Ok(v) = serde_json::from_str::<Value>(e.payload()) {
            if v["type"] == "driving" {
                s.lock().unwrap().push(v["by"].clone());
            }
        }
    });
    assert_eq!(ipc(&main, "browser_driving", json!({})).unwrap(), Value::Null);
    ipc(&main, "browser_set_agent", json!({"sessionId": chat, "on": true})).unwrap();
    let hub = app.state::<BrowserState>().0.clone();
    assert!(hub.begin_driving("main", &chat));
    super::driving::refresh(app.handle(), "main");
    let agent = json!({"kind": "agent", "sessionId": chat});
    assert_eq!(ipc(&main, "browser_driving", json!({})).unwrap(), agent);
    // The panel's Stop is the chat's Stop.
    ipc(&main, "browser_agent_stop", json!({"sessionId": chat})).unwrap();
    assert_eq!(ipc(&main, "browser_driving", json!({})).unwrap(), Value::Null);
    // Agent mode turned off while driving.
    ipc(&main, "browser_set_agent", json!({"sessionId": chat, "on": true})).unwrap();
    super::driving::refresh(app.handle(), "main");
    ipc(&main, "browser_set_agent", json!({"sessionId": chat, "on": false})).unwrap();
    // Research, and its Stop.
    hub.end_driving("main", &chat);
    let key = super::hub::research_key(1);
    hub.chat("main", &key).set_agent(true, Default::default());
    assert!(hub.begin_driving("main", &key));
    super::driving::refresh(app.handle(), "main");
    assert_eq!(ipc(&main, "browser_driving", json!({})).unwrap(), json!({"kind": "research"}));
    // What the research job's Cancel does (`jobs_cancel` from the panel).
    hub.stop_research("main");
    super::driving::refresh(app.handle(), "main");
    assert_eq!(ipc(&main, "browser_driving", json!({})).unwrap(), Value::Null);
    let by = seen.lock().unwrap().clone();
    assert_eq!(
        by,
        vec![
            agent.clone(),
            Value::Null,
            agent,
            Value::Null,
            json!({"kind": "research"}),
            Value::Null
        ],
        "each change announced once"
    );
}

#[test]
fn stop_ends_every_hold_and_reports_whether_a_browser_ran() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let chat = main_window(&app).ai.chats.create().unwrap();
    ipc(&main, "browser_set_agent", json!({"sessionId": chat, "on": true})).unwrap();
    let hub = app.state::<BrowserState>().0.clone();
    assert!(hub.chat("main", &chat).agent());
    assert_eq!(
        ipc(&main, "browser_stop", json!({})).unwrap(),
        json!(false),
        "no browser was running"
    );
    assert!(!hub.chat("main", &chat).may_act(), "the hold was stopped");
}
