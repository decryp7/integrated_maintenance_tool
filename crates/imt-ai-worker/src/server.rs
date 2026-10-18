//! The worker's backend: runs the official prebuilt `llama-server` as a child and talks to it over HTTP on
//! 127.0.0.1 (ADR 0009). It chooses the build (CUDA, Vulkan, Metal, CPU, ...), plans the GPU layers from
//! the free memory the build reports, falls back to the next build when one does not start, restarts a
//! server that died, and maps everything to the worker protocol's typed errors.

use crate::child::{run_briefly, ServerChild, StartError};
use crate::completion::{completion_body, parse_stream_line, server_error, token_count, tokenize_body, Event};
use crate::devices::{best_gpu, parse_list_devices, Device};
use crate::http::{Call, HttpError};
use crate::locate::LlamaConfig;
use crate::variants::{label, needs_gpu, try_order, BuildChoice};
use imt_ai::gguf;
use imt_ai::recommend::{plan_offload, OffloadInput};
use imt_ai::worker::protocol::*;
use imt_ai::worker::{current_rss_bytes, WorkerBackend};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Largest single SSE line accepted (one event carries one token or the final statistics).
const MAX_LINE: usize = 1024 * 1024;
/// Largest answer text (well below the gRPC message limit).
const MAX_TEXT: usize = 8 * 1024 * 1024;

/// Timeouts, adjustable for tests.
#[derive(Debug, Clone)]
pub struct Timeouts {
    /// Loading a model (until `/health` answers 200); just under the supervisor's load timeout.
    pub ready: Duration,
    /// `--list-devices` of one build.
    pub probe: Duration,
    /// Small requests (`/tokenize`, `/props`).
    pub request: Duration,
    /// No byte of an answer for this long fails the chat (the supervisor's stall timeout is shorter
    /// and normally acts first).
    pub stream_idle: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            ready: Duration::from_secs(590),
            probe: Duration::from_secs(30),
            request: Duration::from_secs(60),
            stream_idle: Duration::from_secs(600),
        }
    }
}

/// What `--list-devices` said about one build.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub devices: Vec<Device>,
    /// Set when the build could not run on this machine (missing driver or runtime, wrong CPU, ...).
    pub error: Option<String>,
}

impl Probe {
    pub fn run(exe: &Path, timeout: Duration) -> Probe {
        match run_briefly(exe, &["--list-devices".to_string()], timeout) {
            Ok(out) => Probe {
                devices: parse_list_devices(&out),
                error: None,
            },
            Err(e) => Probe {
                devices: vec![],
                error: Some(e),
            },
        }
    }

    /// Whether the build can be used: it runs, and a GPU build sees a GPU.
    pub fn usable(&self, id: &str) -> Result<(), String> {
        match &self.error {
            Some(e) => Err(e.clone()),
            None if needs_gpu(id) && best_gpu(&self.devices).is_none() => Err("no supported GPU was found".into()),
            None => Ok(()),
        }
    }
}

/// How the server was started, kept to restart it after a crash.
struct Launch {
    exe: PathBuf,
    args: Vec<String>,
}

struct Running {
    child: ServerChild,
    launch: Launch,
    n_ctx: usize,
}

pub struct ServerBackend {
    cfg: LlamaConfig,
    timeouts: Timeouts,
    probes: HashMap<PathBuf, Probe>,
    running: Option<Running>,
}

fn load_failed(m: impl Into<String>) -> RpcError {
    RpcError::new(code::LOAD_FAILED, m)
}

fn engine(m: impl Into<String>) -> RpcError {
    RpcError::new(code::ENGINE, m)
}

/// Total resident memory of the worker and its llama-server.
fn rss_with(child_pid: Option<u32>) -> u64 {
    let mut total = current_rss_bytes();
    if let Some(pid) = child_pid {
        let pid = sysinfo::Pid::from_u32(pid);
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        total += sys.process(pid).map(|p| p.memory()).unwrap_or(0);
    }
    total
}

impl ServerBackend {
    pub fn new(cfg: LlamaConfig) -> Self {
        Self::with_timeouts(cfg, Timeouts::default())
    }

    pub fn with_timeouts(cfg: LlamaConfig, timeouts: Timeouts) -> Self {
        ServerBackend {
            cfg,
            timeouts,
            probes: HashMap::new(),
            running: None,
        }
    }

