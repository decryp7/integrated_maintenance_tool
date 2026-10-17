//! Tauri shell: exposes the imt-core workspace to the Angular UI. The command
//! names and payloads are documented in app/src/app/core/api.types.ts.

mod clef;

use imt_core::profile::CompiledProfile;
use imt_core::workspace::{ChartRequest, ExtractPreviewRequest, FacetRequest, PageRequest, TimelineRequest};
use imt_core::{LogStore, Profile, Workspace, DEFAULT_PROFILE_TOML};
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
}

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

#[tauri::command]
fn validate_profile(toml: String) -> ProfileValidation {
    match Profile::from_toml(&toml) {
        Err(e) => ProfileValidation {
            ok: false,
            error: Some(e.message),
            line: e.line,
        },
        Ok(p) => match CompiledProfile::compile(p) {
            Ok(_) => ProfileValidation {
                ok: true,
                error: None,
                line: None,
            },
            Err(e) => ProfileValidation {
                ok: false,
                line: find_line(&toml, &e),
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

/// The saved profile if it still parses, else the built-in default.
fn load_profile(dir: Option<&PathBuf>) -> (Profile, ProfileDoc) {
    if let Some(path) = dir.map(|d| d.join(PROFILE_FILE)) {
        if let Ok(toml) = std::fs::read_to_string(&path) {
            match Profile::from_toml(&toml) {
                Ok(p) if CompiledProfile::compile(p.clone()).is_ok() => {
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

/// Loads the saved profile and opens the paths given on the command line.
fn app_state(profile_dir: Option<PathBuf>, args: &[PathBuf]) -> anyhow::Result<AppState> {
    let (profile, doc) = load_profile(profile_dir.as_ref());
    let mut workspace = Workspace::new(profile)?;
    let paths: Vec<PathBuf> = args.iter().filter(|p| p.exists()).cloned().collect();
    if !paths.is_empty() {
        if let Err(e) = workspace.open(&paths) {
            log::error!("could not open {paths:?}: {e:#}");
        }
    }
    let workspace: Box<dyn LogStore> = Box::new(workspace);
    Ok(AppState {
        workspace: Arc::new(RwLock::new(workspace)),
        profile: RwLock::new(doc),
        profile_dir,
    })
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
            get_profile,
            validate_profile,
            save_profile,
            import_profile,
            export_profile,
        ])
        .build(tauri::generate_context!())
        .expect("error while building IMT")
        .run(|_, event| {
            if let tauri::RunEvent::Exit = event {
                log::info!("Application is shutting down");
            }
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

    fn app(state: AppState) -> tauri::App<MockRuntime> {
        mock_builder().manage(state).build(mock_context(noop_assets())).unwrap()
    }

    fn block<F: std::future::Future>(f: F) -> F::Output {
        tauri::async_runtime::block_on(f)
    }

    #[test]
    fn commands_round_trip() {
        let dir = TempDir::new("cmds");
        let app = app(app_state(
            Some(dir.0.clone()),
            &[samples().join("serilog-clef"), PathBuf::from("/does/not/exist")],
        )
        .unwrap());
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

        let ok = validate_profile("name = 'x'".into());
        assert!(ok.ok && ok.error.is_none());
        let syntax = validate_profile("name = 'x'\n[parse\n".into());
        assert_eq!((syntax.ok, syntax.line), (false, Some(2)));
        let semantic = validate_profile("name = 'x'\n\n[[extract]]\nname = \"stats\"\nregex = '('\n".into());
        assert!(!semantic.ok);
        assert_eq!(semantic.line, Some(4));
        let unlocated = validate_profile("[parse]\nassume_timezone = 'Mars'".into());
        assert_eq!((unlocated.ok, unlocated.line), (false, None));

        assert!(block(save_profile(st(), "[parse".into())).is_err());
        let summary = block(save_profile(st(), "name = 'Saved'".into())).unwrap();
        assert_eq!(summary["profileName"], "Saved");
        let doc = get_profile(st()).unwrap();
        assert_eq!(doc.name, "Saved");
        let saved = PathBuf::from(doc.path.unwrap());
        assert!(saved.exists());

        // A fresh start picks the saved profile up; an invalid one falls back to the default.
        assert_eq!(load_profile(Some(&dir.0)).1.name, "Saved");
        std::fs::write(&saved, "[parse]\nregexes = ['(']").unwrap();
        assert_eq!(load_profile(Some(&dir.0)).1.name, "Default (Serilog)");
        assert_eq!(load_profile(None).1.path, None);

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
}
