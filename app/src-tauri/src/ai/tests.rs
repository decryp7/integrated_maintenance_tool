//! The AI command layer through the real IPC path (`tauri::test`), with the fakes of `debrief-ai`: a scripted
//! model, a fake engine runtime and a fake model host. Set-up, models, use cases, chat streaming,
//! background tasks, the settings section and saved chats.

use super::commands::{check_link, markdown_target, sample_of};
use super::core::{AiCore, AiDeps};
use super::{AiState, SETUP_EVENT};
use crate::jobs::PhaseScale;
use crate::test_support::{samples, test_app_safe_mode, test_app_with_commands, window, TempDir};
use crate::tools::tests::ipc;
use debrief_ai::hardware::{Gpu, Hardware, HardwareProbe};
use debrief_ai::llm::{LlmProvider, Reply, ScriptedProvider};
use debrief_ai::models::source::{ModelSource, Progress, RemoteFile};
use debrief_ai::models::DiskSpace;
use debrief_ai::runtime::{LoadRequest, Loaded, ModelRuntime};
use debrief_ai::AiError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::test::MockRuntime;
use tauri::{Listener, Manager};

/// The laptop of the UI mock: 16 GB RAM and a GPU with 8 GB.
fn laptop() -> Hardware {
    Hardware {
        os: "linux".into(),
        arch: "x86_64".into(),
        cpu_cores: 8,
        total_ram_gb: 16.0,
        available_ram_gb: 12.0,
        gpus: vec![Gpu {
            name: "Laptop GPU".into(),
            vram_gb: Some(8.0),
            unified: false,
        }],
    }
}

struct Probe;
impl HardwareProbe for Probe {
    fn probe(&self) -> Hardware {
        laptop()
    }
}

/// An engine that "loads" any model into a scripted provider (first reply: the warm-up).
struct FakeRuntime {
    replies: Mutex<Vec<Reply>>,
    loads: Mutex<usize>,
    /// The projector of every load request (ADR 0025).
    projectors: Mutex<Vec<Option<PathBuf>>>,
}

impl FakeRuntime {
    fn new(replies: Vec<Reply>) -> Arc<FakeRuntime> {
        Arc::new(FakeRuntime {
            replies: Mutex::new(replies),
            loads: Mutex::new(0),
            projectors: Mutex::new(vec![]),
        })
    }

    fn projectors(&self) -> Vec<Option<PathBuf>> {
        self.projectors.lock().unwrap().clone()
    }
}

impl ModelRuntime for FakeRuntime {
    fn load(&self, req: &LoadRequest) -> debrief_ai::Result<Loaded> {
        *self.loads.lock().unwrap() += 1;
        self.projectors.lock().unwrap().push(req.projector.clone());
        let mut replies = vec![Reply::Text("OK".into())];
        replies.extend(self.replies.lock().unwrap().drain(..));
        Ok(Loaded {
            provider: Arc::new(ScriptedProvider::new(replies).with_context(req.context)),
            backend: "Fake GPU".into(),
            server: Some(debrief_ai::worker::protocol::ServerInfo {
                n_ctx: Some(req.context),
                build: Some("b1-fake".into()),
                ..Default::default()
            }),
        })
    }
}

/// A model host serving one file slowly, honouring cancel.
struct FakeHost {
    remote: RemoteFile,
    bytes: Vec<u8>,
    delay: Duration,
}

impl ModelSource for FakeHost {
    fn describe(&self, _: &str, _: &str) -> debrief_ai::Result<RemoteFile> {
        Ok(self.remote.clone())
    }
    fn fetch(
        &self,
        _: &RemoteFile,
        offset: u64,
        sink: &mut dyn FnMut(&[u8]) -> debrief_ai::Result<()>,
        cancel: &AtomicBool,
    ) -> debrief_ai::Result<()> {
        for chunk in self.bytes[offset as usize..].chunks(8 * 1024) {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            std::thread::sleep(self.delay);
            sink(chunk)?;
        }
        Ok(())
    }
}

struct BigDisk;
impl DiskSpace for BigDisk {
    fn available_bytes(&self, _: &Path) -> Option<u64> {
        Some(1 << 40)
    }
}

const TEST_MODEL: &str = "test-4b";
const TEST_FILE: &str = "Test-4B-Q4_K_M.gguf";

fn model_bytes() -> Vec<u8> {
    debrief_ai::testing::gguf::qwen_like()
}

fn sha256_of(bytes: &[u8], dir: &Path) -> String {
    let p = dir.join("hash.bin");
    std::fs::write(&p, bytes).unwrap();
    let h = debrief_ai::models::download::sha256_file(&p, &Progress::new()).unwrap();
    std::fs::remove_file(&p).unwrap();
    h
}

/// An AI folder with a catalog naming the test model (so the fake host's bytes verify).
fn ai_dir(tmp: &TempDir) -> PathBuf {
    let dir = tmp.0.join("ai");
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = model_bytes();
    let catalog = json!({"version": 1, "models": [{
        "id": TEST_MODEL, "name": "Test 4B", "family": "qwen3", "paramsB": 4.0, "quant": "Q4_K_M",
        "hfRepo": "test/Test-4B-GGUF", "file": TEST_FILE, "sha256": sha256_of(&bytes, &tmp.0),
        "sizeBytes": bytes.len(), "license": "apache-2.0", "toolCalling": true, "contextDefault": 4096
    }]});
    std::fs::write(dir.join("local-models.json"), catalog.to_string()).unwrap();
    dir
}

struct Fixture {
    tmp: TempDir,
    runtime: Arc<FakeRuntime>,
    host_delay: Duration,
    worker: Option<PathBuf>,
    safe_mode: bool,
    build_tests: Arc<BuildTests>,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        Fixture {
            tmp: TempDir::new(&format!("ai-{tag}")),
            runtime: FakeRuntime::new(vec![]),
            host_delay: Duration::ZERO,
            worker: None,
            safe_mode: false,
            build_tests: Arc::default(),
        }
    }

    fn deps(&self) -> AiDeps {
        let bytes = model_bytes();
        let remote = RemoteFile {
            repo: "test/Test-4B-GGUF".into(),
            file: TEST_FILE.into(),
            size: bytes.len() as u64,
            sha256: sha256_of(&bytes, &self.tmp.0),
            license: Some("apache-2.0".into()),
            gated: false,
        };
        let delay = self.host_delay;
        AiDeps {
            runtime: self.runtime.clone(),
            health: None,
            probe: Arc::new(Probe),
            source: Arc::new(move |_| {
                Ok(Box::new(FakeHost {
                    remote: remote.clone(),
                    bytes: bytes.clone(),
                    delay,
                }) as Box<dyn ModelSource>)
            }),
            disk: Arc::new(BigDisk),
            worker_path: self.worker.clone(),
            download_backoff: Duration::from_millis(1),
            runtimes: test_runtimes_with(&self.tmp, self.host_delay, self.build_tests.clone()),
            settings: Arc::default(),
            pitwall: None,
        }
    }

    fn core(&self) -> Arc<AiCore> {
        Arc::new(AiCore::new(
            Some(ai_dir(&self.tmp)),
            self.tmp.0.join("models"),
            self.safe_mode,
            self.deps(),
        ))
    }

    fn app(&self, core: Arc<AiCore>) -> tauri::App<MockRuntime> {
        let app = test_app_with_commands(Some(self.tmp.0.join("config/profiles")));
        super::manage(app.handle(), core);
        app
    }
}

fn scripted(replies: Vec<Reply>) -> Arc<dyn LlmProvider> {
    Arc::new(ScriptedProvider::new(replies))
}

fn text(s: &str) -> Reply {
    Reply::Text(s.to_string())
}

