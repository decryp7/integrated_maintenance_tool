//! The llama-server backend against the fake llama-server (`src/bin/fake_llama_server.rs`): loading,
//! streaming, schema passthrough, cancellation, context overflow, crashes and restarts, build selection
//! and fallback, and the failures a user can meet (missing binary, model that does not load).

use imt_ai::testing::gguf::qwen_like;
use imt_ai::worker::protocol::*;
use imt_ai::worker::WorkerBackend;
use imt_ai_worker::listing;
use imt_ai_worker::locate::LlamaConfig;
use imt_ai_worker::server::{ServerBackend, Timeouts};
use imt_ai_worker::variants::{server_file_name, BuildChoice};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_imt-fake-llama-server");
const GPU: &str = "  Vulkan0: Test GPU (8192 MiB, 8000 MiB free)\n";

struct Lab {
    dir: tempfile::TempDir,
}

impl Lab {
    fn new() -> Lab {
        let lab = Lab {
            dir: tempfile::tempdir().unwrap(),
        };
        std::fs::write(lab.model(), qwen_like()).unwrap();
        lab
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("llama")
    }

    /// Installs the fake as build `id`, with optional marker files (`devices.txt`, `start-fail`, `list-fail`).
    fn build(&self, id: &str, files: &[(&str, &str)]) -> &Lab {
        let d = self.root().join(id);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::copy(FAKE, d.join(server_file_name())).unwrap();
        for (name, text) in files {
            std::fs::write(d.join(name), text).unwrap();
        }
        self
    }

    fn model(&self) -> PathBuf {
        self.dir.path().join("m.gguf")
    }

    fn script(&self, v: Value) -> &Lab {
        std::fs::write(side(&self.model(), ".fake.json"), v.to_string()).unwrap();
        self
    }

    fn backend(&self, choice: BuildChoice) -> ServerBackend {
        ServerBackend::with_timeouts(
            LlamaConfig {
                custom: None,
                roots: vec![self.root()],
                choice,
            },
            Timeouts {
                ready: Duration::from_secs(20),
                ..Timeouts::default()
            },
        )
    }

    fn load_params(&self, gpu_layers: i32) -> LoadParams {
        LoadParams {
            model_path: self.model().to_string_lossy().into_owned(),
            gpu_layers,
            context: 8192,
            threads: 2,
        }
    }

    fn args(&self) -> Vec<String> {
        serde_json::from_str(&std::fs::read_to_string(side(&self.model(), ".args.json")).unwrap()).unwrap()
    }

    fn requests(&self, path: &str) -> Vec<Value> {
        std::fs::read_to_string(side(&self.model(), ".requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["path"] == path)
            .map(|v| v["body"].clone())
            .collect()
    }
}

fn side(model: &Path, ext: &str) -> PathBuf {
    let mut s = model.as_os_str().to_owned();
    s.push(ext);
    PathBuf::from(s)
}

fn chat_params(prompt: &str) -> ChatParams {
    ChatParams {
        prompt: prompt.into(),
        max_tokens: 256,
        temperature: 0.0,
        json_schema: None,
    }
}

fn chat(b: &mut ServerBackend, p: &ChatParams) -> (Result<ChatResult, RpcError>, Vec<String>) {
    let mut deltas = vec![];
    let r = b.chat(p, &mut |d| deltas.push(d.to_string()), &|| false);
    (r, deltas)
}

fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

#[test]
fn loads_and_streams_an_answer() {
    let lab = Lab::new();
    lab.build("cpu", &[]);
    let mut b = lab.backend(BuildChoice::Auto);
    let loaded = b.load(&lab.load_params(-1)).unwrap();
    assert_eq!(
        (loaded.backend.as_str(), loaded.context, loaded.name.as_str()),
        ("CPU", 8192, "Qwen3 Test")
    );
    assert!(loaded.rss_bytes > 0);
    let args = lab.args();
    assert_eq!(arg_after(&args, "-ngl"), Some("0"));
    assert_eq!(arg_after(&args, "-c"), Some("8192"));
    assert_eq!(arg_after(&args, "--threads"), Some("2"));
    assert_eq!(arg_after(&args, "--host"), Some("127.0.0.1"));
    assert!(args.iter().any(|a| a == "--no-webui") && !args.iter().any(|a| a.contains("api-key")));

    let (r, deltas) = chat(&mut b, &chat_params("say hello please"));
    let r = r.unwrap();
    assert_eq!(r.text, "Hello from the fake server.");
    assert_eq!(deltas.concat(), r.text);
    assert_eq!(deltas.len(), 5);
    assert_eq!((r.prompt_tokens, r.completion_tokens, r.finish), (4, 5, WireFinish::Stop));
    let sent = lab.requests("/completion");
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0]["stream"].as_bool(), sent[0]["cache_prompt"].as_bool()),
        (Some(true), Some(true))
    );
    assert_eq!(sent[0]["top_k"], 1);
    assert!(sent[0].get("json_schema").is_none());
    assert_eq!(lab.requests("/tokenize")[0]["content"], "say hello please");
    b.unload();
}

