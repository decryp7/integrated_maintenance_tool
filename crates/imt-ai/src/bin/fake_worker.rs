//! A fake inference worker for the supervisor tests (a test helper, not shipped). Its behaviour is chosen by command-line flags:
//! crash, hang, slow answers, failing GPU loads, crash-once, ignoring cancel.

use imt_ai::worker::protocol::*;
use imt_ai::worker::{run_worker, WorkerBackend};

struct Fake {
    args: Vec<String>,
}

impl Fake {
    fn has(&self, f: &str) -> bool {
        self.args.iter().any(|a| a == f)
    }
    fn value(&self, f: &str) -> Option<String> {
        self.args.iter().find_map(|a| a.strip_prefix(&format!("{f}=")).map(str::to_string))
    }
}

impl WorkerBackend for Fake {
    fn load(&mut self, p: &LoadParams) -> Result<LoadResult, RpcError> {
        if self.has("--crash-on-load") {
            crash();
        }
        if self.has("--fail-load") || (self.has("--fail-gpu-load") && p.gpu_layers != 0) {
            return Err(RpcError::new(code::LOAD_FAILED, "device initialisation failed"));
        }
        let backend = if p.gpu_layers == 0 { "CPU" } else { "Fake GPU" };
        Ok(LoadResult {
            backend: backend.into(),
            context: p.context,
            name: "fake-model".into(),
            rss_bytes: 1 << 20,
            vision: p.mmproj_path.is_some(),
        })
    }

    fn chat(&mut self, p: &ChatParams, delta: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> Result<ChatResult, RpcError> {
        if self.has("--crash-on-chat") {
            crash();
        }
        if let Some(marker) = self.value("--crash-once") {
            if !std::path::Path::new(&marker).exists() {
                let _ = std::fs::write(&marker, b"crashed");
                crash();
            }
        }
        if self.has("--hang-on-chat") {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        }
        if self.has("--overflow") {
            return Err(context_overflow(5000, 4096));
        }
        let slow: u64 = self.value("--slow-ms").and_then(|v| v.parse().ok()).unwrap_or(0);
        let ignore_cancel = self.has("--ignore-cancel");
        let tail: String = p.prompt.chars().rev().take(30).collect::<Vec<_>>().into_iter().rev().collect();
        let full = match p.images.len() {
            0 => format!("echo:{tail}"),
            n => format!("images:{n} echo:{tail}"),
        };
        let mut sent = String::new();
        let steps = if slow > 0 { 40 } else { 1 };
        let chunk = full.len().div_ceil(steps).max(1);
        for piece in full.as_bytes().chunks(chunk) {
            if slow > 0 {
                std::thread::sleep(std::time::Duration::from_millis(slow));
            }
            if !ignore_cancel && cancelled() {
                return Ok(ChatResult {
                    text: sent,
                    prompt_tokens: 1,
                    completion_tokens: 0,
                    finish: WireFinish::Cancelled,
                });
            }
            let t = String::from_utf8_lossy(piece).into_owned();
            delta(&t);
            sent.push_str(&t);
        }
        Ok(ChatResult {
            text: sent,
            prompt_tokens: p.prompt.len() / 4,
            completion_tokens: full.len() / 4,
            finish: WireFinish::Stop,
        })
    }

    fn unload(&mut self) {}
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--exit-immediately") {
        return;
    }
    if let Err(e) = run_worker(move || Box::new(Fake { args })) {
        eprintln!("fake worker: {e}");
        std::process::exit(1);
    }
}

/// Dies like a crash. On Windows `abort` goes through Windows Error Reporting, which can keep a crashing process
/// alive for seconds on a busy machine (and the crash then looks like a hang); exit at once instead, with the
/// status an abort leaves (STATUS_STACK_BUFFER_OVERRUN).
fn crash() -> ! {
    #[cfg(windows)]
    std::process::exit(0xC000_0409_u32 as i32);
    #[cfg(not(windows))]
    std::process::abort();
}
