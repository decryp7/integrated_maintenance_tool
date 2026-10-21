//! The browser commands through the real IPC path (`tauri::test`): the guard that keeps every command from
//! a page (docked in an IMT window or in a window of its own), the page's placement in the panel and its pop-out,
//! argument checks, a chat's read-along and agent switches with Stop and Continue, approvals and the
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
    assert_eq!((s["page"]["open"].as_bool(), s["engine"].as_str()), (Some(false), Some("google")));
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
    assert_eq!(q["source"], "engine", "no model: the engine builds it");
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

#[test]
fn research_settings_default_to_automatic_with_five_results_and_are_validated() {
    use imt_core::settings::{sanitize, Configurable};
    let hub = std::sync::Arc::new(BrowserHub::default());
    let section = super::settings_section::BrowserSection { hub: hub.clone() };
    let schema = section.settings_section();
    let values = |v: Value| sanitize(&schema, &v).0;
    section.apply_settings(&values(json!({}))).unwrap();
    let r = hub.settings().research;
    assert_eq!((r.auto, r.max_results), (true, 5));
    section
        .apply_settings(&values(json!({"autoResearch": false, "researchResults": 3})))
        .unwrap();
    let r = hub.settings().research;
    assert_eq!((r.auto, r.max_results), (false, 3));
    // Out of range: the settings service refuses the value and the default applies.
    section.apply_settings(&values(json!({"researchResults": 30}))).unwrap();
    assert_eq!(hub.settings().research.max_results, 5);
    let field = schema.fields.iter().find(|f| f.key == "researchResults").expect("the field");
    assert!(matches!(field.kind, imt_core::settings::FieldKind::Int { min: 1, max: 10, .. }));
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
    assert_eq!(super::research::settings(app.handle()).max_results, 5);
}

#[test]
fn google_is_the_default_engine_of_settings_and_listed_first() {
    use imt_core::settings::{sanitize, Configurable, FieldKind};
    let hub = std::sync::Arc::new(BrowserHub::default());
    let section = super::settings_section::BrowserSection { hub: hub.clone() };
    let schema = section.settings_section();
    let field = schema.fields.iter().find(|f| f.key == "searchEngine").expect("the field");
    assert_eq!(field.default, json!("google"));
    let FieldKind::Enum { options } = &field.kind else {
        panic!("an enum")
    };
    assert_eq!(options[0].value, "google");
    section.apply_settings(&sanitize(&schema, &json!({})).0).unwrap();
    assert_eq!(hub.settings().search.engine, imt_ai::browser::SearchEngine::Google);
}

/// Any webview as the IPC test helper wants it.
struct Any<'a>(&'a tauri::Webview<tauri::test::MockRuntime>);

impl AsRef<tauri::Webview<tauri::test::MockRuntime>> for Any<'_> {
    fn as_ref(&self) -> &tauri::Webview<tauri::test::MockRuntime> {
        self.0
    }
}

/// Invokes a command from any webview (a page webview is no webview window).
fn ipc_from(w: &tauri::Webview<tauri::test::MockRuntime>, cmd: &str, body: Value) -> Result<Value, Value> {
    use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
    use tauri::test::{get_ipc_response, INVOKE_KEY};
    get_ipc_response(
        &Any(w),
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: crate::test_support::ipc_url(),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| match body {
        InvokeResponseBody::Json(s) => serde_json::from_str::<Value>(&s).unwrap(),
        InvokeResponseBody::Raw(_) => Value::Null,
    })
}

/// A child webview `label` in window `host` (what `browser_open` builds on Windows and macOS for `browser-<owner>`).
fn child(app: &tauri::App<tauri::test::MockRuntime>, label: &str, host: &str) -> tauri::Webview<tauri::test::MockRuntime> {
    window(app, host);
    app.get_window(host)
        .unwrap()
        .add_child(
            tauri::webview::WebviewBuilder::new(label, tauri::WebviewUrl::External("https://example.org/".parse().unwrap())),
            tauri::PhysicalPosition::new(0, 0),
            tauri::PhysicalSize::new(1, 1),
        )
        .unwrap()
}

#[test]
fn a_page_docked_in_an_imt_window_gets_no_command_at_all() {
    let app = test_app_with_commands(None);
    // Taken before the child is added: a window with a second webview is no "webview window" for Tauri any more.
    let main = window(&app, "main");
    let page = child(&app, "browser-main", "main");
    assert_eq!(page.window().label(), "main", "a child of the IMT window");
    for cmd in [
        "browser_state",
        "browser_place",
        "list_windows",
        "open_paths",
        "jobs_cancel",
        "browser_agent_respond",
        "no_such_command",
    ] {
        let e = err(ipc_from(&page, cmd, json!({})));
        assert!(e.contains("not available to web pages"), "{cmd}: {e}");
    }
    // Its window's own webview is unaffected.
    assert!(ipc(&main, "browser_state", json!({})).is_ok());
}

