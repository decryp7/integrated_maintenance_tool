//! Settings ▸ Storage: what IMT keeps on disk, and clearing it.
//!
//! Categories ([`model::CategoryId`]): temporary copies of opened bundles, the webview's caches and
//! runtime builds of releases IMT no longer pins are `cache` (rebuilt when needed, cleared together by
//! "Clear cache"); the internal browser's profile, the AI runtimes and models, IMT's logs, error
//! snapshots, session backups and (once they exist) persistent stores are `data` (each cleared on its own,
//! after a confirmation in the UI).
//!
//! Folders come from [`crate::app_dirs`] (and `share::bundles_root` for the bundles); the UI only ever
//! names category ids, never paths. Sizes are computed off the UI thread with a bounded traversal
//! ([`walk`]); clearing runs as a background task (progress, stoppable from the tray) and deletes only
//! inside each category's canonical root, never follows links, skips and reports files in use, and keeps
//! today's log, this run's sessions and the AI build and model in use ([`plan`]).
//!
//! Commands: `storage_usage`, `storage_clear`, `storage_reveal`. The wire contract is
//! `app/src/app/settings/storage/storage.types.ts`.

mod model;
mod plan;
mod stores;
mod walk;

use crate::activity::{domain_event, OpId};
use crate::ai::{AiCore, AiState};
use crate::jobs::{Controls, Done, JobKind, JobProgress, JobResult, JobSpec};
use crate::{err, CmdResult};
use model::{CategoryClearResult, CategoryId, StorageClearResult, StorageUsage};
use plan::{AiPort, Category, Folders};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager, Runtime};
use walk::Budget;

/// The AI engine as the storage view sees it.
struct AiAdapter(Arc<AiCore>);

impl AiAdapter {
    fn state(&self) -> String {
        serde_json::to_value(self.0.status().state)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    }
}

impl AiPort for AiAdapter {
    fn engine_busy(&self) -> Option<String> {
        if self.0.setup_running() {
            return Some("a model is being set up; wait for it to finish".into());
        }
        matches!(self.state().as_str(), "ready" | "loading")
            .then(|| "in use by the AI engine: remove it in Settings ▸ AI, which stops the engine first".into())
    }

    fn setup_busy(&self) -> Option<String> {
        self.0
            .setup_running()
            .then(|| "a model is being set up; wait for it to finish".into())
    }

    fn custom_server(&self) -> Option<PathBuf> {
        self.0.settings().llama_server_path.map(PathBuf::from)
    }

    fn models_dir(&self) -> PathBuf {
        self.0.models_dir()
    }

    fn models(&self) -> Vec<(String, bool)> {
        self.0.models().into_iter().map(|m| (m.file_name, m.active)).collect()
    }

    fn delete_runtime(&self, tag: &str, id: &str) -> Result<(), String> {
        self.0.delete_runtime(tag, id).map_err(|e| e.message)
    }

    fn delete_model(&self, file_name: &str) -> Result<(), String> {
        self.0.delete_model(file_name).map_err(|e| e.message)
    }
}

fn ai_port<R: Runtime>(app: &AppHandle<R>) -> Option<AiAdapter> {
    app.try_state::<AiState>().map(|s| AiAdapter(s.0.clone()))
}

/// When this process started (whole seconds, rounded down), so files of this run are recognised.
fn session_start() -> Option<SystemTime> {
    crate::lifecycle::os_probe(std::process::id()).map(|secs| UNIX_EPOCH + Duration::from_secs(secs))
}

/// Resolves every folder of the categories (see the module docs).
fn folders<'a, R: Runtime>(app: &AppHandle<R>, ai: Option<&'a dyn AiPort>) -> Folders<'a> {
    let log = crate::app_dirs::log(app).ok();
    Folders {
        bundles: Some(crate::share::bundles_root()),
        webview: crate::app_dirs::webview_data(app).ok(),
        data: crate::app_dirs::data(app).ok(),
        active_log: log.as_deref().map(crate::logging::active_log_file),
        log,
        runtime_tag: imt_ai::runtimes::RuntimeCatalog::embedded().tag,
        session_start: session_start(),
        main_instance: app.try_state::<crate::instance::SingleInstance>().is_some_and(|s| s.0),
        // Without the lifecycle (tests) assume the worst: keep the previous run's sessions.
        previous_crash: app.try_state::<crate::lifecycle::Lifecycle>().is_none_or(|l| l.previous_crash),
        browser_open: app.webviews().keys().any(|l| crate::browser::is_browser_label(l)),
        ai,
    }
}

