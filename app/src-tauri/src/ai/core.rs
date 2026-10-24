//! The app-wide AI runtime: settings, the model catalog and folder, set-up, and the one active model shared
//! by every window (one worker process, loaded lazily and unloaded when idle). Plain Rust, no Tauri, so it is
//! testable with the fakes of `debrief-ai` (scripted provider, fake runtime).

use super::error::{AiCmdError, AiErrorCode, AiResult};
use super::runtimes::{RuntimeDeps, RuntimeJobs};
use debrief_ai::assistant::{Assistant, AssistantConfig};
use debrief_ai::catalog::Catalog;
use debrief_ai::engine::pitwall::{Pitwall, PitwallView, Priority};
use debrief_ai::engine::{select, AiEngine, BuiltInEngine, EngineChoice, EngineKind, PitwallEngine, SelectInput};
use debrief_ai::fit::{self, assess, Assessment, Live, Subject};
use debrief_ai::gguf::{self, GgufInfo};
use debrief_ai::hardware::{Hardware, HardwareProbe, SystemProbe};
use debrief_ai::knowledge::{KnowledgeFile, KnowledgeProvider};
use debrief_ai::llm::{LazyProvider, LlmProvider, LoadState, ModelLoader};
use debrief_ai::models::source::{ModelSource, Progress};
use debrief_ai::models::store::InstalledProjector;
use debrief_ai::models::verify::verify_local;
use debrief_ai::models::{AddMode, DiskSpace, HfConfig, HuggingFace, ModelStore, SystemDisk};
use debrief_ai::recommend::{recommend_with, Recommendation, Recommendations};
use debrief_ai::runtime::{check_memory, effective_context, load_with_fallback, projector_bytes, LoadRequest, ModelRuntime};
use debrief_ai::settings::AiSettings;
use debrief_ai::setup::{Setup, SetupPlan, SetupStage};
use debrief_ai::worker::protocol::ServerInfo;
use debrief_ai::worker::{FramesLauncher, Health, WorkerConfig, WorkerRuntime};
use debrief_ai::AiError;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

const SETTINGS_FILE: &str = "ai.json";
const CATALOG_FILE: &str = "local-models.json";
const MAX_TOOL_ROUNDS: usize = 6;
const MAX_PATH_CHARS: usize = 4096;

/// Builds the download source from the current settings (endpoint, proxy).
pub type SourceFactory = Arc<dyn Fn(&AiSettings) -> debrief_ai::Result<Box<dyn ModelSource>> + Send + Sync>;

/// The adapters the runtime works with. [`AiDeps::system`] is the real machine; tests pass fakes.
pub struct AiDeps {
    pub runtime: Arc<dyn ModelRuntime>,
    /// Crash history of the worker (quarantine), when the runtime has one.
    pub health: Option<Arc<Health>>,
    pub probe: Arc<dyn HardwareProbe + Send + Sync>,
    pub source: SourceFactory,
    pub disk: Arc<dyn DiskSpace>,
    /// The worker binary; when it is missing the AI is unavailable. `None` skips the check (tests).
    pub worker_path: Option<PathBuf>,
    pub download_backoff: Duration,
    /// The inference runtimes (llama-server builds) and how to download and check them.
    pub runtimes: RuntimeDeps,
    /// The current settings, shared with the worker launcher (its environment names the chosen build and is
    /// evaluated at every launch). [`AiCore`] keeps it up to date.
    pub settings: Arc<RwLock<AiSettings>>,
    /// Pitwall, when Debrief may look for it (ADR 0059). `None`: only the built-in engine (tests that do not set one).
    pub pitwall: Option<Arc<Pitwall>>,
}

impl AiDeps {
    /// The real adapters: the worker at `worker` (running the llama-server builds kept in
    /// `<data_dir>/ai/runtimes`), the system's hardware, Hugging Face and GitHub releases, the keychain.
    pub fn system(worker: PathBuf, data_dir: &Path) -> AiDeps {
        AiDeps::system_capped(worker, data_dir, Arc::new(|| None), None)
    }

    /// [`AiDeps::system`] with a memory cap on the worker's own process, read at every launch, and a listener for
    /// its memory watcher (Settings ▸ Performance, ADR 0031).
    pub fn system_capped(
        worker: PathBuf,
        data_dir: &Path,
        memory_cap: debrief_ai::worker::WorkerMemoryCap,
        listener: Option<debrief_ipc::memory_watch::MemoryListener>,
    ) -> AiDeps {
        let probe = Arc::new(SystemProbe::new(debrief_collect::SystemRunner));
        let p = probe.clone();
        let settings = Arc::new(RwLock::new(AiSettings::default()));
        let runtimes = RuntimeDeps::system(data_dir.join("ai").join("runtimes"), worker.clone());
        let env = runtimes.worker_env_source(settings.clone());
        let cfg = WorkerConfig::new(Arc::new(debrief_ai::worker::CappedLauncher {
            launcher: FramesLauncher {
                exe: worker.clone(),
                args: vec![],
                env,
            },
            memory_cap,
            listener,
        }));
        let runtime = Arc::new(WorkerRuntime::new(cfg, Box::new(move || p.probe())));
        AiDeps {
            health: Some(runtime.health().clone()),
            runtime,
            probe,
            source: Arc::new(|s: &AiSettings| {
                let mut cfg = HfConfig::default();
                cfg.endpoint = s.hf_endpoint.clone();
                cfg.proxy = s.download_proxy.clone();
                Ok(Box::new(HuggingFace::new(cfg)?) as Box<dyn ModelSource>)
            }),
            disk: Arc::new(SystemDisk),
            worker_path: Some(worker),
            download_backoff: Duration::from_secs(2),
            runtimes,
            settings,
            pitwall: None,
        }
    }
}

