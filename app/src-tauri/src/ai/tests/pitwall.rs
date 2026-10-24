//! Pitwall as the engine through AiCore and the IPC (ADR 0059), against the fake Pitwall of `debrief-ai` on an ephemeral
//! port with a temporary discovery folder and an in-memory key store: never the user's Pitwall, port, folder or keychain.

use super::*;
use crate::ai::EngineStateDto;
use crate::health::commands::{ai_health, ai_health_of};
use crate::health::hub::AiHealth;
use debrief_ai::engine::pitwall::fake::{ChatScript, FakePitwall, FakeState};
use debrief_ai::engine::pitwall::{Locator, MemoryKeys, Pitwall, Tuning};
use debrief_ai::engine::{EngineChoice, EngineKind};
use debrief_ai::llm::{ChatMessage, ChatRequest};

struct World {
    f: Fixture,
    fake: FakePitwall,
    pw: Arc<Pitwall>,
    core: Arc<AiCore>,
}

fn world(tag: &str, state: FakeState, key: Option<&str>) -> World {
    let f = Fixture::new(tag);
    let fake = FakePitwall::start(state);
    let dir = f.tmp.0.join("pitwall-data");
    fake.write_discovery(&dir, std::process::id(), 1);
    let locator = Locator {
        data_dir: dir,
        fallback_port: None,
        pid_alive: Arc::new(|pid| pid == std::process::id()),
    };
    let keys: Arc<dyn debrief_ai::engine::pitwall::KeyStore> = Arc::new(key.map(MemoryKeys::with).unwrap_or_default());
    let pw = Pitwall::new(
        locator,
        keys,
        Tuning {
            poll_every: Duration::from_millis(10),
            ..Tuning::default()
        },
    );
    let deps = AiDeps {
        pitwall: Some(pw.clone()),
        ..f.deps()
    };
    let core = Arc::new(AiCore::new(Some(ai_dir(&f.tmp)), f.tmp.0.join("models"), false, deps));
    World { f, fake, pw, core }
}

fn paired() -> FakeState {
    FakeState {
        keys: vec!["pw_ok".into()],
        ..FakeState::default()
    }
}

fn hello() -> ChatRequest {
    ChatRequest {
        messages: vec![ChatMessage::user("hi")],
        max_tokens: 64,
        ..ChatRequest::default()
    }
}

#[test]
fn automatic_uses_a_paired_pitwall_and_keeps_the_built_in_model_out_of_memory() {
    let w = world("pw-auto", paired(), Some("pw_ok"));
    assert_eq!(w.core.engine_kind(), Some(EngineKind::BuiltIn), "before the first look");
    w.pw.probe();
    assert_eq!(w.core.engine_kind(), Some(EngineKind::Pitwall));
    assert!(w.core.uses_pitwall());
    let s = w.core.status();
    assert_eq!(s.state, EngineStateDto::Ready);
    assert_eq!(
        (s.model.as_deref(), s.backend.as_deref()),
        (Some("qwen3-8b-q4km"), Some("Pitwall 1.0.4"))
    );
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["engine"]["choice"], "auto");
    assert_eq!(json["engine"]["active"], "pitwall");
    assert_eq!(json["engine"]["pitwall"]["found"], "running");
    assert_eq!(json["engine"]["pitwall"]["usable"], true);
    let r = w.core.provider().unwrap().chat(&hello(), &mut |_| {}).unwrap();
    assert_eq!(r.text, "Hello from Pitwall.");
    // Background work goes behind what the user waits for.
    w.core.background_provider().unwrap().chat(&hello(), &mut |_| {}).unwrap();
    let prio: Vec<String> = w
        .fake
        .requests_to("/v1/chat/completions")
        .into_iter()
        .filter_map(|(_, h, _)| h.into_iter().find(|(n, _)| n == "x-pitwall-priority").map(|(_, v)| v))
        .collect();
    assert_eq!(prio, ["interactive", "background"]);
    // No built-in model was loaded and none would preload.
    w.core.autoload();
    assert_eq!(*w.f.runtime.loads.lock().unwrap(), 0);
    assert_eq!(ai_health_of(&s), AiHealth::Fine);
}

#[test]
fn automatic_without_pairing_stays_on_the_built_in_engine() {
    let w = world("pw-unpaired", FakeState::default(), None);
    let v = w.pw.probe();
    assert!(!v.paired && !v.usable);
    assert_eq!(w.core.engine_kind(), Some(EngineKind::BuiltIn));
    assert_eq!(w.core.provider().err().unwrap().code, super::super::error::AiErrorCode::NoModel);
    let s = w.core.status();
    assert_eq!((s.state, s.engine.active), (EngineStateDto::NoModel, Some(EngineKind::BuiltIn)));
}

