//! The app-wide AI runtime: settings, the model catalog and folder, set-up, and the one active model shared
//! by every window (one worker process, loaded lazily and unloaded when idle). Plain Rust, no Tauri, so it is
//! testable with the fakes of `imt-ai` (scripted provider, fake runtime, memory secrets).

use super::approvals::Approvals;
use super::error::{AiCmdError, AiErrorCode, AiResult};
use imt_ai::assistant::{Assistant, AssistantConfig};
use imt_ai::catalog::Catalog;
use imt_ai::gguf::{self, GgufInfo};
use imt_ai::hardware::{Hardware, HardwareProbe, SystemProbe};
use imt_ai::llm::{LazyProvider, LlmProvider, LoadState, ModelLoader};
use imt_ai::models::source::{ModelSource, Progress};
use imt_ai::models::{AddMode, DiskSpace, HfConfig, HuggingFace, ModelStore, SystemDisk};
use imt_ai::recommend::{assess, recommend, Assessment, Recommendation, Recommendations};
use imt_ai::runtime::{check_memory, effective_context, load_with_fallback, LoadRequest, ModelRuntime};
use imt_ai::settings::AiSettings;
use imt_ai::setup::{Setup, SetupPlan, SetupStage};
use imt_ai::web::{KeyringSecretStore, LogAudit, SecretStore, WebHttpConfig, WebProvider, WebToolkit, BRAVE_SECRET_NAME};
use imt_ai::worker::{GrpcLauncher, Health, WorkerConfig, WorkerRuntime};
use imt_ai::AiError;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

/// Keychain service name for IMT's secrets.
pub const KEYCHAIN_SERVICE: &str = "net.decryptology.imt";
const SETTINGS_FILE: &str = "ai.json";
const CATALOG_FILE: &str = "local-models.json";
const MAX_TOOL_ROUNDS: usize = 6;
const MAX_KEY_CHARS: usize = 200;
const MAX_PATH_CHARS: usize = 4096;

/// Builds the download source from the current settings (endpoint, proxy).
pub type SourceFactory = Arc<dyn Fn(&AiSettings) -> imt_ai::Result<Box<dyn ModelSource>> + Send + Sync>;

/// The adapters the runtime works with. [`AiDeps::system`] is the real machine; tests pass fakes.
pub struct AiDeps {
    pub runtime: Arc<dyn ModelRuntime>,
    /// Crash history of the worker (quarantine), when the runtime has one.
    pub health: Option<Arc<Health>>,
    pub probe: Arc<dyn HardwareProbe + Send + Sync>,
    pub source: SourceFactory,
    pub disk: Arc<dyn DiskSpace>,
    pub secrets: Arc<dyn SecretStore>,
    /// The worker binary; when it is missing the AI is unavailable. `None` skips the check (tests).
    pub worker_path: Option<PathBuf>,
    pub download_backoff: Duration,
}

