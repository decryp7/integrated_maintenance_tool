//! Inference runtimes in the app (ADR 0009): which llama-server builds are installed and work here, and
//! installing, choosing and deleting them. Plain Rust over `imt_ai::runtimes`, no Tauri: the commands in
//! `runtime_commands.rs` validate the IPC arguments and stream the progress.

use super::core::load_request;
use super::core::AiCore;
use super::error::{AiCmdError, AiErrorCode, AiResult};
use imt_ai::hardware::{HardwareProbe, SystemProbe};
use imt_ai::models::source::{ModelSource, Progress};
use imt_ai::models::DownloadOptions;
use imt_ai::runtime::ModelRuntime;
use imt_ai::runtimes::builds::{is_build_id, label};
use imt_ai::runtimes::selftest::self_test;
use imt_ai::runtimes::store::{server_file_name, RuntimeCheck};
use imt_ai::runtimes::{
    current_platform, list_builds, overview, suggest, usable_tag, BuildChoice, BuildsReport, GithubReleases, InstalledRuntime,
    OverviewInput, ReleaseConfig, RuntimeCatalog, RuntimeOverview, RuntimeStage, RuntimeStore,
};
use imt_ai::settings::AiSettings;
use imt_ai::worker::{fixed_env, FramesLauncher, WorkerConfig, WorkerEnv, WorkerRuntime};
use imt_ai::AiError;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// Builds the runtime download source from the current settings (proxy) and the pinned catalog.
pub type RuntimeSourceFactory = Arc<dyn Fn(&AiSettings, &RuntimeCatalog) -> imt_ai::Result<Box<dyn ModelSource>> + Send + Sync>;
/// Asks the worker which installed builds work here (`imt-ai-worker --list-builds`), with the worker's environment.
pub type BuildsReporter = Arc<dyn Fn(&[(String, String)]) -> imt_ai::Result<BuildsReport> + Send + Sync>;
/// A model runtime that runs exactly the llama-server at the given path (the self-test of a new build).
pub type BuildRuntime = Arc<dyn Fn(&Path) -> Arc<dyn ModelRuntime> + Send + Sync>;

/// Longest the worker may take to check every installed build (each runs `llama-server --list-devices`).
const REPORT_TIMEOUT: Duration = Duration::from_secs(90);
/// How long a delete keeps retrying while the stopped worker's files are still locked (Windows).
const DELETE_RETRY: Duration = Duration::from_secs(5);

/// The adapters of the runtime manager. [`RuntimeDeps::system`] is the real machine; tests pass fakes.
pub struct RuntimeDeps {
    pub store: Arc<RuntimeStore>,
    pub catalog: RuntimeCatalog,
    /// The catalog key of this platform (`windows-x64`, ...).
    pub platform: String,
    pub source: RuntimeSourceFactory,
    pub report: BuildsReporter,
    /// Runs one unpacked build for its self-test, in its own worker (never the active engine).
    pub build_runtime: BuildRuntime,
}

impl RuntimeDeps {
    /// The builds kept in `root`, downloaded from GitHub with the embedded catalog, checked by `worker`.
    pub fn system(root: PathBuf, worker: PathBuf) -> RuntimeDeps {
        RuntimeDeps {
            store: Arc::new(RuntimeStore::new(root)),
            catalog: RuntimeCatalog::embedded(),
            platform: current_platform(),
            source: Arc::new(|s: &AiSettings, catalog: &RuntimeCatalog| {
                let cfg = ReleaseConfig {
                    proxy: s.download_proxy.clone(),
                    ..ReleaseConfig::default()
                };
                Ok(Box::new(GithubReleases::new(cfg, catalog.clone())?) as Box<dyn ModelSource>)
            }),
            report: {
                let worker = worker.clone();
                Arc::new(move |env: &[(String, String)]| list_builds(&worker, env, REPORT_TIMEOUT))
            },
            build_runtime: Arc::new(move |server: &Path| {
                let cfg = WorkerConfig::new(Arc::new(FramesLauncher {
                    exe: worker.clone(),
                    args: vec![],
                    env: fixed_env(vec![("IMT_LLAMA_SERVER".into(), server.to_string_lossy().into_owned())]),
                }));
                let probe = SystemProbe::new(imt_collect::SystemRunner);
                Arc::new(WorkerRuntime::new(cfg, Box::new(move || probe.probe()))) as Arc<dyn ModelRuntime>
            }),
        }
    }