#[test]
fn schemas_and_sampling_are_passed_through() {
    let lab = Lab::new();
    lab.build("cpu", &[]);
    let mut b = lab.backend(BuildChoice::Auto);
    b.load(&lab.load_params(-1)).unwrap();
    let schema = json!({"type": "object", "properties": {"a": {"type": "integer"}}});
    let p = ChatParams {
        temperature: 0.7,
        json_schema: Some(schema.clone()),
        ..chat_params("x")
    };
    chat(&mut b, &p).0.unwrap();
    let sent = &lab.requests("/completion")[0];
    assert_eq!(sent["json_schema"], schema);
    assert_eq!((sent["top_k"].as_u64(), sent["seed"].as_u64()), (Some(40), Some(1234)));
    let bad = ChatParams {
        json_schema: Some(json!({"type": "bogus"})),
        ..chat_params("x")
    };
    let e = chat(&mut b, &bad).0.unwrap_err();
    assert_eq!(e.code, code::INVALID_PARAMS);
    assert!(e.message.contains("schema"), "{}", e.message);
}

#[test]
fn special_tags_are_kept_in_the_text() {
    let lab = Lab::new();
    lab.build("cpu", &[]);
    let mut b = lab.backend(BuildChoice::Auto);
    b.load(&lab.load_params(-1)).unwrap();
    let r = chat(&mut b, &chat_params("TOOLTAG")).0.unwrap();
    assert_eq!(r.text, "<tool_call>{\"name\":\"x\"}</tool_call>");
}

#[test]
fn the_answer_budget_gives_length() {
    let lab = Lab::new();
    lab.build("cpu", &[]).script(json!({"reply": "a b c d e f g h"}));
    let mut b = lab.backend(BuildChoice::Auto);
    b.load(&lab.load_params(-1)).unwrap();
    let r = chat(
        &mut b,
        &ChatParams {
            max_tokens: 3,
            ..chat_params("x")
        },
    )
    .0
    .unwrap();
    assert_eq!((r.text.as_str(), r.finish), ("a b c ", WireFinish::Length));
    assert_eq!(lab.requests("/completion")[0]["n_predict"], 3);
}

#[test]
fn cancelling_closes_the_request() {
    let lab = Lab::new();
    lab.build("cpu", &[]).script(json!({"mode": "slow"}));
    let mut b = lab.backend(BuildChoice::Auto);
    b.load(&lab.load_params(-1)).unwrap();
    let seen = AtomicUsize::new(0);
    let start = Instant::now();
    let r = b
        .chat(
            &chat_params("go"),
            &mut |_| {
                seen.fetch_add(1, Ordering::SeqCst);
            },
            &|| seen.load(Ordering::SeqCst) >= 3,
        )
        .unwrap();
    assert_eq!(r.finish, WireFinish::Cancelled);
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(r.completion_tokens, 3);
    // The server notices the closed connection and stops generating.
    let marker = side(&lab.model(), ".cancelled");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(marker.exists(), "the server kept streaming after the cancel");
    // Cancelled before anything was sent.
    let r = b.chat(&chat_params("go"), &mut |_| {}, &|| true).unwrap();
    assert_eq!((r.finish, r.text.as_str()), (WireFinish::Cancelled, ""));
}

