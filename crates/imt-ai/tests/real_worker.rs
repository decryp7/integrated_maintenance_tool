//! Smoke test of the real worker (which runs a prebuilt llama-server, ADR 0009) with a real model.
//! Ignored by default:
//!
//!   cargo build -p imt-ai-worker
//!   IMT_LLAMA_SERVER=/path/llama-server IMT_TEST_GGUF=/path/model.gguf \
//!     cargo test -p imt-ai --test real_worker -- --ignored --nocapture
//!
//! Instead of `IMT_LLAMA_SERVER`, `IMT_LLAMA_DIR` may point at a runtimes folder (`<data>/ai/runtimes/<tag>/`,
//! holding `<id>/llama-server`) with `IMT_LLAMA_BUILD` choosing a build (`auto` by default); the worker
//! inherits these from the test. `IMT_AI_WORKER` may point at the worker binary; otherwise the one next to
//! the test target is used.

mod common;
use imt_ai::assistant::{Assistant, AssistantConfig};
use imt_ai::gguf;
use imt_ai::hardware::{HardwareProbe, SystemProbe};
use imt_ai::llm::{ChatMessage, ChatRequest, LlmProvider, StopReason, ToolSpec};
use imt_ai::runtime::{GpuMode, LoadRequest, ModelRuntime};
use imt_ai::worker::{FramesLauncher, WorkerConfig, WorkerRuntime};
use serde_json::json;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn worker_exe() -> PathBuf {
    if let Ok(p) = std::env::var("IMT_AI_WORKER") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().unwrap();
    exe.parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(if cfg!(windows) { "imt-ai-worker.exe" } else { "imt-ai-worker" })
}

struct Sys;
impl imt_collect::CommandRunner for Sys {
    fn run(&self, p: &str, a: &[String], t: Duration) -> Result<imt_collect::CommandOutput, imt_collect::CommandError> {
        imt_collect::SystemRunner.run(p, a, t)
    }
}