    /// The worker's environment for `settings`: where the builds are and which one to use.
    pub fn worker_env(&self, settings: &AiSettings) -> Vec<(String, String)> {
        settings.worker_env(&builds_dir(&self.store, &self.catalog.tag))
    }

    /// The worker launcher's environment, read from `settings` at every launch.
    pub fn worker_env_source(&self, settings: Arc<RwLock<AiSettings>>) -> WorkerEnv {
        let store = self.store.clone();
        let tag = self.catalog.tag.clone();
        Arc::new(move || {
            let s = crate::sync::read(&settings).clone();
            s.worker_env(&builds_dir(&store, &tag))
        })
    }

    fn current_ids(&self) -> Vec<String> {
        self.store
            .list(&self.catalog.tag)
            .into_iter()
            .filter(|r| r.current)
            .map(|r| r.id)
            .collect()
    }
}

/// The builds the worker runs: the pinned release's, or an older release's until the pinned one is installed
/// (an update keeps the previous build working until the new one is verified).
fn builds_dir(store: &RuntimeStore, pinned: &str) -> PathBuf {
    store.builds_dir(&usable_tag(&store.list(pinned), pinned))
}

/// Progress of an install, streamed as `imt://ai-runtime`; `ready`, `failed` and `cancelled` end it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "stage", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RuntimeEvent {
    /// Downloading asset `index` of `count` (CUDA builds have two: llama-server and its runtime).
    Downloading {
        id: String,
        asset: String,
        index: usize,
        count: usize,
    },
    /// The download ticker: bytes of the current asset (`verifying` while its checksum is computed).
    Progress {
        id: String,
        bytes: u64,
        total: u64,
        bytes_per_sec: f64,
        eta_secs: Option<u64>,
        verifying: bool,
    },
    Unpacking {
        id: String,
        asset: String,
    },
    Finishing {
        id: String,
    },
    /// The unpacked build answers a short text and, with a projector, an image with the active model before it is
    /// switched to.
    Testing {
        id: String,
    },
    Ready {
        id: String,
        runtime: InstalledRuntime,
    },
    Failed {
        id: String,
        code: AiErrorCode,
        message: String,
    },
    Cancelled {
        id: String,
    },
}

impl RuntimeEvent {
    pub fn of_stage(id: &str, st: RuntimeStage) -> RuntimeEvent {
        let id = id.to_string();
        match st {
            RuntimeStage::Downloading { asset, index, count } => RuntimeEvent::Downloading { id, asset, index, count },
            RuntimeStage::Unpacking { asset } => RuntimeEvent::Unpacking { id, asset },
            RuntimeStage::Finishing => RuntimeEvent::Finishing { id },
            RuntimeStage::Testing => RuntimeEvent::Testing { id },
        }
    }

    pub fn of_progress(id: &str, p: &Progress) -> RuntimeEvent {
        let s = p.snapshot();
        RuntimeEvent::Progress {
            id: id.to_string(),
            bytes: s.bytes,
            total: s.total,
            bytes_per_sec: s.bytes_per_sec,
            eta_secs: s.eta_secs,
            verifying: s.phase == imt_ai::models::source::Phase::Verifying,
        }
    }

    /// The final event of an install.
    pub fn of_result(id: &str, r: &AiResult<InstalledRuntime>) -> RuntimeEvent {
        let id = id.to_string();
        match r {
            Ok(runtime) => RuntimeEvent::Ready {
                id,
                runtime: runtime.clone(),
            },
            Err(e) if e.code == AiErrorCode::Cancelled => RuntimeEvent::Cancelled { id },
            Err(e) => RuntimeEvent::Failed {
                id,
                code: e.code,
                message: e.message.clone(),
            },
        }
    }
}

/// The running install (one at a time) and the last build report, keyed by what it depends on.
#[derive(Default)]
pub struct RuntimeJobs {
    job: Mutex<Option<(String, Arc<Progress>)>>,
    report: Mutex<Option<(String, Result<BuildsReport, String>)>>,
}