fn open_samples(w: &tauri::WebviewWindow<MockRuntime>) {
    let dir = samples().join("serilog-clef");
    ipc(w, "open_paths", json!({ "paths": [dir.to_string_lossy()] })).unwrap();
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        // Generous: a loaded runner stalls a test for many seconds; the bound only turns a hang into a message.
        assert!(start.elapsed() < Duration::from_secs(180), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Lets the running job become visible (the test clock moves past the 2 s threshold) and returns its
/// background task id once the tray lists it.
fn job_task(app: &tauri::App<MockRuntime>, main: &tauri::WebviewWindow<MockRuntime>) -> String {
    app.state::<crate::test_support::TestClock>().0.advance(3_000);
    let mut id = String::new();
    wait_until("the job in the background tasks", || {
        let tasks = ipc(main, "background_tasks", json!({})).unwrap();
        let found = tasks
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["kind"] == "job" && t["stoppable"] == true)
            .cloned();
        if let Some(t) = found {
            id = t["id"].as_str().unwrap().to_string();
        }
        !id.is_empty()
    });
    id
}

fn code(e: Value) -> String {
    e["code"].as_str().unwrap_or_default().to_string()
}

/// Waits for the chat's turn to end and returns its events.
fn finish_turn(w: &tauri::WebviewWindow<MockRuntime>, sid: &str) -> Vec<Value> {
    wait_until("the turn to end", || {
        ipc(w, "ai_chat_sessions", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["running"] == false)
    });
    ipc(w, "ai_chat_events_since", json!({"sessionId": sid, "seq": 0}))
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn kinds(events: &[Value]) -> Vec<String> {
    events.iter().map(|e| e["type"].as_str().unwrap().to_string()).collect()
}

#[test]
fn use_cases_route_through_ipc_to_the_model_and_the_engine() {
    let f = Fixture::new("usecases");
    let core = f.core();
    core.set_fixed_provider(scripted(vec![
        text(r#"{"ql": "level >= error", "explanation": "All errors and worse."}"#),
        text("The most likely root cause: not found in the logs."),
    ]));
    let app = f.app(core);
    let main = window(&app, "main");
    open_samples(&main);
    let status = ipc(&main, "ai_status", json!({})).unwrap();
    assert_eq!(
        (status["available"].as_bool(), status["state"].as_str()),
        (Some(true), Some("ready"))
    );
    // A query proposal: the job starts at once; a dialog that waits gets the whole answer.
    let id = start(&main, json!({"op": "query", "question": "show me the errors"}));
    let outcome = ipc(&main, "jobs_wait", json!({"id": id})).unwrap();
    assert_eq!(outcome["state"], "done");
    let q = outcome["result"]["proposal"].clone();
    assert_eq!(q["ql"], "level >= error");
    assert!(q["estimatedMatches"].as_u64().unwrap() > 0);
    // Root causes runs in the background: the command returns the job at once; the answer is its result.
    let id = start(&main, json!({"op": "rootCauses"}));
    let job = ended(&app, id).expect("an answer is kept: it needs the user's attention");
    assert_eq!(job.state, crate::jobs::JobState::Done);
    let n = ipc(&main, "jobs_result", json!({"id": id})).unwrap();
    assert_eq!(n["type"], "narrative");
    assert!(n["narrative"]["text"].as_str().unwrap().contains("not found in the logs"));
    let notices = ipc(&main, "notifications_list", json!({})).unwrap();
    let ready = notices["notices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["title"] == "Root causes ready")
        .expect("a message")
        .clone();
    assert_eq!(ready["primary"], json!({"type": "viewJob", "jobId": id}));
    // Chat turns and unknown requests do not start this way.
    assert_eq!(
        code(ipc(&main, "ai_start", json!({"request": {"op": "chat", "sessionId": "c", "text": "x"}})).unwrap_err()),
        "invalid"
    );
    assert_eq!(
        code(ipc(&main, "ai_start", json!({"request": {"op": "nope"}})).unwrap_err()),
        "invalid"
    );
    // Arguments are validated before anything runs.
    let invalid = |r: Value| code(ipc(&main, "ai_start", json!({ "request": r })).unwrap_err());
    assert_eq!(invalid(json!({"op": "query", "question": "  "})), "invalid");
    assert_eq!(invalid(json!({"op": "regex", "lines": []})), "invalid");
    assert_eq!(
        invalid(json!({"op": "pipeline", "goal": "drop health checks"})),
        "invalid",
        "no sample and no event"
    );
    // The scripted model has no more replies: the job fails with the engine's reason, and offers Retry.
    let id = start(&main, json!({"op": "summarize", "from": 0, "to": i64::MAX}));
    let failed = ended(&app, id).expect("failures are kept");
    assert_eq!(failed.state, crate::jobs::JobState::Failed);
    assert!(failed.retryable);
    assert_eq!(ipc(&main, "ai_cancel", json!({})).unwrap(), 0);
    let hw = ipc(&main, "ai_hardware", json!({})).unwrap();
    assert_eq!(hw["gpus"][0]["vramGb"], 8.0);
    let rec = ipc(&main, "ai_recommend", json!({})).unwrap();
    assert_eq!(rec["best"]["model"]["id"], TEST_MODEL);
    assert_eq!(ipc(&main, "ai_catalog", json!({})).unwrap().as_array().unwrap().len(), 1);
}

#[test]
fn proposal_samples_come_from_the_ui_or_the_selected_event() {
    let mut ws = debrief_core::Workspace::new(debrief_core::Profile::default_profile()).unwrap();
    ws.open(&[samples().join("serilog-text")]).unwrap();
    let ws = debrief_core::workspace::WorkspaceStore::new(ws);
    let direct: &dyn debrief_core::ports::Store = &ws;
    let store: debrief_ai::StoreRef = direct.into();
    let (s, f) = sample_of(store, None, Some("a\nb".into()), Some("x.log".into())).unwrap();
    assert_eq!((s.as_str(), f.as_str()), ("a\nb", "x.log"));
    assert_eq!(sample_of(store, None, Some("a".into()), None).unwrap().1, "sample.log");
    assert!(sample_of(store, None, Some("a".repeat(70_000)), None).is_err());
    let (s, f) = sample_of(store, Some(0), None, None).unwrap();
    assert!(!s.is_empty() && !f.is_empty(), "{f}");
    assert_eq!(
        sample_of(store, Some(u32::MAX), None, None).unwrap_err().code,
        super::error::AiErrorCode::NotFound
    );
    assert_eq!(
        sample_of(store, None, Some(" ".into()), None).unwrap_err().code,
        super::error::AiErrorCode::Invalid
    );
}

#[test]
fn a_missing_worker_safe_mode_or_no_runtime_are_typed_errors() {
    let mut f = Fixture::new("missing");
    f.worker = Some(f.tmp.0.join("no-such-worker"));
    let app = f.app(f.core());
    let main = window(&app, "main");
    let status = ipc(&main, "ai_status", json!({})).unwrap();
    assert_eq!(
        (status["available"].as_bool(), status["state"].as_str()),
        (Some(false), Some("unavailable"))
    );
    assert!(status["reason"].as_str().unwrap().contains("not installed"));
    for (cmd, body) in [
        ("ai_start", json!({"request": {"op": "query", "question": "errors"}})),
        ("ai_recommend", json!({})),
        ("ai_setup_plan", json!({})),
        ("ai_setup_run", json!({"modelId": TEST_MODEL})),
        ("ai_setup_background", json!({"modelId": TEST_MODEL})),
        ("ai_suggested_prompts", json!({})),
        ("ai_chat_send", json!({"sessionId": "chat-1", "text": "hi"})),
    ] {
        let e = ipc(&main, cmd, body).unwrap_err();
        assert_eq!(code(e.clone()), "unavailable", "{cmd}: {e}");
    }

    // Safe mode: no AI at all.
    let safe = test_app_safe_mode();
    let mut f = Fixture::new("safe");
    f.safe_mode = true;
    super::manage(safe.handle(), f.core());
    let s = ipc(&window(&safe, "main"), "ai_status", json!({})).unwrap();
    assert!(s["reason"].as_str().unwrap().contains("safe mode") && s["safeMode"] == true);

    // A build without the AI runtime: every command says so.
    let bare = test_app_with_commands(None);
    let e = ipc(&window(&bare, "main"), "ai_status", json!({})).unwrap_err();
    assert_eq!(code(e), "unavailable");
}

#[test]
fn before_set_up_there_is_no_model() {
    let f = Fixture::new("nomodel");
    let app = f.app(f.core());
    let main = window(&app, "main");
    assert_eq!(ipc(&main, "ai_status", json!({})).unwrap()["state"], "noModel");
    assert_eq!(
        code(ipc(&main, "ai_start", json!({"request": {"op": "query", "question": "errors"}})).unwrap_err()),
        "noModel"
    );
    let chat = ipc(&main, "ai_chat_new", json!({})).unwrap();
    let e = ipc(&main, "ai_chat_send", json!({"sessionId": chat["id"], "text": "hi"})).unwrap_err();
    assert_eq!(code(e), "noModel");
    let plan = ipc(&main, "ai_setup_plan", json!({})).unwrap();
    assert_eq!(plan["chosen"]["model"]["id"], TEST_MODEL);
    assert!(plan["downloadBytes"].as_u64().unwrap() > 0);
    assert_eq!(
        code(ipc(&main, "ai_setup_plan", json!({"modelId": "nope"})).unwrap_err()),
        "notFound"
    );
    assert_eq!(
        code(ipc(&main, "ai_setup_run", json!({"modelId": "nope"})).unwrap_err()),
        "notFound"
    );
}

fn setup_events(app: &tauri::App<MockRuntime>) -> Arc<Mutex<Vec<Value>>> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let e = events.clone();
    app.listen_any(SETUP_EVENT, move |ev| {
        e.lock().unwrap().push(serde_json::from_str::<Value>(ev.payload()).unwrap());
    });
    events
}

fn stages(events: &Arc<Mutex<Vec<Value>>>) -> Vec<String> {
    let mut s: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .map(|e| e["stage"].as_str().unwrap().to_string())
        .collect();
    s.dedup();
    s
}

#[test]
fn set_up_downloads_as_a_job_and_becomes_ready() {
    let mut f = Fixture::new("setup");
    f.host_delay = Duration::from_millis(15);
    let core = f.core();
    let app = f.app(core.clone());
    let events = setup_events(&app);
    let main = window(&app, "main");
    let finished = Arc::new(Mutex::new(Vec::<Value>::new()));
    let fin = finished.clone();
    app.state::<crate::jobs::Jobs>().subscribe(move |e| {
        if let crate::jobs::JobsEvent::Finished(f) = e {
            fin.lock().unwrap().push(serde_json::to_value(&f.job).unwrap());
        }
    });
    ipc(&main, "ai_setup_run", json!({"modelId": TEST_MODEL})).unwrap();
    assert_eq!(
        code(ipc(&main, "ai_setup_run", json!({"modelId": TEST_MODEL})).unwrap_err()),
        "busy"
    );
    assert_eq!(ipc(&main, "ai_status", json!({})).unwrap()["setupRunning"], true);
    wait_until("ready", || stages(&events).last().is_some_and(|s| s == "ready"));
    let s = stages(&events);
    assert_eq!(&s[..3], ["probing", "recommending", "downloading"], "{s:?}");
    assert!(s.contains(&"warmingUp".to_string()), "{s:?}");
    wait_until("the task to go", || {
        ipc(&main, "background_tasks", json!({})).unwrap().as_array().unwrap().is_empty() && !core.setup_running()
    });
    // The download ran as a download job whose View opens the model manager.
    wait_until("the job to end", || !finished.lock().unwrap().is_empty());
    let job = finished.lock().unwrap()[0].clone();
    assert_eq!((job["kind"].as_str(), job["state"].as_str()), (Some("download"), Some("done")));
    assert_eq!(job["result"], json!({"type": "model", "fileName": TEST_FILE}));
    assert_eq!(job["request"], json!({"op": "model", "modelId": TEST_MODEL}));
    let status = ipc(&main, "ai_status", json!({})).unwrap();
    assert_eq!(status["model"], TEST_FILE);
    wait_until("the primed model", || {
        ipc(&main, "ai_status", json!({})).unwrap()["state"] == "ready"
    });
    assert_eq!(ipc(&main, "ai_status", json!({})).unwrap()["backend"], "Fake GPU");
    assert_eq!(*f.runtime.loads.lock().unwrap(), 1, "the set-up's model is used, not loaded twice");
    let models = ipc(&main, "ai_models", json!({})).unwrap();
    assert_eq!(models[0]["active"], true);
    assert_eq!(models[0]["catalogId"], TEST_MODEL);
    assert_eq!(models[0]["assessment"]["fit"], "comfortable");
    // The choice is remembered in ai.json.
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(f.tmp.0.join("ai/ai.json")).unwrap()).unwrap();
    assert_eq!(saved["active_model"], TEST_FILE);
}

fn notices_titled(main: &tauri::WebviewWindow<MockRuntime>, title: &str) -> Vec<Value> {
    let all = ipc(main, "notifications_list", json!({})).unwrap();
    all["notices"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["title"] == title)
        .cloned()
        .collect()
}

#[test]
fn the_assistant_sets_itself_up_in_the_background_and_says_so_once() {
    let f = Fixture::new("assistant-bg");
    let core = f.core();
    let app = f.app(core.clone());
    let main = window(&app, "main");
    // Nothing is installed: the runtime comes first (a quiet job), then the model (a quiet job); no dialog is involved.
    let first = ipc(&main, "ai_setup_background", json!({"modelId": TEST_MODEL})).unwrap();
    assert!(first.as_u64().is_some(), "the first job is returned at once: {first}");
    wait_until("the assistant to be ready", || {
        !notices_titled(&main, "AI assistant ready").is_empty()
    });
    wait_until("the engine", || ipc(&main, "ai_status", json!({})).unwrap()["state"] == "ready");
    let ready = notices_titled(&main, "AI assistant ready");
    assert_eq!(ready.len(), 1, "one message: {ready:?}");
    assert_eq!(ready[0]["primary"], json!({"type": "openAssistant"}));
    assert_eq!(ready[0]["severity"], "success");
    assert!(notices_titled(&main, "The AI assistant could not be set up").is_empty());
    let o = ipc(&main, "ai_runtimes", json!({})).unwrap();
    assert_eq!(o["needsDownload"], false, "the runtime was installed first");
    // The jobs say what they are, and Retry runs the whole set-up again.
    let jobs = ipc(&main, "jobs_list", json!({})).unwrap()["jobs"].clone();
    let titles: Vec<String> = jobs
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|j| j["title"].as_str().map(str::to_string))
        .collect();
    assert!(titles.iter().all(|t| t.starts_with("Setting up the AI assistant")), "{titles:?}");
    assert!(
        jobs.as_array()
            .unwrap()
            .iter()
            .all(|j| j["request"] == json!({"op": "assistant", "modelId": TEST_MODEL})),
        "{jobs}"
    );
}

#[test]
fn a_failed_background_set_up_is_one_plain_message_with_retry_and_a_cancel_is_silent() {
    let f = Fixture::new("assistant-fail");
    let core = f.core();
    let app = f.app(core.clone());
    let main = window(&app, "main");
    // Another set-up holds the single slot: the model step cannot start once the runtime is in.
    let held = core.begin_setup().unwrap();
    ipc(&main, "ai_setup_background", json!({"modelId": TEST_MODEL})).unwrap();
    wait_until("the failure message", || {
        !notices_titled(&main, "The AI assistant could not be set up").is_empty()
    });
    let failed = notices_titled(&main, "The AI assistant could not be set up");
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["primary"], json!({"type": "setUpAssistant", "modelId": TEST_MODEL}));
    assert_eq!(failed[0]["severity"], "error");
    assert!(notices_titled(&main, "AI assistant ready").is_empty());
    drop(held);
    core.end_setup();
    // Unknown models are refused before anything is queued.
    assert_eq!(
        code(ipc(&main, "ai_setup_background", json!({"modelId": "nope"})).unwrap_err()),
        "notFound"
    );
}

#[test]
fn a_set_up_can_be_cancelled_from_the_app_or_the_tray() {
    let mut f = Fixture::new("cancel");
    f.host_delay = Duration::from_millis(40);
    let app = f.app(f.core());
    let events = setup_events(&app);
    let main = window(&app, "main");
    let job = ipc(&main, "ai_setup_run", json!({"modelId": TEST_MODEL})).unwrap();
    assert!(job.as_u64().is_some(), "the set-up is a job: {job}");
    wait_until("downloading", || stages(&events).contains(&"downloading".to_string()));
    // Stop it like the tray does (the running download job is listed there).
    let task = job_task(&app, &main);
    ipc(&main, "stop_background_task", json!({"id": task})).unwrap();
    wait_until("cancelled", || stages(&events).last().is_some_and(|s| s == "cancelled"));
    wait_until("no task", || {
        ipc(&main, "background_tasks", json!({})).unwrap().as_array().unwrap().is_empty()
    });
    assert_eq!(ipc(&main, "ai_status", json!({})).unwrap()["state"], "noModel");
}