#[test]
fn a_webview_that_is_not_its_windows_own_never_acts_for_the_window() {
    // A webview named like an IMT window passes the label guards; the window it lives in still does not answer it.
    let app = test_app_with_commands(None);
    let intruder = child(&app, "w-9", "main");
    let e = err(ipc_from(&intruder, "browser_state", json!({})));
    assert!(e.contains("not IMT's"), "{e}");
}

#[test]
fn the_panel_places_the_page_and_the_page_follows_it_into_a_pop_out_and_back() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let place = |w: &tauri::WebviewWindow<tauri::test::MockRuntime>, p: Value| ipc(w, "browser_place", json!({ "placement": p }));
    let shown = json!({"visible": true, "rect": {"x": 10, "y": 20, "width": 300, "height": 200}, "scale": 1.5});
    // No page yet: the place is remembered for when one opens.
    assert_eq!(place(&main, shown.clone()).unwrap(), true);
    let hub = app.state::<BrowserState>().0.clone();
    let at = super::geometry::View::At(super::geometry::PhysicalRect {
        x: 15,
        y: 30,
        width: 450,
        height: 300,
    });
    assert_eq!(hub.placement("main"), (Some("main".into()), at));
    // Broken reports are refused.
    for bad in [
        json!({"visible": true, "rect": null, "scale": 0}),
        json!({"visible": true, "rect": {"x": "a", "y": 0, "width": 1, "height": 1}, "scale": 1}),
        json!({"visible": true, "rect": {"x": 0, "y": 0, "width": -1, "height": 1}, "scale": 1}),
    ] {
        assert!(place(&main, bad.clone()).is_err(), "{bad}");
    }
    let page = child(&app, "browser-main", "main");
    assert_eq!(
        place(&main, json!({"visible": false, "rect": null, "scale": 1})).unwrap(),
        false,
        "hidden"
    );
    // The panel popped out: the page moves into the pop-out window. Linux (ADR 0024): the page has a window of its own
    // and is never reparented; only the panel's place is remembered.
    let popout = window(&app, "p-main-1");
    assert_eq!(place(&popout, shown).unwrap(), true);
    let in_popout = if super::view::DOCKED { "p-main-1" } else { "main" };
    assert_eq!(page.window().label(), in_popout);
    assert_eq!(hub.placement("main").0.as_deref(), Some("p-main-1"));
    // The pop-out closes: the page goes back to its window, hidden, and lives on.
    super::host_closing(app.handle(), "p-main-1");
    assert_eq!(page.window().label(), "main");
    assert_eq!(hub.placement("main"), (Some("main".into()), super::geometry::View::Hidden));
    super::host_closing(app.handle(), "main");
    assert_eq!(page.window().label(), "main", "only a pop-out hands it back");
    // Closing the page: the panel shows none, the page webview is gone.
    assert_eq!(ipc(&main, "browser_close", json!({})).unwrap(), true);
    assert!(app.get_webview("browser-main").is_none());
    assert_eq!(ipc(&main, "browser_close", json!({})).unwrap(), false);
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
fn keys_taken_from_the_page_reach_the_panel() {
    use std::sync::{Arc, Mutex};
    use tauri::Listener;
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let s = seen.clone();
    main.listen(super::BROWSER_EVENT, move |e| {
        if let Ok(v) = serde_json::from_str::<Value>(e.payload()) {
            s.lock().unwrap().push(v);
        }
    });
    super::view::key_pressed(app.handle(), "main", super::PageKey::FocusAddress);
    assert_eq!(seen.lock().unwrap().clone(), vec![json!({"type": "key", "action": "focusAddress"})]);
}

// Regression: Tauri stops counting a window with a second webview as a "webview window" (`get_webview_window` and
// `webview_windows` miss it), so the window list, reveal and the target for files lost the window hosting the page.
#[test]
fn an_imt_window_hosting_the_page_is_still_listed_and_reachable() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    child(&app, "browser-main", "main");
    assert!(app.get_webview_window("main").is_none(), "Tauri: no webview window any more");
    let listed: Vec<String> = crate::window_manager::list(app.handle()).into_iter().map(|w| w.label).collect();
    assert_eq!(listed, vec!["main".to_string()], "listed once, the page is no window of its own");
    assert!(crate::window_manager::reveal(app.handle(), "main").is_ok());
    assert_eq!(crate::window_manager::target(app.handle()).as_deref(), Some("main"));
    assert!(ipc(&main, "list_windows", json!({})).is_ok(), "its commands still work");
}