/// Every category's row, sized within one shared budget.
fn usage_of(categories: &[Category], budget: &mut Budget) -> StorageUsage {
    let rows: Vec<_> = categories.iter().map(|c| plan::usage(c, budget)).collect();
    StorageUsage {
        total_bytes: rows.iter().map(|r| r.bytes).sum(),
        capped: rows.iter().any(|r| r.capped),
        categories: rows,
    }
}

/// Clears `ids` among `categories` in order; `progress` gets the fraction done after each item.
fn clear_all(
    categories: &[Category],
    ids: &[CategoryId],
    ai: Option<&dyn AiPort>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> StorageClearResult {
    let chosen: Vec<&Category> = ids.iter().filter_map(|id| categories.iter().find(|c| c.id == *id)).collect();
    let total = chosen.iter().map(|c| c.items.len()).sum::<usize>().max(1);
    let mut done = 0usize;
    let mut results = Vec::new();
    for id in ids {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let result = match chosen.iter().find(|c| c.id == *id) {
            Some(c) => plan::clear(c, ai, cancel, &mut || {
                done += 1;
                progress(done as f32 / total as f32);
            }),
            // Not on this computer (e.g. no stores yet): nothing to clear.
            None => CategoryClearResult::new(*id),
        };
        results.push(result);
    }
    StorageClearResult {
        freed_bytes: results.iter().map(|r| r.freed_bytes).sum(),
        categories: results,
        cancelled: cancel.load(Ordering::Relaxed),
    }
}

/// The single clear slot (one clear at a time), managed on first use.
#[derive(Default)]
struct StorageJobs {
    running: Mutex<bool>,
}

/// Holds the slot while a clear runs; releases it when dropped. The clear itself is a maintenance job.
struct Job<R: Runtime> {
    app: AppHandle<R>,
    cancel: Arc<AtomicBool>,
}

impl<R: Runtime> Job<R> {
    fn begin(app: &AppHandle<R>) -> CmdResult<Job<R>> {
        if app.try_state::<StorageJobs>().is_none() {
            app.manage(StorageJobs::default());
        }
        let jobs = app.state::<StorageJobs>();
        let mut running = crate::sync::lock(&jobs.running);
        if *running {
            return Err("IMT is already clearing its storage; wait for it to finish".into());
        }
        *running = true;
        Ok(Job {
            app: app.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl<R: Runtime> Drop for Job<R> {
    fn drop(&mut self) {
        if let Some(jobs) = self.app.try_state::<StorageJobs>() {
            *crate::sync::lock(&jobs.running) = false;
        }
    }
}

/// What IMT stores, per category (sizes are lower bounds when `capped`).
#[tauri::command]
pub async fn storage_usage<R: Runtime>(app: AppHandle<R>) -> CmdResult<StorageUsage> {
    crate::error::lift(
        tauri::async_runtime::spawn_blocking(move || {
            let ai = ai_port(&app);
            let folders = folders(&app, ai.as_ref().map(|a| a as &dyn AiPort));
            usage_of(&plan::categories(&folders), &mut Budget::standard())
        })
        .await
        .map_err(err),
    )
}

/// Starts clearing the categories `ids` (validated here) as a background maintenance job and returns it at
/// once: Settings may be closed meanwhile. What was freed and what stayed, per category, is the job's result
/// (`jobs_wait`; "Finished · View" opens Settings ▸ Storage with it).
#[tauri::command]
pub async fn storage_clear<R: Runtime>(app: AppHandle<R>, op: OpId, ids: Vec<String>) -> CmdResult<u64> {
    let ids = plan::parse_ids(&ids)?;
    let job = Job::begin(&app)?;
    let started = Instant::now();
    let names = ids.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(",");
    let flag = job.cancel.clone();
    let spec = JobSpec::new(JobKind::Maintenance, "Clear IMT's storage");
    crate::error::lift(
        crate::jobs::spawn(
            &app,
            spec,
            Controls::cancel(move || flag.store(true, Ordering::Relaxed)),
            move |ctx| {
                let ai = ai_port(&job.app);
                let port = ai.as_ref().map(|a| a as &dyn AiPort);
                let folders = folders(&job.app, port);
                let r = clear_all(&plan::categories(&folders), &ids, port, &job.cancel, &mut |f| {
                    ctx.progress(JobProgress::fraction(f))
                });
                let freed = r.freed_bytes.to_string();
                let skipped = r.categories.iter().map(|c| c.skipped_count).sum::<u64>();
                let ms = started.elapsed().as_millis().to_string();
                domain_event(
                    "storage.cleared",
                    op.as_deref(),
                    &[
                        ("Categories", &names),
                        ("FreedBytes", &freed),
                        ("Skipped", &skipped.to_string()),
                        ("ElapsedMs", &ms),
                    ],
                );
                ctx.message(&cleared_line(r.freed_bytes, skipped));
                let cancelled = r.cancelled;
                let result = serde_json::to_value(&r).map_err(|e| e.to_string())?;
                Ok::<_, String>(Done::new(()).with_result(JobResult::Storage { result }).cancelled_if(cancelled))
            },
        )
        .map(|id| id.0),
    )
}

/// "Freed 12 MB; 3 items stayed": the line the job's message shows.
fn cleared_line(freed: u64, skipped: u64) -> String {
    let freed = imt_plugin_api::units::bytes(freed);
    match skipped {
        0 => format!("Freed {freed}."),
        1 => format!("Freed {freed}; 1 item stayed (in use or not allowed)."),
        n => format!(
            "Freed {freed}; {} items stayed (in use or not allowed).",
            imt_plugin_api::units::count(n)
        ),
    }
}

/// Shows a category's folder in the OS file manager; returns its path.
#[tauri::command]
pub async fn storage_reveal<R: Runtime>(app: AppHandle<R>, id: String) -> CmdResult<String> {
    let id = CategoryId::parse(&id).ok_or_else(|| "unknown storage category".to_string())?;
    let dir = tauri::async_runtime::spawn_blocking(move || {
        let ai = ai_port(&app);
        let folders = folders(&app, ai.as_ref().map(|a| a as &dyn AiPort));
        plan::categories(&folders).into_iter().find(|c| c.id == id).and_then(|c| c.shown)
    })
    .await
    .map_err(err)?
    .ok_or_else(|| "IMT cannot find this folder on this computer".to_string())?;
    if !dir.is_dir() {
        return Err(crate::CmdError::from(format!(
            "{} does not exist yet: IMT has not stored anything there",
            dir.display()
        )));
    }
    open_in_file_manager(&dir).map_err(|e| format!("could not open the folder ({e}); it is at {}", dir.display()))?;
    Ok(dir.to_string_lossy().into_owned())
}

/// Starts the OS file manager on `dir` without waiting (see [`crate::opener`]).
fn open_in_file_manager(dir: &std::path::Path) -> std::io::Result<()> {
    crate::opener::start(&crate::opener::open_folder(crate::opener::Os::CURRENT, dir))
}

#[cfg(test)]
mod tests;

// ---- store folders (housekeeping) ----

const POLICY_FILE: &str = "storage-policy.json";
const POLICY_VERSION: u32 = 1;

/// Whether an IMT process with this id runs (the owner of a scratch store folder).
fn process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    let p = sysinfo::Pid::from_u32(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[p]), true);
    sys.process(p).is_some()
}

/// Store folders the windows have open.
fn open_stores<R: Runtime>(app: &AppHandle<R>) -> Vec<PathBuf> {
    app.try_state::<crate::windows::Windows>()
        .map(|w| {
            w.all()
                .iter()
                .filter_map(|win| win.engine.opened())
                .map(|s| s.store_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// Store folders autosaved sessions reopen.
fn kept_stores<R: Runtime>(app: &AppHandle<R>) -> Vec<PathBuf> {
    app.try_state::<crate::lifecycle::Lifecycle>()
        .map(|l| l.session_stores())
        .unwrap_or_default()
}

fn store_root_of<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.try_state::<crate::AppState>().map(|s| s.store_root.clone())
}

/// The store folders: size, last use and why each is kept or may go.
#[tauri::command]
pub async fn storage_stores<R: Runtime>(app: AppHandle<R>) -> CmdResult<stores::StoreFolders> {
    let Some(root) = store_root_of(&app) else {
        return Ok(stores::StoreFolders::default());
    };
    let (open, kept) = (open_stores(&app), kept_stores(&app));
    crate::sync::blocking("listing the stores", move || {
        let needs = stores::Needs {
            root: &root,
            open: &open,
            kept: &kept,
            alive: &process_alive,
        };
        Ok::<_, crate::CmdError>(stores::list(&needs, &mut Budget::standard()))
    })
    .await
}

/// Removes the orphaned store folders `ids` (folder names from `storage_stores`); anything still needed stays, with why.
#[tauri::command]
pub async fn storage_stores_clear<R: Runtime>(app: AppHandle<R>, op: OpId, ids: Vec<String>) -> CmdResult<stores::StoreClearResult> {
    let job = Job::begin(&app)?;
    let Some(root) = store_root_of(&app) else {
        return Ok(stores::StoreClearResult::default());
    };
    let (open, kept) = (open_stores(&app), kept_stores(&app));
    let result = crate::sync::blocking("removing the stores", move || {
        let needs = stores::Needs {
            root: &root,
            open: &open,
            kept: &kept,
            alive: &process_alive,
        };
        let r = stores::clear(&needs, &ids, &job.cancel);
        drop(job);
        Ok::<_, crate::CmdError>(r)
    })
    .await?;
    domain_event(
        "storage.stores-cleared",
        op.as_deref(),
        &[
            ("Removed", &result.removed.len().to_string()),
            ("FreedBytes", &result.freed_bytes.to_string()),
        ],
    );
    Ok(result)
}

fn policy_store<R: Runtime>(app: &AppHandle<R>) -> crate::config_store::ConfigStore {
    crate::config_store::ConfigStore::new(crate::app_dirs::config(app).ok())
}

/// The automatic clean-up policy (off by default).
#[tauri::command]
pub async fn storage_policy<R: Runtime>(app: AppHandle<R>) -> CmdResult<stores::StoragePolicy> {
    Ok(policy_store(&app).load(POLICY_FILE, POLICY_VERSION))
}

/// Sets the automatic clean-up policy; refused with what to enter when out of range.
#[tauri::command]
pub async fn storage_set_policy<R: Runtime>(app: AppHandle<R>, policy: stores::StoragePolicy) -> CmdResult<stores::StoragePolicy> {
    let policy = policy.checked().map_err(crate::CmdError::invalid)?;
    policy_store(&app)
        .save(POLICY_FILE, POLICY_VERSION, &policy)
        .map_err(|e| crate::CmdError::new(crate::ErrorCode::Io, format!("cannot save the setting: {e}")))?;
    Ok(policy)
}

/// At start-up, after the sweep: removes orphaned store folders unused for the policy's number of days.
pub(crate) fn auto_clean_stores(config: Option<PathBuf>, root: &std::path::Path, kept: &[PathBuf]) {
    let policy: stores::StoragePolicy = crate::config_store::ConfigStore::new(config).load(POLICY_FILE, POLICY_VERSION);
    if policy.auto_clean_days == 0 {
        return;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    let needs = stores::Needs {
        root,
        open: &[],
        kept,
        alive: &process_alive,
    };
    let r = stores::clear_unused_for(&needs, policy.auto_clean_days, now, &AtomicBool::new(false));
    if !r.removed.is_empty() || !r.skipped.is_empty() {
        log::info!(Removed = r.removed.len() as u64, FreedBytes = r.freed_bytes, Kept = r.skipped.len() as u64; "Unused stores cleaned up");
    }
}
