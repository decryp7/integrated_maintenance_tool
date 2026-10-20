//! Signed auto-update (`tauri-plugin-updater`), off by default.
//!
//! - **Opt-in.** Settings ▸ Updates ▸ "Check for updates automatically" is off by default; Help ▸
//!   Check for updates… works regardless. A check is one HTTPS request to the endpoint configured in
//!   `tauri.conf.json` (`plugins.updater.endpoints`: `latest.json` of this repository's latest Gitea
//!   release) and nothing else is contacted.
//! - **Signatures are mandatory.** The minisign public key is embedded at build time: the
//!   `TAURI_UPDATER_PUBKEY` environment variable (a CI secret) or `plugins.updater.pubkey` in the
//!   configuration (CI passes it with `tauri build --config`). Without a valid key the phase is
//!   [`UpdatePhase::Unavailable`] for good: nothing is checked, nothing unsigned is ever installed.
//!   `requireSignedVersion` is on, so a signature only counts for the version it was made for
//!   (GHSA-j38x-g3m3-95fr: without it a crafted `latest.json` could pair a newer version number with an
//!   older, validly signed build). The plugin must be ≥ 2.12 for that; IMT uses 2.14.
//! - **Flow** ([`state`]): check → available (version, plain-text notes, size) → download with
//!   progress and cancel → verified → "Restart to update", or install when IMT quits. Errors are typed
//!   with a next step ([`error`]). Installing happens in [`on_exit`], after windows and plugins shut
//!   down, so sessions are saved as on any quit.
//!
//! Commands: `updates_status`, `updates_check`, `updates_download`, `updates_discard`,
//! `updates_install_on_quit`, `updates_restart`, `updates_open_releases`. Event: `imt://updates`
//! (the [`UpdateStatus`], to every window: the updater is app-wide). Wire types:
//! `app/src/app/updates/updates.types.ts`.

mod client;
mod error;
mod manifest;
mod record;
mod section;
mod state;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub use client::BoxFuture;
pub use client::{OnChunk, UpdateClient};
pub use error::UpdateError;
#[cfg(test)]
pub use state::UpdateInfo;
pub use state::UpdatePhase;

use section::{SharedSettings, UpdatesSection};
use serde::Serialize;
use state::Event;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

/// The status after every change, to every window.
pub const STATUS_EVENT: &str = "imt://updates";
/// Where users download installers by hand (the next step of most errors).
pub const RELEASES_PAGE: &str = "https://dev.decryptology.net/decryp7/integrated_maintenance_tool/releases";
/// The update public key given to this build (CI secret `TAURI_UPDATER_PUBKEY`). Cargo rebuilds when it changes.
const BUILD_PUBKEY: Option<&str> = option_env!("TAURI_UPDATER_PUBKEY");
/// Progress is sent to the UI at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);

/// The plugin, registered in `run`. Its own IPC commands are not granted to any window (no capability):
/// the UI goes through the commands below.
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R, tauri_plugin_updater::Config> {
    tauri_plugin_updater::Builder::new().build()
}

/// The key to verify updates with: the build-time one, else the configured one; `None` when neither is
/// a valid minisign public key.
pub fn resolve_pubkey(build: Option<&str>, config: Option<&str>) -> Option<String> {
    let pick = |k: Option<&str>| k.map(str::trim).filter(|k| !k.is_empty()).map(str::to_string);
    let key = pick(build).or_else(|| pick(config))?;
    if manifest::is_public_key(&key) {
        Some(key)
    } else {
        log::error!("The update public key of this build is not a minisign public key; updates are off");
        None
    }
}

/// What the Updates UI shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    pub phase: UpdatePhase,
    /// RFC 3339.
    pub last_checked: Option<String>,
    pub auto_check: bool,
    pub channel: &'static str,
    /// The host a check contacts (`None` in a build without updates).
    pub endpoint_host: Option<String>,
    pub releases_page: &'static str,
    /// The job of the download in flight (cancel it with `jobs_cancel`).
    pub download_job: Option<u64>,
}

/// What happens when IMT exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnExit {
    Nothing,
    Install,
    InstallAndRelaunch,
}