    /// The port of the running server (tests).
    pub fn port(&self) -> Option<u16> {
        self.running.as_ref().map(|r| r.child.port())
    }

    fn probe(&mut self, exe: &Path) -> Probe {
        let timeout = self.timeouts.probe;
        self.probes
            .entry(exe.to_path_buf())
            .or_insert_with(|| Probe::run(exe, timeout))
            .clone()
    }

    fn props_ctx(&self, child: &ServerChild) -> Option<usize> {
        let call = Call {
            method: "GET",
            path: "/props",
            body: None,
            idle: self.timeouts.request,
            cancelled: &|| false,
        };
        let r = child.client().send(&call, 4 * 1024 * 1024).ok().filter(|r| r.status == 200)?;
        let v = r.json()?;
        v.pointer("/default_generation_settings/n_ctx")
            .or_else(|| v.get("n_ctx"))
            .and_then(Value::as_u64)
            .map(|n| n as usize)
    }

    /// Starts llama-server again after it died, with the arguments of the last load.
    fn restart(&mut self) -> Result<(), RpcError> {
        let Some(r) = self.running.as_mut() else {
            return Err(RpcError::new(code::NOT_LOADED, "no model is loaded"));
        };
        if r.child.alive() {
            return Ok(());
        }
        let died = r.child.describe_exit(None);
        match ServerChild::start(&r.launch.exe, &r.launch.args, self.timeouts.ready, &|| false) {
            Ok(child) => {
                r.child = child;
                Ok(())
            }
            Err(e) => {
                self.running = None;
                Err(engine(format!("{died}; restarting it failed: {e}. Load the model again.")))
            }
        }
    }
}

/// The command line for one model and build (the host, port and key are added when starting).
fn server_args(model: &Path, context: usize, layers: u32, moe_to_cpu: bool, threads: u32) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-m".into(),
        model.to_string_lossy().into_owned(),
        "-c".into(),
        context.to_string(),
        "-ngl".into(),
        if layers == u32::MAX { "all".into() } else { layers.to_string() },
        "--threads".into(),
        threads.to_string(),
        "--threads-batch".into(),
        threads.to_string(),
        "--parallel".into(),
        "1".into(),
        "--fit".into(),
        "off".into(),
        // No web UI, no extra endpoints, no network access for downloads, errors only in the log (never
        // the prompt or the answer).
        "--no-webui".into(),
        "--no-slots".into(),
        "--offline".into(),
        "--log-verbosity".into(),
        "1".into(),
        "--log-colors".into(),
        "off".into(),
    ];
    if moe_to_cpu {
        a.push("--cpu-moe".into());
    }
    a
}