#[test]
fn local_models_are_added_activated_and_deleted() {
    let f = Fixture::new("models");
    let core = f.core();
    let app = f.app(core.clone());
    let events = setup_events(&app);
    let main = window(&app, "main");
    crate::paths::allow_under(app.handle(), "main", &f.tmp.0);
    let src = f.tmp.0.join("My Model.gguf");
    std::fs::write(&src, model_bytes()).unwrap();
    // The check of the file is a job: the command returns its id at once and the model is the job's value.
    let started = crate::test_support::returns_fast(
        app.handle(),
        "ai_model_add_local",
        super::commands::ai_model_add_local(
            app.handle().clone(),
            crate::paths::WindowGrants::of(app.handle(), "main"),
            src.to_string_lossy().into_owned(),
        ),
    );
    let model = crate::test_support::job_value(&app, started.unwrap()).unwrap();
    assert_eq!(model["fileName"], TEST_FILE);
    let _ = ipc(&main, "ai_model_delete", json!({"fileName": TEST_FILE}));
    let added = ipc(&main, "ai_model_add_local", json!({"path": src.to_string_lossy()})).unwrap();
    // The bytes are the catalog model's (a manual download the browser saved under another name): verified
    // by SHA-256 and filed under the catalog's name, so it is recognised.
    assert_eq!(added["fileName"], TEST_FILE);
    assert_eq!(added["catalogId"], TEST_MODEL);
    assert_eq!(added["active"], false);
    for bad in [
        json!("relative.gguf"),
        json!(f.tmp.0.join("x.bin").to_string_lossy()),
        json!(f.tmp.0.join("none.gguf").to_string_lossy()),
        json!(""),
    ] {
        assert!(ipc(&main, "ai_model_add_local", json!({ "path": bad })).is_err());
    }
    assert_eq!(
        code(ipc(&main, "ai_model_activate", json!({"fileName": "../x.gguf"})).unwrap_err()),
        "invalid"
    );
    ipc(&main, "ai_model_activate", json!({"fileName": TEST_FILE})).unwrap();
    wait_until("ready", || stages(&events).last().is_some_and(|s| s == "ready"));
    wait_until("idle setup", || !core.setup_running());
    assert_eq!(ipc(&main, "ai_models", json!({})).unwrap()[0]["active"], true);
    // A failed activation reports a typed failure event.
    ipc(&main, "ai_model_activate", json!({"fileName": "missing.gguf"})).unwrap();
    wait_until("failed", || stages(&events).last().is_some_and(|s| s == "failed"));
    assert!(events.lock().unwrap().last().unwrap()["message"].as_str().is_some());
    wait_until("idle setup", || !core.setup_running());
    // Deleting the active model leaves no model.
    ipc(&main, "ai_model_delete", json!({"fileName": TEST_FILE})).unwrap();
    assert_eq!(ipc(&main, "ai_status", json!({})).unwrap()["state"], "noModel");
    assert!(ipc(&main, "ai_models", json!({})).unwrap().as_array().unwrap().is_empty());
    assert_eq!(
        code(ipc(&main, "ai_model_delete", json!({"fileName": TEST_FILE})).unwrap_err()),
        "notFound"
    );
    // Reset is harmless without a quarantine.
    assert_eq!(ipc(&main, "ai_reset", json!({})).unwrap()["state"], "noModel");
}

#[test]
fn chat_streams_to_its_window_and_history_round_trips_through_the_project() {
    let f = Fixture::new("chat");
    let core = f.core();
    core.set_fixed_provider(scripted(vec![
        Reply::Tools(String::new(), vec![("run_query".into(), json!({"ql": "level = fatal"}))]),
        text("The service crashed; see `level = fatal`."),
    ]));
    let app = f.app(core);
    let main = window(&app, "main");
    open_samples(&main);
    let chat = ipc(&main, "ai_chat_new", json!({})).unwrap();
    let sid = chat["id"].as_str().unwrap().to_string();
    ipc(
        &main,
        "ai_chat_send",
        json!({"sessionId": sid, "text": "What crashed?", "context": {"openView": "grid", "selectedIds": [999999]}}),
    )
    .unwrap();
    let ev = finish_turn(&main, &sid);
    let k = kinds(&ev);
    assert_eq!(&k[..2], ["toolStarted", "toolFinished"], "{k:?}");
    assert_eq!(k.last().unwrap(), "done");
    let done = ev.last().unwrap();
    assert!(done["citations"].as_array().unwrap().iter().any(|c| c["kind"] == "query"));
    assert!(done["notes"][0].as_str().unwrap().contains("do not exist"));
    assert!(ev.iter().all(|e| e["sessionId"].is_null()), "replayed envelopes are the library's");
    let since = ipc(&main, "ai_chat_events_since", json!({"sessionId": sid, "seq": 2})).unwrap();
    assert_eq!(since.as_array().unwrap()[0]["seq"], 3);
    let sessions = ipc(&main, "ai_chat_sessions", json!({})).unwrap();
    assert_eq!(
        (sessions[0]["title"].as_str(), sessions[0]["turns"].as_u64()),
        (Some("What crashed?"), Some(1))
    );
    let rec = ipc(&main, "ai_chat_get", json!({"sessionId": sid})).unwrap();
    assert!(rec["turns"][0]["assistant"].as_str().unwrap().contains("crashed"));
    assert_eq!(
        code(ipc(&main, "ai_chat_get", json!({"sessionId": "nope"})).unwrap_err()),
        "notFound"
    );
    assert_eq!(code(ipc(&main, "ai_chat_get", json!({"sessionId": ""})).unwrap_err()), "invalid");
    assert_eq!(
        code(ipc(&main, "ai_chat_send", json!({"sessionId": "nope", "text": "x"})).unwrap_err()),
        "notFound"
    );
    // The model has no more replies: the turn ends with an error event, and the chat is free again.
    ipc(&main, "ai_chat_send", json!({"sessionId": sid, "text": "And then?"})).unwrap();
    assert_eq!(kinds(&finish_turn(&main, &sid)).last().unwrap(), "error");

    // Export to Markdown, with the overwrite guard.
    let md = f.tmp.0.join("chat.md");
    let path = md.to_string_lossy().to_string();
    // Only to a file the user chose in the save dialog.
    assert_eq!(
        code(ipc(&main, "ai_chat_export_markdown", json!({"sessionId": sid, "path": path})).unwrap_err()),
        "notGranted"
    );
    assert!(!md.exists());
    crate::paths::grant(
        app.handle(),
        "main",
        &md,
        crate::paths::Access::Write,
        crate::paths::GrantSource::Dialog,
    )
    .unwrap();
    ipc(&main, "ai_chat_export_markdown", json!({"sessionId": sid, "path": path})).unwrap();
    assert!(std::fs::read_to_string(&md).unwrap().contains("**You:** What crashed?"));
    assert_eq!(
        code(ipc(&main, "ai_chat_export_markdown", json!({"sessionId": sid, "path": path})).unwrap_err()),
        "fileExists"
    );
    ipc(
        &main,
        "ai_chat_export_markdown",
        json!({"sessionId": sid, "path": path, "overwrite": true}),
    )
    .unwrap();

    // Saved in the project's UI state under `chats`, reopened in another window.
    let history = ipc(&main, "ai_chat_history", json!({})).unwrap();
    assert_eq!(history["version"], 1);
    let project = f.tmp.0.join("p.debriefproj");
    let file = debrief_core::project::ProjectFile {
        version: debrief_core::project::PROJECT_VERSION,
        name: "p".into(),
        sources: vec![],
        removed: vec![],
        profile: String::new(),
        ui: json!({ "version": 1, "chats": history }),
        clock: Default::default(),
        store: None,
        reports: vec![],
    };
    file.save(&project).unwrap();
    let loaded = debrief_core::project::ProjectFile::load(&project).unwrap();
    let other = window(&app, "w-2");
    let r = ipc(&other, "ai_chat_restore", json!({"record": loaded.ui["chats"]})).unwrap();
    assert!(r["warning"].is_null());
    assert_eq!(ipc(&other, "ai_chat_history", json!({})).unwrap(), history);
    // Old projects (no chats), damaged and newer records.
    assert_eq!(ipc(&other, "ai_chat_restore", json!({})).unwrap()["sessions"], json!([]));
    assert!(
        ipc(&other, "ai_chat_restore", json!({"record": {"version": "x"}})).unwrap()["warning"]
            .as_str()
            .unwrap()
            .contains("damaged")
    );
    assert!(
        ipc(&other, "ai_chat_restore", json!({"record": {"version": 9}})).unwrap()["warning"]
            .as_str()
            .unwrap()
            .contains("newer")
    );
    assert_eq!(ipc(&main, "ai_chat_delete", json!({"sessionId": sid})).unwrap(), true);
    assert!(ipc(&main, "ai_suggested_prompts", json!({})).unwrap().as_array().unwrap().len() >= 3);
}

