//! Tauri shell: exposes the imt-core workspace to the Angular UI. The command
//! names and payloads are documented in app/src/app/core/api.types.ts.

mod clef;

use imt_core::project::ProjectFile;
use imt_core::workspace::{
    ChartRequest, ContextRequest, ExportRequest, ExtractPreviewRequest, FacetRequest, PageRequest, TimelineRequest, WorkspaceState,
};
use imt_core::{LogStore, PluginRegistry, Profile, Workspace, DEFAULT_PROFILE_TOML};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::{Manager, State};

const PROFILE_FILE: &str = "active.imtprofile.toml";

struct AppState {
    workspace: Arc<RwLock<Box<dyn LogStore>>>,
    profile: RwLock<ProfileDoc>,
    profile_dir: Option<PathBuf>,
    /// Paths to open that arrived before or outside the UI (launch arguments,
    /// files opened from Finder/Explorer); the UI takes them after `imt://open`.
    pending: std::sync::Mutex<Vec<String>>,
    /// File watcher while live tail is on.
    watcher: std::sync::Mutex<Option<notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>>>,
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

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// Runs read-only work on the blocking pool so large scans never stall the UI thread.
async fn read<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&dyn LogStore) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let ws = state.workspace.clone();
    tauri::async_runtime::spawn_blocking(move || f(ws.read().map_err(err)?.as_ref()).map_err(err))
        .await
        .map_err(err)?
}

async fn write<T: Send + 'static>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&mut dyn LogStore) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let ws = state.workspace.clone();
    tauri::async_runtime::spawn_blocking(move || f(ws.write().map_err(err)?.as_mut()).map_err(err))
        .await
        .map_err(err)?
}

#[tauri::command]
async fn workspace_summary(state: State<'_, AppState>) -> CmdResult<Value> {
    read(&state, |ws| serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)).await
}

