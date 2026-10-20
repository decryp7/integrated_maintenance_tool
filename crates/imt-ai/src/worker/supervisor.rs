//! Keeps the inference worker healthy. Every call has a timeout; a crash, hang or malformed answer kills
//! the worker, fails the call with a clear message and restarts the process with backoff; after repeated
//! failures the AI is switched off for the session (quarantine, like plugins) with a reason the UI can show.

use super::protocol::*;
use super::transport::{CallOpts, TransportError, WorkerLauncher, WorkerTransport};
use crate::error::{AiError, Result};
use crate::gguf::GgufInfo;
use crate::hardware::Hardware;
use crate::llm::prompted::{Finish, GenParams, Generated, PromptedProvider, TextEngine};
use crate::llm::PromptFormat;
use crate::runtime::{check_memory, GpuMode, LoadRequest, Loaded, ModelRuntime};
use imt_ipc::supervisor::CrashLedger;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

#[derive(Clone)]
pub struct WorkerConfig {
    pub launcher: Arc<dyn WorkerLauncher>,
    pub hello_timeout: Duration,
    /// Loading a large model from a slow disk takes minutes.
    pub load_timeout: Duration,
    pub control_timeout: Duration,
    /// Longest silence while a chat runs (reading a long prompt on a CPU is slow, so this is generous).
    pub stall_timeout: Duration,
    pub cancel_grace: Duration,
    pub max_crashes: usize,
    pub crash_window: Duration,
    pub backoff_base: Duration,
}