impl AiDeps {
    /// The real adapters: the llama.cpp worker at `worker`, the system's hardware, Hugging Face, the keychain.
    pub fn system(worker: PathBuf) -> AiDeps {
        let probe = Arc::new(SystemProbe::new(imt_collect::SystemRunner));
        let p = probe.clone();
        let cfg = WorkerConfig::new(Arc::new(GrpcLauncher {
            exe: worker.clone(),
            args: vec![],
            env: vec![],
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
            secrets: Arc::new(KeyringSecretStore::new(KEYCHAIN_SERVICE)),
            worker_path: Some(worker),
            download_backoff: Duration::from_secs(2),
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
    pub web_search: WebProvider,
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
}

/// Progress of a set-up or an activation, streamed as `imt://ai-setup`.
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
struct EngineInfo {
    backend: Option<String>,
    notes: Vec<String>,
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

/// A loaded provider with its backend name and notes (GPU fallback), from the set-up.
type Primed = (Arc<dyn LlmProvider>, String, Vec<String>);

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

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl ModelLoader for AppLoader {
    fn load(&self) -> imt_ai::Result<Arc<dyn LlmProvider>> {
        if let Some((p, backend, notes)) = lock(&self.primed).take() {
            *lock(&self.info) = EngineInfo {
                backend: Some(backend),
                notes,
            };
            return Ok(p);
        }
        let path = self.store.path_of(&self.file)?;
        let info = gguf::inspect(&path)?;
        check_memory(&self.probe.probe(), &info)?;
        let req = LoadRequest {
            context: effective_context(&self.settings, &info),
            path,
            info,
            gpu: self.settings.gpu_offload.into(),
            threads: self.settings.threads,
        };
        let (loaded, note) = load_with_fallback(self.runtime.as_ref(), req)?;
        *lock(&self.info) = EngineInfo {
            backend: Some(loaded.backend),
            notes: note.into_iter().collect(),
        };
        Ok(loaded.provider)
    }

    fn context_hint(&self) -> usize {
        self.settings.context_length
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
    deps: AiDeps,
    settings: RwLock<AiSettings>,
    warnings: Vec<String>,
    catalog: Catalog,
    active: Mutex<Active>,
    info: Arc<Mutex<EngineInfo>>,
    setup: Mutex<Option<Arc<Progress>>>,
    pub approvals: Arc<Approvals>,
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
        for w in &warnings {
            log::warn!("AI: {w}");
        }
        let core = AiCore {
            dir,
            default_models_dir: models_dir,
            safe_mode,
            deps,
            settings: RwLock::new(settings),
            warnings,
            catalog,
            active: Mutex::new(Active::None),
            info: Arc::default(),
            setup: Mutex::default(),
            approvals: Arc::default(),
        };
        core.install_active(None);
        core
    }

    /// Loads the model in the background after startup when the settings ask for it.
    pub fn autoload(&self) {
        if self.availability().is_err() || !self.settings().autoload {
            return;
        }
        if let Active::Lazy { lazy, .. } = &*lock(&self.active) {
            lazy.preload();
        }
    }

    /// Ok when AI can be used in this run; otherwise why not.
    pub fn availability(&self) -> AiResult<()> {
        if self.safe_mode {
            return Err(AiCmdError::unavailable(
                "IMT is running in safe mode, so the AI assistant is off. Restart normally to use it.",
            ));
        }
        if let Some(p) = &self.deps.worker_path {
            if !p.is_file() {
                return Err(AiCmdError::unavailable(format!(
                    "The AI engine is not installed (missing {}). Reinstall IMT, or use a build that includes the AI assistant.",
                    p.display()
                )));
            }
        }
        Ok(())
    }

    pub fn settings(&self) -> AiSettings {
        self.settings.read().unwrap_or_else(|p| p.into_inner()).clone()
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

    fn store(&self) -> ModelStore {
        ModelStore::new(self.models_dir())
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn hardware(&self) -> Hardware {
        self.deps.probe.probe()
    }

    fn installed_files(&self) -> Vec<String> {
        self.store().list().into_iter().map(|m| m.file_name).collect()
    }

    pub fn recommendations(&self) -> Recommendations {
        recommend(&self.hardware(), &self.catalog, &self.installed_files())
    }

    /// Makes the active model the one named in the settings (or `primed`, already loaded by the set-up).
    fn install_active(&self, primed: Option<Primed>) {
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

    /// The model, for a use case or a chat turn.
    pub fn provider(&self) -> AiResult<Arc<dyn LlmProvider>> {
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

    /// An assistant on the active model with the current settings and its own cancel flag.
    pub fn assistant(&self, cancel: Arc<AtomicBool>) -> AiResult<Assistant<Arc<dyn LlmProvider>>> {
        let provider = self.provider()?;
        let s = self.settings();
        Ok(Assistant::new(
            provider,
            AssistantConfig {
                max_tokens: s.max_tokens,
                temperature: s.temperature,
                max_tool_rounds: MAX_TOOL_ROUNDS,
                cancel,
            },
        ))
    }

    /// The web tools for a chat turn, when web search is configured (it is off by default).
    pub fn web_toolkit(&self) -> Option<WebToolkit> {
        let s = self.settings();
        if s.web_search.provider == WebProvider::None {
            return None;
        }
        match WebToolkit::from_settings(
            &s.web_search,
            self.deps.secrets.clone(),
            WebHttpConfig::default(),
            Arc::new(LogAudit),
        ) {
            Ok(kit) => Some(kit),
            Err(e) => {
                log::warn!("AI: web search is configured but unusable: {e}");
                None
            }
        }
    }

    pub fn status(&self) -> AiStatus {
        let settings = self.settings();
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
            web_search: settings.web_search.provider,
        };
        drop(info);
        if let Err(e) = self.availability() {
            s.available = false;
            s.state = EngineStateDto::Unavailable;
            s.reason = Some(e.message);
            return s;
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
            }
            Active::Lazy { lazy, file } => {
                s.model = Some(file.clone());
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

    fn with_setup<T>(&self, f: impl FnOnce(&Setup<'_>) -> imt_ai::Result<T>) -> AiResult<T> {
        let settings = self.settings();
        let source = (self.deps.source)(&settings)?;
        let store = self.store();
        let scratch;
        let settings_path = match self.settings_path() {
            Some(p) => p,
            None => {
                scratch = std::env::temp_dir().join(format!("imt-ai-{}.json", std::process::id()));
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

    fn adopt(&self, outcome: imt_ai::setup::SetupOutcome) {
        *self.settings.write().unwrap_or_else(|p| p.into_inner()) = outcome.settings;
        self.install_active(Some((outcome.provider, outcome.backend, outcome.notes)));
    }

    // ---- models ----

    pub fn models(&self) -> Vec<ModelDto> {
        let hw = self.hardware();
        let active = match &*lock(&self.active) {
            Active::Lazy { file, .. } => Some(file.clone()),
            _ => None,
        };
        self.store()
            .list()
            .into_iter()
            .map(|m| {
                let cat = self.catalog.models.iter().find(|c| c.file == m.file_name);
                ModelDto {
                    active: active.as_deref() == Some(m.file_name.as_str()),
                    assessment: m.info.as_ref().map(|i| assess(&hw, i.file_size, i.params_b, i.is_moe(), 0.0)),
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

    /// Adds a GGUF file the user chose (hard link when possible, otherwise a copy) after validating it.
    pub fn add_local(&self, path: &str) -> AiResult<ModelDto> {
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
        let added = self.store().add_local(&canon, AddMode::Link)?;
        self.models()
            .into_iter()
            .find(|m| m.file_name == added.file_name)
            .ok_or_else(|| AiCmdError::new(AiErrorCode::Io, "the model was added but cannot be listed"))
    }

    // ---- settings and secrets ----

    fn update_settings(&self, f: impl FnOnce(&mut AiSettings)) -> AiResult<AiSettings> {
        let mut s = self.settings();
        f(&mut s);
        let s = s.clamp();
        if let Some(p) = self.settings_path() {
            s.save(&p)?;
        }
        *self.settings.write().unwrap_or_else(|p| p.into_inner()) = s.clone();
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
        let idle_changed = before.idle_unload_minutes != next.idle_unload_minutes;
        self.update_settings(|s| *s = next.clone())?;
        if reload {
            self.install_active(None);
        } else if idle_changed {
            if let Active::Lazy { lazy, .. } = &*lock(&self.active) {
                lazy.set_idle(idle_duration(&next));
            }
        }
        Ok(())
    }

    /// Stores the Brave Search API key in the OS keychain (never in a file; it cannot be read back).
    pub fn set_brave_key(&self, key: &str) -> AiResult<()> {
        let k = key.trim();
        if k.is_empty() || k.chars().count() > MAX_KEY_CHARS || k.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(AiCmdError::invalid("paste the API key exactly as Brave shows it"));
        }
        Ok(self.deps.secrets.set(BRAVE_SECRET_NAME, k)?)
    }

    pub fn clear_brave_key(&self) -> AiResult<()> {
        Ok(self.deps.secrets.delete(BRAVE_SECRET_NAME)?)
    }

    /// Whether a key is stored (never the key itself).
    pub fn brave_key_set(&self) -> bool {
        self.deps.secrets.get(BRAVE_SECRET_NAME).ok().flatten().is_some()
    }
}

impl Drop for AiCore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Resident memory of this process's inference workers.
pub fn worker_rss_bytes() -> u64 {
    use sysinfo::{ProcessesToUpdate, System};
    let Ok(me) = sysinfo::get_current_pid() else { return 0 };
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes()
        .values()
        .filter(|p| p.parent() == Some(me) && p.name().to_string_lossy().starts_with("imt-ai-worker"))
        .map(|p| p.memory())
        .sum()
}