#[test]
fn an_oversized_prompt_is_a_context_overflow() {
    let lab = Lab::new();
    lab.build("cpu", &[]).script(json!({"n_ctx": 64}));
    let mut b = lab.backend(BuildChoice::Auto);
    let loaded = b.load(&lab.load_params(-1)).unwrap();
    assert_eq!(loaded.context, 64, "the context the server really has is reported");
    let e = chat(&mut b, &chat_params(&"word ".repeat(100))).0.unwrap_err();
    assert_eq!(e, context_overflow(101, 64));
    assert!(lab.requests("/completion").is_empty());
    let e = chat(&mut b, &chat_params(&"word ".repeat(48))).0.unwrap_err();
    assert_eq!(e.code, code::CONTEXT_OVERFLOW, "16 tokens stay free for the answer");
}

#[test]
fn a_crashed_server_is_reported_and_restarted() {
    let lab = Lab::new();
    lab.build("cpu", &[]).script(json!({"mode": "crash_on_chat"}));
    let mut b = lab.backend(BuildChoice::Auto);
    b.load(&lab.load_params(-1)).unwrap();
    let e = chat(&mut b, &chat_params("hi")).0.unwrap_err();
    assert_eq!(e.code, code::ENGINE);
    assert!(e.message.contains("llama-server exited"), "{}", e.message);
    assert!(e.message.contains("GGML_ASSERT"), "the server's own error is shown: {}", e.message);
    // The next request starts it again (it crashes only once).
    let r = chat(&mut b, &chat_params("hi")).0.unwrap();
    assert_eq!(r.finish, WireFinish::Stop);
    // And a new load replaces it.
    b.load(&lab.load_params(-1)).unwrap();
    assert!(chat(&mut b, &chat_params("hi")).0.is_ok());
}

#[test]
fn unload_stops_the_server() {
    let lab = Lab::new();
    lab.build("cpu", &[]);
    let mut b = lab.backend(BuildChoice::Auto);
    b.load(&lab.load_params(-1)).unwrap();
    let port = b.port().unwrap();
    b.unload();
    assert!(b.port().is_none());
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    assert_eq!(chat(&mut b, &chat_params("x")).0.unwrap_err().code, code::NOT_LOADED);
}

#[test]
fn missing_llama_server_says_what_to_do() {
    let lab = Lab::new();
    let mut b = lab.backend(BuildChoice::Auto);
    let e = b.load(&lab.load_params(-1)).unwrap_err();
    assert_eq!(e.code, code::LOAD_FAILED);
    assert!(
        e.message.contains("no inference runtime") && e.message.contains("Settings → AI"),
        "{}",
        e.message
    );
    assert_eq!(e.data.unwrap()["reason"], imt_ai_worker::locate::NO_RUNTIME);
    let mut custom = ServerBackend::new(LlamaConfig {
        custom: Some(lab.dir.path().join("nope.exe")),
        roots: vec![],
        choice: BuildChoice::Auto,
    });
    assert!(custom
        .load(&lab.load_params(-1))
        .unwrap_err()
        .message
        .contains("custom llama-server"));
}

#[test]
fn a_custom_llama_server_path_is_used() {
    let lab = Lab::new();
    let mut b = ServerBackend::new(LlamaConfig {
        custom: Some(PathBuf::from(FAKE)),
        roots: vec![lab.root()],
        choice: BuildChoice::Fixed("vulkan".into()),
    });
    let loaded = b.load(&lab.load_params(-1)).unwrap();
    assert!(loaded.backend.starts_with("CPU"), "{}", loaded.backend);
    assert!(chat(&mut b, &chat_params("x")).0.is_ok());
}