use crate::sync::lock;

impl AiCore {
    /// What the runtime manager shows: every build for this platform, installed or not, whether it works
    /// here, and which one is suggested and in use. Checking the builds runs the worker, so call it off
    /// the UI thread; the answer is cached until the installed builds or the settings change.
    pub fn runtime_overview(&self) -> AiResult<RuntimeOverview> {
        self.availability()?;
        let s = self.settings();
        let rt = &self.deps.runtimes;
        let installed = rt.store.list(&rt.catalog.tag);
        let custom = s.llama_server_path.clone();
        let (report, report_problem) = if installed.iter().any(|r| r.current) || custom.is_some() {
            match self.builds_report(&s, &installed) {
                Ok(r) => (Some(r), None),
                Err(e) => (None, Some(e)),
            }
        } else {
            (None, None)
        };
        let choice = BuildChoice::parse(&s.inference_build);
        Ok(overview(&OverviewInput {
            catalog: &rt.catalog,
            platform: &rt.platform,
            installed: &installed,
            report: report.as_ref(),
            report_problem,
            suggested: suggest(&rt.platform, &self.hardware(), &rt.catalog),
            choice: &choice,
            custom_path: custom,
        }))
    }

    fn builds_report(&self, s: &AiSettings, installed: &[InstalledRuntime]) -> Result<BuildsReport, String> {
        let rt = &self.deps.runtimes;
        let key = format!(
            "{}|{}|{}",
            s.inference_build,
            s.llama_server_path.as_deref().unwrap_or(""),
            installed
                .iter()
                .map(|r| format!("{}/{}:{}", r.tag, r.id, r.bytes))
                .collect::<Vec<_>>()
                .join(",")
        );
        if let Some((k, r)) = &*lock(&self.jobs.report) {
            if *k == key {
                return r.clone();
            }
        }
        let r = (rt.report)(&rt.worker_env(s)).map_err(|e| e.to_string());
        if let Err(e) = &r {
            log::warn!("AI: cannot check the inference builds: {e}");
        }
        *lock(&self.jobs.report) = Some((key, r.clone()));
        r
    }

    /// The installed builds of the pinned release (the options of the setting *Inference build*).
    pub fn installed_build_ids(&self) -> Vec<String> {
        self.deps.runtimes.current_ids()
    }

    /// Reserves the (single) runtime install slot for build `id`; the progress carries the cancel flag.
    pub fn begin_runtime_install(&self, id: &str) -> AiResult<Arc<Progress>> {
        self.availability()?;
        let rt = &self.deps.runtimes;
        if !is_build_id(id) || rt.catalog.find(&rt.platform, id).is_none() {
            return Err(AiCmdError::not_found(format!(
                "there is no inference build '{id}' for this computer"
            )));
        }
        let mut slot = lock(&self.jobs.job);
        if let Some((running, _)) = &*slot {
            return Err(AiCmdError::busy(format!(
                "The {} runtime is being downloaded; wait for it or cancel it first.",
                label(running)
            )));
        }
        let p = Arc::new(Progress::new());
        *slot = Some((id.to_string(), p.clone()));
        Ok(p)
    }

    pub fn end_runtime_install(&self) {
        *lock(&self.jobs.job) = None;
    }

    /// Stops the running install (download or unpack). False when none runs.
    pub fn cancel_runtime_install(&self) -> bool {
        match &*lock(&self.jobs.job) {
            Some((_, p)) => {
                p.cancel();
                true
            }
            None => false,
        }
    }