impl WorkerConfig {
    pub fn new(launcher: Arc<dyn WorkerLauncher>) -> Self {
        WorkerConfig {
            launcher,
            hello_timeout: Duration::from_secs(15),
            load_timeout: Duration::from_secs(600),
            control_timeout: Duration::from_secs(15),
            stall_timeout: Duration::from_secs(180),
            cancel_grace: Duration::from_secs(5),
            max_crashes: 3,
            crash_window: Duration::from_secs(600),
            backoff_base: Duration::from_millis(500),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineState {
    /// No worker process (not started yet, or unloaded).
    Stopped,
    Running,
    /// Switched off for this session after repeated failures.
    Disabled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub state: EngineState,
    pub pid: Option<u32>,
    pub rss_bytes: u64,
    pub backend: String,
    pub model: String,
    pub crashes_in_window: usize,
    pub restarts: usize,
    /// Why the AI is disabled, or the last failure.
    pub reason: Option<String>,
}

/// Crash history shared by every worker this runtime starts, so quarantine survives a model switch. The
/// bookkeeping is the shared `imt_ipc::supervisor::CrashLedger`.
pub struct Health {
    ledger: CrashLedger,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Health {
    pub fn new(max: usize, window: Duration) -> Self {
        Health {
            ledger: CrashLedger::new(max, window),
        }
    }

    fn recent(&self) -> usize {
        self.ledger.recent()
    }

    /// Records a failure; returns whether the AI is now disabled.
    pub fn record_failure(&self, reason: &str) -> bool {
        let minutes = self.ledger.window().as_secs() / 60;
        let disabled = self.ledger.record_failure(reason, |n, reason| {
            format!("The AI engine failed {n} times in {minutes} minutes (last: {reason}). The AI assistant is off for this session.")
        });
        if disabled {
            // In the worker's child log too, for problem reports.
            imt_ipc::child_log::get(&super::frames::worker_log()).quarantined(&self.disabled_reason().unwrap_or_default());
        }
        disabled
    }

    pub fn check(&self) -> Result<()> {
        match self.ledger.disabled_reason() {
            Some(r) => Err(AiError::Engine(r)),
            None => Ok(()),
        }
    }

    pub fn disabled_reason(&self) -> Option<String> {
        self.ledger.disabled_reason()
    }

    /// Lets the user try again (like resetting a quarantined plugin).
    pub fn reset(&self) {
        self.ledger.reset();
    }
}

struct Inner {
    transport: Option<Box<dyn WorkerTransport>>,
    spec: Option<LoadParams>,
    loaded: Option<LoadResult>,
}

pub struct Supervisor {
    cfg: WorkerConfig,
    health: Arc<Health>,
    inner: Mutex<Inner>,
}

fn opts(timeout: Duration, grace: Duration) -> CallOpts {
    CallOpts {
        timeout,
        cancel_grace: grace,
    }
}

impl Supervisor {
    pub fn new(cfg: WorkerConfig, health: Arc<Health>) -> Self {
        Supervisor {
            cfg,
            health,
            inner: Mutex::new(Inner {
                transport: None,
                spec: None,
                loaded: None,
            }),
        }
    }

    pub fn health(&self) -> &Arc<Health> {
        &self.health
    }

    fn typed<T: serde::de::DeserializeOwned>(v: Value) -> std::result::Result<T, TransportError> {
        serde_json::from_value(v).map_err(|e| TransportError::Protocol(format!("unexpected answer: {e}")))
    }

    /// Starts a worker and checks it speaks our protocol. Waits out the restart backoff first.
    fn start(&self, inner: &mut Inner) -> Result<()> {
        self.health.check()?;
        self.health.ledger.wait_backoff(self.cfg.backoff_base, Duration::from_secs(8));
        let mut t = self.cfg.launcher.launch().map_err(|e| self.fail(inner, e))?;
        let hello = json!(HelloParams {
            protocol_version: PROTOCOL_VERSION,
            client: format!("imt-ai {}", env!("CARGO_PKG_VERSION"))
        });
        let r = t.call(
            method::HELLO,
            hello,
            opts(self.cfg.hello_timeout, self.cfg.cancel_grace),
            None,
            &mut |_| {},
        );
        match r.and_then(Self::typed::<HelloResult>) {
            Ok(h) if h.protocol_version == PROTOCOL_VERSION => {
                inner.transport = Some(t);
                Ok(())
            }
            Ok(h) => {
                t.kill();
                Err(AiError::Engine(format!(
                    "the AI engine speaks protocol {} but IMT needs {PROTOCOL_VERSION}; reinstall IMT",
                    h.protocol_version
                )))
            }
            Err(e) => {
                t.kill();
                Err(self.fail(inner, e))
            }
        }
    }

    /// A worker failure: kill it, count it, and describe what happened.
    fn fail(&self, inner: &mut Inner, e: TransportError) -> AiError {
        if let Some(mut t) = inner.transport.take() {
            t.kill();
        }
        inner.loaded = None;
        let what = match &e {
            TransportError::Spawn(m) => m.clone(),
            TransportError::Died(m) => format!("it crashed ({m})"),
            TransportError::Timeout => "it stopped responding".into(),
            TransportError::Protocol(m) => format!("it sent an invalid response ({m})"),
            TransportError::Remote(r) => r.message.clone(),
            TransportError::Denied(m) => m.clone(),
        };
        let disabled = self.health.record_failure(&what);
        self.health.ledger.count_restart();
        if disabled {
            return AiError::Engine(self.health.disabled_reason().unwrap_or_default());
        }
        match e {
            TransportError::Spawn(m) => AiError::Engine(m),
            TransportError::Died(_) => AiError::Engine("The AI engine crashed and was restarted. Ask again to continue.".into()),
            TransportError::Timeout => AiError::Engine("The AI engine stopped responding and was restarted. Ask again to continue.".into()),
            TransportError::Protocol(_) => {
                AiError::Engine("The AI engine sent an invalid response and was restarted. Ask again to continue.".into())
            }
            TransportError::Remote(r) => AiError::Engine(r.message),
            TransportError::Denied(m) => AiError::Engine(m),
        }
    }

    fn map_remote(r: RpcError) -> AiError {
        match r.code {
            code::CONTEXT_OVERFLOW => {
                let get = |k: &str| r.data.as_ref().and_then(|d| d.get(k)).and_then(Value::as_u64).unwrap_or(0) as usize;
                AiError::ContextOverflow {
                    prompt: get("prompt"),
                    limit: get("limit"),
                }
            }
            code::LOAD_FAILED if reason_of(&r) == Some(NO_RUNTIME_REASON) => AiError::NoRuntime,
            code::LOAD_FAILED => AiError::Engine(format!("cannot load the model: {}", r.message)),
            _ => AiError::Engine(r.message),
        }
    }

    /// Runs one call on the live worker, starting it (and reloading the model after a restart) when needed.
    fn call(
        &self,
        method_name: &str,
        params: Value,
        o: CallOpts,
        cancel: Option<&AtomicBool>,
        on_stream: &mut dyn FnMut(Value),
    ) -> Result<Value> {
        let mut inner = lock(&self.inner);
        self.ensure_ready(&mut inner)?;
        let t = inner
            .transport
            .as_mut()
            .ok_or_else(|| AiError::Engine("the AI engine is not running".into()))?;
        match t.call(method_name, params, o, cancel, on_stream) {
            Ok(v) => Ok(v),
            Err(TransportError::Remote(r)) => Err(Self::map_remote(r)),
            Err(e) => Err(self.fail(&mut inner, e)),
        }
    }

    fn ensure_ready(&self, inner: &mut Inner) -> Result<()> {
        if inner.transport.is_none() {
            self.start(inner)?;
        }
        if inner.loaded.is_none() {
            if let Some(spec) = inner.spec.clone() {
                // After a restart the model is loaded again on first use.
                let t = inner
                    .transport
                    .as_mut()
                    .ok_or_else(|| AiError::Engine("the AI engine is not running".into()))?;
                match t
                    .call(
                        method::LOAD,
                        json!(spec),
                        opts(self.cfg.load_timeout, self.cfg.cancel_grace),
                        None,
                        &mut |_| {},
                    )
                    .and_then(Self::typed::<LoadResult>)
                {
                    Ok(l) => inner.loaded = Some(l),
                    Err(TransportError::Remote(r)) => return Err(Self::map_remote(r)),
                    Err(e) => return Err(self.fail(inner, e)),
                }
            }
        }
        Ok(())
    }

    /// Loads a model (starting the worker if needed). Remembered, so a restarted worker reloads it.
    pub fn load(&self, params: LoadParams) -> Result<LoadResult> {
        let mut inner = lock(&self.inner);
        inner.loaded = None;
        if inner.transport.is_none() {
            self.start(&mut inner)?;
        }
        let t = inner
            .transport
            .as_mut()
            .ok_or_else(|| AiError::Engine("the AI engine is not running".into()))?;
        match t
            .call(
                method::LOAD,
                json!(params),
                opts(self.cfg.load_timeout, self.cfg.cancel_grace),
                None,
                &mut |_| {},
            )
            .and_then(Self::typed::<LoadResult>)
        {
            Ok(l) => {
                inner.spec = Some(params);
                inner.loaded = Some(l.clone());
                Ok(l)
            }
            Err(TransportError::Remote(r)) => Err(Self::map_remote(r)),
            Err(e) => Err(self.fail(&mut inner, e)),
        }
    }

    pub fn chat(&self, p: &ChatParams, cancel: Option<&AtomicBool>, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResult> {
        let mut stream = |v: Value| {
            if let Some(t) = v.get("text").and_then(Value::as_str) {
                on_delta(t);
            }
        };
        let v = self.call(
            method::CHAT,
            json!(p),
            opts(self.cfg.stall_timeout, self.cfg.cancel_grace),
            cancel,
            &mut stream,
        )?;
        serde_json::from_value(v).map_err(|e| AiError::Malformed(format!("the engine's answer is unreadable: {e}")))
    }

    pub fn status(&self) -> EngineStatus {
        let mut inner = lock(&self.inner);
        let reason = self.health.disabled_reason().or_else(|| self.health.ledger.last_failure());
        let mut s = EngineStatus {
            state: if self.health.disabled_reason().is_some() {
                EngineState::Disabled
            } else if inner.transport.is_some() {
                EngineState::Running
            } else {
                EngineState::Stopped
            },
            pid: inner.transport.as_ref().and_then(|t| t.pid()),
            rss_bytes: 0,
            backend: inner.loaded.as_ref().map(|l| l.backend.clone()).unwrap_or_default(),
            model: inner.loaded.as_ref().map(|l| l.name.clone()).unwrap_or_default(),
            crashes_in_window: self.health.recent(),
            restarts: self.health.ledger.restarts(),
            reason,
        };
        if let Some(t) = inner.transport.as_mut() {
            if let Ok(v) = t.call(
                method::STATUS,
                json!({}),
                opts(self.cfg.control_timeout, self.cfg.cancel_grace),
                None,
                &mut |_| {},
            ) {
                if let Ok(st) = serde_json::from_value::<StatusResult>(v) {
                    s.rss_bytes = st.rss_bytes;
                }
            }
        }
        s
    }

    /// Stops the worker (politely, then by force). The remembered model is kept for the next use.
    pub fn shutdown(&self) {
        let mut inner = lock(&self.inner);
        if let Some(mut t) = inner.transport.take() {
            let _ = t.call(
                method::SHUTDOWN,
                json!({}),
                opts(Duration::from_secs(2), Duration::from_secs(1)),
                None,
                &mut |_| {},
            );
            t.kill();
        }
        inner.loaded = None;
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// A raw text engine backed by the worker.
pub struct WorkerEngine {
    sup: Arc<Supervisor>,
    name: String,
    context: usize,
}

impl WorkerEngine {
    pub fn supervisor(&self) -> &Arc<Supervisor> {
        &self.sup
    }
}

impl TextEngine for WorkerEngine {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn context_length(&self) -> usize {
        self.context
    }

    fn generate(&self, prompt: &str, p: &GenParams, on_text: &mut dyn FnMut(&str)) -> Result<Generated> {
        let params = ChatParams {
            prompt: prompt.to_string(),
            max_tokens: p.max_tokens,
            temperature: p.temperature,
            json_schema: p.json_schema.clone(),
        };
        let r = self.sup.chat(&params, p.cancel.as_deref(), on_text)?;
        Ok(Generated {
            text: r.text,
            prompt_tokens: r.prompt_tokens,
            completion_tokens: r.completion_tokens,
            finish: match r.finish {
                WireFinish::Stop => Finish::Stop,
                WireFinish::Length => Finish::Length,
                WireFinish::Cancelled => Finish::Cancelled,
            },
        })
    }
}

/// `ModelRuntime` over the worker process: this is what the app uses.
pub struct WorkerRuntime {
    cfg: WorkerConfig,
    health: Arc<Health>,
    hardware: Box<dyn Fn() -> Hardware + Send + Sync>,
}

impl WorkerRuntime {
    pub fn new(cfg: WorkerConfig, hardware: Box<dyn Fn() -> Hardware + Send + Sync>) -> Self {
        let health = Arc::new(Health::new(cfg.max_crashes, cfg.crash_window));
        WorkerRuntime { cfg, health, hardware }
    }

    pub fn health(&self) -> &Arc<Health> {
        &self.health
    }

    /// Why the AI is off for this session, if it is.
    pub fn disabled_reason(&self) -> Option<String> {
        self.health.disabled_reason()
    }

    pub fn reset(&self) {
        self.health.reset();
    }
}

fn gpu_layers(g: GpuMode) -> i32 {
    match g {
        GpuMode::Auto => -1,
        GpuMode::Cpu => 0,
        GpuMode::Layers(n) => n.min(i32::MAX as u32) as i32,
    }
}

pub fn prompt_format_of(info: &GgufInfo) -> PromptFormat {
    PromptFormat::detect(&info.architecture, info.chat_template.as_deref())
}

impl ModelRuntime for WorkerRuntime {
    fn load(&self, req: &LoadRequest) -> Result<Loaded> {
        self.health.check()?;
        // Refuse a model that cannot fit the memory that is free right now.
        check_memory(&(self.hardware)(), &req.info)?;
        let sup = Arc::new(Supervisor::new(self.cfg.clone(), self.health.clone()));
        let l = sup.load(LoadParams {
            model_path: req.path.to_string_lossy().into_owned(),
            gpu_layers: gpu_layers(req.gpu),
            context: req.context,
            threads: req.threads,
        })?;
        let engine = WorkerEngine {
            sup,
            name: l.name.clone(),
            context: l.context,
        };
        Ok(Loaded {
            provider: Arc::new(PromptedProvider::new(engine, prompt_format_of(&req.info))),
            backend: l.backend,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_engine_is_recorded_in_the_worker_log() {
        let h = Health::new(2, Duration::from_secs(60));
        assert!(!h.record_failure("it crashed (exit code 3)"));
        assert!(h.record_failure("it crashed (exit code 3)"));
        let s = imt_ipc::child_log::find(super::super::frames::WORKER_LOG).unwrap().status();
        assert!(s.quarantined, "{s:?}");
        assert!(s.quarantine_reason.unwrap().contains("failed 2 times"));
        assert_eq!(s.kind, imt_ipc::child_log::ChildKind::AiWorker);
        let spec = super::super::frames::worker_log();
        assert_eq!(spec.forwarded[0].name, super::super::frames::LLAMA_LOG);
    }

    #[test]
    fn typed_worker_errors_map_to_library_errors() {
        let nr = RpcError {
            code: code::LOAD_FAILED,
            message: "no llama-server".into(),
            data: Some(json!({"reason": NO_RUNTIME_REASON}).into()),
        };
        assert_eq!(Supervisor::map_remote(nr), AiError::NoRuntime);
        let other = RpcError {
            code: code::LOAD_FAILED,
            message: "bad file".into(),
            data: Some(json!({"reason": "other"}).into()),
        };
        assert_eq!(
            Supervisor::map_remote(other),
            AiError::Engine("cannot load the model: bad file".into())
        );
        assert_eq!(
            Supervisor::map_remote(context_overflow(9, 4)),
            AiError::ContextOverflow { prompt: 9, limit: 4 }
        );
        assert_eq!(
            Supervisor::map_remote(RpcError::new(code::ENGINE, "x")),
            AiError::Engine("x".into())
        );
    }
}