#[test]
fn a_model_that_does_not_load_is_explained() {
    let lab = Lab::new();
    lab.build("cpu", &[]).script(json!({"mode": "exit_on_start"}));
    let e = lab.backend(BuildChoice::Auto).load(&lab.load_params(-1)).unwrap_err();
    assert_eq!(e.code, code::LOAD_FAILED);
    assert!(
        e.message.contains("unknown model architecture") && e.message.contains("update IMT"),
        "{}",
        e.message
    );
    std::fs::write(lab.model(), b"not a gguf").unwrap();
    assert_eq!(
        lab.backend(BuildChoice::Auto).load(&lab.load_params(-1)).unwrap_err().code,
        code::LOAD_FAILED
    );
}

#[test]
fn a_server_that_never_gets_ready_times_out() {
    let lab = Lab::new();
    lab.build("cpu", &[]).script(json!({"mode": "never_ready"}));
    let mut b = ServerBackend::with_timeouts(
        LlamaConfig {
            custom: None,
            roots: vec![lab.root()],
            choice: BuildChoice::Auto,
        },
        Timeouts {
            ready: Duration::from_secs(1),
            ..Timeouts::default()
        },
    );
    let e = b.load(&lab.load_params(-1)).unwrap_err();
    assert!(e.message.contains("did not load the model within 1 s"), "{}", e.message);
}

#[test]
fn auto_prefers_a_gpu_build_and_plans_layers() {
    let lab = Lab::new();
    lab.build("cpu", &[])
        .build("vulkan", &[("devices.txt", GPU)])
        .build("cuda-13", &[("list-fail", "")]);
    let mut b = lab.backend(BuildChoice::Auto);
    let loaded = b.load(&lab.load_params(-1)).unwrap();
    assert_eq!(loaded.backend, "Vulkan");
    assert_ne!(arg_after(&lab.args(), "-ngl"), Some("0"));
    assert!(chat(&mut b, &chat_params("x")).0.is_ok());
    // CPU only: the CPU build, even though a GPU build works.
    let loaded = b.load(&lab.load_params(0)).unwrap();
    assert_eq!(loaded.backend, "CPU");
    assert_eq!(arg_after(&lab.args(), "-ngl"), Some("0"));
    // A fixed layer count is passed as is.
    b.load(&lab.load_params(7)).unwrap();
    assert_eq!(arg_after(&lab.args(), "-ngl"), Some("7"));
}

#[test]
fn a_build_that_fails_falls_back_with_a_message() {
    let lab = Lab::new();
    lab.build("cpu", &[]).build("vulkan", &[("devices.txt", GPU), ("start-fail", "")]);
    let loaded = lab.backend(BuildChoice::Fixed("vulkan".into())).load(&lab.load_params(-1)).unwrap();
    assert!(loaded.backend.starts_with("CPU (fallback: the Vulkan"), "{}", loaded.backend);
    assert!(loaded.backend.contains("ggml_vulkan: failed to initialize"), "{}", loaded.backend);
    let loaded = lab.backend(BuildChoice::Fixed("rocm".into())).load(&lab.load_params(-1)).unwrap();
    assert!(
        loaded.backend.contains("ROCm / HIP (AMD) build is not installed"),
        "{}",
        loaded.backend
    );
}

#[test]
fn a_chosen_build_that_cannot_run_here_falls_back() {
    let lab = Lab::new();
    lab.build("cpu", &[]).build("cuda-13", &[("list-fail", "")]);
    let loaded = lab
        .backend(BuildChoice::Fixed("cuda-13".into()))
        .load(&lab.load_params(-1))
        .unwrap();
    assert!(
        loaded
            .backend
            .starts_with("CPU (fallback: the CUDA 13 (NVIDIA) build cannot be used here"),
        "{}",
        loaded.backend
    );
    assert!(loaded.backend.contains("libcudart"), "{}", loaded.backend);
    // Automatic: an unusable build is skipped without a note.
    assert_eq!(lab.backend(BuildChoice::Auto).load(&lab.load_params(-1)).unwrap().backend, "CPU");
}

#[test]
fn ambient_llama_variables_never_reach_the_server() {
    // The fake exits when it sees any LLAMA_ARG_* variable (they could enable llama-server's agent tools).
    std::env::set_var("LLAMA_ARG_TOOLS", "all");
    let lab = Lab::new();
    lab.build("cpu", &[]);
    let r = lab.backend(BuildChoice::Auto).load(&lab.load_params(-1));
    std::env::remove_var("LLAMA_ARG_TOOLS");
    r.unwrap();
}