/// What the engine is doing, for the status bar and to hide or grey AI features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineStateDto {
    /// AI is off in this run (safe mode, no worker binary).
    Unavailable,
    /// Switched off for the session after repeated crashes; `ai_reset` tries again.
    Disabled,
    NoModel,
    /// A model is chosen but not in memory (loads on first use).
    Idle,
    Loading,
    Ready,
    /// The last load failed; the next use tries again.
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub available: bool,
    pub state: EngineStateDto,
    pub reason: Option<String>,
    /// The active model's file name.
    pub model: Option<String>,
    /// "Metal", "Vulkan", "CPU", ... once loaded.
    pub backend: Option<String>,
    /// Resident memory of the worker process (0 when not running or unknown).
    pub rss_bytes: u64,
    /// Things the user should know (GPU fallback, settings or catalog problems).
    pub notes: Vec<String>,
    pub safe_mode: bool,
    pub setup_running: bool,
    /// The active model reads images ("Text + images"): its projector is installed (ADR 0025).
    pub vision: bool,
    /// What the running inference server reported about itself, once a model is loaded.
    pub server: Option<AiServerDto>,
    /// Which engine answers and what Debrief knows about Pitwall (ADR 0059).
    pub engine: AiEngineStatus,
}

/// The engine choice and state (wire contract: `AiEngineStatus` in `ai.types.ts`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiEngineStatus {
    pub choice: EngineChoice,
    /// The engine requests go to now; `None`: the wanted one (Pitwall) is not usable.
    pub active: Option<EngineKind>,
    /// The user switched this session to the built-in engine after Pitwall went away.
    pub built_in_for_session: bool,
    pub pitwall: PitwallView,
}

/// An installed model with what it means for this machine.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDto {
    pub file_name: String,
    pub size_bytes: u64,
    pub info: Option<GgufInfo>,
    pub problem: Option<String>,
    pub active: bool,
    pub assessment: Option<Assessment>,
    /// The catalog entry for this file, if it is one.
    pub catalog_id: Option<String>,
    pub name: String,
    pub license: Option<String>,
    pub tool_calling: bool,
    /// Reads images: its projector is installed and fits it.
    pub vision: bool,
    /// The installed projector (also one that cannot be used, with its problem).
    pub projector: Option<InstalledProjector>,
    /// Bytes of the catalog projector that would add image support, when it is not installed.
    pub projector_download_bytes: Option<u64>,
}

/// Progress of a set-up or an activation, streamed as `debrief://ai-setup`.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SetupEvent {
    Stage(SetupStage),
    End(SetupEnd),
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SetupEnd {
    Failed { code: AiErrorCode, message: String },
    Cancelled,
}

impl SetupEvent {
    pub fn from_result(r: &AiResult<()>) -> Option<SetupEvent> {
        match r {
            Ok(()) => None,
            Err(e) if e.code == AiErrorCode::Cancelled => Some(SetupEvent::End(SetupEnd::Cancelled)),
            Err(e) => Some(SetupEvent::End(SetupEnd::Failed {
                code: e.code,
                message: e.message.clone(),
            })),
        }
    }
}

#[derive(Default)]
pub(super) struct EngineInfo {
    backend: Option<String>,
    notes: Vec<String>,
    server: Option<ServerInfo>,
}

/// What the running inference server reported about itself (`/props`, ADR 0009), for the AI status and problem
/// reports. Fields the server did not report are `null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiServerDto {
    pub context: Option<usize>,
    pub vision: Option<bool>,
    pub audio: Option<bool>,
    /// `null`: the server named none, so llama.cpp's default `<__media__>` is used.
    pub media_marker: Option<String>,
    pub model: Option<String>,
    pub build: Option<String>,
    pub chat_template: bool,
}

impl From<&ServerInfo> for AiServerDto {
    fn from(s: &ServerInfo) -> Self {
        AiServerDto {
            context: s.n_ctx,
            vision: s.vision,
            audio: s.audio,
            media_marker: s.media_marker.clone(),
            model: s.model.clone(),
            build: s.build.clone(),
            chat_template: s.chat_template,
        }
    }
}

enum Active {
    None,
    Lazy {
        lazy: Arc<LazyProvider>,
        file: String,
    },
    /// A provider given directly (tests).
    #[cfg_attr(not(test), allow(dead_code))]
    Fixed(Arc<dyn LlmProvider>),
}

/// A loaded provider with its backend name, notes (GPU fallback) and server report, from the set-up.
type Primed = (Arc<dyn LlmProvider>, EngineInfo);

/// Loads the active model on demand, remembering the backend and any GPU fallback for the status.
struct AppLoader {
    runtime: Arc<dyn ModelRuntime>,
    store: ModelStore,
    file: String,
    settings: AiSettings,
    probe: Arc<dyn HardwareProbe + Send + Sync>,
    info: Arc<Mutex<EngineInfo>>,
    /// A model the set-up already loaded: handed over once instead of loading it again.
    primed: Mutex<Option<Primed>>,
}

use crate::sync::lock;

/// How model `file` of `store` loads with `settings`: its path, GGUF facts, image projector (loaded into the same
/// engine, one model, ADR 0025), context and GPU mode.
pub(super) fn load_request(store: &ModelStore, file: &str, settings: &AiSettings) -> debrief_ai::Result<LoadRequest> {
    let path = store.path_of(file)?;
    let info = gguf::inspect(&path)?;
    let projector = store.projector_path(file)?;
    Ok(LoadRequest {
        context: effective_context(settings, &info),
        path,
        projector,
        info,
        gpu: settings.gpu_offload.into(),
        threads: settings.threads,
    })
}