struct Inner {
    phase: UpdatePhase,
    last_checked: Option<chrono::DateTime<chrono::Utc>>,
    /// Bumped by every download and cancel; results of an older download are ignored.
    generation: u64,
    task: Option<tauri::async_runtime::JoinHandle<()>>,
    last_progress: Option<Instant>,
    on_exit: OnExit,
    /// The job listing the download in flight.
    job: Option<u64>,
}

type Emit = Box<dyn Fn(&UpdateStatus) + Send + Sync>;

/// The app-wide updater (managed as `Arc<Updates>`).
pub struct Updates {
    client: Option<Arc<dyn UpdateClient>>,
    current_version: String,
    endpoint_host: Option<String>,
    settings: SharedSettings,
    record_dir: Option<PathBuf>,
    inner: Mutex<Inner>,
    emit: Emit,
    started: Instant,
}

impl Updates {
    /// `client` is `None` in a build without an update key. `record_dir` keeps `updates.json`.
    pub fn new(
        client: Option<Arc<dyn UpdateClient>>,
        current_version: String,
        endpoint_host: Option<String>,
        settings: SharedSettings,
        record_dir: Option<PathBuf>,
        emit: Emit,
    ) -> Updates {
        let mut rec = record::load(record_dir.as_deref());
        let phase = match (&client, rec.failed_install.take()) {
            (None, _) => UpdatePhase::Unavailable,
            (Some(_), Some(failed)) => {
                record::save(record_dir.as_deref(), &rec);
                UpdatePhase::Failed {
                    error: failed.error(),
                    info: None,
                }
            }
            (Some(_), None) => UpdatePhase::Idle,
        };
        let last_checked = rec
            .last_checked
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&chrono::Utc));
        Updates {
            client,
            current_version,
            endpoint_host,
            settings,
            record_dir,
            inner: Mutex::new(Inner {
                phase,
                last_checked,
                generation: 0,
                task: None,
                last_progress: None,
                on_exit: OnExit::Nothing,
                job: None,
            }),
            emit,
            started: Instant::now(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn status_of(&self, inner: &Inner) -> UpdateStatus {
        let settings = self.settings.get();
        UpdateStatus {
            current_version: self.current_version.clone(),
            phase: inner.phase.clone(),
            last_checked: inner.last_checked.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            auto_check: settings.auto_check,
            channel: settings.channel.as_str(),
            endpoint_host: self.endpoint_host.clone(),
            releases_page: RELEASES_PAGE,
            download_job: inner.job.filter(|_| matches!(inner.phase, UpdatePhase::Downloading { .. })),
        }
    }

    /// The download in flight is listed as job `id`.
    pub fn set_download_job(&self, id: u64) {
        let status = {
            let mut inner = self.lock();
            inner.job = Some(id);
            self.status_of(&inner)
        };
        (self.emit)(&status);
    }

    /// Whether a download is in flight.
    pub fn downloading(&self) -> bool {
        matches!(self.lock().phase, UpdatePhase::Downloading { .. })
    }

    pub fn status(&self) -> UpdateStatus {
        self.status_of(&self.lock())
    }

    fn client(&self) -> Result<Arc<dyn UpdateClient>, UpdateError> {
        self.client.clone().ok_or(UpdateError::NotAvailable)
    }

    /// Applies `event`; `then` runs under the lock with the new phase (e.g. to cancel the download task).
    fn apply(&self, event: Event, then: impl FnOnce(&mut Inner)) -> Result<UpdateStatus, UpdateError> {
        let status = {
            let mut inner = self.lock();
            inner.phase = inner.phase.next(event)?;
            then(&mut inner);
            self.status_of(&inner)
        };
        (self.emit)(&status);
        Ok(status)
    }

    /// Checks the endpoint for a newer release.
    pub async fn check(&self) -> Result<UpdateStatus, UpdateError> {
        let client = self.client()?;
        self.apply(Event::Check, |_| {})?;
        let result = client.check().await;
        if let Err(e) = &result {
            log::warn!(Kind = e.kind(); "Update check failed: {e}");
        }
        let now = chrono::Utc::now();
        self.lock().last_checked = Some(now);
        let mut rec = record::load(self.record_dir.as_deref());
        rec.last_checked = Some(now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        record::save(self.record_dir.as_deref(), &rec);
        Ok(self.apply(Event::Checked(result), |_| {}).unwrap_or_else(|_| self.status()))
    }

    /// Starts downloading the update found (progress arrives as status events).
    pub fn download(self: &Arc<Self>) -> Result<UpdateStatus, UpdateError> {
        let client = self.client()?;
        let mut generation = 0;
        let status = self.apply(Event::Download, |inner| {
            inner.generation += 1;
            generation = inner.generation;
            inner.last_progress = None;
        })?;
        if !matches!(status.phase, UpdatePhase::Downloading { .. }) {
            return Ok(status);
        }
        let me = self.clone();
        let progress = self.clone();
        let task = tauri::async_runtime::spawn(async move {
            let on_chunk: OnChunk = Box::new(move |n, total| progress.progress(generation, n, total));
            let result = client.download(on_chunk).await;
            me.downloaded(generation, result);
        });
        let mut inner = self.lock();
        if inner.generation == generation && matches!(inner.phase, UpdatePhase::Downloading { .. }) {
            inner.task = Some(task);
        } else {
            task.abort();
        }
        Ok(status)
    }

    fn progress(&self, generation: u64, chunk: u64, total: Option<u64>) {
        let status = {
            let mut inner = self.lock();
            if inner.generation != generation {
                return;
            }
            let Ok(next) = inner.phase.next(Event::Progress { chunk, total }) else {
                return;
            };
            let failed = matches!(next, UpdatePhase::Failed { .. });
            inner.phase = next;
            if failed {
                // Too large: stop the transfer.
                inner.generation += 1;
                if let Some(t) = inner.task.take() {
                    t.abort();
                }
            } else if inner.last_progress.is_some_and(|t| t.elapsed() < PROGRESS_EVERY) {
                return;
            }
            inner.last_progress = Some(Instant::now());
            self.status_of(&inner)
        };
        (self.emit)(&status);
    }

    fn downloaded(&self, generation: u64, result: Result<(), UpdateError>) {
        if let Err(e) = &result {
            log::warn!(Kind = e.kind(); "Update download failed: {e}");
        }
        let current = self.lock().generation == generation;
        let applied = current && self.apply(Event::Downloaded(result), |inner| inner.task = None).is_ok();
        if !applied {
            if let Some(c) = &self.client {
                c.discard();
            }
        }
    }

    /// Stops the download (or drops a downloaded update); the update stays available.
    pub fn cancel(&self) -> Result<UpdateStatus, UpdateError> {
        let client = self.client()?;
        let status = self.apply(Event::Cancel, |inner| {
            inner.generation += 1;
            inner.on_exit = OnExit::Nothing;
            if let Some(t) = inner.task.take() {
                t.abort();
            }
        })?;
        client.discard();
        Ok(status)
    }

    /// Installs the verified update when IMT quits (or not).
    pub fn install_on_quit(&self, enabled: bool) -> Result<UpdateStatus, UpdateError> {
        self.client()?;
        self.apply(Event::InstallOnQuit(enabled), |inner| {
            inner.on_exit = if enabled { OnExit::Install } else { OnExit::Nothing };
        })
    }

    /// Marks the verified update for installing with a relaunch; the caller then quits.
    pub fn prepare_restart(&self) -> Result<UpdateStatus, UpdateError> {
        self.client()?;
        self.apply(Event::Install, |inner| inner.on_exit = OnExit::InstallAndRelaunch)
    }

    /// Whether the automatic check should run now.
    pub fn auto_check_due(&self) -> bool {
        let inner = self.lock();
        let since = inner.last_checked.map(|t| (chrono::Utc::now() - t).num_seconds().max(0) as u64);
        self.client.is_some()
            && state::auto_check_due(
                self.settings.get().auto_check,
                &inner.phase,
                self.started.elapsed().as_secs(),
                since,
            )
    }

    /// Runs at exit: installs the update when asked to. Returns whether IMT should be started again
    /// (Windows' installer does that itself).
    pub fn install_at_exit(&self) -> bool {
        let (action, version) = {
            let mut inner = self.lock();
            let version = match &inner.phase {
                UpdatePhase::Ready { info, .. } | UpdatePhase::Installing { info } => info.version.clone(),
                _ => return false,
            };
            (std::mem::replace(&mut inner.on_exit, OnExit::Nothing), version)
        };
        let relaunch = action == OnExit::InstallAndRelaunch;
        let Some(client) = &self.client else { return false };
        if action == OnExit::Nothing {
            return false;
        }
        log::info!(Version = version.as_str(); "Installing the update");
        match client.install(relaunch) {
            Ok(()) => relaunch && !cfg!(windows),
            Err(e) => {
                log::error!(Kind = e.kind(); "Installing the update failed: {e}");
                let mut rec = record::load(self.record_dir.as_deref());
                rec.failed_install = Some(record::FailedInstall::of(&version, &e));
                record::save(self.record_dir.as_deref(), &rec);
                let _ = self.apply(Event::InstallFailed(e), |_| {});
                // Start the old version again so the user sees what happened.
                relaunch
            }
        }
    }
}

fn plugin_config<R: Runtime>(app: &AppHandle<R>) -> (Option<String>, Option<String>) {
    let cfg = app.config().plugins.0.get("updater");
    let pubkey = cfg.and_then(|c| c.get("pubkey")).and_then(|v| v.as_str()).map(str::to_string);
    let host = cfg
        .and_then(|c| c.get("endpoints"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .and_then(|u| tauri::Url::parse(u).ok())
        .and_then(|u| u.host_str().map(str::to_string));
    (pubkey, host)
}

/// Sets up the updater: the settings section, the managed [`Updates`] and the automatic check.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let (config_key, host) = plugin_config(app);
    let pubkey = resolve_pubkey(BUILD_PUBKEY, config_key.as_deref());
    let available = pubkey.is_some() && host.is_some();
    let client: Option<Arc<dyn UpdateClient>> = match (pubkey, available) {
        (Some(key), true) => Some(Arc::new(client::PluginClient::new(app.clone(), key))),
        _ => None,
    };
    let host = host.filter(|_| available);
    let settings = SharedSettings::default();
    let section = UpdatesSection {
        settings: settings.clone(),
        endpoint_host: host.clone(),
    };
    if let Err(e) = crate::settings::register(app, Arc::new(section), crate::settings::SectionGroup::App) {
        log::error!("Could not register the updates settings: {e:#}");
    }
    let handle = app.clone();
    let updates = Arc::new(Updates::new(
        client,
        app.package_info().version.to_string(),
        host,
        settings,
        crate::app_dirs::config(app).ok(),
        Box::new(move |s| {
            let _ = handle.emit(STATUS_EVENT, s);
            if let UpdatePhase::Available { info } = &s.phase {
                announce(&handle, info);
            }
        }),
    ));
    log::info!(Available = available; "Updates are {}", if available { "available" } else { "not available in this build" });
    if available {
        start_auto_check(updates.clone());
    }
    app.manage(updates);
}

/// An available update is a notification-centre message (once per version) with "Install update".
fn announce<R: Runtime>(app: &AppHandle<R>, info: &state::UpdateInfo) {
    use crate::notification_centre::centre::{NewNotice, NoticeAction, NoticeScope, NoticeSeverity, NoticeSource};
    let notice = NewNotice::new(
        NoticeSource::Update,
        NoticeSeverity::Info,
        format!("IMT {} is available", info.version),
        NoticeScope::App,
    )
    .body(format!("You have {}.", info.current_version))
    .primary(NoticeAction::InstallUpdate)
    .dedupe(format!("update:{}", info.version))
    .once();
    crate::notification_centre::post(app, None, notice, Some(crate::notify::NotificationKind::UpdateAvailable));
}

fn start_auto_check(updates: Arc<Updates>) {
    let spawned = std::thread::Builder::new().name("imt-update-check".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_secs(60));
        if updates.auto_check_due() {
            let _ = tauri::async_runtime::block_on(updates.check());
        }
    });
    if let Err(e) = spawned {
        log::warn!("Automatic update checks are off: {e}");
    }
}

/// Called from `RunEvent::Exit`, last: installs a pending update and starts IMT again when asked.
pub fn on_exit<R: Runtime>(app: &AppHandle<R>) {
    let Some(updates) = app.try_state::<Arc<Updates>>() else { return };
    if updates.install_at_exit() {
        crate::instance::release(app);
        match tauri::process::current_binary(&app.env()) {
            Ok(exe) => {
                if let Err(e) = imt_core::process::quiet_command(exe).spawn() {
                    log::error!("Could not start IMT again after the update: {e}");
                }
            }
            Err(e) => log::error!("Could not start IMT again after the update: {e}"),
        }
    }
}

fn updates_state<'a>(state: &'a Option<State<'_, Arc<Updates>>>) -> Result<&'a Arc<Updates>, UpdateError> {
    state.as_ref().map(|s| s.inner()).ok_or(UpdateError::NotAvailable)
}

#[tauri::command]
pub fn updates_status<R: Runtime>(app: AppHandle<R>) -> Result<UpdateStatus, UpdateError> {
    let state = app.try_state::<Arc<Updates>>();
    Ok(updates_state(&state)?.status())
}

#[tauri::command]
pub async fn updates_check<R: Runtime>(app: AppHandle<R>) -> Result<UpdateStatus, UpdateError> {
    let updates = updates_state(&app.try_state::<Arc<Updates>>())?.clone();
    updates.check().await
}

/// Starts downloading the update as a download job (its id is the status's `downloadJob`; `jobs_cancel`
/// stops it, the update stays available).
#[tauri::command]
pub fn updates_download<R: Runtime>(app: AppHandle<R>) -> Result<UpdateStatus, UpdateError> {
    let updates = updates_state(&app.try_state::<Arc<Updates>>())?.clone();
    let status = updates.download()?;
    if !matches!(status.phase, UpdatePhase::Downloading { .. }) {
        return Ok(status);
    }
    let (stop, watch) = (updates.clone(), updates.clone());
    let controls = crate::jobs::Controls::cancel(move || {
        let _ = stop.cancel();
    });
    let spec = crate::jobs::JobSpec::new(crate::jobs::JobKind::Download, "Downloading the IMT update");
    let job = crate::jobs::spawn(&app, spec, controls, move |ctx| {
        // The transfer runs on the async runtime; the job follows its phase.
        while watch.downloading() && !ctx.is_cancelled() {
            std::thread::sleep(DOWNLOAD_WATCH);
        }
        match watch.status().phase {
            UpdatePhase::Ready { .. } => Ok(crate::jobs::Done::new(())),
            UpdatePhase::Failed { error, .. } => Err(error.to_string()),
            _ => Ok(crate::jobs::Done::new(()).cancelled_if(true)),
        }
    });
    match job {
        Ok(id) => updates.set_download_job(id.0),
        Err(e) => log::warn!("the update download is not listed as a job: {e}"),
    }
    Ok(updates.status())
}

/// How often the download job looks at the transfer.
const DOWNLOAD_WATCH: Duration = Duration::from_millis(250);

/// Drops a downloaded update (it stays available). A download in flight is stopped through its job.
#[tauri::command]
pub fn updates_discard<R: Runtime>(app: AppHandle<R>) -> Result<UpdateStatus, UpdateError> {
    let state = app.try_state::<Arc<Updates>>();
    let updates = updates_state(&state)?;
    if updates.downloading() {
        return Err(UpdateError::Busy {
            detail: "the update is still downloading: cancel its job".into(),
        });
    }
    updates.cancel()
}

#[tauri::command]
pub fn updates_install_on_quit<R: Runtime>(app: AppHandle<R>, enabled: bool) -> Result<UpdateStatus, UpdateError> {
    updates_state(&app.try_state::<Arc<Updates>>())?.install_on_quit(enabled)
}

/// Quits IMT, installs the verified update and starts the new version.
#[tauri::command]
pub fn updates_restart<R: Runtime>(app: AppHandle<R>) -> Result<(), UpdateError> {
    updates_state(&app.try_state::<Arc<Updates>>())?.prepare_restart()?;
    log::info!("Restarting to update");
    crate::window_manager::quit(&app);
    Ok(())
}

/// Opens the releases page in the browser (only that fixed address).
#[tauri::command]
pub fn updates_open_releases() -> crate::CmdResult<()> {
    open::that_detached(RELEASES_PAGE).map_err(crate::err)
}