#[test]
fn the_build_list_reports_what_works() {
    let lab = Lab::new();
    lab.build("cpu", &[])
        .build("vulkan", &[("devices.txt", GPU)])
        .build("cuda-13", &[("list-fail", "")]);
    std::fs::write(lab.root().join("manifest.json"), r#"{"tag":"b11669"}"#).unwrap();
    let cfg = LlamaConfig {
        custom: None,
        roots: vec![lab.root()],
        choice: BuildChoice::Auto,
    };
    let r = listing::report(&cfg, Duration::from_secs(20));
    assert_eq!(
        (r.source.as_str(), r.release.as_deref(), r.auto.as_deref()),
        ("installed", Some("b11669"), Some("vulkan"))
    );
    let ids: Vec<(&str, bool)> = r.builds.iter().map(|b| (b.id.as_str(), b.usable)).collect();
    assert_eq!(ids, [("cpu", true), ("cuda-13", false), ("vulkan", true)]);
    assert!(r.builds[1].problem.as_deref().unwrap().contains("libcudart"));
    assert_eq!(r.builds[2].devices[0].free_mib, 8000);
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["builds"][2]["devices"][0]["freeMib"], 8000);
}

/// Downloads the CPU build (Metal on Apple Silicon) of the pinned release into `dir` exactly as IMT does.
fn download_runtime(dir: &Path) -> PathBuf {
    use imt_ai::models::{Progress, SystemDisk};
    use imt_ai::runtimes::{current_platform, GithubReleases, ReleaseConfig, RuntimeCatalog, RuntimeStore};
    let catalog = RuntimeCatalog::embedded();
    let platform = current_platform();
    let id = if platform == "macos-arm64" { "metal" } else { "cpu" };
    let build = catalog
        .find(&platform, id)
        .unwrap_or_else(|| panic!("no {id} build for {platform}"));
    let source = GithubReleases::new(ReleaseConfig::default(), catalog.clone()).unwrap();
    let store = RuntimeStore::new(dir);
    let rt = store
        .install(
            &catalog,
            build,
            &source,
            &SystemDisk,
            Default::default(),
            &Progress::new(),
            &mut |s| println!("{s:?}"),
        )
        .unwrap();
    rt.dir.join(server_file_name())
}

/// A tiny model (stories15M, 19 MB) from the ggml-org test models.
fn download_model(dir: &Path) -> PathBuf {
    let url = "https://huggingface.co/ggml-org/models/resolve/main/tinyllamas/stories15M-q4_0.gguf";
    // IMT's client (OS certificate store, so it works behind a TLS-inspecting proxy too).
    let agent = imt_ai::net::agent(&imt_ai::net::HttpConfig {
        proxy: None,
        connect_timeout: std::time::Duration::from_secs(15),
        timeout: std::time::Duration::from_secs(300),
    })
    .unwrap();
    // It follows no redirects by itself (IMT's adapters allow-list them): the CDN hop is followed here.
    let mut resp = agent.get(url).call().unwrap();
    for _ in 0..5 {
        let Some(next) = resp.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string) else {
            break;
        };
        let next = if next.starts_with('/') {
            format!("https://huggingface.co{next}")
        } else {
            next
        };
        resp = agent.get(&next).call().unwrap();
    }
    let mut body = resp.into_body();
    let bytes = body.with_config().limit(64 * 1024 * 1024).read_to_vec().unwrap();
    assert!(bytes.starts_with(b"GGUF"));
    let p = dir.join("stories15M-q4_0.gguf");
    std::fs::write(&p, bytes).unwrap();
    p
}