#[test]
#[ignore]
fn the_real_worker_loads_a_model_and_answers() {
    let Ok(model) = std::env::var("IMT_TEST_GGUF") else {
        panic!("set IMT_TEST_GGUF to a .gguf file")
    };
    let path = PathBuf::from(model);
    let info = gguf::inspect(&path).unwrap();
    println!(
        "model: {:?} {} B, arch {}, ctx {:?}",
        info.name, info.params_b, info.architecture, info.context_length
    );
    let mut cfg = WorkerConfig::new(Arc::new(FramesLauncher {
        exe: worker_exe(),
        args: vec![],
        env: imt_ai::worker::fixed_env(vec![]),
    }));
    cfg.stall_timeout = Duration::from_secs(120);
    let rt = WorkerRuntime::new(cfg, Box::new(|| SystemProbe::new(Sys).probe()));
    let loaded = rt
        .load(&LoadRequest {
            path,
            projector: None,
            info,
            context: 4096,
            gpu: GpuMode::Auto,
            threads: 0,
        })
        .unwrap();
    println!("backend: {}", loaded.backend);
    let p = loaded.provider;

    // plain answer, streamed
    let mut streamed = String::new();
    let r = p
        .chat(
            &ChatRequest {
                system: "You are terse.".into(),
                messages: vec![ChatMessage::user("What is the capital of France? Answer in one word.")],
                max_tokens: 24,
                temperature: Some(0.0),
                ..ChatRequest::default()
            },
            &mut |d| streamed.push_str(d),
        )
        .unwrap();
    println!("answer: {:?}", r.text);
    assert!(r.text.to_lowercase().contains("paris"), "{}", r.text);
    assert_eq!(streamed.trim(), r.text);

    // grammar-constrained JSON
    let schema = json!({"type": "object", "properties": {"city": {"type": "string"}, "country": {"type": "string"}}, "required": ["city", "country"]});
    let r = p
        .chat(
            &ChatRequest {
                messages: vec![ChatMessage::user("Name a European capital and its country.")],
                max_tokens: 60,
                temperature: Some(0.0),
                json_schema: Some(schema),
                ..ChatRequest::default()
            },
            &mut |_| {},
        )
        .unwrap();
    println!("json: {}", r.text);
    let v: serde_json::Value = serde_json::from_str(&r.text).expect("constrained output is valid JSON");
    assert!(v["city"].is_string() && v["country"].is_string());

    // tool calling through the text protocol
    let tool = ToolSpec {
        name: "run_query".into(),
        description: "Run a log query".into(),
        input_schema: json!({"type": "object", "properties": {"ql": {"type": "string"}}, "required": ["ql"]}),
    };
    let r = p
        .chat(
            &ChatRequest {
                system: "Use the tool to answer.".into(),
                messages: vec![ChatMessage::user("Find the error events with the run_query tool.")],
                tools: vec![tool],
                max_tokens: 120,
                temperature: Some(0.0),
                ..ChatRequest::default()
            },
            &mut |_| {},
        )
        .unwrap();
    println!("tool round: text={:?} calls={:?}", r.text, r.tool_calls);

    // cancellation
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let mut n = 0;
    let r = p
        .chat(
            &ChatRequest {
                messages: vec![ChatMessage::user("Count from 1 to 500, one number per line.")],
                max_tokens: 400,
                cancel: Some(cancel),
                ..ChatRequest::default()
            },
            &mut |_| {
                n += 1;
                if n == 5 {
                    flag.store(true, Ordering::SeqCst);
                }
            },
        )
        .unwrap();
    assert_eq!(r.stop_reason, StopReason::Cancelled);
    assert!(rt.disabled_reason().is_none());

    // a use case end to end (small models may need the repair round or fail; the outcome is printed, not asserted)
    let a = Assistant::new(p, AssistantConfig::default());
    match a.nl_to_query(common::store(), "show me the errors from WEB02") {
        Ok(q) => println!(
            "nl_to_query: {} ({:?} matches, repaired: {})",
            q.ql, q.estimated_matches, q.repaired
        ),
        Err(e) => println!("nl_to_query failed (acceptable for a tiny model): {e}"),
    }
    let sample = common::billing_sample(60);
    match a.propose_pipeline(
        common::store(),
        "drop the invoice written lines and hide the invoice numbers",
        &sample,
        None,
    ) {
        Ok(p) => println!(
            "propose_pipeline: kept {} dropped {} changed {}\n{}",
            p.kept, p.dropped, p.changed, p.toml
        ),
        Err(e) => println!("propose_pipeline failed: {e}"),
    }
    match a.propose_markers(common::store(), "mark waiting for payment confirmation", &sample, None) {
        Ok(p) => println!("propose_markers: {:?}\n{}", p.markers, p.toml),
        Err(e) => println!("propose_markers failed: {e}"),
    }
    match a.propose_extractors(common::store(), "extract the invoice number", &sample, None) {
        Ok(p) => println!(
            "propose_extractors: {:?}",
            p.extractors.iter().map(|e| (&e.name, e.matches)).collect::<Vec<_>>()
        ),
        Err(e) => println!("propose_extractors failed: {e}"),
    }
    match a.propose_parser(common::store(), &sample, "billing.log") {
        Ok(p) => println!("propose_parser: {} parsed {} importer {:?}", p.name, p.parsed, p.importer),
        Err(e) => println!("propose_parser failed: {e}"),
    }
    match a.propose_regex(
        &[
            "Invoice INV-236778 written in 12 ms".to_string(),
            "Invoice INV-741653 written in 7 ms".to_string(),
            "Invoice INV-755059 written in 31 ms".to_string(),
        ],
        Some("capture the invoice number and the duration"),
    ) {
        Ok(r) => println!("propose_regex: {} ({}/{})", r.pattern, r.match_count, r.total),
        Err(e) => println!("propose_regex failed (acceptable for a tiny model): {e}"),
    }
}
