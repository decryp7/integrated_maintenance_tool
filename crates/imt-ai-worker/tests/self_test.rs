//! The runtime self-test (ADR 0009) through the real worker binary and the fake llama-server: what IMT runs
//! before it switches to a newly downloaded or updated build. A build whose server answers passes; one that cannot
//! read the test image, or does not start, fails with a typed error that says why and what to do.

use imt_ai::gguf;
use imt_ai::hardware::Hardware;
use imt_ai::runtime::{GpuMode, LoadRequest};
use imt_ai::runtimes::selftest::{self_test, SelfTestReport};
use imt_ai::worker::{fixed_env, FramesLauncher, WorkerConfig, WorkerRuntime};
use imt_ai::AiError;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

const FAKE: &str = env!("CARGO_BIN_EXE_imt-fake-llama-server");
const WORKER: &str = env!("CARGO_BIN_EXE_imt-ai-worker");

fn side(model: &Path, ext: &str) -> PathBuf {
    let mut s = model.as_os_str().to_owned();
    s.push(ext);
    PathBuf::from(s)
}

/// A model (with a projector when `vision`) scripted with `script`, and the fake server as the build under test.
fn run(script: Value, vision: bool) -> (Result<SelfTestReport, AiError>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let model = dir.path().join("tiny.gguf");
    std::fs::write(&model, imt_ai::testing::gguf::qwen_like()).unwrap();
    std::fs::write(side(&model, ".fake.json"), script.to_string()).unwrap();
    let projector = vision.then(|| {
        let p = dir.path().join("tiny.mmproj.gguf");
        std::fs::write(&p, imt_ai::testing::gguf::projector_like(2560)).unwrap();
        p
    });
    let server = dir.path().join("new-build").join(imt_ai::runtimes::store::server_file_name());
    std::fs::create_dir_all(server.parent().unwrap()).unwrap();
    std::fs::copy(FAKE, &server).unwrap();
    let mut cfg = WorkerConfig::new(Arc::new(FramesLauncher {
        exe: PathBuf::from(WORKER),
        args: vec![],
        env: fixed_env(vec![("IMT_LLAMA_SERVER".into(), server.to_string_lossy().into_owned())]),
    }));
    cfg.load_timeout = Duration::from_secs(60);
    let runtime = WorkerRuntime::new(
        cfg,
        Box::new(|| Hardware {
            os: "x".into(),
            arch: "x".into(),
            cpu_cores: 4,
            total_ram_gb: 64.0,
            available_ram_gb: 48.0,
            gpus: vec![],
        }),
    );
    let req = LoadRequest {
        info: gguf::inspect(&model).unwrap(),
        path: model,
        projector,
        context: 4096,
        gpu: GpuMode::Cpu,
        threads: 1,
    };
    let r = self_test(&runtime, req, "CPU", &Arc::new(AtomicBool::new(false)));
    (r, dir)
}

fn requests(dir: &Path, path: &str) -> Vec<Value> {
    std::fs::read_to_string(side(&dir.join("tiny.gguf"), ".requests.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["path"] == path)
        .map(|v| v["body"].clone())
        .collect()
}

#[test]
fn a_working_build_passes_with_text_and_the_test_image() {
    let (r, dir) = run(json!({"reply": "OK"}), true);
    let r = r.unwrap();
    assert!(r.image_checked);
    assert_eq!(r.server.unwrap().build.as_deref(), Some("b0-fake"));
    let sent = requests(dir.path(), "/completion");
    assert_eq!(sent.len(), 2, "one text completion, one image question");
    let image = &sent[1]["prompt"];
    assert_eq!(image["multimodal_data"].as_array().unwrap().len(), 1);
    assert!(
        image["prompt_string"].as_str().unwrap().contains("<__media_fake_7f3a__>"),
        "the server's own marker"
    );
}

#[test]
fn a_build_without_a_media_marker_and_a_text_model_pass() {
    let (r, dir) = run(json!({"reply": "OK", "media_marker": false}), true);
    assert!(r.unwrap().image_checked);
    assert!(requests(dir.path(), "/completion")[1]["prompt"]["prompt_string"]
        .as_str()
        .unwrap()
        .contains("<__media__>"));
    let (r, dir) = run(json!({"reply": "OK"}), false);
    assert!(!r.unwrap().image_checked);
    assert_eq!(requests(dir.path(), "/completion").len(), 1);
}

#[test]
fn a_build_that_cannot_read_the_image_fails_and_says_what_to_do() {
    let (r, _dir) = run(json!({"reply": "OK", "mode": "colour_blind"}), true);
    let e = r.unwrap_err();
    let AiError::SelfTest(m) = &e else { panic!("{e:?}") };
    assert!(m.contains("reading an image") && m.contains("\"I cannot tell.\""), "{m}");
    assert!(m.contains("keeps using the previous runtime"), "{m}");
    // A build that reports it cannot read images at all.
    let (r, _dir) = run(json!({"reply": "OK", "vision": false}), true);
    assert!(matches!(r.unwrap_err(), AiError::SelfTest(m) if m.contains("cannot read images")));
}

#[test]
fn a_build_that_does_not_start_fails() {
    let (r, _dir) = run(json!({"mode": "exit_on_start"}), false);
    let e = r.unwrap_err();
    assert!(matches!(&e, AiError::SelfTest(m) if m.contains("did not load the model")), "{e:?}");
}