#[tauri::command]
async fn open_paths(state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<Value> {
    write(&state, move |ws| {
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
async fn reload_workspace(state: State<'_, AppState>) -> CmdResult<Value> {
    write(&state, |ws| {
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
    let store = state.workspace.read().map_err(err)?;
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
    let name = profile.name.clone();
    let path = state.profile_dir.as_ref().map(|d| d.join(PROFILE_FILE));
    if let Some(p) = &path {
        std::fs::create_dir_all(p.parent().unwrap()).map_err(err)?;
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
    let ws_state = state.workspace.read().map_err(err)?.state();
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
async fn open_project(state: State<'_, AppState>, path: String) -> CmdResult<Value> {
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
    let (summary, missing) = write(&state, move |ws| {
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

/// Logs go to rotating files in the app's log directory (and stdout) as CLEF,
/// Serilog's compact JSON, including events forwarded from the UI.
fn log_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
    tauri_plugin_log::Builder::new()
        .targets([
            Target::new(TargetKind::LogDir {
                file_name: Some("imt".into()),
            }),
            Target::new(TargetKind::Stdout),
        ])
        .level(log::LevelFilter::Info)
        .rotation_strategy(RotationStrategy::KeepSome(5))
        .max_file_size(10 * 1024 * 1024)
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

/// The engine plus every compiled-in plugin. Add new plugins here.
fn plugins() -> anyhow::Result<PluginRegistry> {
    let mut registry = PluginRegistry::new();
    registry.install(&imt_plugin_registry::RegistryPlugin)?;
    Ok(registry)
}

/// Loads the saved profile and opens the paths given on the command line.
/// Loads the saved profile; paths given on the command line are queued for
/// the UI, which opens them after the window is shown.
fn app_state(profile_dir: Option<PathBuf>, args: &[PathBuf]) -> anyhow::Result<AppState> {
    let mut workspace = Workspace::with_registry(Profile::default_profile(), plugins()?)?;
    let (profile, doc) = load_profile(profile_dir.as_ref(), &workspace);
    workspace.set_profile(profile)?;
    let pending = args
        .iter()
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let workspace: Box<dyn LogStore> = Box::new(workspace);
    Ok(AppState {
        workspace: Arc::new(RwLock::new(workspace)),
        profile: RwLock::new(doc),
        profile_dir,
        pending: std::sync::Mutex::new(pending),
        watcher: std::sync::Mutex::new(None),
    })
}

/// Turns live tail on or off. While on, the opened files and folders are
/// watched; changed and new files are re-imported and `imt://workspace-changed`
/// is emitted. Call again after opening more files to watch them too.
/// Returns how many paths are watched.
#[tauri::command]
fn set_live<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: State<'_, AppState>, enabled: bool) -> CmdResult<usize> {
    use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
    let mut slot = state.watcher.lock().map_err(err)?;
    *slot = None;
    if !enabled {
        return Ok(0);
    }
    let roots = state.workspace.read().map_err(err)?.state().roots;
    let handle = app.clone();
    let mut debouncer = new_debouncer(
        std::time::Duration::from_millis(750),
        move |res: notify_debouncer_mini::DebounceEventResult| {
            if let Ok(events) = res {
                let paths: Vec<PathBuf> = events.into_iter().map(|e| e.path).collect();
                if apply_changes(&handle.state::<AppState>(), &paths) > 0 {
                    use tauri::Emitter;
                    let _ = handle.emit(CHANGED_EVENT, ());
                }
            }
        },
    )
    .map_err(err)?;
    let mut watched = 0;
    for root in &roots {
        let path = std::path::Path::new(root.split(imt_core::archive::ENTRY_SEPARATOR).next().unwrap_or(root));
        let mode = if path.is_dir() {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        if debouncer.watcher().watch(path, mode).is_ok() {
            watched += 1;
        }
    }
    *slot = Some(debouncer);
    log::info!(Watched = watched as u64; "Live tail on");
    Ok(watched)
}

/// Re-imports changed files; returns how many sources were refreshed.
fn apply_changes(state: &AppState, paths: &[PathBuf]) -> usize {
    match state.workspace.write() {
        Ok(mut ws) => ws.refresh_files(paths).unwrap_or_else(|e| {
            log::warn!("live tail refresh failed: {e:#}");
            0
        }),
        Err(_) => 0,
    }
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
        .plugin(log_plugin())
        .setup(|app| {
            let profile_dir = app.path().app_config_dir().ok().map(|d| d.join("profiles"));
            // `imt-app <paths...>` opens files and folders at startup.
            let args: Vec<PathBuf> = std::env::args()
                .skip(1)
                .filter(|a| !a.starts_with('-'))
                .map(PathBuf::from)
                .collect();
            log::info!(Version = env!("CARGO_PKG_VERSION"); "Application starting — IMT {}", env!("CARGO_PKG_VERSION"));
            app.manage(app_state(profile_dir, &args)?);
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
            save_project,
            open_project,
            get_profile,
            validate_profile,
            save_profile,
            import_profile,
            export_profile,
        ])
        .build(tauri::generate_context!())
        .expect("error while building IMT")
        .run(|app, event| match event {
            tauri::RunEvent::Exit => log::info!("Application is shutting down"),
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
        block(open_paths(app.state::<AppState>(), pending)).unwrap();
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

        let summary = block(open_paths(st(), vec![samples().join("log4net").to_string_lossy().into_owned()])).unwrap();
        assert_eq!(summary["sources"].as_array().unwrap().len(), 3);
        assert!(block(open_paths(st(), vec!["/does/not/exist".into()]))
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
        let summary = block(reload_workspace(st())).unwrap();
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
        assert!(plugins().unwrap().plugins().len() == 1);
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
        let opened = block(open_project(st(), file.clone())).unwrap();
        assert_eq!(opened["name"], "Incident");
        assert_eq!(opened["ui"]["bookmarks"][0], 3);
        assert_eq!(opened["profile"]["name"], "Project profile");
        assert_eq!(opened["summary"]["profileName"], "Project profile");
        assert!(opened["summary"]["total"].as_u64().unwrap() > 0);
        assert_eq!(opened["missing"].as_array().unwrap().len(), 0);
        assert_eq!(get_profile(st()).unwrap().name, "Project profile");
        assert!(block(open_project(st(), dir.0.join("nope.imtproj").to_string_lossy().into_owned())).is_err());
        std::fs::write(
            dir.0.join("bad.imtproj"),
            r#"{"version":1,"name":"x","sources":[],"profile":"[parse"}"#,
        )
        .unwrap();
        assert!(block(open_project(st(), dir.0.join("bad.imtproj").to_string_lossy().into_owned())).is_err());
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
        assert_eq!(set_live(app.handle().clone(), st(), true).unwrap(), 1);
        assert!(st().watcher.lock().unwrap().is_some());
        std::fs::write(
            dir.0.join("a.log"),
            "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:01 INFO second\n",
        )
        .unwrap();
        assert_eq!(apply_changes(&st(), &[dir.0.join("a.log")]), 1);
        assert_eq!(block(workspace_summary(st())).unwrap()["total"], 2);
        assert_eq!(set_live(app.handle().clone(), st(), false).unwrap(), 0);
        assert!(st().watcher.lock().unwrap().is_none());
    }
}