impl ModelLoader for AppLoader {
    fn load(&self) -> debrief_ai::Result<Arc<dyn LlmProvider>> {
        if let Some((p, info)) = lock(&self.primed).take() {
            *lock(&self.info) = info;
            return Ok(p);
        }
        let req = load_request(&self.store, &self.file, &self.settings)?;
        check_memory(
            &self.probe.probe(),
            &req.info,
            projector_bytes(req.projector.as_deref()),
            req.context,
        )?;
        let (loaded, note) = load_with_fallback(self.runtime.as_ref(), req)?;
        *lock(&self.info) = EngineInfo {
            backend: Some(loaded.backend),
            notes: note.into_iter().collect(),
            server: loaded.server,
        };
        Ok(loaded.provider)
    }

    fn context_hint(&self) -> usize {
        self.settings.context_length
    }

    fn vision_hint(&self) -> bool {
        self.store.projector_path(&self.file).is_ok_and(|p| p.is_some())
    }
}

fn idle_duration(settings: &AiSettings) -> Duration {
    match settings.idle_unload_minutes {
        0 => Duration::MAX,
        m => Duration::from_secs(u64::from(m) * 60),
    }
}

/// The app-wide AI state. Managed once; every window shares the active model.
pub struct AiCore {
    dir: Option<PathBuf>,
    default_models_dir: PathBuf,
    safe_mode: bool,
    pub(super) deps: AiDeps,
    /// Shared with the worker launcher (see [`AiDeps::settings`]).
    settings: Arc<RwLock<AiSettings>>,
    warnings: Vec<String>,
    catalog: Catalog,
    active: Mutex<Active>,
    info: Arc<Mutex<EngineInfo>>,
    setup: Mutex<Option<Arc<Progress>>>,
    /// Runtime installs and the cached build report.
    pub(super) jobs: RuntimeJobs,
    /// The global knowledge files (`<dir>/knowledge`, ADR 0029).
    knowledge: super::knowledge::GlobalKnowledge,
    /// Header-derived KV shapes of catalog models (see `Setup::refine`).
    shapes: debrief_ai::fit::ShapeCache,
    /// The repair "Use the fallback": the built-in engine for the rest of the session (ADR 0059).
    built_in_for_session: AtomicBool,
}

impl AiCore {
    /// `dir` holds `ai.json` and an optional newer `local-models.json`; models go to `models_dir` unless the
    /// settings name another folder.
    pub fn new(dir: Option<PathBuf>, models_dir: PathBuf, safe_mode: bool, deps: AiDeps) -> AiCore {
        let (settings, settings_warning) = match &dir {
            Some(d) => AiSettings::load(&d.join(SETTINGS_FILE)),
            None => (AiSettings::default(), None),
        };
        let (catalog, mut warnings) = match &dir {
            Some(d) => Catalog::load(&d.join(CATALOG_FILE)),
            None => (Catalog::embedded(), vec![]),
        };
        warnings.extend(settings_warning);
        *crate::sync::write(&deps.settings) = settings;
        for w in &warnings {
            log::warn!("AI: {w}");
        }
        let knowledge = super::knowledge::GlobalKnowledge::open(dir.as_ref().map(|d| d.join("knowledge")));
        let core = AiCore {
            knowledge,
            dir,
            default_models_dir: models_dir,
            safe_mode,
            settings: deps.settings.clone(),
            deps,
            warnings,
            catalog,
            active: Mutex::new(Active::None),
            info: Arc::default(),
            setup: Mutex::default(),
            jobs: RuntimeJobs::default(),
            shapes: debrief_ai::fit::ShapeCache::default(),
            built_in_for_session: AtomicBool::new(false),
        };
        core.install_active(None);
        core.pitwall_needs();
        core
    }

    /// Loads the model in the background after startup when the settings ask for it.
    pub fn autoload(&self) {
        // While Pitwall answers, the built-in model stays out of memory (one model, ADR 0059).
        if self.availability().is_err() || !self.settings().autoload || self.engine_kind() != Some(EngineKind::BuiltIn) {
            return;
        }
        if let Active::Lazy { lazy, .. } = &*lock(&self.active) {
            lazy.preload();
        }
    }

    /// Whether background work (the name aliases) may use the model without loading it by itself: it is in memory, or
    /// it is loaded automatically anyway.
    pub fn background_ready(&self) -> bool {
        match self.status().state {
            EngineStateDto::Ready => true,
            EngineStateDto::Idle => self.settings().autoload,
            _ => false,
        }
    }

    /// Ok when AI can be used in this run; otherwise why not.
    pub fn availability(&self) -> AiResult<()> {
        if self.safe_mode {
            return Err(AiCmdError::unavailable(
                "Debrief is running in safe mode, so the AI assistant is off. Restart normally to use it.",
            ));
        }
        if let Some(p) = &self.deps.worker_path {
            if !p.is_file() {
                return Err(AiCmdError::unavailable(format!(
                    "The AI engine is not installed (missing {}). Reinstall Debrief, or use a build that includes the AI assistant.",
                    p.display()
                )));
            }
        }
        Ok(())
    }

    pub fn settings(&self) -> AiSettings {
        crate::sync::read(&self.settings).clone()
    }

