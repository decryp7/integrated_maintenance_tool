//! The Pitwall adapter against the fake server: discovery, pairing (approved, denied, cancelled), version mismatch,
//! streamed chat, a server that dies mid-stream, slow answers, revoked keys. Temporary folders and ephemeral ports only.

use super::fake::{ChatScript, FakePitwall, FakeState, PairMode};
use super::*;
use crate::llm::{ChatMessage, ChatRequest, LlmProvider, StopReason, ToolSpec};
use std::path::Path;
use std::time::Instant;

fn locator(dir: &Path) -> Locator {
    Locator {
        data_dir: dir.to_path_buf(),
        fallback_port: None,
        pid_alive: Arc::new(|pid| pid == std::process::id()),
    }
}

fn quick() -> Tuning {
    Tuning {
        poll_every: Duration::from_millis(10),
        chat: http::Timeouts {
            connect: Duration::from_secs(1),
            first_byte: Duration::from_secs(5),
            idle: Duration::from_secs(5),
        },
    }
}

/// A fake server, discovered through a temporary folder, and Debrief's side with `key` stored.
fn setup(state: FakeState, key: Option<&str>) -> (FakePitwall, Arc<Pitwall>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakePitwall::start(state);
    fake.write_discovery(dir.path(), std::process::id(), 1);
    let keys: Arc<dyn KeyStore> = Arc::new(key.map(MemoryKeys::with).unwrap_or_default());
    let pw = Pitwall::new(locator(dir.path()), keys, quick());
    (fake, pw, dir)
}