#[test]
fn pitwall_going_away_mid_session_is_degraded_with_the_fallback_never_a_silent_switch() {
    let state = FakeState {
        chat: ChatScript::Answer {
            pieces: vec!["ok".into()],
            finish: "stop".into(),
        },
        ..paired()
    };
    let w = world("pw-lost", state, Some("pw_ok"));
    w.pw.probe();
    w.core.provider().unwrap().chat(&hello(), &mut |_| {}).unwrap();
    w.fake.state().chat = ChatScript::DieAfter(vec!["Half".into()]);
    let err = w.core.provider().unwrap().chat(&hello(), &mut |_| {}).unwrap_err();
    assert!(err.to_string().contains("stopped answering"), "{err}");
    // Automatic does not continue on the built-in engine by itself.
    assert_eq!(w.core.engine_kind(), None);
    let s = w.core.status();
    assert_eq!(s.state, EngineStateDto::Failed);
    assert!(s.reason.as_deref().unwrap().contains("stopped answering"));
    assert_eq!(ai_health_of(&s), AiHealth::EngineLost);
    let e = w.core.provider().err().unwrap();
    assert_eq!(e.code, super::super::error::AiErrorCode::Engine);
    // The repair: the built-in engine for the session.
    w.core.use_built_in_for_session();
    assert_eq!(w.core.engine_kind(), Some(EngineKind::BuiltIn));
    assert!(w.core.status().engine.built_in_for_session);
    // Changing the setting ends the session's fallback.
    let mut next = w.core.settings();
    next.engine = EngineChoice::BuiltIn;
    w.core.apply_settings(next).unwrap();
    assert!(!w.core.status().engine.built_in_for_session);
    assert_eq!(ai_health(EngineStateDto::Ready), AiHealth::Fine);
}

#[test]
fn choosing_pitwall_when_it_is_not_there_says_so_instead_of_using_the_built_in_engine() {
    let w = world("pw-chosen", paired(), Some("pw_ok"));
    let mut s = w.core.settings();
    s.engine = EngineChoice::Pitwall;
    w.core.apply_settings(s).unwrap();
    w.fake.kill();
    std::thread::sleep(Duration::from_millis(50));
    w.pw.probe();
    assert_eq!(w.core.engine_kind(), None);
    assert_eq!(ai_health_of(&w.core.status()), AiHealth::EngineLost);
    // Built-in chosen: Pitwall is not used even when it is there.
    let w = world("pw-builtin", paired(), Some("pw_ok"));
    w.pw.probe();
    let mut s = w.core.settings();
    s.engine = EngineChoice::BuiltIn;
    w.core.apply_settings(s).unwrap();
    assert_eq!(w.core.engine_kind(), Some(EngineKind::BuiltIn));
    assert!(!w.core.uses_pitwall());
}

#[test]
fn use_pitwall_pairs_through_the_ipc_and_makes_pitwall_the_setting() {
    let w = world("pw-ipc", FakeState::default(), None);
    let app = w.f.app(w.core.clone());
    super::super::pitwall::start(app.handle(), w.core.clone());
    let main = window(&app, "main");
    let statuses = Arc::new(Mutex::new(Vec::<Value>::new()));
    let s2 = statuses.clone();
    app.listen_any(super::super::STATUS_EVENT, move |e| {
        s2.lock().unwrap().push(serde_json::from_str(e.payload()).unwrap());
    });
    wait_until("the first look at Pitwall", || {
        w.pw.view().found == debrief_ai::engine::pitwall::FoundKind::Running
    });
    let v = ipc(&main, "ai_pitwall_connect", json!({"useForDebrief": true})).unwrap();
    assert_eq!(v["pairing"]["code"], "482913");
    wait_until("the approval", || w.core.engine_kind() == Some(EngineKind::Pitwall));
    wait_until("the setting", || w.core.settings().engine == EngineChoice::Pitwall);
    wait_until("a status event for Pitwall", || {
        statuses.lock().unwrap().iter().any(|s| s["engine"]["active"] == "pitwall")
    });
    let status = ipc(&main, "ai_pitwall_refresh", json!({})).unwrap();
    assert_eq!(status["engine"]["pitwall"]["paired"], true);
    assert_eq!(ipc(&main, "ai_pitwall_cancel", json!({})).unwrap(), json!(false));
    let status = ipc(&main, "ai_engine_use_built_in", json!({})).unwrap();
    assert_eq!(status["engine"]["active"], "builtIn");
    let status = ipc(&main, "ai_pitwall_forget", json!({})).unwrap();
    assert_eq!(status["engine"]["pitwall"]["paired"], false);
}