    /// The folder of `ai.json` (none in tests without one).
    pub(super) fn config_dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    fn settings_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(SETTINGS_FILE))
    }

    pub fn models_dir(&self) -> PathBuf {
        self.settings()
            .models_dir
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| self.default_models_dir.clone())
    }

    pub(super) fn store(&self) -> ModelStore {
        ModelStore::new(self.models_dir())
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn hardware(&self) -> Hardware {
        self.deps.probe.probe()
    }

    pub(super) fn installed_files(&self) -> Vec<String> {
        self.store().list().into_iter().map(|m| m.file_name).collect()
    }

    /// Memory (GB) that unloading Debrief's own loaded model would give back: its file plus the context.
    pub(super) fn reclaim_gb(&self) -> f64 {
        let file = match &*lock(&self.active) {
            Active::Lazy { lazy, file } if lazy.state() == LoadState::Ready => file.clone(),
            _ => return 0.0,
        };
        let settings = self.settings();
        self.store()
            .list()
            .into_iter()
            .find(|m| m.file_name == file)
            .and_then(|m| m.info)
            .map_or(0.0, |i| {
                let ctx = effective_context(&settings, &i);
                fit::need(&fit::Subject::from_gguf(&i, 0, ctx)).total_gb
            })
    }

    fn live(&self) -> Live {
        Live {
            reclaim_gb: self.reclaim_gb(),
            ..self.deps.probe.live()
        }
    }

    pub fn recommendations(&self) -> Recommendations {
        recommend_with(
            &self.hardware(),
            &self.live(),
            &self.catalog,
            &self.installed_files(),
            self.settings().context_length,
            &self.shapes.snapshot(),
        )
    }

    /// As [`recommendations`](Self::recommendations), after reading the headers of the models that could fit
    /// (first bytes only, nothing is downloaded) so that the verdicts use their real architecture.
    pub fn refine_fit(&self) -> AiResult<Recommendations> {
        self.availability()?;
        self.with_setup(|s| Ok(s.refine()))
    }

    /// Makes the active model the one named in the settings (or `primed`, already loaded by the set-up).
    pub(super) fn install_active(&self, primed: Option<Primed>) {
        let settings = self.settings();
        let store = self.store();
        let preload = primed.is_some();
        let next = match settings.active_model.clone() {
            Some(file) if primed.is_some() || store.path_of(&file).is_ok_and(|p| p.is_file()) => {
                let loader = AppLoader {
                    runtime: self.deps.runtime.clone(),
                    store,
                    file: file.clone(),
                    probe: self.deps.probe.clone(),
                    info: self.info.clone(),
                    primed: Mutex::new(primed),
                    settings: settings.clone(),
                };
                let lazy = Arc::new(LazyProvider::new(Arc::new(loader), idle_duration(&settings)));
                lazy.spawn_idle_watcher(Duration::from_secs(30));
                Active::Lazy { lazy, file }
            }
            _ => Active::None,
        };
        let old = std::mem::replace(&mut *lock(&self.active), next);
        if let Active::Lazy { lazy, .. } = old {
            lazy.shutdown();
            // Free the worker now (it may hold files a caller is about to delete); a busy one ends with its call.
            lazy.unload();
        }
        *lock(&self.info) = EngineInfo::default();
        if preload {
            if let Active::Lazy { lazy, .. } = &*lock(&self.active) {
                lazy.preload();
            }
        }
    }

    /// Uses `provider` as the model (tests).
    #[cfg(test)]
    pub fn set_fixed_provider(&self, provider: Arc<dyn LlmProvider>) {
        let old = std::mem::replace(&mut *lock(&self.active), Active::Fixed(provider));
        if let Active::Lazy { lazy, .. } = old {
            lazy.shutdown();
        }
    }

    /// Pitwall, when Debrief looks for it.
    pub fn pitwall(&self) -> Option<&Arc<Pitwall>> {
        self.deps.pitwall.as_ref()
    }

    /// Which engine answers now (ADR 0059): the setting, what the last look at Pitwall found and the session's fallback.
    pub fn engine_kind(&self) -> Option<EngineKind> {
        let Some(pw) = self.pitwall() else {
            return Some(EngineKind::BuiltIn);
        };
        select(SelectInput {
            choice: self.settings().engine,
            pitwall_usable: pw.view().usable,
            pitwall_served: pw.served(),
            built_in_for_session: self.built_in_for_session.load(Ordering::Relaxed),
        })
    }

    /// Switches the session to the built-in engine (the repair of a lost Pitwall; never depends on Pitwall).
    pub fn use_built_in_for_session(&self) {
        self.built_in_for_session.store(true, Ordering::Relaxed);
    }

    /// Ends the session's fallback (the user changed the engine setting or connected to Pitwall again).
    pub fn clear_session_fallback(&self) {
        self.built_in_for_session.store(false, Ordering::Relaxed);
    }

    /// Whether Pitwall answers Debrief's requests now: the built-in engine's downloads and updates wait (no second copy).
    pub fn uses_pitwall(&self) -> bool {
        self.engine_kind() == Some(EngineKind::Pitwall)
    }

    /// The engine requests go to (ADR 0059), with `priority` for Pitwall's queue.
    pub fn engine(&self, priority: Priority) -> AiResult<Arc<dyn AiEngine>> {
        self.availability()?;
        match self.engine_kind() {
            Some(EngineKind::Pitwall) => match self.pitwall() {
                Some(pw) => Ok(Arc::new(PitwallEngine {
                    pitwall: pw.clone(),
                    priority,
                })),
                None => Err(AiError::NoModel.into()),
            },
            None => Err(AiCmdError::from(AiError::Engine(self.engine_lost_reason())).in_outage_of(crate::health::hub::AI)),
            Some(EngineKind::BuiltIn) => {
                if let Some(h) = &self.deps.health {
                    h.check().map_err(|e| AiCmdError::unavailable(e.to_string()))?;
                }
                match &*lock(&self.active) {
                    Active::None => Err(AiError::NoModel.into()),
                    Active::Lazy { lazy, file } => Ok(Arc::new(BuiltInEngine {
                        lazy: lazy.clone(),
                        file: file.clone(),
                    })),
                    Active::Fixed(p) => Ok(Arc::new(FixedEngine(p.clone()))),
                }
            }
        }
    }

    /// Why no engine answers (Pitwall chosen or lost, not usable).
    pub fn engine_lost_reason(&self) -> String {
        self.pitwall()
            .and_then(|p| p.view().problem)
            .unwrap_or_else(|| "Pitwall is not ready for Debrief. Open Pitwall, or use the built-in engine (Settings ▸ AI).".into())
    }

    /// The model for background work (Pitwall queues it behind what the user waits for).
    pub fn background_provider(&self) -> AiResult<Arc<dyn LlmProvider>> {
        Ok(self.engine(Priority::Background)?.llm())
    }

    /// The model, for a use case or a chat turn.
    pub fn provider(&self) -> AiResult<Arc<dyn LlmProvider>> {
        if self.engine_kind() != Some(EngineKind::BuiltIn) {
            return Ok(self.engine(Priority::Interactive)?.llm());
        }
        self.availability()?;
        if let Some(h) = &self.deps.health {
            h.check().map_err(|e| AiCmdError::unavailable(e.to_string()))?;
        }
        match &*lock(&self.active) {
            Active::None => Err(AiError::NoModel.into()),
            Active::Lazy { lazy, .. } => Ok(lazy.clone() as Arc<dyn LlmProvider>),
            Active::Fixed(p) => Ok(p.clone()),
        }
    }

    /// The global knowledge files.
    pub fn knowledge(&self) -> &super::knowledge::GlobalKnowledge {
        &self.knowledge
    }

    /// The active model reading the enabled knowledge files: the window's `project` ones, then the global ones
    /// (ADR 0029). Every AI feature's assistant is made on it.
    pub fn provider_with_knowledge(&self, project: &[KnowledgeFile]) -> AiResult<Arc<dyn LlmProvider>> {
        let provider = self.provider()?;
        let global = self.knowledge.snapshot();
        Ok(Arc::new(KnowledgeProvider::new(provider, project, global.files())))
    }

    /// An assistant on the active model with the current settings, the knowledge files (`project`: the window's) and
    /// its own cancel flag.
    pub fn assistant(&self, project: &[KnowledgeFile], cancel: Arc<AtomicBool>) -> AiResult<Assistant<Arc<dyn LlmProvider>>> {
        self.assistant_with_rounds(project, cancel, MAX_TOOL_ROUNDS)
    }

    /// Like [`Self::assistant`] with more tool rounds (agent mode takes a round per browser step).
    pub fn assistant_with_rounds(
        &self,
        project: &[KnowledgeFile],
        cancel: Arc<AtomicBool>,
        rounds: usize,
    ) -> AiResult<Assistant<Arc<dyn LlmProvider>>> {
        let provider = self.provider_with_knowledge(project)?;
        let s = self.settings();
        Ok(Assistant::new(
            provider,
            AssistantConfig {
                max_tokens: s.max_tokens,
                temperature: s.temperature,
                max_tool_rounds: rounds.clamp(1, 120),
                cancel,
            },
        ))
    }

    pub fn status(&self) -> AiStatus {
        let mut notes = self.warnings.clone();
        let info = lock(&self.info);
        notes.extend(info.notes.iter().cloned());
        let mut s = AiStatus {
            available: true,
            state: EngineStateDto::NoModel,
            reason: None,
            model: None,
            backend: info.backend.clone(),
            rss_bytes: 0,
            notes,
            safe_mode: self.safe_mode,
            setup_running: self.setup_running(),
            vision: false,
            server: info.server.as_ref().map(AiServerDto::from),
            engine: self.engine_status(),
        };
        drop(info);
        if let Err(e) = self.availability() {
            s.available = false;
            s.state = EngineStateDto::Unavailable;
            s.reason = Some(e.message);
            return s;
        }
        match s.engine.active {
            Some(EngineKind::Pitwall) => {
                let p = &s.engine.pitwall;
                s.state = EngineStateDto::Ready;
                s.model = p.model.clone();
                s.backend = Some(match &p.version {
                    Some(v) => format!("Pitwall {v}"),
                    None => "Pitwall".into(),
                });
                s.vision = self
                    .pitwall()
                    .and_then(|pw| pw.provider(Priority::Interactive).ok())
                    .is_some_and(|p| p.vision());
                s.server = None;
                return s;
            }
            None => {
                s.state = EngineStateDto::Failed;
                s.reason = Some(self.engine_lost_reason());
                s.server = None;
                return s;
            }
            Some(EngineKind::BuiltIn) => {}
        }
        if let Some(reason) = self.deps.health.as_ref().and_then(|h| h.disabled_reason()) {
            s.state = EngineStateDto::Disabled;
            s.reason = Some(reason);
            return s;
        }
        match &*lock(&self.active) {
            Active::None => {}
            Active::Fixed(p) => {
                s.state = EngineStateDto::Ready;
                s.model = Some(p.display_name());
                s.vision = p.vision();
            }
            Active::Lazy { lazy, file } => {
                s.model = Some(file.clone());
                s.vision = lazy.vision();
                s.state = match lazy.state() {
                    LoadState::Unloaded => EngineStateDto::Idle,
                    LoadState::Loading => EngineStateDto::Loading,
                    LoadState::Ready => EngineStateDto::Ready,
                    LoadState::Failed(m) => {
                        s.reason = Some(m);
                        EngineStateDto::Failed
                    }
                };
            }
        }
        if s.state == EngineStateDto::Ready || s.state == EngineStateDto::Loading {
            s.rss_bytes = worker_rss_bytes();
        }
        s
    }

    /// The engine part of the status.
    pub fn engine_status(&self) -> AiEngineStatus {
        AiEngineStatus {
            choice: self.settings().engine,
            active: self.engine_kind(),
            built_in_for_session: self.built_in_for_session.load(Ordering::Relaxed),
            pitwall: self.pitwall().map(|p| p.view()).unwrap_or_default(),
        }
    }

    /// Why the engine was switched off for the session (repeated worker crashes), if it was.
    pub fn quarantine_reason(&self) -> Option<String> {
        self.deps.health.as_ref().and_then(|h| h.disabled_reason())
    }

    /// Lets the user try again after the engine was switched off or a load failed.
    pub fn reset(&self) {
        if let Some(h) = &self.deps.health {
            h.reset();
        }
        if let Active::Lazy { lazy, .. } = &*lock(&self.active) {
            if matches!(lazy.state(), LoadState::Failed(_)) {
                lazy.unload();
            }
        }
    }

    /// Whether a model is loaded or loading: its worker (and llama-server) may be using a runtime's files.
    pub(super) fn engine_running(&self) -> bool {
        matches!(&*lock(&self.active), Active::Lazy { lazy, .. } if matches!(lazy.state(), LoadState::Ready | LoadState::Loading))
    }

    /// Starts the engine afresh with the current settings, e.g. after the inference build changed: the old
    /// worker exits and a new one starts (with the new environment) at once when a model was loaded,
    /// otherwise on first use.
    pub(super) fn restart_engine(&self) {
        let was_running = self.engine_running();
        self.install_active(None);
        if was_running && self.availability().is_ok() {
            if let Active::Lazy { lazy, .. } = &*lock(&self.active) {
                lazy.preload();
            }
        }
    }

    /// Frees the model's memory (app exit).
    pub fn shutdown(&self) {
        let old = std::mem::replace(&mut *lock(&self.active), Active::None);
        if let Active::Lazy { lazy, .. } = old {
            lazy.shutdown();
            lazy.unload();
        }
    }

    // ---- set-up ----

    pub fn setup_running(&self) -> bool {
        lock(&self.setup).is_some()
    }

    /// Reserves the (single) set-up slot; the returned progress carries the cancel flag.
    pub fn begin_setup(&self) -> AiResult<Arc<Progress>> {
        self.availability()?;
        let mut slot = lock(&self.setup);
        if slot.is_some() {
            return Err(AiCmdError::busy("A model is already being set up; wait for it or cancel it first."));
        }
        let p = Arc::new(Progress::new());
        *slot = Some(p.clone());
        Ok(p)
    }

    pub fn end_setup(&self) {
        *lock(&self.setup) = None;
    }

    /// Stops a running set-up (download, verification, load). False when none runs.
    pub fn cancel_setup(&self) -> bool {
        match &*lock(&self.setup) {
            Some(p) => {
                p.cancel();
                true
            }
            None => false,
        }
    }

    pub(super) fn with_setup<T>(&self, f: impl FnOnce(&Setup<'_>) -> debrief_ai::Result<T>) -> AiResult<T> {
        let settings = self.settings();
        let source = (self.deps.source)(&settings)?;
        let store = self.store();
        let scratch;
        let settings_path = match self.settings_path() {
            Some(p) => p,
            None => {
                scratch = std::env::temp_dir().join(format!("debrief-ai-{}.json", std::process::id()));
                scratch
            }
        };
        let setup = Setup {
            probe: self.deps.probe.as_ref(),
            source: source.as_ref(),
            disk: self.deps.disk.as_ref(),
            runtime: self.deps.runtime.as_ref(),
            store: &store,
            catalog: &self.catalog,
            settings_path: &settings_path,
            settings,
            backoff: self.deps.download_backoff,
            shapes: &self.shapes,
            reclaim_gb: self.reclaim_gb(),
        };
        Ok(f(&setup)?)
    }

    /// What would be set up (the best model unless `model_id` names another), and what it costs.
    pub fn plan(&self, model_id: Option<&str>) -> AiResult<SetupPlan> {
        self.availability()?;
        let mut plan = self.with_setup(|s| s.plan())?;
        if let Some(id) = model_id.filter(|id| plan.chosen.as_ref().is_none_or(|c| c.model.id != *id)) {
            let pick: Recommendation = plan
                .recommendations
                .ranked
                .iter()
                .find(|r| r.model.id == id)
                .cloned()
                .ok_or_else(|| AiCmdError::not_found(format!("'{id}' is not in the model list")))?;
            plan.already_installed = pick.installed;
            plan.download_bytes = if pick.installed { 0 } else { pick.model.size_bytes };
            plan.license = Some(pick.model.license.clone());
            plan.gated = false;
            plan.note = None;
            plan.chosen = Some(pick);
        }
        Ok(plan)
    }

    /// Downloads (if needed), verifies, loads and warms up a catalog model, then makes it the active one.
    /// Blocks; call off the UI thread between [`begin_setup`](Self::begin_setup) and [`end_setup`](Self::end_setup).
    pub fn run_setup(&self, model_id: &str, progress: &Progress, on_stage: &mut dyn FnMut(SetupStage)) -> AiResult<()> {
        let choice = self
            .catalog
            .find(model_id)
            .cloned()
            .ok_or_else(|| AiCmdError::not_found(format!("'{model_id}' is not in the model list")))?;
        let outcome = self.with_setup(|s| s.run(&choice, progress, on_stage))?;
        self.adopt(outcome);
        Ok(())
    }

    /// Loads and warms up an installed model and makes it the active one. Blocks like [`run_setup`](Self::run_setup).
    pub fn activate(&self, file_name: &str, progress: &Progress, on_stage: &mut dyn FnMut(SetupStage)) -> AiResult<()> {
        let outcome = self.with_setup(|s| s.activate(file_name, progress, on_stage))?;
        self.adopt(outcome);
        Ok(())
    }

    fn adopt(&self, outcome: debrief_ai::setup::SetupOutcome) {
        *crate::sync::write(&self.settings) = outcome.settings;
        let info = EngineInfo {
            backend: Some(outcome.backend),
            notes: outcome.notes,
            server: outcome.server,
        };
        self.install_active(Some((outcome.provider, info)));
    }

    // ---- models ----

    pub fn models(&self) -> Vec<ModelDto> {
        let hw = self.hardware();
        let live = self.live();
        let settings = self.settings();
        let active = match &*lock(&self.active) {
            Active::Lazy { file, .. } => Some(file.clone()),
            _ => None,
        };
        self.store()
            .list()
            .into_iter()
            .map(|m| {
                let cat = self.catalog.models.iter().find(|c| c.file == m.file_name);
                let projector_download_bytes = cat.and_then(|c| c.projector.as_ref()).filter(|_| !m.vision()).map(|p| p.size_bytes);
                ModelDto {
                    vision: m.vision(),
                    projector: m.projector.clone(),
                    projector_download_bytes,
                    active: active.as_deref() == Some(m.file_name.as_str()),
                    // Its projector loads with it (ADR 0025).
                    assessment: m.info.as_ref().map(|i| {
                        let extra = m.projector.as_ref().filter(|p| p.problem.is_none()).map_or(0, |p| p.size_bytes);
                        assess(&hw, &live, &Subject::from_gguf(i, extra, effective_context(&settings, i)))
                    }),
                    catalog_id: cat.map(|c| c.id.clone()),
                    name: cat
                        .map(|c| c.name.clone())
                        .or_else(|| m.info.as_ref().and_then(|i| i.name.clone()))
                        .unwrap_or_else(|| m.file_name.clone()),
                    license: cat.map(|c| c.license.clone()),
                    tool_calling: cat.is_some_and(|c| c.tool_calling),
                    file_name: m.file_name,
                    size_bytes: m.size_bytes,
                    info: m.info,
                    problem: m.problem,
                }
            })
            .collect()
    }

    /// Deletes an installed model; deleting the active one switches the AI to "no model".
    pub fn delete_model(&self, file_name: &str) -> AiResult<()> {
        let is_active = matches!(&*lock(&self.active), Active::Lazy { file, .. } if file == file_name);
        if is_active {
            if self.setup_running() {
                return Err(AiCmdError::busy(
                    "A model is being set up; wait for it before deleting the active model.",
                ));
            }
            self.update_settings(|s| s.active_model = None)?;
            self.install_active(None);
        }
        self.store().delete(file_name)?;
        Ok(())
    }

    /// Adds a GGUF file the user chose (hard link when possible, otherwise a copy) after validating it. A file
    /// named like a catalog model, or of its exact size, must have the catalog's SHA-256 (a manual download is
    /// checked like Debrief's own); a verified one is filed under the catalog's file name. Hashing reads the whole
    /// file: call off the UI thread; `progress` reports it and cancels it.
    pub fn add_local(&self, path: &str, progress: &Progress) -> AiResult<ModelDto> {
        if path.is_empty() || path.chars().count() > MAX_PATH_CHARS || path.contains('\0') {
            return Err(AiCmdError::invalid("choose a .gguf model file"));
        }
        let p = Path::new(path);
        if !p.is_absolute() {
            return Err(AiCmdError::invalid("the model path must be absolute"));
        }
        if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("gguf")) {
            return Err(AiCmdError::invalid("model files end in .gguf"));
        }
        let canon = p
            .canonicalize()
            .map_err(|e| AiCmdError::not_found(format!("cannot open {path}: {e}")))?;
        if !canon.is_file() {
            return Err(AiCmdError::invalid(format!("{path} is not a file")));
        }
        // Not a model at all: say so before reading gigabytes for the checksum.
        gguf::inspect(&canon)?;
        let known = verify_local(&self.catalog, &canon, progress)?.map(|m| m.file.clone());
        let added = self.store().add_local_as(&canon, AddMode::Link, known.as_deref())?;
        self.models()
            .into_iter()
            .find(|m| m.file_name == added.file_name)
            .ok_or_else(|| AiCmdError::new(AiErrorCode::Io, "the model was added but cannot be listed"))
    }

    // ---- image support (ADR 0025) ----

    /// The pixel budget of an image for the active model: what its projector reads, else the default.
    pub fn image_target_pixels(&self) -> u64 {
        let file = match &*lock(&self.active) {
            Active::Lazy { file, .. } => Some(file.clone()),
            _ => None,
        };
        file.and_then(|f| self.store().projector_path(&f).ok().flatten())
            .and_then(|p| gguf::inspect_projector(&p).ok())
            .map_or(debrief_ai::vision::image::DEFAULT_TARGET_PIXELS, |p| p.max_image_pixels())
    }

    /// The catalog entry of an installed model file that offers a projector.
    fn projector_entry(&self, file_name: &str) -> AiResult<debrief_ai::catalog::CatalogModel> {
        let entry = self.catalog.models.iter().find(|c| c.file == file_name).ok_or_else(|| {
            AiCmdError::not_found(format!(
                "{file_name} is not a model from the list, so Debrief does not know its image projector; add the projector file yourself"
            ))
        })?;
        if entry.projector.is_none() {
            return Err(AiCmdError::not_found(format!(
                "{} cannot read images: choose a model marked \"Text + images\"",
                entry.name
            )));
        }
        Ok(entry.clone())
    }

    /// Bytes "Add image support" downloads for `file_name` (0 when it is installed already).
    pub fn projector_download_bytes(&self, file_name: &str) -> AiResult<u64> {
        let entry = self.projector_entry(file_name)?;
        self.with_setup(|s| Ok(s.missing_projector_bytes(&entry)))
    }

    /// Downloads and verifies the catalog projector of the installed model `file_name` (never automatically: the
    /// user asked). The active model is reloaded with it on its next use. Blocks; call off the UI thread.
    pub fn install_projector(&self, file_name: &str, progress: &Progress, on_stage: &mut dyn FnMut(SetupStage)) -> AiResult<()> {
        self.availability()?;
        let entry = self.projector_entry(file_name)?;
        if !self.store().path_of(file_name)?.is_file() {
            return Err(AiCmdError::not_found(format!("{file_name} is not installed")));
        }
        self.with_setup(|s| s.download_projector(&entry, progress, on_stage))?;
        self.reload_if_active(file_name);
        Ok(())
    }

    /// Adds a projector file the user chose for the installed model `file_name`: it must be a projector made for
    /// this model; a file of the catalog projector's size must have its SHA-256. Hashing reads the file: call off
    /// the UI thread.
    pub fn add_projector(&self, file_name: &str, path: &Path, progress: &Progress) -> AiResult<ModelDto> {
        self.availability()?;
        let store = self.store();
        store.path_of(file_name)?;
        let info = gguf::inspect_projector(path)?;
        let pinned = self
            .catalog
            .models
            .iter()
            .find(|c| c.file == file_name)
            .and_then(|c| c.projector.clone());
        if let Some(p) = pinned.filter(|p| p.size_bytes == info.file_size) {
            progress.begin(debrief_ai::models::Phase::Verifying, info.file_size, 0);
            let got = debrief_ai::models::download::sha256_file(path, progress)?;
            if !got.eq_ignore_ascii_case(&p.sha256) {
                return Err(AiError::Integrity(format!(
                    "this is not the original projector of {file_name}: its SHA-256 is {got}, expected {}; download it again",
                    p.sha256
                ))
                .into());
            }
        }
        store.add_projector(file_name, path, AddMode::Link)?;
        self.reload_if_active(file_name);
        self.models()
            .into_iter()
            .find(|m| m.file_name == file_name)
            .ok_or_else(|| AiCmdError::new(AiErrorCode::Io, "the projector was added but the model cannot be listed"))
    }

    /// The active model is `file_name`: load it again (with its projector) on its next use.
    fn reload_if_active(&self, file_name: &str) {
        let active = matches!(&*lock(&self.active), Active::Lazy { file, .. } if file == file_name);
        if active && !self.setup_running() {
            self.install_active(None);
        }
    }

    // ---- settings ----

    pub(super) fn update_settings(&self, f: impl FnOnce(&mut AiSettings)) -> AiResult<AiSettings> {
        let mut s = self.settings();
        f(&mut s);
        let s = s.clamp();
        if let Some(p) = self.settings_path() {
            s.save(&p)?;
        }
        *crate::sync::write(&self.settings) = s.clone();
        Ok(s)
    }

    /// Applies new settings from the Settings screen. The active model is kept (it is chosen in the model
    /// manager); a change to how the model loads unloads it so the next use reloads with the new values.
    pub fn apply_settings(&self, mut next: AiSettings) -> AiResult<()> {
        let before = self.settings();
        next.active_model = before.active_model.clone();
        let next = next.clamp();
        let problems = next.validate();
        if !problems.is_empty() {
            return Err(AiCmdError::invalid(problems.join("; ")));
        }
        let reload = before.gpu_offload != next.gpu_offload
            || before.context_length != next.context_length
            || before.threads != next.threads
            || before.models_dir != next.models_dir;
        // Another llama-server build needs a new worker process (its environment names the build).
        let runtime_changed = before.inference_build != next.inference_build || before.llama_server_path != next.llama_server_path;
        let idle_changed = before.idle_unload_minutes != next.idle_unload_minutes;
        if before.engine != next.engine {
            self.clear_session_fallback();
        }
        self.update_settings(|s| *s = next.clone())?;
        self.pitwall_needs();
        if runtime_changed {
            self.restart_engine();
        } else if reload {
            self.install_active(None);
        } else if idle_changed {
            if let Active::Lazy { lazy, .. } = &*lock(&self.active) {
                lazy.set_idle(idle_duration(&next));
            }
        }
        Ok(())
    }
}