impl WorkerBackend for ServerBackend {
    fn load(&mut self, p: &LoadParams) -> Result<LoadResult, RpcError> {
        self.unload();
        let model = Path::new(&p.model_path);
        let info = gguf::inspect(model).map_err(|e| load_failed(e.to_string()))?;
        let context = match info.context_length {
            Some(c) if c >= 512 => p.context.min(c as usize),
            _ => p.context,
        };
        let found = self.cfg.find()?;
        let ids: Vec<String> = found.builds.iter().map(|b| b.id.clone()).collect();
        let mut notes: Vec<String> = Vec::new();
        let choice = match &self.cfg.choice {
            BuildChoice::Fixed(id) if !ids.contains(id) => {
                notes.push(format!("the {} build is not installed", label(id)));
                BuildChoice::Auto
            }
            // CPU only: no need to start a GPU build.
            BuildChoice::Auto if p.gpu_layers == 0 && ids.iter().any(|i| i == "cpu") => BuildChoice::Fixed("cpu".into()),
            c => c.clone(),
        };
        let explicit = matches!(self.cfg.choice, BuildChoice::Fixed(_));
        let threads = if p.threads == 0 {
            std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4)
        } else {
            p.threads
        };
        let mut last_error: Option<String> = None;
        for id in try_order(&ids, &choice) {
            let Some(build) = found.builds.iter().find(|b| b.id == id) else {
                continue;
            };
            let probe = self.probe(&build.exe);
            if let Err(why) = probe.usable(&id) {
                // Unusable automatic candidates are skipped quietly; a chosen one is worth a note.
                if explicit && BuildChoice::Fixed(id.clone()) == self.cfg.choice {
                    notes.push(format!("the {} build cannot be used here ({why})", label(&id)));
                }
                last_error = Some(why);
                continue;
            }
            let gpu = best_gpu(&probe.devices).cloned();
            let (layers, moe_to_cpu) = match (p.gpu_layers, &gpu) {
                (0, _) | (_, None) => (0, false),
                (n, Some(_)) if n > 0 => (n as u32, false),
                (_, Some(g)) => {
                    let plan = plan_offload(&OffloadInput {
                        file_bytes: info.file_size,
                        block_count: info.block_count.unwrap_or(32),
                        kv_bytes_per_token: info.kv_bytes_per_token(),
                        context,
                        free_gpu_gb: Some(g.free_gb()),
                        moe: info.is_moe(),
                    });
                    (plan.gpu_layers, plan.moe_to_cpu)
                }
            };
            let args = server_args(model, context, layers, moe_to_cpu, threads);
            match ServerChild::start(&build.exe, &args, self.timeouts.ready, &|| false) {
                Ok(child) => {
                    let n_ctx = self.props_ctx(&child).filter(|n| *n > 0).unwrap_or(context);
                    let mut backend = match (&gpu, layers) {
                        (Some(g), n) if n > 0 => g.backend(),
                        _ => "CPU".to_string(),
                    };
                    if !notes.is_empty() {
                        backend = format!("{backend} (fallback: {})", notes.join("; "));
                    }
                    let name = info
                        .name
                        .clone()
                        .unwrap_or_else(|| model.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
                    let pid = child.pid();
                    self.running = Some(Running {
                        child,
                        launch: Launch {
                            exe: build.exe.clone(),
                            args,
                        },
                        n_ctx,
                    });
                    return Ok(LoadResult {
                        backend,
                        context: n_ctx,
                        name,
                        rss_bytes: rss_with(pid),
                    });
                }
                Err(e) => {
                    let why = match e {
                        StartError::TimedOut => format!("it did not load the model within {} s", self.timeouts.ready.as_secs()),
                        e => e.to_string(),
                    };
                    notes.push(format!("the {} build did not start ({why})", label(&id)));
                    last_error = Some(why);
                }
            }
        }
        Err(load_failed(match last_error {
            Some(e) => format!(
                "llama-server could not load the model: {e}. If the model is new, update IMT; otherwise try another model file or another inference build in Settings → AI."
            ),
            None => "no installed inference runtime works on this computer; download another one (the CPU build always works) in the AI set-up".into(),
        }))
    }