#[test]
fn the_ai_settings_section_is_registered_and_applies() {
    let f = Fixture::new("settings");
    let core = f.core();
    let app = f.app(core.clone());
    let main = window(&app, "main");
    let overview = ipc(&main, "settings_sections", json!({})).unwrap();
    let ai = overview["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "ai")
        .unwrap()
        .clone();
    assert_eq!((ai["group"].as_str(), ai["status"].as_str()), (Some("ai"), Some("active")));
    let keys: Vec<&str> = ai["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["key"].as_str().unwrap())
        .collect();
    assert!(keys.contains(&"inference_build") && keys.contains(&"context_length"), "{keys:?}");
    assert!(!keys.contains(&"active_model"), "the model is chosen in the model manager");
    let r = ipc(
        &main,
        "set_settings",
        json!({"section": "ai", "values": {"autoload": false, "max_tokens": 512}}),
    )
    .unwrap();
    assert_eq!(r["applied"], true, "{r}");
    assert!(!core.settings().autoload);
    assert_eq!(core.settings().max_tokens, 512);
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(f.tmp.0.join("ai/ai.json")).unwrap()).unwrap();
    assert_eq!(
        (saved["autoload"].as_bool(), saved["max_tokens"].as_u64()),
        (Some(false), Some(512))
    );
    // Values the library refuses are not applied.
    let mut bad = core.settings();
    bad.hf_endpoint = "http://insecure".into();
    assert_eq!(core.apply_settings(bad).unwrap_err().code, super::error::AiErrorCode::Invalid);
    assert!(app.try_state::<AiState>().is_some());
}

#[test]
fn chat_exports_go_only_to_markdown_files_in_existing_folders() {
    let tmp = TempDir::new("ai-md");
    let ok = tmp.0.join("a.md");
    assert!(markdown_target(&ok.to_string_lossy(), false).is_ok());
    std::fs::write(&ok, "x").unwrap();
    assert_eq!(
        markdown_target(&ok.to_string_lossy(), false).unwrap_err().code,
        super::error::AiErrorCode::FileExists
    );
    assert!(markdown_target(&ok.to_string_lossy(), true).is_ok());
    for bad in ["", "rel.md", "/x/\0.md"] {
        assert_eq!(
            markdown_target(bad, false).unwrap_err().code,
            super::error::AiErrorCode::Invalid,
            "{bad}"
        );
    }
    assert!(markdown_target(&tmp.0.join("a.txt").to_string_lossy(), false).is_err());
    assert!(markdown_target(&tmp.0.join("nope/a.md").to_string_lossy(), false).is_err());
    let dir = tmp.0.join("d.md");
    std::fs::create_dir(&dir).unwrap();
    assert!(markdown_target(&dir.to_string_lossy(), true).is_err());
}

#[test]
fn only_public_https_links_from_answers_are_opened() {
    for ok in ["https://learn.example.com/a?b=c#d", "https://example.com:8443/x"] {
        assert!(check_link(ok).is_ok(), "{ok}");
    }
    for bad in [
        "http://example.com",
        "file:///etc/passwd",
        "https://localhost/x",
        "https://127.0.0.1/",
        "https://[::1]/",
        "https://10.0.0.5/",
        "https://user@example.com/",
        "https:///x",
        "https://exa mple.com",
        "javascript:alert(1)",
    ] {
        assert!(check_link(bad).is_err(), "{bad}");
    }
    assert!(check_link(&format!("https://example.com/{}", "a".repeat(3000))).is_err());
    // A public link that no answer showed is refused before anything is opened.
    let f = Fixture::new("links");
    let app = f.app(f.core());
    let main = window(&app, "main");
    let e = ipc(&main, "ai_open_link", json!({"url": "https://example.com/"})).unwrap_err();
    assert_eq!(code(e), "refused");
    assert!(!main
        .state::<crate::windows::Windows>()
        .get("main")
        .unwrap()
        .ai
        .chats
        .knows_url("https://example.com/"));
}

/// A model that, from its second answer on, keeps "thinking" until the test opens the gate (at most 20 s), to
/// show the workspace is free while it generates.
struct SlowModel {
    inner: ScriptedProvider,
    calls: std::sync::atomic::AtomicUsize,
    gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

impl LlmProvider for SlowModel {
    fn id(&self) -> &str {
        "slow"
    }
    fn display_name(&self) -> String {
        "Slow scripted model".into()
    }
    fn context_length(&self) -> usize {
        self.inner.context_length()
    }
    fn chat(
        &self,
        req: &debrief_ai::llm::ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> debrief_ai::Result<debrief_ai::llm::ChatResponse> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            let (open, cv) = &*self.gate;
            let guard = open.lock().unwrap();
            let _unused = cv.wait_timeout_while(guard, Duration::from_secs(180), |o| !*o).unwrap();
        }
        self.inner.chat(req, on_delta)
    }
}

#[test]
fn imports_do_not_wait_for_a_slow_turn_and_changed_citations_are_flagged() {
    let f = Fixture::new("slow");
    let core = f.core();
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    core.set_fixed_provider(Arc::new(SlowModel {
        inner: ScriptedProvider::new(vec![
            Reply::Tools(String::new(), vec![("get_events".into(), json!({"ids": [10]}))]),
            text("The failure is event 10."),
        ]),
        calls: Default::default(),
        gate: gate.clone(),
    }));
    let app = f.app(core);
    let main = window(&app, "main");
    open_samples(&main);
    let sid = ipc(&main, "ai_chat_new", json!({})).unwrap()["id"].as_str().unwrap().to_string();
    ipc(&main, "ai_chat_send", json!({"sessionId": sid, "text": "What failed?"})).unwrap();
    // The model read event 10 and is now writing its answer (until the gate opens).
    wait_until("the tool result", || {
        ipc(&main, "ai_chat_events_since", json!({"sessionId": sid, "seq": 0}))
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "toolFinished")
    });
    // An import into the same window (a write, like a live tail append) completes while the turn still runs,
    // and its files interleave with the open ones, so the ids are re-sorted.
    let start = Instant::now();
    let more = samples().join("serilog-text");
    ipc(&main, "open_paths", json!({ "paths": [more.to_string_lossy()] })).unwrap();
    let import_took = start.elapsed();
    let running = ipc(&main, "ai_chat_sessions", json!({})).unwrap()[0]["running"] == true;
    assert!(running, "the import finished in {import_took:?}, before the answer");
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    let ev = finish_turn(&main, &sid);
    let done = ev.iter().find(|e| e["type"] == "done").unwrap();
    let cited = done["citations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "record")
        .unwrap();
    // Event ids are stable in the store (they are never renumbered when files interleave), so the citation
    // still names the same event and stays verified.
    assert_eq!(
        (cited["id"].as_u64(), cited["verified"].as_bool()),
        (Some(10), Some(true)),
        "{done}"
    );
}

/// The real engine through the command layer:
///
///   scripts/build-worker.sh --release
///   cargo test -p debrief-app ai::tests::real -- --ignored --nocapture
///
/// `DEBRIEF_TEST_GGUF` picks the model (default: Qwen3.5 4B in /private/tmp/claude-501/models); `DEBRIEF_AI_WORKER`
/// the worker binary (default: the one `build-worker.sh` puts in `app/src-tauri/binaries`).
#[test]
#[ignore]
fn real_worker_set_up_with_a_local_model_answers_through_the_commands() {
    let model = std::env::var("DEBRIEF_TEST_GGUF").unwrap_or_else(|_| "/private/tmp/claude-501/models/Qwen3.5-4B-Q4_K_M.gguf".into());
    let worker = std::env::var("DEBRIEF_AI_WORKER").map(PathBuf::from).unwrap_or_else(|_| {
        let bins = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
        std::fs::read_dir(&bins)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("debrief-ai-worker-")))
            .expect("run scripts/build-worker.sh first")
    });
    let tmp = TempDir::new("ai-real");
    let core = Arc::new(AiCore::new(
        Some(tmp.0.join("ai")),
        tmp.0.join("models"),
        false,
        AiDeps::system(worker, &tmp.0),
    ));
    let app = test_app_with_commands(None);
    super::manage(app.handle(), core.clone());
    let events = setup_events(&app);
    let main = window(&app, "main");
    open_samples(&main);
    let t = Instant::now();
    let added = ipc(&main, "ai_model_add_local", json!({ "path": model })).unwrap();
    let file = added["fileName"].as_str().unwrap().to_string();
    println!("added {file} ({} bytes) in {:?}", added["sizeBytes"], t.elapsed());
    ipc(&main, "ai_model_activate", json!({ "fileName": file })).unwrap();
    let began = Instant::now();
    while !stages(&events).last().is_some_and(|s| s == "ready" || s == "failed") {
        assert!(
            began.elapsed() < Duration::from_secs(600),
            "activation timed out: {:?}",
            stages(&events)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("stages {:?} in {:?}", stages(&events), began.elapsed());
    assert_eq!(stages(&events).last().unwrap(), "ready", "{:?}", events.lock().unwrap().last());
    while core.setup_running() {
        std::thread::sleep(Duration::from_millis(50));
    }
    let status = ipc(&main, "ai_status", json!({})).unwrap();
    println!("status {status}");
    let t = Instant::now();
    let id = start(
        &main,
        json!({"op": "query", "question": "Which errors came from the billing service?"}),
    );
    let q = ipc(&main, "jobs_wait", json!({"id": id})).unwrap()["result"]["proposal"].clone();
    println!("nl_to_query in {:?}: {q}", t.elapsed());
    assert!(!q["ql"].as_str().unwrap().is_empty());
    assert!(q["estimatedMatches"].as_u64().is_some());
    let status = ipc(&main, "ai_status", json!({})).unwrap();
    println!("status after {status}");
    assert_eq!(status["state"], "ready");
    assert!(status["rssBytes"].as_u64().unwrap() > 0);
}

// ---- inference runtimes ----

const RT_TAG: &str = "b7";
const SERVER_FILE: &str = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// The archives of the test catalog: `cpu` and `vulkan` hold llama-server; `cuda-13` does not (broken).
fn runtime_assets() -> Vec<(String, Vec<u8>)> {
    let server = zip_of(&[(SERVER_FILE, b"fake server"), ("LICENSE", b"MIT")]);
    // Incompressible, so the download takes a while (cancel, ticker).
    let mut x: u32 = 1;
    let noise: Vec<u8> = (0..128 * 1024)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect();
    let gpu = zip_of(&[(SERVER_FILE, &noise), ("ggml-vulkan.dll", b"lib")]);
    let broken = zip_of(&[("README.md", b"no server here")]);
    vec![
        ("llama-b7-bin-test-cpu.zip".into(), server),
        ("llama-b7-bin-test-vulkan.zip".into(), gpu),
        ("llama-b7-bin-test-cuda-13.zip".into(), broken),
    ]
}

fn runtime_catalog(assets: &[(String, Vec<u8>)], dir: &Path) -> debrief_ai::runtimes::RuntimeCatalog {
    let builds: Vec<Value> = assets
        .iter()
        .map(|(name, data)| {
            let id = name.trim_start_matches("llama-b7-bin-test-").trim_end_matches(".zip");
            json!({"id": id, "assets": [{
                "name": name,
                "url": format!("https://github.com/ggml-org/llama.cpp/releases/download/{RT_TAG}/{name}"),
                "size": data.len(),
                "sha256": sha256_of(data, dir),
            }]})
        })
        .collect();
    let json = json!({"version": 1, "release": "v9", "tag": RT_TAG, "platforms": {"test": builds}});
    debrief_ai::runtimes::RuntimeCatalog::parse(&json.to_string()).unwrap()
}

/// GitHub releases, served from memory (slowly when asked, honouring cancel).
struct FakeReleases {
    assets: Vec<(String, Vec<u8>)>,
    catalog: debrief_ai::runtimes::RuntimeCatalog,
    delay: Duration,
}

impl ModelSource for FakeReleases {
    fn describe(&self, tag: &str, name: &str) -> debrief_ai::Result<RemoteFile> {
        let a = self
            .catalog
            .platforms
            .values()
            .flatten()
            .flat_map(|b| &b.assets)
            .find(|a| a.name == name)
            .ok_or_else(|| AiError::NotFound(name.into()))?;
        Ok(RemoteFile {
            repo: tag.into(),
            file: a.name.clone(),
            size: a.size,
            sha256: a.sha256.clone(),
            license: Some("MIT".into()),
            gated: false,
        })
    }
    fn fetch(
        &self,
        remote: &RemoteFile,
        offset: u64,
        sink: &mut dyn FnMut(&[u8]) -> debrief_ai::Result<()>,
        cancel: &AtomicBool,
    ) -> debrief_ai::Result<()> {
        let bytes = &self.assets.iter().find(|(n, _)| *n == remote.file).unwrap().1;
        for chunk in bytes[offset as usize..].chunks(4 * 1024) {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            std::thread::sleep(self.delay);
            sink(chunk)?;
        }
        Ok(())
    }
}

/// What the self-test of a new build sees: the servers it was started with and the replies its model gives (each
/// test run takes the next script; none left: it answers "OK" and "Red.").
#[derive(Default)]
struct BuildTests {
    servers: Mutex<Vec<PathBuf>>,
    scripts: Mutex<Vec<Vec<Reply>>>,
}

struct BuildUnderTest(Mutex<Vec<Reply>>);

impl ModelRuntime for BuildUnderTest {
    fn load(&self, req: &LoadRequest) -> debrief_ai::Result<Loaded> {
        let replies = std::mem::take(&mut *self.0.lock().unwrap());
        Ok(Loaded {
            provider: Arc::new(ScriptedProvider::new(replies).with_context(req.context)),
            backend: "CPU".into(),
            server: None,
        })
    }
}

/// The runtime manager's adapters for tests: builds in `<tmp>/runtimes`, the fake releases, and a build
/// report that lists every installed build as usable except Vulkan (no real GPU), like the worker would.
fn test_runtimes(tmp: &TempDir, delay: Duration) -> super::runtimes::RuntimeDeps {
    test_runtimes_with(tmp, delay, Arc::new(BuildTests::default()))
}

fn test_runtimes_with(tmp: &TempDir, delay: Duration, tests: Arc<BuildTests>) -> super::runtimes::RuntimeDeps {
    let assets = runtime_assets();
    let catalog = runtime_catalog(&assets, &tmp.0);
    let c = catalog.clone();
    super::runtimes::RuntimeDeps {
        store: Arc::new(debrief_ai::runtimes::RuntimeStore::new(tmp.0.join("runtimes"))),
        catalog,
        platform: "test".into(),
        source: Arc::new(move |_, _| {
            Ok(Box::new(FakeReleases {
                assets: assets.clone(),
                catalog: c.clone(),
                delay,
            }) as Box<dyn ModelSource>)
        }),
        report: Arc::new(|env: &[(String, String)]| {
            let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
            if get("DEBRIEF_LLAMA_SERVER").is_some_and(|p| p.contains("broken")) {
                return Err(AiError::Engine("checking the inference builds failed".into()));
            }
            let dir = PathBuf::from(get("DEBRIEF_LLAMA_DIR").unwrap_or_default());
            let mut builds: Vec<debrief_ai::runtimes::BuildInfo> = std::fs::read_dir(&dir)
                .map(|it| {
                    it.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
                .into_iter()
                .filter(|id| !id.starts_with('.') && id != "manifest.json")
                .map(|id| debrief_ai::runtimes::BuildInfo {
                    usable: id != "vulkan",
                    problem: (id == "vulkan").then(|| "no supported GPU was found".to_string()),
                    devices: vec![],
                    label: debrief_ai::runtimes::builds::label(&id),
                    id,
                })
                .collect();
            builds.sort_by(|a, b| a.id.cmp(&b.id));
            Ok(debrief_ai::runtimes::BuildsReport {
                release: Some(RT_TAG.into()),
                source: "installed".into(),
                choice: get("DEBRIEF_LLAMA_BUILD").unwrap_or_default(),
                auto: None,
                builds,
                problem: None,
            })
        }),
        smoke: Arc::new(|_: &Path, _: &str, _: &Arc<std::sync::atomic::AtomicBool>| Ok(())),
        build_runtime: Arc::new(move |server: &Path| {
            tests.servers.lock().unwrap().push(server.to_path_buf());
            let mut scripts = tests.scripts.lock().unwrap();
            let replies = if scripts.is_empty() {
                vec![text("OK"), text("Red.")]
            } else {
                scripts.remove(0)
            };
            Arc::new(BuildUnderTest(Mutex::new(replies))) as Arc<dyn ModelRuntime>
        }),
    }
}

fn runtime_events(app: &tauri::App<MockRuntime>) -> Arc<Mutex<Vec<Value>>> {
    let events = Arc::new(Mutex::new(Vec::new()));
    let e = events.clone();
    app.listen_any(super::RUNTIME_EVENT, move |ev| {
        e.lock().unwrap().push(serde_json::from_str::<Value>(ev.payload()).unwrap());
    });
    events
}

/// Waits for the install of `id` to end and returns its final event.
fn install_end(events: &Arc<Mutex<Vec<Value>>>, id: &str) -> Value {
    let end = |events: &Arc<Mutex<Vec<Value>>>| {
        events
            .lock()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id && matches!(e["stage"].as_str(), Some("ready" | "failed" | "cancelled")))
            .cloned()
    };
    wait_until("the install to end", || end(events).is_some());
    end(events).unwrap()
}

fn build<'a>(overview: &'a Value, id: &str) -> &'a Value {
    overview["builds"].as_array().unwrap().iter().find(|b| b["id"] == id).unwrap()
}