impl AiCore {
    /// Tells Pitwall what Debrief's prompts need (the context length setting).
    pub fn pitwall_needs(&self) {
        if let Some(pw) = self.pitwall() {
            pw.set_needs(debrief_ai::engine::pitwall::Needs {
                context_tokens: self.settings().context_length,
            });
        }
    }
}

/// A provider given directly (tests) as an engine.
struct FixedEngine(Arc<dyn LlmProvider>);

impl AiEngine for FixedEngine {
    fn kind(&self) -> EngineKind {
        EngineKind::BuiltIn
    }
    fn state(&self) -> LoadState {
        LoadState::Ready
    }
    fn model(&self) -> Option<String> {
        Some(self.0.display_name())
    }
    fn vision(&self) -> bool {
        self.0.vision()
    }
    fn preload(&self) {}
    fn release(&self) {}
    fn llm(&self) -> Arc<dyn LlmProvider> {
        self.0.clone()
    }
}

impl Drop for AiCore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// One process as [`tree_rss_bytes`] sees it: pid, parent pid, name, resident bytes.
pub(super) type ProcessRow = (u32, Option<u32>, String, u64);

/// Resident memory of `me`'s inference workers and their children (the worker runs the model in a
/// llama-server child, ADR 0009, so the worker alone is only a few MB). Pure.
pub(super) fn tree_rss_bytes(me: u32, procs: &[ProcessRow]) -> u64 {
    let workers: Vec<u32> = procs
        .iter()
        .filter(|(_, parent, name, _)| *parent == Some(me) && name.starts_with("debrief-ai-worker"))
        .map(|(pid, ..)| *pid)
        .collect();
    procs
        .iter()
        .filter(|(pid, parent, ..)| workers.contains(pid) || parent.is_some_and(|p| workers.contains(&p)))
        .map(|(.., mem)| *mem)
        .sum()
}

/// Resident memory of this process's inference workers, including their llama-server children.
pub fn worker_rss_bytes() -> u64 {
    use sysinfo::{ProcessesToUpdate, System};
    let Ok(me) = sysinfo::get_current_pid() else { return 0 };
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let rows: Vec<ProcessRow> = sys
        .processes()
        .values()
        .map(|p| {
            (
                p.pid().as_u32(),
                p.parent().map(|q| q.as_u32()),
                p.name().to_string_lossy().into_owned(),
                p.memory(),
            )
        })
        .collect();
    tree_rss_bytes(me.as_u32(), &rows)
}
