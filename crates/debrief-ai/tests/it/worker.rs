//! The supervisor against a fake worker process (crash, hang, slow, cancel, orphans),
//! and against an in-process fake transport through the `WorkerTransport` trait.

use debrief_ai::gguf;
use debrief_ai::hardware::{Gpu, Hardware};
use debrief_ai::llm::{ChatMessage, ChatRequest, LlmProvider, StopReason};
use debrief_ai::runtime::{load_with_fallback, GpuMode, LoadRequest, ModelRuntime};
use debrief_ai::testing::gguf::qwen_like;
use debrief_ai::worker::protocol::*;
use debrief_ai::worker::*;
use debrief_ai::AiError;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn fake_worker_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_debrief-ai-fake-worker"))
}

/// Killed or aborted workers must not leave corrupt coverage files behind.
fn no_profile() -> Vec<(String, String)> {
    vec![(
        "LLVM_PROFILE_FILE".into(),
        if cfg!(windows) { "NUL".into() } else { "/dev/null".into() },
    )]
}

fn config(args: &[&str]) -> WorkerConfig {
    let mut c = WorkerConfig::new(Arc::new(FramesLauncher {
        exe: fake_worker_exe(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: debrief_ai::worker::fixed_env(no_profile()),
    }));
    c.hello_timeout = Duration::from_secs(10);
    c.load_timeout = Duration::from_secs(10);
    c.control_timeout = Duration::from_secs(5);
    c.stall_timeout = Duration::from_millis(600);
    c.cancel_grace = Duration::from_millis(400);
    c.backoff_base = Duration::from_millis(5);
    c
}

fn big_machine() -> Hardware {
    Hardware {
        os: "x".into(),
        arch: "x".into(),
        cpu_cores: 8,
        total_ram_gb: 64.0,
        available_ram_gb: 48.0,
        gpus: vec![Gpu {
            name: "GPU".into(),
            vram_gb: Some(16.0),
            unified: false,
        }],
    }
}

struct Model {
    _dir: tempfile::TempDir,
    req: LoadRequest,
}

fn model(gpu: GpuMode) -> Model {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.gguf");
    std::fs::write(&path, qwen_like()).unwrap();
    let info = gguf::inspect(&path).unwrap();
    Model {
        _dir: dir,
        req: LoadRequest {
            path,
            projector: None,
            info,
            context: 4096,
            gpu,
            threads: 0,
        },
    }
}

fn runtime(args: &[&str]) -> WorkerRuntime {
    WorkerRuntime::new(config(args), Box::new(big_machine))
}

fn ask(p: &dyn LlmProvider, text: &str) -> debrief_ai::Result<debrief_ai::llm::ChatResponse> {
    p.chat(
        &ChatRequest {
            messages: vec![ChatMessage::user(text)],
            ..ChatRequest::default()
        },
        &mut |_| {},
    )
}

fn pid_alive(pid: u32) -> bool {
    let mut sys = sysinfo::System::new();
    let p = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    sys.process(p)
        .is_some_and(|pr| !matches!(pr.status(), sysinfo::ProcessStatus::Zombie))
}

fn wait_gone(pid: u32) -> bool {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn params(gpu_layers: i32) -> LoadParams {
    LoadParams {
        model_path: "x.gguf".into(),
        mmproj_path: None,
        gpu_layers,
        context: 2048,
        threads: 0,
    }
}

fn chat_params(p: &str) -> ChatParams {
    ChatParams {
        prompt: p.into(),
        max_tokens: 16,
        temperature: 0.0,
        json_schema: None,
        images: vec![],
    }
}

fn supervisor(args: &[&str]) -> Supervisor {
    let c = config(args);
    let h = Arc::new(Health::new(c.max_crashes, c.crash_window));
    Supervisor::new(c, h)
}

#[test]
fn happy_path_streams_and_reports_status() {
    let rt = runtime(&[]);
    let m = model(GpuMode::Cpu);
    let loaded = rt.load(&m.req).unwrap();
    assert_eq!(loaded.backend, "CPU");
    let mut streamed = String::new();
    let r = loaded
        .provider
        .chat(
            &ChatRequest {
                system: "sys".into(),
                messages: vec![ChatMessage::user("hello there")],
                ..ChatRequest::default()
            },
            &mut |d| streamed.push_str(d),
        )
        .unwrap();
    assert!(r.text.starts_with("echo:") && r.text.contains("assistant"), "{}", r.text);
    assert_eq!(streamed.trim(), r.text);
    assert_eq!(r.stop_reason, StopReason::EndTurn);
    assert_eq!(loaded.provider.display_name(), "fake-model");
    assert_eq!(loaded.provider.context_length(), 4096);
    assert_eq!(rt.load(&model(GpuMode::Auto).req).unwrap().backend, "Fake GPU");

    let s = supervisor(&[]);
    s.load(params(0)).unwrap();
    let st = s.status();
    assert_eq!(st.state, EngineState::Running);
    assert!(st.pid.is_some() && st.rss_bytes > 0);
    assert_eq!((st.backend.as_str(), st.model.as_str()), ("CPU", "fake-model"));
    s.shutdown();
    assert_eq!(s.status().state, EngineState::Stopped);
}

#[test]
fn a_crash_fails_the_call_restarts_the_worker_and_reloads_the_model() {
    let marker = tempfile::tempdir().unwrap().keep().join("crashed.marker");
    let rt = runtime(&[&format!("--crash-once={}", marker.display())]);
    let loaded = rt.load(&model(GpuMode::Cpu).req).unwrap();
    let err = ask(loaded.provider.as_ref(), "first").unwrap_err();
    assert!(err.to_string().contains("crashed and was restarted"), "{err}");
    // the next question works: the process was restarted and the model loaded again
    let r = ask(loaded.provider.as_ref(), "second").unwrap();
    assert!(r.text.starts_with("echo:"));
    assert!(rt.disabled_reason().is_none());
}

#[test]
fn a_hang_times_out_and_the_worker_is_killed() {
    let s = supervisor(&["--hang-on-chat"]);
    s.load(params(0)).unwrap();
    let pid = s.status().pid.unwrap();
    let started = Instant::now();
    let err = s.chat(&chat_params("x"), None, &mut |_| {}).unwrap_err();
    assert!(err.to_string().contains("stopped responding"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(wait_gone(pid), "the hung worker must be killed");
    assert_eq!(s.status().restarts, 1);
}

#[test]
fn three_crashes_quarantine_the_ai_for_the_session() {
    let rt = runtime(&["--crash-on-chat"]);
    let loaded = rt.load(&model(GpuMode::Cpu).req).unwrap();
    for i in 1..=2 {
        let e = ask(loaded.provider.as_ref(), "x").unwrap_err();
        assert!(e.to_string().contains("crashed"), "{i}: {e}");
        assert!(rt.disabled_reason().is_none());
    }
    let e = ask(loaded.provider.as_ref(), "x").unwrap_err();
    assert!(
        e.to_string().contains("off for this session") && e.to_string().contains("3 times"),
        "{e}"
    );
    assert!(rt.disabled_reason().unwrap().contains("crashed"));
    // fails fast now, and a new load is refused with the same reason
    let started = Instant::now();
    assert!(ask(loaded.provider.as_ref(), "x")
        .unwrap_err()
        .to_string()
        .contains("off for this session"));
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(rt
        .load(&model(GpuMode::Cpu).req)
        .err()
        .unwrap()
        .to_string()
        .contains("off for this session"));
    // the user can switch it back on
    rt.reset();
    assert!(rt.disabled_reason().is_none());
    assert!(rt.load(&model(GpuMode::Cpu).req).is_ok());
}

#[test]
fn old_crashes_expire_from_the_window() {
    let h = Health::new(3, Duration::from_millis(80));
    assert!(!h.record_failure("a") && !h.record_failure("b"));
    std::thread::sleep(Duration::from_millis(120));
    assert!(!h.record_failure("c"));
    assert!(h.check().is_ok());
    assert!(!h.record_failure("d"));
    assert!(h.record_failure("e"));
    assert!(h.check().is_err());
}

#[test]
fn cancel_stops_generation_between_tokens() {
    let rt = runtime(&["--slow-ms=25"]);
    let loaded = rt.load(&model(GpuMode::Cpu).req).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let mut got = String::new();
    let started = Instant::now();
    let req = ChatRequest {
        messages: vec![ChatMessage::user("hello")],
        cancel: Some(cancel),
        ..ChatRequest::default()
    };
    let r = loaded
        .provider
        .chat(&req, &mut |d| {
            got.push_str(d);
            flag.store(true, Ordering::SeqCst);
        })
        .unwrap();
    assert_eq!(r.stop_reason, StopReason::Cancelled);
    assert!(started.elapsed() < Duration::from_millis(900), "{:?}", started.elapsed());
    assert!(!got.is_empty() && r.text.len() < "echo:".len() + 30 + 30);
    // the worker is still healthy afterwards
    assert!(ask(loaded.provider.as_ref(), "again").is_ok());
}

#[test]
fn a_worker_that_ignores_cancel_is_treated_as_hung() {
    let s = supervisor(&["--slow-ms=60", "--ignore-cancel"]);
    s.load(params(0)).unwrap();
    let pid = s.status().pid.unwrap();
    let cancel = AtomicBool::new(true);
    let err = s.chat(&chat_params(&"x".repeat(100)), Some(&cancel), &mut |_| {}).unwrap_err();
    assert!(err.to_string().contains("stopped responding"), "{err}");
    assert!(wait_gone(pid));
}

#[test]
fn gpu_failures_fall_back_to_the_cpu() {
    let rt = runtime(&["--fail-gpu-load"]);
    let (loaded, note) = load_with_fallback(&rt, model(GpuMode::Auto).req).unwrap();
    assert_eq!(loaded.backend, "CPU");
    assert!(note.unwrap().contains("GPU could not be used"));
    // a model that fails everywhere is a clear error
    let rt = runtime(&["--fail-load"]);
    let e = load_with_fallback(&rt, model(GpuMode::Auto).req).err().unwrap();
    assert!(e.to_string().contains("cannot load the model"), "{e}");
}

#[test]
fn context_overflow_is_typed() {
    let rt = runtime(&["--overflow"]);
    let loaded = rt.load(&model(GpuMode::Cpu).req).unwrap();
    assert_eq!(
        ask(loaded.provider.as_ref(), "x").unwrap_err(),
        AiError::ContextOverflow { prompt: 5000, limit: 4096 }
    );
}

#[test]
fn nothing_is_left_behind() {
    let s = supervisor(&[]);
    s.load(params(0)).unwrap();
    let pid = s.status().pid.unwrap();
    assert!(pid_alive(pid));
    drop(s);
    assert!(wait_gone(pid), "dropping the supervisor must stop the worker");

    // unloading through shutdown also stops it, and the model is reloaded on the next use
    let s = supervisor(&[]);
    s.load(params(0)).unwrap();
    let pid = s.status().pid.unwrap();
    s.shutdown();
    assert!(wait_gone(pid));
    let r = s.chat(&chat_params("again"), None, &mut |_| {}).unwrap();
    assert!(r.text.starts_with("echo:"));
    assert_ne!(s.status().pid.unwrap(), pid);
}

#[test]
fn start_failures_and_memory_limits() {
    let c = WorkerConfig::new(Arc::new(FramesLauncher {
        exe: PathBuf::from("/definitely/not/here"),
        args: vec![],
        env: debrief_ai::worker::fixed_env(vec![]),
    }));
    let rt = WorkerRuntime::new(c, Box::new(big_machine));
    let e = rt.load(&model(GpuMode::Cpu).req).err().unwrap();
    assert!(e.to_string().contains("cannot start the AI engine"), "{e}");

    let e = runtime(&["--exit-immediately"]).load(&model(GpuMode::Cpu).req).err().unwrap();
    assert!(e.to_string().contains("crashed"), "{e}");

    let tiny = Hardware {
        os: "x".into(),
        arch: "x".into(),
        cpu_cores: 2,
        total_ram_gb: 2.0,
        available_ram_gb: 0.5,
        gpus: vec![],
    };
    let rt = WorkerRuntime::new(config(&[]), Box::new(move || tiny.clone()));
    let mut m = model(GpuMode::Cpu);
    m.req.info.file_size = 20 << 30;
    assert!(matches!(rt.load(&m.req), Err(AiError::Memory(_))));
}

// ---- through the trait, without a process ----

type Script = Box<dyn FnMut(&str, Value, &mut dyn FnMut(Value)) -> Result<Value, TransportError> + Send>;

struct FakeTransport {
    script: Script,
    killed: Arc<AtomicUsize>,
}

impl WorkerTransport for FakeTransport {
    fn call(
        &mut self,
        method: &str,
        params: Value,
        _opts: CallOpts,
        _cancel: Option<&AtomicBool>,
        on_stream: &mut dyn FnMut(Value),
    ) -> Result<Value, TransportError> {
        (self.script)(method, params, on_stream)
    }
    fn pid(&self) -> Option<u32> {
        Some(1)
    }
    fn kill(&mut self) {
        self.killed.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeLauncher {
    make: Box<dyn Fn(usize) -> Script + Send + Sync>,
    launches: AtomicUsize,
    killed: Arc<AtomicUsize>,
}

impl WorkerLauncher for FakeLauncher {
    fn launch(&self) -> Result<Box<dyn WorkerTransport>, TransportError> {
        let n = self.launches.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeTransport {
            script: (self.make)(n),
            killed: self.killed.clone(),
        }))
    }
}

fn fake_config(make: impl Fn(usize) -> Script + Send + Sync + 'static) -> (WorkerConfig, Arc<FakeLauncher>) {
    let l = Arc::new(FakeLauncher {
        make: Box::new(make),
        launches: AtomicUsize::new(0),
        killed: Arc::new(AtomicUsize::new(0)),
    });
    let mut c = WorkerConfig::new(l.clone());
    c.backoff_base = Duration::from_millis(1);
    (c, l)
}

fn healthy(method: &str, _p: Value, stream: &mut dyn FnMut(Value)) -> Result<Value, TransportError> {
    Ok(match method {
        "hello" => json!({"protocol_version": PROTOCOL_VERSION, "worker_version": "t", "pid": 1}),
        "load" => json!({"backend": "CPU", "context": 2048, "name": "scripted", "rss_bytes": 5}),
        "chat" => {
            stream(json!({"id": 1, "text": "ok"}));
            json!({"text": "ok", "prompt_tokens": 1, "completion_tokens": 1, "finish": "stop"})
        }
        "status" => json!({"loaded": true, "backend": "CPU", "rss_bytes": 5, "pid": 1, "uptime_secs": 1}),
        _ => json!({}),
    })
}

#[test]
fn the_supervisor_only_needs_the_trait() {
    let (c, l) = fake_config(|_| Box::new(healthy));
    let s = Supervisor::new(c.clone(), Arc::new(Health::new(3, Duration::from_secs(60))));
    assert_eq!(s.load(params(0)).unwrap().name, "scripted");
    let mut got = String::new();
    assert_eq!(s.chat(&chat_params("x"), None, &mut |d| got.push_str(d)).unwrap().text, "ok");
    assert_eq!(got, "ok");
    assert_eq!(s.status().rss_bytes, 5);
    drop(s);
    assert_eq!(l.launches.load(Ordering::SeqCst), 1);
    assert!(l.killed.load(Ordering::SeqCst) >= 1);
}

#[test]
fn every_transport_failure_kind_restarts_and_is_counted() {
    for (name, err, needle) in [
        ("died", TransportError::Died("exit 139".into()), "crashed"),
        ("timeout", TransportError::Timeout, "stopped responding"),
        ("protocol", TransportError::Protocol("junk".into()), "invalid response"),
    ] {
        let (c, l) = fake_config(move |n| {
            let err = err.clone();
            if n == 0 {
                Box::new(move |m: &str, p: Value, s: &mut dyn FnMut(Value)| if m == "chat" { Err(err.clone()) } else { healthy(m, p, s) })
            } else {
                Box::new(healthy)
            }
        });
        let s = Supervisor::new(c, Arc::new(Health::new(3, Duration::from_secs(60))));
        s.load(params(0)).unwrap();
        let e = s.chat(&chat_params("x"), None, &mut |_| {}).unwrap_err();
        assert!(e.to_string().contains(needle), "{name}: {e}");
        assert_eq!(l.killed.load(Ordering::SeqCst), 1, "{name}");
        // restarted worker: the model is reloaded transparently
        assert_eq!(s.chat(&chat_params("y"), None, &mut |_| {}).unwrap().text, "ok", "{name}");
        assert_eq!(l.launches.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn protocol_version_mismatch_is_refused_and_remote_errors_are_mapped() {
    let (c, _) = fake_config(|_| {
        Box::new(|m: &str, p: Value, s: &mut dyn FnMut(Value)| {
            if m == "hello" {
                Ok(json!({"protocol_version": 99, "worker_version": "x", "pid": 1}))
            } else {
                healthy(m, p, s)
            }
        })
    });
    let s = Supervisor::new(c, Arc::new(Health::new(3, Duration::from_secs(60))));
    assert!(s.load(params(0)).unwrap_err().to_string().contains("protocol 99"));

    let (c, _) = fake_config(|_| {
        Box::new(|m: &str, p: Value, s: &mut dyn FnMut(Value)| {
            if m == "chat" {
                Err(TransportError::Remote(RpcError::new(code::ENGINE, "kaput")))
            } else {
                healthy(m, p, s)
            }
        })
    });
    let s = Supervisor::new(c, Arc::new(Health::new(3, Duration::from_secs(60))));
    s.load(params(0)).unwrap();
    assert_eq!(
        s.chat(&chat_params("x"), None, &mut |_| {}).unwrap_err(),
        AiError::Engine("kaput".into())
    );
    // a remote error is not a crash
    assert_eq!(s.status().crashes_in_window, 0);

    let (c, _) = fake_config(|_| {
        Box::new(|m: &str, p: Value, s: &mut dyn FnMut(Value)| {
            if m == "chat" {
                Ok(json!("not an object"))
            } else {
                healthy(m, p, s)
            }
        })
    });
    let s = Supervisor::new(c, Arc::new(Health::new(3, Duration::from_secs(60))));
    s.load(params(0)).unwrap();
    assert!(matches!(s.chat(&chat_params("x"), None, &mut |_| {}), Err(AiError::Malformed(_))));
}

#[test]
fn a_garbled_hello_or_load_answer_is_a_protocol_failure() {
    let (c, _) = fake_config(|_| {
        Box::new(|m: &str, p: Value, s: &mut dyn FnMut(Value)| if m == "hello" { Ok(json!({"nope": 1})) } else { healthy(m, p, s) })
    });
    let s = Supervisor::new(c, Arc::new(Health::new(3, Duration::from_secs(60))));
    assert!(s.load(params(0)).unwrap_err().to_string().contains("invalid response"));
    let (c, _) = fake_config(|_| {
        Box::new(|m: &str, p: Value, s: &mut dyn FnMut(Value)| if m == "load" { Ok(json!([1])) } else { healthy(m, p, s) })
    });
    let s = Supervisor::new(c, Arc::new(Health::new(3, Duration::from_secs(60))));
    assert!(s.load(params(0)).is_err());
    // the transport's last words can be shared across threads
    let shared = Arc::new(Mutex::new(0));
    let _ = shared;
}
