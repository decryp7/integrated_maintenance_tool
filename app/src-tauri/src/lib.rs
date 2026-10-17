//! Tauri shell: exposes the imt-core workspace to the Angular UI. The command
//! names and payloads are documented in app/src/app/core/api.types.ts.

mod clef;
mod diagnostics;
mod engine_errors;
mod insights;
mod lifecycle;
mod live;
mod settings;
mod snapshots;
mod system;
mod tools;

use imt_core::project::ProjectFile;
use imt_core::workspace::{
    ChartRequest, ContextRequest, ExportRequest, ExtractPreviewRequest, FacetRequest, PageRequest, TimelineRequest, WorkspaceState,
};
use imt_core::{LogStore, Profile, Workspace, DEFAULT_PROFILE_TOML};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::Manager;

const PROFILE_FILE: &str = "active.imtprofile.toml";

pub(crate) struct AppState {
    workspace: Arc<RwLock<Box<dyn LogStore>>>,
    profile: RwLock<ProfileDoc>,
    profile_dir: Option<PathBuf>,
    /// Paths to open that arrived before or outside the UI (launch arguments,
    /// files opened from Finder/Explorer); the UI takes them after `imt://open`.
    pending: std::sync::Mutex<Vec<String>>,
    /// Import progress, shared with the store (no lock needed to read or cancel).
    progress: std::sync::Arc<imt_core::workspace::ImportProgress>,
    /// The app's configuration folder (plugin settings).
    config_dir: Option<PathBuf>,
    /// File watcher while live tail is on.
    watcher: std::sync::Mutex<Option<notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>>>,
}

impl AppState {
    /// The store for reading. A panic during an earlier command must not make
    /// every later command fail, so a poisoned lock is recovered.
    pub(crate) fn store_read(&self) -> std::sync::RwLockReadGuard<'_, Box<dyn LogStore>> {
        self.workspace.read().unwrap_or_else(|p| {
            log::warn!("recovering the workspace lock after a crash");
            p.into_inner()
        })
    }

    pub(crate) fn store_write(&self) -> std::sync::RwLockWriteGuard<'_, Box<dyn LogStore>> {
        self.workspace.write().unwrap_or_else(|p| {
            log::warn!("recovering the workspace lock after a crash");
            p.into_inner()
        })
    }
}

/// Event telling the UI that live tail changed the workspace.
const CHANGED_EVENT: &str = "imt://workspace-changed";

/// Event telling the UI that `take_pending_open` has paths for it.
const OPEN_EVENT: &str = "imt://open";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileDoc {
    name: String,
    path: Option<String>,
    toml: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileValidation {
    ok: bool,
    error: Option<String>,
    line: Option<usize>,
}

pub(crate) type CmdResult<T> = Result<T, String>;
pub(crate) use tauri::State;

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// Runs read-only work on the blocking pool so large scans never stall the UI thread.
pub(crate) async fn read<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&dyn LogStore) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let ws = state.workspace.clone();
    tauri::async_runtime::spawn_blocking(move || f(ws.read().unwrap_or_else(|p| p.into_inner()).as_ref()).map_err(err))
        .await
        .map_err(err)?
}

async fn write<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&mut dyn LogStore) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let ws = state.workspace.clone();
    tauri::async_runtime::spawn_blocking(move || f(ws.write().unwrap_or_else(|p| p.into_inner()).as_mut()).map_err(err))
        .await
        .map_err(err)?
}

/// Like [`write`], and tells the UI (`imt://error`) about plugin crashes,
/// quarantines and import crashes the engine contained meanwhile.
async fn write_reporting<R: tauri::Runtime, T: Send + 'static>(
    app: &tauri::AppHandle<R>,
    state: &State<'_, AppState>,
    f: impl FnOnce(&mut dyn LogStore) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let (out, errors) = write(state, move |ws| {
        let before = engine_errors::HealthSnapshot::of(&ws.summary());
        let out = f(ws);
        let errors = engine_errors::new_errors(&before, &engine_errors::HealthSnapshot::of(&ws.summary()));
        Ok((out, errors))
    })
    .await?;
    for e in &errors {
        snapshots::emit_engine_error(app, e);
    }
    out.map_err(err)
}

#[tauri::command]
async fn workspace_summary(state: State<'_, AppState>) -> CmdResult<Value> {
    read(&state, |ws| serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)).await
}