fn paired_state() -> FakeState {
    FakeState {
        keys: vec!["pw_ok".into()],
        ..FakeState::default()
    }
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(t.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn ask(p: &dyn LlmProvider, req: &ChatRequest) -> (crate::Result<crate::llm::ChatResponse>, String) {
    let mut streamed = String::new();
    let r = p.chat(req, &mut |d| streamed.push_str(d));
    (r, streamed)
}

fn hello() -> ChatRequest {
    ChatRequest {
        system: "be brief".into(),
        messages: vec![ChatMessage::user("hi")],
        max_tokens: 64,
        ..ChatRequest::default()
    }
}

#[test]
fn absent_installed_and_running_are_told_apart() {
    let dir = tempfile::tempdir().unwrap();
    let pw = Pitwall::new(locator(&dir.path().join("none")), Arc::new(MemoryKeys::default()), quick());
    assert_eq!(pw.probe().found, FoundKind::Absent);
    let pw = Pitwall::new(locator(dir.path()), Arc::new(MemoryKeys::default()), quick());
    assert_eq!(pw.probe().found, FoundKind::Installed);
    let (_fake, pw, _d) = setup(FakeState::default(), None);
    let v = pw.probe();
    assert_eq!(
        (v.found, v.version.as_deref(), v.paired, v.usable),
        (FoundKind::Running, Some("1.0.4"), false, false)
    );
    assert!(pw.provider(Priority::Interactive).is_err(), "not paired: no provider");
}

#[test]
fn the_well_known_port_is_tried_without_a_discovery_file() {
    let fake = FakePitwall::start(paired_state());
    let dir = tempfile::tempdir().unwrap();
    let loc = Locator {
        fallback_port: Some(fake.port),
        ..locator(&dir.path().join("none"))
    };
    let pw = Pitwall::new(loc, Arc::new(MemoryKeys::with("pw_ok")), quick());
    let v = pw.probe();
    assert!(v.usable, "{v:?}");
    assert_eq!(v.found, FoundKind::Running);
}

#[test]
fn another_api_version_is_incompatible_and_says_who_to_update() {
    let (fake, pw, dir) = setup(paired_state(), Some("pw_ok"));
    fake.write_discovery(dir.path(), std::process::id(), 2);
    let v = pw.probe();
    assert_eq!(v.found, FoundKind::Incompatible);
    assert!(v.problem.as_deref().unwrap().contains("Update Debrief"), "{v:?}");
    // The file says 1 but the server says 0.
    fake.write_discovery(dir.path(), std::process::id(), 1);
    fake.state().api_version = 0;
    let v = pw.probe();
    assert_eq!(v.found, FoundKind::Incompatible);
    assert!(v.problem.as_deref().unwrap().contains("Update Pitwall"));
    assert!(pw.connect(true).is_err(), "no pairing with an incompatible server");
}

#[test]
fn pairing_needs_the_users_approval_and_stores_the_key_once() {
    let (fake, pw, _d) = setup(
        FakeState {
            pending_polls: 3,
            ..FakeState::default()
        },
        None,
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let e = events.clone();
    pw.set_listener(Arc::new(move |ev| e.lock().unwrap().push(ev)));
    let v = pw.connect(true).unwrap();
    assert_eq!(v.pairing.as_ref().map(|p| p.code.as_str()), Some("482913"));
    assert!(!v.usable);
    wait_for("the approval", || pw.view().usable);
    assert!(pw.view().paired && pw.view().pairing.is_none());
    assert_eq!(pw.view().model.as_deref(), Some("qwen3-8b-q4km"));
    assert!(events.lock().unwrap().contains(&PitwallEvent::Paired { use_for_debrief: true }));
    // Debrief paired with its own name and no key.
    let pair = fake.requests_to("/x/v1/pair");
    assert!(pair[0].2.contains("\"Debrief\"") && !pair[0].1.iter().any(|(n, _)| n == "authorization"));
    // Connecting again when usable needs no new pairing.
    let v = pw.connect(false).unwrap();
    assert!(v.usable && fake.requests_to("/x/v1/pair").len() == 1);
}

#[test]
fn a_denied_or_cancelled_pairing_changes_nothing_but_says_so() {
    let (_fake, pw, _d) = setup(
        FakeState {
            pair: PairMode::Deny,
            ..FakeState::default()
        },
        None,
    );
    pw.connect(true).unwrap();
    wait_for("the denial", || pw.view().pairing.is_none());
    let v = pw.view();
    assert!(!v.paired && !v.usable);
    assert!(v.problem.as_deref().unwrap().contains("denied"), "{v:?}");

    let (_fake, pw, _d) = setup(
        FakeState {
            pair: PairMode::Pending,
            ..FakeState::default()
        },
        None,
    );
    pw.connect(false).unwrap();
    assert!(pw.view().pairing.is_some());
    assert!(pw.cancel_pairing());
    assert!(!pw.cancel_pairing());
    std::thread::sleep(Duration::from_millis(50));
    let v = pw.view();
    assert!(v.pairing.is_none() && v.problem.is_none() && !v.paired);
}

#[test]
fn pairing_is_refused_when_pitwall_is_not_running() {
    let dir = tempfile::tempdir().unwrap();
    let pw = Pitwall::new(locator(dir.path()), Arc::new(MemoryKeys::default()), quick());
    assert!(pw.connect(true).unwrap_err().to_string().contains("not running"));
}

#[test]
fn a_chat_streams_through_the_shared_preparation_with_provenance_and_priority() {
    let (fake, pw, _d) = setup(paired_state(), Some("pw_ok"));
    assert!(pw.probe().usable);
    let p = pw.provider(Priority::Background).unwrap();
    assert_eq!((p.id(), p.context_length(), p.vision()), ("pitwall", 8192, false));
    assert!(p.display_name().contains("qwen3-8b-q4km"));
    let (r, streamed) = ask(&p, &hello());
    let r = r.unwrap();
    assert_eq!((r.text.as_str(), streamed.as_str()), ("Hello from Pitwall.", "Hello from Pitwall."));
    assert_eq!(r.stop_reason, StopReason::EndTurn);
    assert_eq!(r.usage.map(|u| (u.input_tokens, u.output_tokens)), Some((11, 3)));
    assert!(pw.served());
    let (_, headers, body) = fake.requests_to("/v1/chat/completions").remove(0);
    let h = |n: &str| headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
    assert_eq!(h("authorization").as_deref(), Some("Bearer pw_ok"));
    assert_eq!(h("x-pitwall-priority").as_deref(), Some("background"));
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["model"], "auto:chat,8k");
    assert_eq!(body["messages"][0]["content"], "be brief");
    assert_eq!(p.with_priority(Priority::Interactive).priority, Priority::Interactive);
}

#[test]
fn tool_calls_use_debriefs_own_protocol_like_the_built_in_engine() {
    let state = FakeState {
        chat: ChatScript::Answer {
            pieces: vec![
                "Checking.\n<tool_".into(),
                "call>{\"name\":\"run_query\",\"arguments\":{\"ql\":\"level=error\"}}</tool_call>".into(),
            ],
            finish: "stop".into(),
        },
        ..paired_state()
    };
    let (fake, pw, _d) = setup(state, Some("pw_ok"));
    pw.probe();
    let req = ChatRequest {
        tools: vec![ToolSpec {
            name: "run_query".into(),
            description: "q".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }],
        ..hello()
    };
    let (r, streamed) = ask(&pw.provider(Priority::Interactive).unwrap(), &req);
    let r = r.unwrap();
    assert_eq!(r.stop_reason, StopReason::ToolUse);
    assert_eq!(r.tool_calls[0].name, "run_query");
    assert_eq!(streamed.trim(), "Checking.");
    let body: serde_json::Value = serde_json::from_str(&fake.requests_to("/v1/chat/completions")[0].2).unwrap();
    assert!(body.get("tools").is_none(), "Pitwall's tools permission is not needed");
    assert!(body["messages"][0]["content"].as_str().unwrap().contains("<tools>"));
}

#[test]
fn the_users_debrief_preset_wins_and_the_context_follows_the_model() {
    let state = FakeState {
        preset: true,
        models: vec![("small".into(), vec!["chat".into(), "vision".into()], 4096)],
        ..paired_state()
    };
    let (fake, pw, _d) = setup(state, Some("pw_ok"));
    pw.set_needs(Needs { context_tokens: 16384 });
    assert!(pw.probe().usable);
    let p = pw.provider(Priority::Interactive).unwrap();
    assert_eq!((p.spec.as_str(), p.context_length(), p.vision()), ("debrief", 4096, true));
    let sel = fake.requests_to("/x/v1/select");
    assert!(sel[0].2.contains("\"debrief\""));
    // A capability selection adds vision for images; a preset is sent as is.
    let auto = PitwallProvider {
        auto: true,
        spec: "auto:chat,8k".into(),
        ..p.with_priority(Priority::Interactive)
    };
    assert_eq!(auto.model_for(true), "auto:chat,8k,vision");
    assert_eq!(p.model_for(true), "debrief");
}

#[test]
fn no_suitable_model_is_a_plain_problem() {
    let (_fake, pw, _d) = setup(
        FakeState {
            models: vec![],
            ..paired_state()
        },
        Some("pw_ok"),
    );
    let v = pw.probe();
    assert!(!v.usable && v.paired);
    assert!(v.problem.as_deref().unwrap().contains("Download a chat model in Pitwall"), "{v:?}");
    assert!(pw.provider(Priority::Interactive).err().unwrap().to_string().contains("chat model"));
}

#[test]
fn a_paused_pitwall_is_not_usable() {
    let (fake, pw, _d) = setup(paired_state(), Some("pw_ok"));
    fake.state().serving = "paused".into();
    let v = pw.probe();
    assert!(!v.usable && v.problem.as_deref().unwrap().contains("paused"));
    fake.state().serving = "stopped".into();
    assert!(pw.probe().problem.as_deref().unwrap().contains("stopped"));
}

#[test]
fn a_revoked_key_is_forgotten_and_reported() {
    let (fake, pw, _d) = setup(paired_state(), Some("pw_ok"));
    assert!(pw.probe().usable);
    let p = pw.provider(Priority::Interactive).unwrap();
    fake.state().keys.clear();
    let (r, _) = ask(&p, &hello());
    assert!(r.unwrap_err().to_string().contains("Connect again"));
    let v = pw.view();
    assert!(!v.usable && !v.paired, "{v:?}");
    assert!(pw.keys.get().is_none());
    // At probe time too.
    let (fake, pw, _d) = setup(paired_state(), Some("pw_old"));
    let v = pw.probe();
    assert!(!v.paired && v.problem.as_deref().unwrap().contains("no longer accepts"));
    drop(fake);
}

#[test]
fn pitwall_dying_mid_stream_fails_the_request_and_marks_it_gone() {
    let state = FakeState {
        chat: ChatScript::DieAfter(vec!["Half an".into()]),
        ..paired_state()
    };
    let (fake, pw, _d) = setup(state, Some("pw_ok"));
    pw.probe();
    let p = pw.provider(Priority::Interactive).unwrap();
    let (r, streamed) = ask(&p, &hello());
    assert!(r.unwrap_err().to_string().contains("stopped answering"));
    assert_eq!(streamed, "Half an");
    assert!(!pw.view().usable);
    // Quit: nothing listens any more.
    fake.kill();
    std::thread::sleep(Duration::from_millis(50));
    let (r, _) = ask(&p, &hello());
    assert!(r.unwrap_err().to_string().contains("not running"));
    assert_eq!(
        pw.probe().found,
        FoundKind::Installed,
        "the discovery file names a port nobody answers on"
    );
}

#[test]
fn an_error_event_in_the_stream_and_typed_statuses_reach_the_user() {
    let state = FakeState {
        chat: ChatScript::ErrorEvent("the engine crashed".into()),
        ..paired_state()
    };
    let (fake, pw, _d) = setup(state, Some("pw_ok"));
    pw.probe();
    let p = pw.provider(Priority::Interactive).unwrap();
    assert!(ask(&p, &hello()).0.unwrap_err().to_string().contains("the engine crashed"));
    fake.state().chat = ChatScript::Fail {
        status: 503,
        code: "insufficient_memory".into(),
        message: "the model needs 9 GB".into(),
    };
    assert!(matches!(ask(&p, &hello()).0.unwrap_err(), crate::AiError::Memory(m) if m.contains("9 GB")));
    assert!(pw.view().usable, "a busy Pitwall is still there");
}

#[test]
fn slow_answers_time_out_and_cancel_stops_at_once() {
    let state = FakeState {
        chat_delay: Duration::from_millis(600),
        ..paired_state()
    };
    let (_fake, pw, _d) = setup(state, Some("pw_ok"));
    pw.probe();
    let mut p = pw.provider(Priority::Interactive).unwrap();
    p.timeouts.first_byte = Duration::from_millis(150);
    assert!(ask(&p, &hello()).0.unwrap_err().to_string().contains("did not answer in time"));

    p.timeouts.first_byte = Duration::from_secs(30);
    let cancel = Arc::new(AtomicBool::new(false));
    let c = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        c.store(true, Ordering::Relaxed);
    });
    let req = ChatRequest {
        cancel: Some(cancel),
        ..hello()
    };
    let t = Instant::now();
    let r = ask(&p, &req).0.unwrap();
    assert_eq!(r.stop_reason, StopReason::Cancelled);
    assert!(t.elapsed() < Duration::from_millis(500), "{:?}", t.elapsed());
}

#[test]
fn forget_deletes_the_key() {
    let (_fake, pw, _d) = setup(paired_state(), Some("pw_ok"));
    assert!(pw.probe().paired);
    let v = pw.forget().unwrap();
    assert!(!v.paired && !v.usable);
}

#[test]
fn sentences_and_version_messages() {
    assert_eq!(sentence("pitwall stopped."), "Pitwall stopped.");
    assert_eq!(sentence(""), "");
    assert!(incompatible("2.0", 2).contains("newer"));
    assert!(incompatible("0.9", 0).contains("older"));
}