fn ai_section(main: &tauri::WebviewWindow<MockRuntime>) -> Value {
    let overview = ipc(main, "settings_sections", json!({})).unwrap();
    overview["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "ai")
        .unwrap()
        .clone()
}

fn inference_build_options(main: &tauri::WebviewWindow<MockRuntime>) -> Vec<String> {
    let ai = ai_section(main);
    let field = ai["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["key"] == "inference_build")
        .unwrap()
        .clone();
    let options = field["kind"]["options"].as_array().cloned().unwrap_or_default();
    options.iter().map(|o| o["value"].as_str().unwrap().to_string()).collect()
}

#[test]
fn runtimes_are_listed_installed_as_a_background_task_chosen_and_deleted() {
    let mut f = Fixture::new("runtimes");
    f.host_delay = Duration::from_millis(2);
    let core = f.core();
    let app = f.app(core.clone());
    let events = runtime_events(&app);
    let main = window(&app, "main");

    // Nothing installed: the set-up should offer the suggested download (Vulkan for the laptop's GPU).
    let o = ipc(&main, "ai_runtimes", json!({})).unwrap();
    assert_eq!(
        (o["needsDownload"].as_bool(), o["suggested"].as_str()),
        (Some(true), Some("vulkan"))
    );
    assert_eq!(
        (o["tag"].as_str(), o["choice"].as_str(), o["active"].as_str()),
        (Some(RT_TAG), Some("auto"), None)
    );
    assert_eq!(o["builds"].as_array().unwrap().len(), 3);
    assert!(build(&o, "cpu")["downloadBytes"].as_u64().unwrap() > 0 && build(&o, "cpu")["installed"] == false);
    assert_eq!(inference_build_options(&main), ["auto"], "only installed builds can be chosen");

    // Install the CPU build: stages, ticker, ready; listed as a stoppable download meanwhile.
    ipc(&main, "ai_runtime_install", json!({"id": "cpu"})).unwrap();
    let end = install_end(&events, "cpu");
    assert_eq!(end["stage"], "ready", "{end}");
    assert_eq!(
        (end["runtime"]["tag"].as_str(), end["runtime"]["current"].as_bool()),
        (Some(RT_TAG), Some(true))
    );
    let stages: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .map(|e| e["stage"].as_str().unwrap().to_string())
        .collect();
    for s in ["downloading", "unpacking", "finishing", "ready"] {
        assert!(stages.contains(&s.to_string()), "{s} in {stages:?}");
    }
    let first = events.lock().unwrap()[0].clone();
    assert_eq!((first["index"].as_u64(), first["count"].as_u64()), (Some(1), Some(1)));
    wait_until("the task to go", || {
        ipc(&main, "background_tasks", json!({})).unwrap().as_array().unwrap().is_empty()
    });
    let o = ipc(&main, "ai_runtimes", json!({})).unwrap();
    assert_eq!((o["needsDownload"].as_bool(), o["active"].as_str()), (Some(false), Some("cpu")));
    let cpu = build(&o, "cpu");
    assert!(cpu["installed"] == true && cpu["usable"] == true && cpu["active"] == true && cpu["installedBytes"].as_u64() > Some(0));
    assert_eq!(inference_build_options(&main), ["auto", "cpu"]);

    // A broken download is reported and leaves nothing behind.
    ipc(&main, "ai_runtime_install", json!({"id": "cuda-13"})).unwrap();
    let end = install_end(&events, "cuda-13");
    assert_eq!(
        (end["stage"].as_str(), end["code"].as_str()),
        (Some("failed"), Some("integrity")),
        "{end}"
    );
    assert!(!f.tmp.0.join("runtimes").join(RT_TAG).join("cuda-13").exists());

    // An installed GPU build that sees no GPU is listed with its problem and not used automatically.
    ipc(&main, "ai_runtime_install", json!({"id": "vulkan"})).unwrap();
    assert_eq!(install_end(&events, "vulkan")["stage"], "ready");
    let o = ipc(&main, "ai_runtimes", json!({})).unwrap();
    let vk = build(&o, "vulkan");
    assert!(vk["usable"] == false && vk["problem"].as_str().unwrap().contains("GPU") && vk["suggested"] == true);
    assert_eq!(o["active"], "cpu");

    // Choosing goes through the settings (saved and validated there) and reaches the worker's environment.
    ipc(&main, "ai_runtime_choose", json!({"id": "vulkan"})).unwrap();
    assert_eq!(core.settings().inference_build, "vulkan");
    let env = core.deps.runtimes.worker_env(&core.settings());
    assert!(env.contains(&("DEBRIEF_LLAMA_BUILD".to_string(), "vulkan".to_string())), "{env:?}");
    assert_eq!(ai_section(&main)["values"]["inference_build"], "vulkan");
    assert_eq!(ipc(&main, "ai_runtimes", json!({})).unwrap()["choice"], "vulkan");
    ipc(&main, "ai_runtime_choose", json!({"id": "AUTO"})).unwrap();
    assert_eq!(core.settings().inference_build, "auto");
    for (id, want) in [("cuda-13", "notFound"), ("../x", "invalid"), ("", "invalid")] {
        assert_eq!(code(ipc(&main, "ai_runtime_choose", json!({"id": id})).unwrap_err()), want, "{id}");
    }

    // Deleting removes the build (its option disappears); bad names are refused before anything runs.
    ipc(&main, "ai_runtime_delete", json!({"tag": RT_TAG, "id": "vulkan"})).unwrap();
    assert_eq!(inference_build_options(&main), ["auto", "cpu"]);
    assert_eq!(
        code(ipc(&main, "ai_runtime_delete", json!({"tag": RT_TAG, "id": "vulkan"})).unwrap_err()),
        "notFound"
    );
    assert_eq!(
        code(ipc(&main, "ai_runtime_delete", json!({"tag": "..", "id": "cpu"})).unwrap_err()),
        "invalid"
    );
    assert_eq!(
        code(ipc(&main, "ai_runtime_install", json!({"id": "metal"})).unwrap_err()),
        "notFound"
    );
}

#[test]
fn a_runtime_install_can_be_cancelled_and_only_one_runs() {
    let mut f = Fixture::new("rt-cancel");
    f.host_delay = Duration::from_millis(30);
    let app = f.app(f.core());
    let events = runtime_events(&app);
    let main = window(&app, "main");
    let job = ipc(&main, "ai_runtime_install", json!({"id": "vulkan"})).unwrap();
    assert!(job.as_u64().is_some(), "the install is a job: {job}");
    assert_eq!(code(ipc(&main, "ai_runtime_install", json!({"id": "cpu"})).unwrap_err()), "busy");
    let task = job_task(&app, &main);
    wait_until("progress", || events.lock().unwrap().iter().any(|e| e["stage"] == "progress"));
    let tick = events.lock().unwrap().iter().find(|e| e["stage"] == "progress").cloned().unwrap();
    assert!(tick["total"].as_u64().unwrap() > 0 && tick.get("bytesPerSec").is_some() && tick["verifying"] == false);
    // Stop it like the tray does.
    ipc(&main, "stop_background_task", json!({"id": task})).unwrap();
    assert_eq!(install_end(&events, "vulkan")["stage"], "cancelled");
    wait_until("no task", || {
        ipc(&main, "background_tasks", json!({})).unwrap().as_array().unwrap().is_empty()
    });
    assert_eq!(ipc(&main, "ai_runtimes", json!({})).unwrap()["needsDownload"], true);
}

#[test]
fn changing_the_build_restarts_a_loaded_engine_and_deleting_stops_it() {
    let f = Fixture::new("rt-restart");
    let core = f.core();
    let app = f.app(core.clone());
    let events = setup_events(&app);
    let rt_events = runtime_events(&app);
    let main = window(&app, "main");
    ipc(&main, "ai_runtime_install", json!({"id": "cpu"})).unwrap();
    assert_eq!(install_end(&rt_events, "cpu")["stage"], "ready");
    ipc(&main, "ai_setup_run", json!({"modelId": TEST_MODEL})).unwrap();
    wait_until("ready", || stages(&events).last().is_some_and(|s| s == "ready"));
    wait_until("loaded", || ipc(&main, "ai_status", json!({})).unwrap()["state"] == "ready");
    assert_eq!(*f.runtime.loads.lock().unwrap(), 1);
    // A new build: the engine restarts with it (loaded again, in the background).
    ipc(&main, "ai_runtime_choose", json!({"id": "cpu"})).unwrap();
    wait_until("reloaded", || *f.runtime.loads.lock().unwrap() == 2);
    // The same choice again changes nothing.
    ipc(&main, "ai_runtime_choose", json!({"id": "cpu"})).unwrap();
    wait_until("ready again", || ipc(&main, "ai_status", json!({})).unwrap()["state"] == "ready");
    assert_eq!(*f.runtime.loads.lock().unwrap(), 2);
    // Deleting the build in use stops the engine first; the model stays chosen and loads on next use.
    ipc(&main, "ai_runtime_delete", json!({"tag": RT_TAG, "id": "cpu"})).unwrap();
    let status = ipc(&main, "ai_status", json!({})).unwrap();
    assert_eq!(
        (status["state"].as_str(), status["model"].as_str()),
        (Some("idle"), Some(TEST_FILE))
    );
    assert!(!f.tmp.0.join("runtimes").join(RT_TAG).join("cpu").exists());
}

#[test]
fn a_new_runtime_is_switched_to_only_after_it_passes_its_self_test() {
    let f = Fixture::new("rt-selftest");
    let core = f.core();
    let app = f.app(core.clone());
    let events = setup_events(&app);
    let rt_events = runtime_events(&app);
    let main = window(&app, "main");
    // Without an active model there is no answer to test: only the "does it run here" check runs (a seam in these
    // tests), and the build installs.
    ipc(&main, "ai_runtime_install", json!({"id": "cpu"})).unwrap();
    assert_eq!(install_end(&rt_events, "cpu")["stage"], "ready");
    assert!(f.build_tests.servers.lock().unwrap().is_empty());
    let order: Vec<String> = rt_events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| e["stage"].as_str().map(String::from))
        .collect();
    let pos = |s: &str| order.iter().position(|x| x == s);
    assert!(
        pos("unpacking") < pos("finishing") && pos("finishing") < pos("testing") && pos("testing") < pos("ready"),
        "{order:?}"
    );
    ipc(&main, "ai_setup_run", json!({"modelId": TEST_MODEL})).unwrap();
    wait_until("ready", || stages(&events).last().is_some_and(|s| s == "ready"));
    wait_until("loaded", || ipc(&main, "ai_status", json!({})).unwrap()["state"] == "ready");
    let build_dir = f.tmp.0.join("runtimes").join(RT_TAG).join("cpu");
    let marker = std::fs::read_to_string(build_dir.join("runtime.json")).unwrap();
    std::fs::write(build_dir.join("previous.txt"), "the build in use").unwrap();

    // An update whose model cannot answer: refused with a typed error, the build in use stays.
    *f.build_tests.scripts.lock().unwrap() = vec![vec![Reply::Fail(AiError::Engine("llama-server crashed".into()))]];
    rt_events.lock().unwrap().clear();
    ipc(&main, "ai_runtime_install", json!({"id": "cpu"})).unwrap();
    let end = install_end(&rt_events, "cpu");
    assert_eq!(
        (end["stage"].as_str(), end["code"].as_str()),
        (Some("failed"), Some("selfTestFailed")),
        "{end}"
    );
    let msg = end["message"].as_str().unwrap();
    assert!(
        msg.contains("llama-server crashed") && msg.contains("keeps using the previous runtime"),
        "{msg}"
    );
    assert!(rt_events.lock().unwrap().iter().any(|e| e["stage"] == "testing"));
    assert_eq!(std::fs::read_to_string(build_dir.join("previous.txt")).unwrap(), "the build in use");
    assert_eq!(std::fs::read_to_string(build_dir.join("runtime.json")).unwrap(), marker);
    let tested = f.build_tests.servers.lock().unwrap()[0].clone();
    assert!(
        tested.parent().unwrap().ends_with(".staging-cpu"),
        "the unpacked copy is tested: {tested:?}"
    );
    // The engine was stopped for the test and loads again on next use.
    assert_eq!(ipc(&main, "ai_status", json!({})).unwrap()["state"], "idle");

    // One that answers is installed.
    rt_events.lock().unwrap().clear();
    ipc(&main, "ai_runtime_install", json!({"id": "cpu"})).unwrap();
    assert_eq!(install_end(&rt_events, "cpu")["stage"], "ready");
    assert!(!build_dir.join("previous.txt").exists(), "replaced by the tested build");
    assert_eq!(f.build_tests.servers.lock().unwrap().len(), 2);
}

#[test]
fn the_status_reports_what_the_inference_server_said() {
    let f = Fixture::new("rt-server-info");
    let core = f.core();
    let app = f.app(core.clone());
    let events = setup_events(&app);
    let main = window(&app, "main");
    assert_eq!(ipc(&main, "ai_status", json!({})).unwrap()["server"], Value::Null);
    ipc(&main, "ai_setup_run", json!({"modelId": TEST_MODEL})).unwrap();
    wait_until("ready", || stages(&events).last().is_some_and(|s| s == "ready"));
    wait_until("loaded", || ipc(&main, "ai_status", json!({})).unwrap()["state"] == "ready");
    let server = ipc(&main, "ai_status", json!({})).unwrap()["server"].clone();
    assert_eq!(server["build"], "b1-fake");
    assert!(server["context"].as_u64().unwrap() > 0);
    assert_eq!(
        (server["mediaMarker"].clone(), server["chatTemplate"].clone()),
        (Value::Null, json!(false))
    );
}

#[test]
fn a_missing_runtime_is_a_typed_error_and_a_failed_report_is_shown() {
    struct NoRuntime;
    impl ModelRuntime for NoRuntime {
        fn load(&self, _: &LoadRequest) -> debrief_ai::Result<Loaded> {
            Err(AiError::NoRuntime)
        }
    }
    let f = Fixture::new("rt-missing");
    let mut deps = f.deps();
    deps.runtime = Arc::new(NoRuntime);
    let core = Arc::new(AiCore::new(Some(ai_dir(&f.tmp)), f.tmp.0.join("models"), false, deps));
    let app = f.app(core.clone());
    let events = setup_events(&app);
    let main = window(&app, "main");
    ipc(&main, "ai_setup_run", json!({"modelId": TEST_MODEL})).unwrap();
    wait_until("failed", || stages(&events).last().is_some_and(|s| s == "failed"));
    let last = events.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last["code"], "noRuntime", "{last}");
    assert!(last["message"].as_str().unwrap().contains("download one in the AI set-up"));
    // A custom llama-server path replaces the downloads; when the worker cannot check it, the overview says why.
    let mut s = core.settings();
    s.llama_server_path = Some(
        if cfg!(windows) {
            r"C:\broken\llama-server.exe"
        } else {
            "/broken/llama-server"
        }
        .into(),
    );
    core.apply_settings(s).unwrap();
    let o = ipc(&main, "ai_runtimes", json!({})).unwrap();
    assert_eq!(o["needsDownload"], false);
    assert!(o["customPath"].as_str().unwrap().contains("broken"));
    assert!(o["problem"].as_str().unwrap().contains("failed"), "{o}");
}