    fn chat(&mut self, p: &ChatParams, delta: &mut dyn FnMut(&str), cancelled: &dyn Fn() -> bool) -> Result<ChatResult, RpcError> {
        self.restart()?;
        let Some(r) = self.running.as_mut() else {
            return Err(RpcError::new(code::NOT_LOADED, "no model is loaded"));
        };
        let cancelled_result = |prompt_tokens, text: String, completion_tokens| ChatResult {
            text,
            prompt_tokens,
            completion_tokens,
            finish: WireFinish::Cancelled,
        };
        if cancelled() {
            return Ok(cancelled_result(0, String::new(), 0));
        }
        let n_ctx = r.n_ctx;
        let body = tokenize_body(&p.prompt).to_string();
        let call = Call {
            method: "POST",
            path: "/tokenize",
            body: Some(body.as_bytes()),
            idle: self.timeouts.request,
            cancelled,
        };
        let tokens = match r.child.client().send(&call, 64 * 1024 * 1024) {
            Ok(resp) if resp.status == 200 => resp
                .json()
                .as_ref()
                .and_then(token_count)
                .ok_or_else(|| engine("llama-server sent a malformed token count"))?,
            Ok(resp) => return Err(server_error(resp.status, &resp.json().unwrap_or(Value::Null))),
            Err(HttpError::Cancelled) => return Ok(cancelled_result(0, String::new(), 0)),
            Err(e) => return Err(dead_or(&mut r.child, e)),
        };
        if tokens == 0 || tokens + 16 > n_ctx {
            return Err(context_overflow(tokens, n_ctx));
        }
        let n_predict = p.max_tokens.min(n_ctx - tokens - 1);
        let body = completion_body(p, n_predict).to_string();
        let call = Call {
            method: "POST",
            path: "/completion",
            body: Some(body.as_bytes()),
            idle: self.timeouts.stream_idle,
            cancelled,
        };
        let (status, mut stream) = match r.child.client().open(&call) {
            Ok(s) => s,
            Err(HttpError::Cancelled) => return Ok(cancelled_result(tokens, String::new(), 0)),
            Err(e) => return Err(dead_or(&mut r.child, e)),
        };
        if status != 200 {
            let v = stream.read_all(1024 * 1024).ok().and_then(|b| serde_json::from_slice(&b).ok());
            return Err(server_error(status, &v.unwrap_or(Value::Null)));
        }
        let (mut text, mut produced) = (String::new(), 0usize);
        loop {
            if cancelled() {
                return Ok(cancelled_result(tokens, text, produced));
            }
            let line = match stream.next_line(MAX_LINE) {
                Ok(Some(l)) => l,
                Ok(None) => {
                    drop(stream);
                    return Err(match has_died(&mut r.child) {
                        false => engine("llama-server ended the answer early"),
                        true => engine(format!("{}; it is restarted on the next request", r.child.describe_exit(None))),
                    });
                }
                Err(HttpError::Cancelled) => return Ok(cancelled_result(tokens, text, produced)),
                Err(e) => {
                    drop(stream);
                    return Err(dead_or(&mut r.child, e));
                }
            };
            for event in parse_stream_line(&line) {
                match event {
                    Event::Delta(piece) => {
                        produced += 1;
                        text.push_str(&piece);
                        delta(&piece);
                        if text.len() > MAX_TEXT {
                            return Ok(ChatResult {
                                text,
                                prompt_tokens: tokens,
                                completion_tokens: produced,
                                finish: WireFinish::Length,
                            });
                        }
                    }
                    Event::Done {
                        finish,
                        prompt_tokens,
                        completion_tokens,
                    } => {
                        return Ok(ChatResult {
                            text,
                            prompt_tokens: prompt_tokens.unwrap_or(tokens),
                            completion_tokens: completion_tokens.unwrap_or(produced),
                            finish,
                        })
                    }
                    Event::Error(e) => return Err(e),
                }
            }
        }
    }

    fn unload(&mut self) {
        // Dropping the child kills and reaps it.
        self.running = None;
    }
}

/// Whether the server has exited, allowing it a moment: a crashing process closes its sockets before
/// the OS reports it as exited.
fn has_died(child: &mut ServerChild) -> bool {
    for _ in 0..40 {
        if !child.alive() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

/// A failed request: when the server has died, say so (it is restarted on the next request).
fn dead_or(child: &mut ServerChild, e: HttpError) -> RpcError {
    if !has_died(child) {
        engine(e.to_string())
    } else {
        engine(format!("{}; it is restarted on the next request", child.describe_exit(None)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_explicit_and_quiet() {
        let a = server_args(Path::new("/m/x.gguf"), 4096, u32::MAX, true, 6);
        let s = a.join(" ");
        assert!(s.starts_with("-m /m/x.gguf -c 4096 -ngl all --threads 6"));
        for flag in [
            "--no-webui",
            "--no-slots",
            "--offline",
            "--fit off",
            "--log-verbosity 1",
            "--cpu-moe",
            "--parallel 1",
        ] {
            assert!(s.contains(flag), "{flag}");
        }
        assert!(!s.contains("api-key"), "the key never appears on the command line");
        assert!(server_args(Path::new("m"), 1, 8, false, 1).join(" ").contains("-ngl 8"));
    }

    #[test]
    fn probes_decide_usability() {
        let gpu = Device {
            name: "Vulkan0".into(),
            description: "GPU".into(),
            total_mib: 8000,
            free_mib: 7000,
        };
        let ok = Probe {
            devices: vec![gpu],
            error: None,
        };
        assert!(ok.usable("vulkan").is_ok());
        let none = Probe {
            devices: vec![],
            error: None,
        };
        assert!(none.usable("vulkan").unwrap_err().contains("no supported GPU"));
        assert!(none.usable("cpu").is_ok());
        let broken = Probe {
            devices: vec![],
            error: Some("missing cudart".into()),
        };
        assert_eq!(broken.usable("cpu").unwrap_err(), "missing cudart");
        let missing = Probe::run(Path::new("/no/such/llama-server"), Duration::from_secs(1));
        assert!(missing.error.is_some());
        assert!(rss_with(None) > 0);
        assert!(rss_with(Some(std::process::id())) > current_rss_bytes() / 2);
    }
}