/// The real llama-server with a real model. Opt in with either
///   IMT_LLAMA_SERVER=/path/llama-server IMT_TEST_GGUF=/path/model.gguf, or
///   IMT_LLAMA_E2E=1 (downloads the pinned CPU build and a tiny model into a temporary folder):
///     cargo test -p imt-ai-worker --test server_backend -- --ignored --nocapture
#[test]
#[ignore]
fn the_real_llama_server_answers() {
    let tmp = tempfile::tempdir().unwrap();
    let e2e = std::env::var("IMT_LLAMA_E2E").is_ok_and(|v| v == "1");
    let server = match std::env::var("IMT_LLAMA_SERVER") {
        Ok(s) => PathBuf::from(s),
        Err(_) if e2e => download_runtime(&tmp.path().join("runtimes")),
        Err(_) => panic!("set IMT_LLAMA_E2E=1, or IMT_LLAMA_SERVER and IMT_TEST_GGUF"),
    };
    let model = match std::env::var("IMT_TEST_GGUF") {
        Ok(m) => m,
        Err(_) if e2e => download_model(tmp.path()).to_string_lossy().into_owned(),
        Err(_) => panic!("set IMT_TEST_GGUF"),
    };
    let mut b = ServerBackend::new(LlamaConfig {
        custom: Some(server),
        roots: vec![],
        choice: BuildChoice::Auto,
    });
    let loaded = b
        .load(&LoadParams {
            model_path: model,
            gpu_layers: -1,
            context: 2048,
            threads: 0,
        })
        .unwrap();
    println!("loaded: {loaded:?}");
    let p = ChatParams {
        prompt: "Once upon a time".into(),
        max_tokens: 32,
        temperature: 0.0,
        json_schema: None,
    };
    let (r, deltas) = chat(&mut b, &p);
    let r = r.unwrap();
    println!("answer: {r:?}");
    assert!(!r.text.is_empty() && deltas.len() > 1);
    assert_eq!(deltas.concat(), r.text);
    assert!(r.completion_tokens > 0 && r.prompt_tokens > 0);
    // Greedy is deterministic.
    assert_eq!(chat(&mut b, &p).0.unwrap().text, r.text);
    // A schema constrains the output.
    let schema = json!({"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"]});
    let s = chat(
        &mut b,
        &ChatParams {
            json_schema: Some(schema),
            max_tokens: 40,
            ..p.clone()
        },
    )
    .0
    .unwrap();
    println!("schema answer: {s:?}");
    let v: Value = serde_json::from_str(s.text.trim().trim_end_matches("</s>")).unwrap();
    assert!(v["n"].is_i64() || v["n"].is_u64());
    // Overflow and cancel.
    let e = chat(
        &mut b,
        &ChatParams {
            prompt: "word ".repeat(5000),
            ..p.clone()
        },
    )
    .0
    .unwrap_err();
    assert_eq!(e.code, code::CONTEXT_OVERFLOW);
    let n = AtomicUsize::new(0);
    let c = b
        .chat(
            &ChatParams {
                max_tokens: 500,
                ..p.clone()
            },
            &mut |_| {
                n.fetch_add(1, Ordering::SeqCst);
            },
            &|| n.load(Ordering::SeqCst) >= 2,
        )
        .unwrap();
    assert_eq!(c.finish, WireFinish::Cancelled);
    b.unload();
}

#[test]
fn the_worker_binary_lists_builds_for_imt() {
    let lab = Lab::new();
    lab.build("cpu", &[]).build("vulkan", &[("devices.txt", GPU)]);
    let env = vec![
        ("IMT_LLAMA_DIR".to_string(), lab.root().to_string_lossy().into_owned()),
        ("IMT_LLAMA_BUILD".to_string(), "vulkan".to_string()),
    ];
    let worker = Path::new(env!("CARGO_BIN_EXE_imt-ai-worker"));
    let r = imt_ai::runtimes::list_builds(worker, &env, Duration::from_secs(60)).unwrap();
    assert_eq!(
        (r.source.as_str(), r.choice.as_str(), r.auto.as_deref()),
        ("installed", "vulkan", Some("vulkan"))
    );
    assert_eq!(r.builds.len(), 2);
    let none = imt_ai::runtimes::list_builds(worker, &[], Duration::from_secs(60)).unwrap();
    assert_eq!(none.source, "missing");
    assert!(none.problem.unwrap().contains("download"));
}