#[test]
fn the_worker_environment_follows_the_settings_at_each_launch() {
    let tmp = TempDir::new("rt-env");
    let rt = test_runtimes(&tmp, Duration::ZERO);
    let cell = Arc::new(std::sync::RwLock::new(debrief_ai::settings::AiSettings::default()));
    let env = rt.worker_env_source(cell.clone());
    let chosen = |env: &[(String, String)]| env.iter().find(|(k, _)| k == "DEBRIEF_LLAMA_BUILD").map(|(_, v)| v.clone());
    assert_eq!(chosen(&env()).as_deref(), Some("auto"));
    assert!(env().contains(&(
        "DEBRIEF_LLAMA_DIR".to_string(),
        tmp.0.join("runtimes").join(RT_TAG).to_string_lossy().into_owned()
    )));
    cell.write().unwrap().inference_build = "cpu".into();
    assert_eq!(chosen(&env()).as_deref(), Some("cpu"));
}

#[test]
fn worker_memory_counts_the_workers_llama_server_children() {
    use super::core::tree_rss_bytes;
    let rows = vec![
        (10, Some(1), "debrief-ai-worker".to_string(), 30),
        (11, Some(10), "llama-server".to_string(), 3000),
        (12, Some(11), "grandchild".to_string(), 7),
        (20, Some(1), "debrief-plugin-host".to_string(), 500),
        (21, Some(20), "llama-server".to_string(), 999),
        (30, Some(2), "debrief-ai-worker".to_string(), 40),
        (31, Some(30), "llama-server".to_string(), 4000),
    ];
    assert_eq!(
        tree_rss_bytes(1, &rows),
        3030,
        "only this Debrief's workers and their direct children"
    );
    assert_eq!(tree_rss_bytes(99, &rows), 0);
}

/// What the gated model does with its next call.
enum Step {
    Answer,
    /// The worker died mid-answer.
    Crash,
}

/// A model that answers (from a script) only when the test says so: requests visibly wait in the AI queue.
struct GatedProvider {
    inner: ScriptedProvider,
    steps: Mutex<std::sync::mpsc::Receiver<Step>>,
    /// How many requests reached the model (and wait for their step).
    entered: Arc<std::sync::atomic::AtomicUsize>,
}