#[tauri::command]
async fn open_paths<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<Value> {
    write_reporting(&app, &state, move |ws| {
        ws.open(&paths.into_iter().map(PathBuf::from).collect::<Vec<_>>())?;
        let summary = ws.summary();
        // Counts only: file names and contents stay out of the logs.
        log::info!(
            Sources = summary.sources.len() as u64,
            Unrecognized = summary.unrecognized.len() as u64,
            Events = summary.total as u64,
            ElapsedMs = summary.load_ms;
            "Workspace opened"
        );
        serde_json::to_value(summary).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
async fn remove_source(state: State<'_, AppState>, source_id: u32) -> CmdResult<Value> {
    write(&state, move |ws| {
        ws.remove_source(source_id);
        serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
async fn clear_workspace(state: State<'_, AppState>) -> CmdResult<Value> {
    write(&state, |ws| {
        ws.clear();
        serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
async fn reload_workspace<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: State<'_, AppState>) -> CmdResult<Value> {
    write_reporting(&app, &state, |ws| {
        ws.reload()?;
        serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
async fn query_page(state: State<'_, AppState>, request: PageRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.page(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
async fn query_timeline(state: State<'_, AppState>, request: TimelineRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.timeline(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
async fn query_facet(state: State<'_, AppState>, request: FacetRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.facet(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
async fn query_chart(state: State<'_, AppState>, request: ChartRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.chart(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
async fn preview_extractor(state: State<'_, AppState>, request: ExtractPreviewRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.preview_extractor(&request)).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
fn get_profile(state: State<'_, AppState>) -> CmdResult<ProfileDoc> {
    Ok(state.profile.read().map_err(err)?.clone())
}

/// Parses and compiles a profile against the installed plugins.
#[tauri::command]
fn validate_profile(state: State<'_, AppState>, toml: String) -> CmdResult<ProfileValidation> {
    let store = state.store_read();
    Ok(validate(store.as_ref(), &toml))
}

fn validate(store: &dyn LogStore, toml: &str) -> ProfileValidation {
    match Profile::from_toml(toml) {
        Err(e) => ProfileValidation {
            ok: false,
            error: Some(e.message),
            line: e.line,
        },
        Ok(p) => match store.validate_profile(p) {
            Ok(_) => ProfileValidation {
                ok: true,
                error: None,
                line: None,
            },
            Err(e) => ProfileValidation {
                ok: false,
                line: find_line(toml, &e),
                error: Some(err(e)),
            },
        },
    }
}

/// Points compile errors ("extract \"stats\": regex") at the rule's line.
fn find_line(toml: &str, e: &anyhow::Error) -> Option<usize> {
    let msg = e.to_string();
    let name = msg.split('"').nth(1)?;
    toml.lines().position(|l| l.contains(&format!("\"{name}\""))).map(|i| i + 1)
}

#[tauri::command]
async fn save_profile(state: State<'_, AppState>, toml: String) -> CmdResult<Value> {
    let profile = Profile::from_toml(&toml).map_err(err)?;
    // Never replace a working profile on disk with one that does not compile.
    state.store_read().validate_profile(profile.clone()).map_err(err)?;
    let name = profile.name.clone();
    let path = state.profile_dir.as_ref().map(|d| d.join(PROFILE_FILE));
    if let Some(p) = &path {
        std::fs::create_dir_all(p.parent().unwrap()).map_err(err)?;
        archive_profile(p);
        std::fs::write(p, &toml).map_err(err)?;
    }
    *state.profile.write().map_err(err)? = ProfileDoc {
        name,
        path: path.map(|p| p.to_string_lossy().into_owned()),
        toml,
    };
    write(&state, move |ws| {
        ws.set_profile(profile)?;
        serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)
    })
    .await
}

/// Saved profile versions kept for "restore".
const PROFILE_HISTORY: usize = 20;

/// Copies the current profile into `history/` before it is replaced.
fn archive_profile(active: &std::path::Path) {
    let Some(dir) = active.parent().map(|d| d.join("history")) else {
        return;
    };
    if !active.exists() || std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S%3f");
    let _ = std::fs::copy(active, dir.join(format!("{stamp}.imtprofile.toml")));
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    for old in files.iter().rev().skip(PROFILE_HISTORY) {
        let _ = std::fs::remove_file(old);
    }
}

/// Previous profile versions, newest first: `[{ file, name, savedAt }]`.
#[tauri::command]
fn profile_history(state: State<'_, AppState>) -> Vec<Value> {
    let Some(dir) = state.profile_dir.as_ref().map(|d| d.join("history")) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    files
        .iter()
        .rev()
        .map(|f| {
            let toml = std::fs::read_to_string(f).unwrap_or_default();
            let name = Profile::from_toml(&toml).map(|p| p.name).unwrap_or_else(|_| "(unreadable)".into());
            let stem = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            serde_json::json!({ "file": stem, "name": name, "savedAt": stem.split('.').next().unwrap_or("") })
        })
        .collect()
}

/// One previous version's text (not applied until saved again).
#[tauri::command]
fn profile_version(state: State<'_, AppState>, file: String) -> CmdResult<ProfileDoc> {
    if file.contains(['/', '\\']) || file.contains("..") {
        return Err("invalid version".into());
    }
    let dir = state.profile_dir.as_ref().ok_or("no profile folder")?.join("history");
    let toml = std::fs::read_to_string(dir.join(&file)).map_err(err)?;
    let name = Profile::from_toml(&toml).map_err(err)?.name;
    Ok(ProfileDoc { name, path: None, toml })
}

/// Progress of the running import (readable while the import holds the workspace).
#[tauri::command]
fn import_progress(state: State<'_, AppState>) -> imt_core::workspace::ProgressSnapshot {
    state.progress.snapshot()
}

/// Stops the running import after the files in flight; files not read are not added.
#[tauri::command]
fn cancel_import(state: State<'_, AppState>) {
    log::info!("Import cancelled by the user");
    state.progress.cancel();
}

/// The automatic health check: findings, most severe first.
#[tauri::command]
async fn overview(state: State<'_, AppState>, request: imt_core::api::QueryRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.overview(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

/// Writes the events matching the query to a file chosen by the user.
#[tauri::command]
async fn export_events(state: State<'_, AppState>, request: ExportRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        let result = ws.export(&request)?;
        log::info!(Format = result.format.as_str(), Events = result.events as u64; "Events exported");
        serde_json::to_value(result).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
async fn source_context(state: State<'_, AppState>, request: ContextRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.source_context(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

/// Saves the open files, the active profile and the UI's state to a `.imtproj`.
#[tauri::command]
async fn save_project(state: State<'_, AppState>, path: String, name: String, ui: Value) -> CmdResult<Value> {
    let profile = state.profile.read().map_err(err)?.toml.clone();
    let ws_state = state.store_read().state();
    let project = ProjectFile {
        version: 1,
        name,
        sources: ws_state.roots,
        removed: ws_state.removed,
        profile,
        ui,
    };
    project.save(std::path::Path::new(&path)).map_err(err)?;
    log::info!(Sources = project.sources.len() as u64; "Project saved");
    Ok(serde_json::json!({ "path": path, "name": project.name }))
}

/// Opens a `.imtproj`: applies its profile, reopens its files and returns the UI state.
#[tauri::command]
async fn open_project<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: State<'_, AppState>, path: String) -> CmdResult<Value> {
    let project = ProjectFile::load(std::path::Path::new(&path)).map_err(err)?;
    let profile = Profile::from_toml(&project.profile).map_err(err)?;
    let doc = ProfileDoc {
        name: profile.name.clone(),
        path: None,
        toml: project.profile.clone(),
    };
    let ws_state = WorkspaceState {
        roots: project.sources.clone(),
        removed: project.removed.clone(),
    };
    let (summary, missing) = write_reporting(&app, &state, move |ws| {
        let missing = ws.restore(profile, &ws_state)?;
        Ok((serde_json::to_value(ws.summary())?, missing))
    })
    .await?;
    *state.profile.write().map_err(err)? = doc.clone();
    log::info!(Sources = project.sources.len() as u64, Missing = missing.len() as u64; "Project opened");
    Ok(serde_json::json!({
        "path": path,
        "name": project.name,
        "summary": summary,
        "profile": doc,
        "ui": project.ui,
        "missing": missing,
    }))
}

/// Reads a profile file chosen by the user (not applied until saved).
#[tauri::command]
fn import_profile(path: String) -> CmdResult<ProfileDoc> {
    let toml = std::fs::read_to_string(&path).map_err(err)?;
    let profile = Profile::from_toml(&toml).map_err(err)?;
    Ok(ProfileDoc {
        name: profile.name,
        path: Some(path),
        toml,
    })
}

#[tauri::command]
fn export_profile(path: String, toml: String) -> CmdResult<()> {
    std::fs::write(path, toml).map_err(err)
}

/// The saved profile if it still compiles against the installed plugins,
/// else the built-in default.
fn load_profile(dir: Option<&PathBuf>, store: &dyn LogStore) -> (Profile, ProfileDoc) {
    if let Some(path) = dir.map(|d| d.join(PROFILE_FILE)) {
        if let Ok(toml) = std::fs::read_to_string(&path) {
            match Profile::from_toml(&toml) {
                Ok(p) if store.validate_profile(p.clone()).is_ok() => {
                    let doc = ProfileDoc {
                        name: p.name.clone(),
                        path: Some(path.to_string_lossy().into_owned()),
                        toml,
                    };
                    return (p, doc);
                }
                _ => log::warn!("ignoring invalid profile {}", path.display()),
            }
        }
    }
    let p = Profile::default_profile();
    let doc = ProfileDoc {
        name: p.name.clone(),
        path: None,
        toml: DEFAULT_PROFILE_TOML.to_string(),
    };
    (p, doc)
}

/// Logs go to rolling files in the app's log directory (and stdout) as CLEF,
/// Serilog's compact JSON, including events forwarded from the UI. File size,
/// count and level come from the user's diagnostics settings.
fn log_plugin<R: tauri::Runtime>(limits: &diagnostics::LogLimits) -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
    tauri_plugin_log::Builder::new()
        .targets([
            Target::new(TargetKind::LogDir {
                file_name: Some(diagnostics::LOG_NAME.into()),
            }),
            Target::new(TargetKind::Stdout),
        ])
        // Everything reaches the dispatcher; `log::set_max_level` applies the
        // configured level, so it can change without a restart.
        .level(log::LevelFilter::Trace)
        .rotation_strategy(RotationStrategy::KeepSome(limits.keep_files as usize))
        .max_file_size(u128::from(limits.max_file_mb) * 1024 * 1024)
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}",
                clef::format(chrono::Utc::now(), record, &message.to_string(), &clef::Enrichment::current())
            ))
        })
        .build()
}

/// Queues paths for the UI and tells it to take them.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
fn queue_open<R: tauri::Runtime>(app: &tauri::AppHandle<R>, paths: Vec<String>) {
    use tauri::Emitter;
    if paths.is_empty() {
        return;
    }
    if let Ok(mut pending) = app.state::<AppState>().pending.lock() {
        pending.extend(paths);
    }
    let _ = app.emit(OPEN_EVENT, ());
}

/// Loads the saved profile and opens the paths given on the command line.
/// Loads the saved profile; paths given on the command line are queued for
/// the UI, which opens them after the window is shown.
#[cfg(test)]
fn app_state(profile_dir: Option<PathBuf>, args: &[PathBuf]) -> anyhow::Result<AppState> {
    app_state_with(profile_dir, args, false)
}

/// In safe mode no plugin is installed and the saved profile is not loaded.
fn app_state_with(profile_dir: Option<PathBuf>, args: &[PathBuf], safe_mode: bool) -> anyhow::Result<AppState> {
    let config_dir = profile_dir.as_ref().and_then(|d| d.parent()).map(PathBuf::from);
    let settings = lifecycle::load_plugin_settings(config_dir.as_deref());
    let mut workspace = Workspace::with_registry(Profile::default_profile(), lifecycle::plugins(safe_mode, &settings.disabled)?)?;
    let (profile, doc) = if safe_mode {
        let p = Profile::default_profile();
        let doc = ProfileDoc {
            name: p.name.clone(),
            path: None,
            toml: DEFAULT_PROFILE_TOML.to_string(),
        };
        (p, doc)
    } else {
        load_profile(profile_dir.as_ref(), &workspace)
    };
    workspace.set_profile(profile)?;
    let pending = args
        .iter()
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let progress = workspace.progress();
    let workspace: Box<dyn LogStore> = Box::new(workspace);
    Ok(AppState {
        workspace: Arc::new(RwLock::new(workspace)),
        profile: RwLock::new(doc),
        profile_dir,
        config_dir,
        pending: std::sync::Mutex::new(pending),
        watcher: std::sync::Mutex::new(None),
        progress,
    })
}

/// Turns live tail on or off. While on, the opened files and folders are
/// watched (OS events, or polling for network shares and after a watcher
/// error); appended lines are read incrementally, rotations and truncations
/// followed, and `imt://workspace-changed` is emitted with a
/// [`imt_core::workspace::TailReport`] payload (at most about four per
/// second); live alerts are emitted as `imt://live-alert`. Call again after
/// opening more files to watch them too. Returns how many folders are watched.
#[tauri::command]
fn set_live<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    enabled: bool,
    options: Option<live::LiveOptions>,
) -> CmdResult<usize> {
    // The engine's live session replaces the old debouncer.
    *state.watcher.lock().map_err(err)? = None;
    live::set(&app, &state, enabled, options.unwrap_or_default())
}

/// Reads what changed in `paths`; a failure is logged and reported as no change.
fn apply_changes(state: &AppState, paths: &[PathBuf]) -> imt_core::workspace::TailReport {
    state.store_write().refresh_tail(paths).unwrap_or_else(|e| {
        log::warn!("live tail refresh failed: {e:#}");
        imt_core::workspace::TailReport::default()
    })
}

/// Paths waiting to be opened (launch arguments, files opened from the OS).
/// `.imtproj` files should be opened with `open_project`, others with `open_paths`.
#[tauri::command]
fn take_pending_open(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    Ok(std::mem::take(&mut *state.pending.lock().map_err(err)?))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let diag = diagnostics::load(app.path().app_config_dir().ok().as_deref());
            app.handle().plugin(log_plugin(&diag.log))?;
            log::set_max_level(diag.log.level.filter());
            if let Ok(dir) = app.path().app_log_dir() {
                diagnostics::prune_old_logs(&dir, diag.log.max_age_days, std::time::SystemTime::now());
            }
            let profile_dir = app.path().app_config_dir().ok().map(|d| d.join("profiles"));
            // `imt-app <paths...>` opens files and folders at startup.
            let raw: Vec<String> = std::env::args().skip(1).collect();
            let safe_mode = raw.iter().any(|a| a == lifecycle::SAFE_MODE_FLAG) || std::env::var_os("IMT_SAFE_MODE").is_some();
            let args: Vec<PathBuf> = raw.iter().filter(|a| !a.starts_with('-')).map(PathBuf::from).collect();
            lifecycle::install_panic_logging();
            let life = lifecycle::Lifecycle::begin(app.path().app_data_dir().ok(), safe_mode);
            log::info!(Version = env!("CARGO_PKG_VERSION"), SafeMode = safe_mode, PreviousCrash = life.previous_crash; "Application starting — IMT {}", env!("CARGO_PKG_VERSION"));
            app.manage(app_state_with(profile_dir, &args, safe_mode)?);
            app.manage(life);
            app.manage(tools::Tools::default());
            settings::init(app.handle());
            snapshots::prune_on_startup(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace_summary,
            open_paths,
            remove_source,
            clear_workspace,
            reload_workspace,
            query_page,
            query_timeline,
            query_facet,
            query_chart,
            preview_extractor,
            export_events,
            source_context,
            take_pending_open,
            set_live,
            live::tail_status,
            live::open_paths_live,
            insights::run_query,
            insights::check_query,
            insights::complete_query,
            insights::query_patterns,
            insights::query_exceptions,
            insights::query_anomalies,
            insights::compare_windows,
            insights::field_stats,
            insights::correlate,
            insights::query_traces,
            insights::query_requests,
            insights::query_moment,
            insights::preview_parse,
            insights::infer_regex,
            insights::explain_regex,
            snapshots::save_error_snapshot,
            snapshots::list_error_snapshots,
            snapshots::read_error_snapshot,
            snapshots::delete_error_snapshot,
            snapshots::open_snapshots_folder,
            snapshots::export_diagnostics,
            system::elevation,
            system::restart_elevated,
            system::read_file_head,
            tools::perfmon_presets,
            tools::perfmon_generate,
            tools::perfmon_install,
            tools::perfmon_control,
            tools::perfmon_outputs,
            tools::monitor_start,
            tools::monitor_stop,
            tools::monitor_status,
            tools::collect_default_plan,
            tools::collect_diagnostics,
            profile_history,
            profile_version,
            import_progress,
            cancel_import,
            overview,
            lifecycle::app_status,
            lifecycle::restart_app,
            lifecycle::plugin_reset,
            lifecycle::plugin_set_enabled,
            lifecycle::save_session,
            lifecycle::last_session,
            save_project,
            open_project,
            get_profile,
            validate_profile,
            save_profile,
            import_profile,
            export_profile,
            diagnostics::diagnostics_settings,
            diagnostics::set_diagnostics_settings,
            settings::settings_sections,
            settings::set_settings,
            settings::reset_settings,
            settings::export_settings,
            settings::import_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building IMT")
        .run(|app, event| match event {
            tauri::RunEvent::Exit => {
                if let Some(life) = app.try_state::<lifecycle::Lifecycle>() {
                    life.end();
                }
                log::info!("Application is shutting down")
            }
            // Files opened from Finder (double-click, "Open with").
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            tauri::RunEvent::Opened { urls } => {
                let paths: Vec<String> = urls
                    .iter()
                    .filter_map(|u| u.to_file_path().ok())
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                queue_open(app, paths);
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::query::Query;
    use serde_json::json;
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

    fn samples() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let p = std::env::temp_dir().join(format!("imt-app-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An app whose workspace has `paths` opened, as the UI does after startup.
    fn opened(dir: Option<PathBuf>, paths: &[PathBuf]) -> tauri::App<MockRuntime> {
        let app = app(app_state(dir, paths).unwrap());
        let pending = take_pending_open(app.state::<AppState>()).unwrap();
        block(open_paths(app.handle().clone(), app.state::<AppState>(), pending)).unwrap();
        app
    }

    fn app(state: AppState) -> tauri::App<MockRuntime> {
        mock_builder().manage(state).build(mock_context(noop_assets())).unwrap()
    }

    fn block<F: std::future::Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn commands_round_trip() {
        let dir = TempDir::new("cmds");
        let app = opened(
            Some(dir.0.clone()),
            &[samples().join("serilog-clef"), PathBuf::from("/does/not/exist")],
        );
        let st = || app.state::<AppState>();

        let summary = block(workspace_summary(st())).unwrap();
        assert_eq!(summary["sources"].as_array().unwrap().len(), 2);
        assert_eq!(summary["profileName"], "Default (Serilog)");

        let summary = block(open_paths(
            app.handle().clone(),
            st(),
            vec![samples().join("log4net").to_string_lossy().into_owned()],
        ))
        .unwrap();
        assert_eq!(summary["sources"].as_array().unwrap().len(), 3);
        assert!(block(open_paths(app.handle().clone(), st(), vec!["/does/not/exist".into()]))
            .unwrap_err()
            .contains("does/not/exist"));

        let page = block(query_page(
            st(),
            PageRequest {
                query: Query::default(),
                offset: 0,
                limit: 5,
                descending: false,
                anchor: None,
            },
        ))
        .unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(), 5);
        assert!(page["rows"][0]["sourceName"].is_string());
        let bad = Query {
            text: "(".into(),
            regex: true,
            ..Default::default()
        };
        assert!(block(query_page(
            st(),
            PageRequest {
                query: bad,
                offset: 0,
                limit: 5,
                descending: false,
                anchor: None
            }
        ))
        .is_err());

        let tl: TimelineRequest = serde_json::from_value(json!({"buckets": 50})).unwrap();
        assert!(!block(query_timeline(st(), tl)).unwrap()["lanes"].as_array().unwrap().is_empty());
        let facet: FacetRequest = serde_json::from_value(json!({"field": "MachineName"})).unwrap();
        assert_eq!(block(query_facet(st(), facet)).unwrap()["distinct"], 2);
        let chart: ChartRequest = serde_json::from_value(json!({"chartId": "events-by-level"})).unwrap();
        assert!(block(query_chart(st(), chart)).unwrap()["series"].is_array());
        let preview: ExtractPreviewRequest = serde_json::from_value(json!({"toml": DEFAULT_PROFILE_TOML, "extractor": "x"})).unwrap();
        assert!(block(preview_extractor(st(), preview)).unwrap()["error"].is_string());

        let id = summary["sources"][0]["id"].as_u64().unwrap() as u32;
        let summary = block(remove_source(st(), id)).unwrap();
        assert_eq!(summary["sources"].as_array().unwrap().len(), 2);
        let summary = block(reload_workspace(app.handle().clone(), st())).unwrap();
        assert_eq!(summary["sources"].as_array().unwrap().len(), 2);
        let summary = block(clear_workspace(st())).unwrap();
        assert_eq!(summary["total"], 0);
    }

    #[test]
    fn profiles_are_validated_saved_and_reloaded() {
        let dir = TempDir::new("profiles");
        let app = app(app_state(Some(dir.0.clone()), &[]).unwrap());
        let st = || app.state::<AppState>();

        let doc = get_profile(st()).unwrap();
        assert_eq!((doc.path, doc.name.as_str()), (None, "Default (Serilog)"));

        let store = st().workspace.clone();
        let store = store.read().unwrap();
        let validate = |t: &str| validate(store.as_ref(), t);
        let ok = validate("name = 'x'");
        assert!(ok.ok && ok.error.is_none());
        let syntax = validate("name = 'x'\n[parse\n");
        assert_eq!((syntax.ok, syntax.line), (false, Some(2)));
        let semantic = validate("name = 'x'\n\n[[extract]]\nname = \"stats\"\nregex = '('\n");
        assert!(!semantic.ok);
        assert_eq!(semantic.line, Some(4));
        let unlocated = validate("[parse]\nassume_timezone = 'Mars'");
        assert!(validate_profile(st(), "name = 'y'".into()).unwrap().ok);
        drop(store);
        assert_eq!((unlocated.ok, unlocated.line), (false, None));

        assert!(block(save_profile(st(), "[parse".into())).is_err());
        let summary = block(save_profile(st(), "name = 'Saved'".into())).unwrap();
        assert_eq!(summary["profileName"], "Saved");
        let doc = get_profile(st()).unwrap();
        assert_eq!(doc.name, "Saved");
        let saved = PathBuf::from(doc.path.unwrap());
        assert!(saved.exists());

        // A fresh start picks the saved profile up; an invalid one falls back to the default.
        assert_eq!(
            load_profile(Some(&dir.0), &Workspace::new(Profile::default_profile()).unwrap())
                .1
                .name,
            "Saved"
        );
        std::fs::write(&saved, "[parse]\nregexes = ['(']").unwrap();
        assert_eq!(
            load_profile(Some(&dir.0), &Workspace::new(Profile::default_profile()).unwrap())
                .1
                .name,
            "Default (Serilog)"
        );
        assert_eq!(
            load_profile(None, &Workspace::new(Profile::default_profile()).unwrap()).1.path,
            None
        );

        let exported = dir.0.join("out.imtprofile.toml");
        export_profile(exported.to_string_lossy().into_owned(), "name = 'Exported'".into()).unwrap();
        let imported = import_profile(exported.to_string_lossy().into_owned()).unwrap();
        assert_eq!(imported.name, "Exported");
        assert!(import_profile(dir.0.join("missing.toml").to_string_lossy().into_owned()).is_err());
        std::fs::write(&exported, "[parse").unwrap();
        assert!(import_profile(exported.to_string_lossy().into_owned()).is_err());
        assert!(export_profile(dir.0.join("no/such/dir/x.toml").to_string_lossy().into_owned(), String::new()).is_err());
    }

    #[test]
    fn saving_without_a_config_dir_still_applies() {
        let app = app(app_state(None, &[]).unwrap());
        let summary = block(save_profile(app.state::<AppState>(), "name = 'Memory only'".into())).unwrap();
        assert_eq!(summary["profileName"], "Memory only");
        assert_eq!(get_profile(app.state::<AppState>()).unwrap().path, None);
    }

    #[test]
    fn plugins_and_export() {
        let dir = TempDir::new("export");
        let app = opened(Some(dir.0.clone()), &[samples().join("unsupported"), samples().join("csv")]);
        let st = || app.state::<AppState>();
        let summary = block(workspace_summary(st())).unwrap();
        assert!(summary["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "net.decryptology.registry"));
        assert!(summary["sources"].as_array().unwrap().iter().any(|s| s["importer"] == "registry"));
        assert_eq!(summary["exporters"].as_array().unwrap().len(), 2);

        let out = dir.0.join("events.clef");
        let req: ExportRequest = serde_json::from_value(json!({"format": "clef", "path": out.to_string_lossy()})).unwrap();
        let res = block(export_events(st(), req)).unwrap();
        assert_eq!(res["events"], summary["total"]);
        assert!(std::fs::read_to_string(&out).unwrap().lines().count() > 0);
        let bad: ExportRequest = serde_json::from_value(json!({"format": "pdf", "path": "x"})).unwrap();
        assert!(block(export_events(st(), bad)).unwrap_err().contains("unknown export format"));
        assert_eq!(lifecycle::plugins(false, &[]).unwrap().plugins().len(), 5);
    }

    #[test]
    fn projects_and_context() {
        let dir = TempDir::new("project");
        let app = opened(Some(dir.0.clone()), &[samples().join("log4net")]);
        let st = || app.state::<AppState>();
        block(save_profile(st(), "name = 'Project profile'".into())).unwrap();
        let file = dir.0.join("incident.imtproj").to_string_lossy().into_owned();
        let saved = block(save_project(st(), file.clone(), "Incident".into(), json!({"bookmarks": [3]}))).unwrap();
        assert_eq!(saved["name"], "Incident");

        let ctx: ContextRequest = serde_json::from_value(json!({"recordId": 5, "before": 1, "after": 1})).unwrap();
        assert_eq!(block(source_context(st(), ctx)).unwrap()["lines"].as_array().unwrap().len(), 3);

        block(clear_workspace(st())).unwrap();
        block(save_profile(st(), "name = 'Other'".into())).unwrap();
        let opened = block(open_project(app.handle().clone(), st(), file.clone())).unwrap();
        assert_eq!(opened["name"], "Incident");
        assert_eq!(opened["ui"]["bookmarks"][0], 3);
        assert_eq!(opened["profile"]["name"], "Project profile");
        assert_eq!(opened["summary"]["profileName"], "Project profile");
        assert!(opened["summary"]["total"].as_u64().unwrap() > 0);
        assert_eq!(opened["missing"].as_array().unwrap().len(), 0);
        assert_eq!(get_profile(st()).unwrap().name, "Project profile");
        assert!(block(open_project(
            app.handle().clone(),
            st(),
            dir.0.join("nope.imtproj").to_string_lossy().into_owned()
        ))
        .is_err());
        std::fs::write(
            dir.0.join("bad.imtproj"),
            r#"{"version":1,"name":"x","sources":[],"profile":"[parse"}"#,
        )
        .unwrap();
        assert!(block(open_project(
            app.handle().clone(),
            st(),
            dir.0.join("bad.imtproj").to_string_lossy().into_owned()
        ))
        .is_err());
        assert!(block(save_project(st(), "/no/such/dir/x.imtproj".into(), "x".into(), Value::Null)).is_err());
    }

    #[test]
    fn pending_open_queue() {
        let app = app(app_state(None, &[samples().join("csv"), PathBuf::from("/does/not/exist")]).unwrap());
        assert_eq!(
            take_pending_open(app.state::<AppState>()).unwrap().len(),
            1,
            "missing paths are not queued"
        );
        assert!(take_pending_open(app.state::<AppState>()).unwrap().is_empty());
        queue_open(app.handle(), vec!["/a/b.imtproj".into()]);
        queue_open(app.handle(), vec![]);
        assert_eq!(take_pending_open(app.state::<AppState>()).unwrap(), ["/a/b.imtproj"]);
    }

    #[test]
    fn live_tail() {
        let dir = TempDir::new("live");
        std::fs::write(dir.0.join("a.log"), "2026-10-14 10:00:00 INFO first\n").unwrap();
        let app = opened(None, std::slice::from_ref(&dir.0));
        let st = || app.state::<AppState>();
        assert_eq!(set_live(app.handle().clone(), st(), true, None).unwrap(), 1);
        assert!(app.state::<live::LiveSlot>().is_running());
        std::fs::write(
            dir.0.join("a.log"),
            "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:01 INFO second\n",
        )
        .unwrap();
        let report = apply_changes(&st(), &[dir.0.join("a.log")]);
        assert_eq!((report.appended, report.sources_changed.len()), (1, 1));
        live::emit_report(app.handle(), &report);
        live::emit_report(app.handle(), &imt_core::workspace::TailReport::default());
        assert_eq!(block(workspace_summary(st())).unwrap()["total"], 2);
        let status = block(live::tail_status(app.handle().clone(), st())).unwrap();
        assert_eq!(status["files"][0]["mode"], "events");
        assert_eq!(status["totals"]["records"], 2);
        assert_eq!(set_live(app.handle().clone(), st(), false, None).unwrap(), 0);
        assert!(!app.state::<live::LiveSlot>().is_running());
        assert!(st().watcher.lock().unwrap().is_none());
        let status = block(live::tail_status(app.handle().clone(), st())).unwrap();
        assert_eq!(status["files"][0]["mode"], "off");

        // Start at end: history skipped, new lines shown.
        let other = TempDir::new("live-end");
        std::fs::write(other.0.join("b.log"), "2026-10-14 10:00:00 INFO old\n").unwrap();
        let summary = block(live::open_paths_live(
            app.handle().clone(),
            st(),
            vec![other.0.to_string_lossy().into_owned()],
            Some(true),
        ))
        .unwrap();
        assert_eq!(summary["total"], 2);
        // A broken refresh is reported as no change.
        assert!(apply_changes(&st(), &[]).is_empty());
    }

    /// Smoke test with the real (polling) watcher: an appended line arrives on its own.
    #[test]
    fn live_tail_watcher_smoke() {
        let dir = TempDir::new("live-smoke");
        let file = dir.0.join("a.log");
        std::fs::write(&file, "2026-10-14 10:00:00 INFO first\n").unwrap();
        let app = opened(None, std::slice::from_ref(&dir.0));
        let st = || app.state::<AppState>();
        let options: live::LiveOptions = serde_json::from_value(serde_json::json!({ "watch": "polling", "pollIntervalMs": 100 })).unwrap();
        assert_eq!(set_live(app.handle().clone(), st(), true, Some(options)).unwrap(), 1);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        std::io::Write::write_all(&mut f, b"2026-10-14 10:00:01 INFO second\n").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while st().store_read().summary().total < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(st().store_read().summary().total, 2);
        let status = block(live::tail_status(app.handle().clone(), st())).unwrap();
        assert_eq!(status["files"][0]["mode"], "polling");
        set_live(app.handle().clone(), st(), false, None).unwrap();
    }

    #[test]
    fn profile_safety_history_progress_and_overview() {
        let dir = TempDir::new("history");
        let app = opened(Some(dir.0.clone()), &[samples().join("log4net")]);
        let st = || app.state::<AppState>();
        // An invalid profile is refused and nothing is written.
        assert!(block(save_profile(st(), "[[pipeline]]\nstep='plugin'\nid='nope'".into())).is_err());
        assert!(!dir.0.join(PROFILE_FILE).exists());
        for name in ["One", "Two", "Three"] {
            block(save_profile(st(), format!("name = '{name}'"))).unwrap();
        }
        let history = profile_history(st());
        assert_eq!(history.len(), 2, "the first save had nothing to archive");
        assert_eq!(history[0]["name"], "Two");
        let file = history[1]["file"].as_str().unwrap().to_string();
        assert_eq!(profile_version(st(), file).unwrap().name, "One");
        assert!(profile_version(st(), "../x".into()).is_err());

        let p = import_progress(st());
        assert!(!p.running && p.total > 0 && p.done == p.total);
        cancel_import(st());
        assert!(import_progress(st()).cancelled);
        let req: imt_core::api::QueryRequest = serde_json::from_value(json!({})).unwrap();
        assert!(block(overview(st(), req)).unwrap().is_array());
    }
}