    /// Downloads, verifies and unpacks build `id`. With `archives` (the build's files the user downloaded
    /// themselves, already canonical paths) those are checked against the pinned SHA-256 and used instead of
    /// downloading. Blocks; call between [`begin_runtime_install`](Self::begin_runtime_install) and
    /// [`end_runtime_install`](Self::end_runtime_install).
    pub fn install_runtime(
        &self,
        id: &str,
        archives: &[PathBuf],
        progress: &Progress,
        on_stage: &mut dyn FnMut(RuntimeStage),
    ) -> AiResult<InstalledRuntime> {
        let rt = &self.deps.runtimes;
        let build = rt
            .catalog
            .find(&rt.platform, id)
            .cloned()
            .ok_or_else(|| AiCmdError::not_found(format!("there is no inference build '{id}' for this computer")))?;
        if !archives.is_empty() {
            rt.store.import_archives(&build, archives, progress)?;
        }
        let s = self.settings();
        let source = (rt.source)(&s, &rt.catalog)?;
        let opts = DownloadOptions {
            backoff: self.deps.download_backoff,
            ..DownloadOptions::default()
        };
        // A new build is switched to only after it answered with the user's active model (ADR 0009).
        let check = |staging: &Path| self.self_test_build(id, staging, &s, progress);
        let active = s
            .active_model
            .as_deref()
            .is_some_and(|f| self.store().path_of(f).is_ok_and(|p| p.is_file()));
        let installed = rt.store.install(
            &rt.catalog,
            &build,
            source.as_ref(),
            self.deps.disk.as_ref(),
            opts,
            progress,
            on_stage,
            active.then_some(&check as RuntimeCheck<'_>),
        )?;
        // A load that failed for want of a runtime can now succeed: let the next use try again.
        if !self.engine_running() {
            self.reset();
        }
        Ok(installed)
    }

    /// The self-test of build `id` unpacked in `staging`, with the active model, in a worker of its own. The active
    /// engine is stopped first (the model must fit twice otherwise, and Windows keeps a running build's files locked);
    /// the next use starts it again. A model file that cannot be read is not the build's fault: the test is skipped.
    fn self_test_build(&self, id: &str, staging: &Path, s: &AiSettings, progress: &Progress) -> imt_ai::Result<()> {
        let Some(file) = s.active_model.as_deref() else {
            return Ok(());
        };
        let req = match load_request(&self.store(), file, s) {
            Ok(r) => r,
            Err(e) => {
                log::warn!(
                    "AI: the new {} runtime is not self-tested: the active model cannot be read ({e})",
                    label(id)
                );
                return Ok(());
            }
        };
        if self.engine_running() && !self.setup_running() {
            self.install_active(None);
        }
        let runtime = (self.deps.runtimes.build_runtime)(&staging.join(server_file_name()));
        let report = self_test(runtime.as_ref(), req, &label(id), &progress.cancel_flag())?;
        log::info!(
            "AI: the new {} runtime passed its self-test with {file} ({}, images {})",
            label(id),
            report.backend,
            if report.image_checked { "checked" } else { "not used" }
        );
        Ok(())
    }

    /// Deletes an installed build. When it belongs to the pinned release and a model is loaded, the
    /// engine is stopped first (Windows keeps a running build's DLLs locked); the next use starts it again.
    pub fn delete_runtime(&self, tag: &str, id: &str) -> AiResult<()> {
        if !is_build_id(tag) || !is_build_id(id) {
            return Err(AiCmdError::invalid("that is not an inference build name"));
        }
        if lock(&self.jobs.job).as_ref().is_some_and(|(running, _)| running == id) && tag == self.deps.runtimes.catalog.tag {
            return Err(AiCmdError::busy("That build is being downloaded; cancel the download first."));
        }
        if tag == self.deps.runtimes.catalog.tag && self.engine_running() {
            if self.setup_running() {
                return Err(AiCmdError::busy(
                    "A model is being set up; wait for it before deleting the inference runtime it uses.",
                ));
            }
            self.install_active(None);
        }
        let deadline = Instant::now() + DELETE_RETRY;
        loop {
            match self.deps.runtimes.store.delete(tag, id) {
                Err(AiError::Io(_)) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(200)),
                r => return Ok(r?),
            }
        }
    }

    /// Checks a build choice from the UI: `auto` or an installed build of the pinned release.
    pub fn check_build_choice(&self, choice: &str) -> AiResult<String> {
        let c = choice.trim().to_ascii_lowercase();
        if c == "auto" {
            return Ok(c);
        }
        if !is_build_id(&c) {
            return Err(AiCmdError::invalid("that is not an inference build name"));
        }
        if !self.installed_build_ids().contains(&c) {
            return Err(AiCmdError::not_found(format!(
                "the {} runtime is not installed; download it first",
                label(&c)
            )));
        }
        Ok(c)
    }
}