impl LlmProvider for GatedProvider {
    fn id(&self) -> &str {
        "gated"
    }
    fn display_name(&self) -> String {
        "Gated test model".into()
    }
    fn context_length(&self) -> usize {
        self.inner.context_length()
    }
    fn chat(
        &self,
        req: &debrief_ai::llm::ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> debrief_ai::Result<debrief_ai::llm::ChatResponse> {
        self.entered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match self.steps.lock().unwrap().recv() {
            Ok(Step::Answer) => self.inner.chat(req, on_delta),
            _ => Err(AiError::Engine("the AI worker crashed (exit code 3)".into())),
        }
    }
}

fn gated(replies: Vec<Reply>) -> (Arc<dyn LlmProvider>, std::sync::mpsc::Sender<Step>) {
    let (p, tx, _) = gated_counted(replies);
    (p, tx)
}

/// Like [`gated`], with the count of requests that reached the model.
fn gated_counted(
    replies: Vec<Reply>,
) -> (
    Arc<dyn LlmProvider>,
    std::sync::mpsc::Sender<Step>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let entered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let p = GatedProvider {
        inner: ScriptedProvider::new(replies),
        steps: Mutex::new(rx),
        entered: entered.clone(),
    };
    (Arc::new(p), tx, entered)
}

/// Starts a background AI answer (explain, summarise, root causes) and returns its job.
fn start(main: &tauri::WebviewWindow<MockRuntime>, request: Value) -> u64 {
    ipc(main, "ai_start", json!({ "request": request })).unwrap().as_u64().unwrap()
}

/// Waits until job `id` ended; its info (none when it ended unseen and was not kept).
fn ended(app: &tauri::App<MockRuntime>, id: u64) -> Option<crate::jobs::queue::JobInfo> {
    let jobs = jobs_of(app);
    let id = crate::jobs::JobId(id);
    wait_until("the job to end", || jobs.state_of(id).is_none_or(|s| s.is_finished()));
    jobs.info(id)
}

fn jobs_of(app: &tauri::App<MockRuntime>) -> crate::jobs::Jobs {
    app.state::<crate::jobs::Jobs>().inner().clone()
}

fn active_ai(app: &tauri::App<MockRuntime>) -> Vec<crate::jobs::JobId> {
    jobs_of(app).find_active(|j| j.kind == crate::jobs::JobKind::Ai)
}

#[test]
fn ai_requests_are_jobs_run_one_at_a_time_and_a_crash_fails_only_the_running_one() {
    let f = Fixture::new("ai-jobs");
    let core = f.core();
    let (model, steps) = gated(vec![text("Chat answer: nothing unusual.")]);
    core.set_fixed_provider(model);
    let app = f.app(core);
    let main = window(&app, "main");
    open_samples(&main);
    // An explain request runs (waiting for the model), a chat turn waits behind it.
    let explain = start(&main, json!({"op": "explain", "recordId": 1}));
    wait_until("the explain job to run", || {
        let ids = active_ai(&app);
        ids.len() == 1 && jobs_of(&app).state_of(ids[0]) == Some(crate::jobs::JobState::Running)
    });
    let sid = ipc(&main, "ai_chat_new", json!({})).unwrap()["id"].as_str().unwrap().to_string();
    let chat_job = ipc(&main, "ai_chat_send", json!({"sessionId": sid, "text": "anything unusual?"}))
        .unwrap()
        .as_u64()
        .unwrap();
    assert_eq!(
        code(ipc(&main, "ai_chat_send", json!({"sessionId": sid, "text": "again"})).unwrap_err()),
        "busy",
        "one turn per chat, also while it waits"
    );
    // Shown in the pane after 2 s: the turn waits at position 1, and may be moved.
    app.state::<crate::test_support::TestClock>().0.advance(3_000);
    let list = ipc(&main, "jobs_list", json!({})).unwrap();
    let jobs = list["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(
        (jobs[0]["state"].as_str(), jobs[0]["title"].as_str()),
        (Some("running"), Some("Explain event 1"))
    );
    assert_eq!(jobs[1]["id"], chat_job);
    assert_eq!(
        (
            jobs[1]["state"].as_str(),
            jobs[1]["position"].as_u64(),
            jobs[1]["movable"].as_bool()
        ),
        (Some("queued"), Some(1), Some(true))
    );
    assert_eq!(jobs[1]["source"]["link"], json!({"type": "chat", "sessionId": sid}));
    assert_eq!(jobs[0]["source"]["link"], json!({"type": "event", "recordId": 1}));
    // The worker crashes during the explain: only it fails, with the reason; the turn runs next.
    steps.send(Step::Crash).unwrap();
    let crashed = ended(&app, explain).expect("the failure is kept");
    assert_eq!(crashed.state, crate::jobs::JobState::Failed);
    assert!(crashed.error.as_deref().unwrap_or_default().contains("crashed"));
    steps.send(Step::Answer).unwrap();
    let events = finish_turn(&main, &sid);
    assert!(kinds(&events).contains(&"done".to_string()), "{events:?}");
    wait_until("both jobs to end", || active_ai(&app).is_empty());
    let list = ipc(&main, "jobs_list", json!({})).unwrap();
    let failed = list["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["state"] == "failed")
        .expect("the crashed request is listed")
        .clone();
    assert!(failed["error"].as_str().unwrap().contains("crashed"));
    assert_eq!(
        (failed["retryable"].as_bool(), failed["unseen"].as_bool()),
        (Some(true), Some(true))
    );
    // Its failure is a message in the notification centre, with Retry.
    let notices = ipc(&main, "notifications_list", json!({})).unwrap();
    let n = notices["notices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["source"] == "job")
        .expect("a job message")
        .clone();
    assert_eq!(n["primary"], json!({"type": "retryJob", "jobId": failed["id"]}));
    // The finished turn's View opens its chat.
    assert_eq!(
        ipc(&main, "jobs_result", json!({"id": chat_job})).unwrap(),
        json!({"type": "chat", "sessionId": sid})
    );
    // A failed request runs again with Retry, as a job of its own linked to the failed one.
    steps.send(Step::Crash).unwrap();
    let retried = ipc(&main, "jobs_retry", json!({"id": failed["id"]})).unwrap().as_u64().unwrap();
    assert_eq!(
        jobs_of(&app).info(crate::jobs::JobId(retried)).and_then(|i| i.retry_of),
        Some(crate::jobs::JobId(failed["id"].as_u64().unwrap()))
    );
    wait_until("the retry to end", || active_ai(&app).is_empty());
}

#[test]
fn a_waiting_request_can_be_cancelled_and_a_queued_turn_frees_its_chat() {
    let f = Fixture::new("ai-cancel-queued");
    let core = f.core();
    let (model, steps) = gated(vec![text("Explained.")]);
    core.set_fixed_provider(model);
    let app = f.app(core);
    let main = window(&app, "main");
    open_samples(&main);
    let first = start(&main, json!({"op": "explain", "recordId": 1}));
    wait_until("running", || active_ai(&app).len() == 1);
    let second = start(&main, json!({"op": "rootCauses"}));
    // The second request is queued before the chat adds its turn (on a busy machine its thread can start late).
    wait_until("the second request queued", || active_ai(&app).len() == 2);
    let sid = ipc(&main, "ai_chat_new", json!({})).unwrap()["id"].as_str().unwrap().to_string();
    ipc(&main, "ai_chat_send", json!({"sessionId": sid, "text": "hello"})).unwrap();
    wait_until("three AI jobs", || active_ai(&app).len() == 3);
    // Stop in the chat drops its waiting turn: the chat is free again and told.
    assert_eq!(ipc(&main, "ai_chat_cancel", json!({"sessionId": sid})).unwrap(), true);
    wait_until("the chat to be free", || {
        ipc(&main, "ai_chat_sessions", json!({})).unwrap()[0]["running"] == false
    });
    assert_eq!(active_ai(&app).len(), 2);
    // The dialog's Cancel (`ai_cancel`) stops the running use case and drops the waiting one.
    assert_eq!(ipc(&main, "ai_cancel", json!({})).unwrap(), 2);
    steps.send(Step::Answer).unwrap();
    for id in [first, second] {
        let state = ended(&app, id).map(|j| j.state);
        assert!(state.is_none_or(|s| s == crate::jobs::JobState::Cancelled), "{state:?}");
    }
    wait_until("no AI jobs", || active_ai(&app).is_empty());
}

#[test]
fn a_quarantined_worker_pauses_the_ai_queue_until_reset() {
    let f = Fixture::new("ai-quarantine");
    let health = Arc::new(debrief_ai::worker::Health::new(1, Duration::from_secs(600)));
    let mut deps = f.deps();
    deps.health = Some(health.clone());
    let core = Arc::new(AiCore::new(Some(ai_dir(&f.tmp)), f.tmp.0.join("models"), false, deps));
    let (model, steps, entered) = gated_counted(vec![text("Explained."), text("Root causes: none found.")]);
    core.set_fixed_provider(model);
    let app = f.app(core);
    let main = window(&app, "main");
    open_samples(&main);
    let first = start(&main, json!({"op": "explain", "recordId": 1}));
    wait_until("running", || active_ai(&app).len() == 1);
    let waiting = start(&main, json!({"op": "rootCauses"}));
    wait_until("queued", || active_ai(&app).len() == 2);
    // The running request waits in the model (not before it, where the quarantine would stop it first).
    wait_until("the model to be asked", || entered.load(std::sync::atomic::Ordering::SeqCst) == 1);
    // The worker crashes and is quarantined: the running request fails, the waiting one stays queued.
    health.record_failure("crashed three times");
    steps.send(Step::Crash).unwrap();
    assert_eq!(ended(&app, first).map(|j| j.state), Some(crate::jobs::JobState::Failed));
    wait_until("the pause", || {
        !ipc(&main, "jobs_list", json!({})).unwrap()["paused"].as_array().unwrap().is_empty()
    });
    let list = ipc(&main, "jobs_list", json!({})).unwrap();
    assert_eq!(list["paused"][0]["kind"], "ai");
    assert_eq!(active_ai(&app).len(), 1, "still waiting");
    // The crash is service health's to state, once: the AI worker is quarantined and one message (cause `health:ai`)
    // offers the repair.
    let notices = ipc(&main, "notifications_list", json!({})).unwrap();
    let ai: Vec<_> = notices["notices"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["cause"] == "health:ai")
        .collect();
    assert_eq!(ai.len(), 1, "{notices}");
    assert_eq!(ai[0]["primary"], json!({"type": "repairService", "service": "ai"}));
    assert_eq!(ai[0]["scope"], "app");
    assert_eq!(ai[0]["severity"], "error");
    assert!(
        notices["notices"].as_array().unwrap().iter().all(|n| n["cause"] != "ai:paused"),
        "no second message for the same failure"
    );
    let health = ipc(&main, "health_snapshot", json!({})).unwrap();
    let ai_state = health["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["service"] == "ai")
        .unwrap()
        .clone();
    assert_eq!(
        (ai_state["state"].as_str(), ai_state["repair"].as_str()),
        (Some("quarantined"), Some("reset"))
    );
    // Reset AI: the queue goes on.
    steps.send(Step::Answer).unwrap();
    ipc(&main, "ai_reset", json!({})).unwrap();
    let waited = ended(&app, waiting);
    assert_eq!(waited.as_ref().map(|j| j.state), Some(crate::jobs::JobState::Done), "{waited:?}");
    assert!(ipc(&main, "jobs_list", json!({})).unwrap()["paused"].as_array().unwrap().is_empty());
    // Reset AI is a repair: the service is back and its message is gone.
    let health = ipc(&main, "health_snapshot", json!({})).unwrap();
    assert!(
        health["services"].as_array().unwrap().iter().all(|s| s["state"] == "ok"),
        "{health}"
    );
    let notices = ipc(&main, "notifications_list", json!({})).unwrap();
    assert!(
        notices["notices"].as_array().unwrap().iter().all(|n| n["cause"] != "health:ai"),
        "{notices}"
    );
}

#[test]
fn research_with_workspace_data_in_the_search_asks_to_confirm_first() {
    let f = Fixture::new("research-confirm");
    let core = f.core();
    core.set_fixed_provider(scripted(vec![]));
    let app = f.app(core);
    let main = window(&app, "main");
    open_samples(&main);
    // The samples' MachineName is WEB01: a name the logs mark as private.
    let r = ipc(
        &main,
        "ai_research",
        json!({"args": {"question": "why does WEB01 time out on checkout?"}}),
    )
    .unwrap();
    assert_eq!(r["status"], "confirm", "{r}");
    let segments = r["checked"]["segments"].as_array().unwrap();
    assert!(segments.iter().any(|s| s["text"] == "WEB01" && !s["leak"].is_null()), "{r}");
    assert!(
        jobs_of(&app).find_active(|j| j.kind == crate::jobs::JobKind::Research).is_empty(),
        "nothing searched yet"
    );
    // Bad arguments are refused before anything runs.
    assert_eq!(code(ipc(&main, "ai_research", json!({"args": {}})).unwrap_err()), "invalid");
    assert_eq!(
        code(ipc(&main, "ai_research", json!({"args": {"question": "  "}})).unwrap_err()),
        "invalid"
    );
    assert_eq!(
        code(ipc(&main, "ai_research", json!({"args": {"question": "q", "sessionId": "nope"}})).unwrap_err()),
        "notFound"
    );
    assert_eq!(
        code(ipc(&main, "ai_research", json!({"args": {"recordId": u32::MAX}})).unwrap_err()),
        "notFound"
    );
}

#[test]
fn the_model_drafts_the_search_text_and_the_leak_guard_still_checks_it() {
    let f = Fixture::new("research-draft");
    let core = f.core();
    core.set_fixed_provider(scripted(vec![
        // "Research this" on an event: the model kept a host name despite the instructions.
        text("\"System.TimeoutException The operation has timed out WEB01\""),
        // "Search the web for this error": the flagged part is taken out.
        text("TimeoutException WEB01 operation timed out"),
        // A chatty reply is not a query: the engine's text is used.
        text("Sure! Here is a query:\nTimeoutException"),
        // A question without an event.
        text("Query: E_ACCESSDENIED IIS app pool"),
    ]));
    let app = f.app(core);
    let main = window(&app, "main");
    open_samples(&main);
    let page = ipc(
        &main,
        "query_page",
        json!({"request": {"query": {"text": "timed out"}, "offset": 0, "limit": 1, "descending": false}}),
    )
    .unwrap();
    let id = page["rows"][0]["id"].as_u64().unwrap();
    let d = ipc(&main, "ai_research_draft", json!({"args": {"recordId": id}})).unwrap();
    assert_eq!(d["source"], "model", "{d}");
    assert_eq!(d["checked"]["query"], "System.TimeoutException The operation has timed out WEB01");
    assert_eq!(d["checked"]["needsConfirm"], true, "WEB01 is a name from the logs: {d}");
    assert!(d["question"].as_str().unwrap().starts_with("What does this error mean"), "{d}");
    assert!(
        jobs_of(&app).find_active(|j| j.kind == crate::jobs::JobKind::Research).is_empty(),
        "drafting never searches"
    );
    let q = ipc(&main, "browser_event_query", json!({"recordId": id})).unwrap();
    assert_eq!(
        (q["source"].as_str(), q["query"].as_str()),
        (Some("model"), Some("TimeoutException operation timed out")),
        "{q}"
    );
    assert_eq!(q["removed"], json!(["1 value from your logs"]));
    let q = ipc(&main, "browser_event_query", json!({"recordId": id})).unwrap();
    assert_eq!(q["source"], "engine", "{q}");
    assert!(!q["query"].as_str().unwrap().contains("WEB0"), "{q}");
    let d = ipc(
        &main,
        "ai_research_draft",
        json!({"args": {"question": "what does E_ACCESSDENIED mean for IIS?"}}),
    )
    .unwrap();
    assert_eq!(
        (d["source"].as_str(), d["checked"]["query"].as_str()),
        (Some("model"), Some("E_ACCESSDENIED IIS app pool"))
    );
    // The model has no more replies: the question is the search text, as before drafting.
    let d = ipc(
        &main,
        "ai_research_draft",
        json!({"args": {"question": "what is HRESULT 0x80070005?"}}),
    )
    .unwrap();
    assert_eq!(
        (d["source"].as_str(), d["checked"]["query"].as_str()),
        (Some("engine"), Some("what is HRESULT 0x80070005?"))
    );
    assert_eq!(code(ipc(&main, "ai_research_draft", json!({"args": {}})).unwrap_err()), "invalid");
    assert_eq!(
        code(ipc(&main, "ai_research_draft", json!({"args": {"recordId": u32::MAX}})).unwrap_err()),
        "notFound"
    );
    let long = "x".repeat(2000);
    assert_eq!(
        code(ipc(&main, "ai_research_draft", json!({"args": {"question": long}})).unwrap_err()),
        "invalid"
    );
}

#[test]
fn a_manual_download_is_checked_against_the_catalog_before_it_is_added() {
    let f = Fixture::new("manual-model");
    let app = f.app(f.core());
    let main = window(&app, "main");
    crate::paths::allow_under(app.handle(), "main", &f.tmp.0);
    // Named like the catalog model, same size, other bytes: damaged or not the original.
    let mut bad = model_bytes();
    let last = bad.len() - 1;
    bad[last] ^= 0x55;
    let dir = f.tmp.0.join("Downloads");
    std::fs::create_dir_all(&dir).unwrap();
    let damaged = dir.join(TEST_FILE);
    std::fs::write(&damaged, &bad).unwrap();
    let e = ipc(&main, "ai_model_add_local", json!({"path": damaged.to_string_lossy()})).unwrap_err();
    // The check runs as a job: it fails with the message (the typed code stays with the command's own errors).
    assert!(e["message"].as_str().unwrap().contains("SHA-256"), "{e}");
    // Incomplete (the browser stopped early).
    std::fs::write(&damaged, &model_bytes()[..model_bytes().len() - 7]).unwrap();
    let e = ipc(&main, "ai_model_add_local", json!({"path": damaged.to_string_lossy()})).unwrap_err();
    assert!(e["message"].as_str().unwrap().contains("incomplete"), "{e}");
    assert!(ipc(&main, "ai_models", json!({})).unwrap().as_array().unwrap().is_empty());
    // The user's own model (not in the catalog) is added without a checksum.
    let own = dir.join("my-own.gguf");
    let mut other = model_bytes();
    other.extend_from_slice(&[0; 32]);
    std::fs::write(&own, &other).unwrap();
    let added = ipc(&main, "ai_model_add_local", json!({"path": own.to_string_lossy()})).unwrap();
    assert_eq!(
        (added["fileName"].as_str(), added["catalogId"].as_str()),
        (Some("my-own.gguf"), None)
    );
    // The original file is accepted.
    std::fs::write(&damaged, model_bytes()).unwrap();
    let added = ipc(&main, "ai_model_add_local", json!({"path": damaged.to_string_lossy()})).unwrap();
    assert_eq!(added["catalogId"], TEST_MODEL);
}

#[test]
fn manual_downloads_name_the_catalog_files_and_open_only_those() {
    let f = Fixture::new("manual-info");
    let core = f.core();
    let app = f.app(core.clone());
    let main = window(&app, "main");
    let m = ipc(&main, "ai_manual_download", json!({"kind": "model", "id": TEST_MODEL})).unwrap();
    assert_eq!(m["kind"], "model");
    assert_eq!(m["name"], "Test 4B");
    let file = &m["files"][0];
    assert_eq!(
        file["url"],
        format!("https://huggingface.co/test/Test-4B-GGUF/resolve/main/{TEST_FILE}?download=true")
    );
    assert_eq!(file["sizeBytes"], model_bytes().len());
    assert_eq!(file["sha256"], sha256_of(&model_bytes(), &f.tmp.0));
    assert!(m["folder"].as_str().unwrap().ends_with("models"));
    let r = ipc(&main, "ai_manual_download", json!({"kind": "runtime", "id": "cpu"})).unwrap();
    assert_eq!(r["files"][0]["name"], "llama-b7-bin-test-cpu.zip");
    assert!(r["files"][0]["url"]
        .as_str()
        .unwrap()
        .starts_with("https://github.com/ggml-org/llama.cpp/releases/download/"));
    assert!(r["files"][0]["sha256"].as_str().is_some_and(|s| s.len() == 64));
    assert!(r["folder"].as_str().unwrap().contains("runtimes"));
    // Unknown ids, kinds and files are refused; the address is never taken from the webview.
    assert_eq!(
        code(ipc(&main, "ai_manual_download", json!({"kind": "model", "id": "nope"})).unwrap_err()),
        "notFound"
    );
    assert_eq!(
        code(ipc(&main, "ai_manual_download", json!({"kind": "runtime", "id": "../x"})).unwrap_err()),
        "notFound"
    );
    assert_eq!(
        code(ipc(&main, "ai_manual_download", json!({"kind": "model", "id": ""})).unwrap_err()),
        "invalid"
    );
    assert!(ipc(&main, "ai_manual_download", json!({"kind": "url", "id": "https://evil.example"})).is_err());
    let e = ipc(
        &main,
        "ai_manual_download_open",
        json!({"kind": "model", "id": TEST_MODEL, "index": 3}),
    )
    .unwrap_err();
    assert_eq!(code(e), "invalid");
    assert_eq!(
        core.manual_download_url(super::manual::ManualKind::Runtime, "cpu", 0).unwrap(),
        format!("https://github.com/ggml-org/llama.cpp/releases/download/{RT_TAG}/llama-b7-bin-test-cpu.zip")
    );
}

#[test]
fn a_runtime_is_installed_from_archives_downloaded_by_hand() {
    let f = Fixture::new("manual-runtime");
    let core = f.core();
    let app = f.app(core.clone());
    let events = runtime_events(&app);
    let main = window(&app, "main");
    crate::paths::allow_under(app.handle(), "main", &f.tmp.0);
    let dir = f.tmp.0.join("Downloads");
    std::fs::create_dir_all(&dir).unwrap();
    let (name, data) = runtime_assets().into_iter().next().unwrap();
    let archive = dir.join(&name);
    // A damaged archive: refused with the expected checksum, nothing installed, nothing downloaded.
    let mut bad = data.clone();
    bad[0] ^= 1;
    std::fs::write(&archive, &bad).unwrap();
    ipc(
        &main,
        "ai_runtime_add_archives",
        json!({"id": "cpu", "paths": [archive.to_string_lossy()]}),
    )
    .unwrap();
    let end = install_end(&events, "cpu");
    assert_eq!(
        (end["stage"].as_str(), end["code"].as_str()),
        (Some("failed"), Some("integrity")),
        "{end}"
    );
    // The install slot is free once the end event is out.
    events.lock().unwrap().clear();
    // The original: installed without downloading.
    std::fs::write(&archive, &data).unwrap();
    ipc(
        &main,
        "ai_runtime_add_archives",
        json!({"id": "cpu", "paths": [archive.to_string_lossy()]}),
    )
    .unwrap();
    let end = install_end(&events, "cpu");
    assert_eq!(end["stage"], "ready", "{end}");
    assert!(build(&ipc(&main, "ai_runtimes", json!({})).unwrap(), "cpu")["installed"] == true);
    // Bad arguments.
    for (paths, want) in [
        (json!([]), "invalid"),
        (json!(["relative.zip"]), "invalid"),
        (json!([f.tmp.0.join("missing.zip").to_string_lossy()]), "notFound"),
        (json!([dir.to_string_lossy()]), "invalid"),
    ] {
        assert_eq!(
            code(ipc(&main, "ai_runtime_add_archives", json!({"id": "cpu", "paths": paths})).unwrap_err()),
            want
        );
    }
}

mod knowledge;
mod pitwall;
mod update_checks;
mod vision;

#[test]
fn job_progress_labels_the_phase_so_a_check_is_not_a_second_download() {
    use super::runtime_commands::phase_progress;
    let d = phase_progress(false, 5, 10);
    let v = phase_progress(true, 10, 10);
    assert_eq!((d.current.as_deref(), d.done, d.total), (Some("Downloading"), Some(5), Some(10)));
    assert_eq!((v.current.as_deref(), v.done, v.total), (Some("Checking"), Some(10), Some(10)));
    assert_eq!(
        d.phase.as_ref().map(|p| (p.label.as_str(), p.scale)),
        Some(("Downloading", PhaseScale::Bytes))
    );
    assert_eq!(v.phase.as_ref().map(|p| p.label.as_str()), Some("Checking"));
}

#[test]
fn an_install_shows_downloading_unpacking_and_the_runtime_check_as_job_phases() {
    use super::runtime_commands::{InstallPhase, TESTING_LABEL};
    use debrief_ai::runtimes::RuntimeStage;
    let phase_of = |st: RuntimeStage| {
        let p = InstallPhase::of(&st).job_progress();
        (p.phase.map(|p| (p.label, p.scale)), p.done, p.total, p.fraction)
    };
    let unpack = phase_of(RuntimeStage::Unpacking {
        asset: "a.zip".into(),
        done: 25,
        total: 100,
    });
    assert_eq!(
        unpack,
        (Some(("Unpacking".into(), PhaseScale::Bytes)), Some(25), Some(100), Some(0.25))
    );
    let test = phase_of(RuntimeStage::Testing);
    assert_eq!(test.0, Some((TESTING_LABEL.into(), PhaseScale::Indeterminate)));
    assert_eq!(test.3, None, "no fraction while it checks");
    assert_eq!(TESTING_LABEL, "Checking that llama.cpp runs on this Mac");
    assert_eq!(phase_of(RuntimeStage::Finishing).0.map(|p| p.0), Some("Finishing".into()));
}

#[test]
fn finding_values_builds_the_names_and_the_alias_job_runs_on_the_local_model_only_when_it_may() {
    let f = Fixture::new("names");
    let core = f.core();
    let provider = Arc::new(ScriptedProvider::new(vec![text(r#"{"items":[]}"#); 40]));
    core.set_fixed_provider(provider.clone());
    let app = f.app(core.clone());
    let main = window(&app, "main");
    open_samples(&main);
    let win = app.state::<crate::windows::Windows>().get("main").unwrap();
    // The first search after the catalogue is ready rebuilds the table and queues the alias job (a quiet one).
    wait_until("the name table", || {
        ipc(&main, "find_values", json!({"request": {"text": "gateway"}})).unwrap();
        win.names.names().is_built()
    });
    // The table is built before the job is queued, so "no job pending" alone can be seen too early: wait for the
    // model to have been asked, then for the job to finish.
    wait_until("the alias job to ask the model", || !provider.requests().is_empty());
    wait_until("the alias job to finish", || !win.names.job_pending());
    let reply = ipc(&main, "find_values", json!({"request": {"text": "gateway"}})).unwrap();
    assert!(reply["values"].is_array(), "{reply}");
    // With the switch off the model is never asked again.
    ipc(&main, "set_settings", json!({"section": "ai", "values": {"name_aliases": false}})).unwrap();
    // The second search may have queued another alias job before the switch went off: let it end first.
    wait_until("the last alias job to finish", || !win.names.job_pending());
    let asked = provider.requests().len();
    assert!(!crate::names::schedule_alias_job(app.handle(), "main", &win.names));
    assert_eq!(provider.requests().len(), asked);
    assert!(!win.names.job_pending(), "a refused job leaves no flag behind");
}

#[test]
fn the_name_commands_validate_what_the_webview_sends() {
    let f = Fixture::new("names-cmd");
    let app = f.app(f.core());
    let main = window(&app, "main");
    for (cmd, args) in [
        ("names_accept", json!({"request": {"text": "gfx", "group": "executable|nothing"}})),
        ("names_reject", json!({"request": {"text": "  ", "group": "x"}})),
        ("names_remove_alias", json!({"request": {"group": "", "alias": "a"}})),
    ] {
        assert_eq!(code(ipc(&main, cmd, args).unwrap_err()), "invalid", "{cmd}");
    }
    assert_eq!(ipc(&main, "names_undo", json!({})).unwrap()["undone"], Value::Null);
}

/// The start-up log line reads the light status: the same state and model as the full one, without the scan of every
/// process for the workers' memory (up to seconds on a busy Mac, which held the first window back).
#[test]
fn the_light_status_is_the_full_status_without_the_memory_scan() {
    let f = Fixture::new("status-light");
    let core = f.core();
    core.set_fixed_provider(scripted(vec![]));
    let light = core.status_light();
    let full = core.status();
    assert_eq!(light.state, super::core::EngineStateDto::Ready);
    assert_eq!(light.rss_bytes, 0, "no process scan");
    assert_eq!(
        (light.state, light.model.clone(), light.available),
        (full.state, full.model.clone(), full.available)
    );
}
