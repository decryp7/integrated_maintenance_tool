//! Workspace commands: open/reload/clear/remove, queries, export, projects,
//! import progress and live tail. Each acts on the calling window's workspace
//! ([`CurrentWindow`]); the names and payloads are documented in
//! app/src/app/core/api.types.ts.

use crate::activity::{domain_event, privacy, OpId};
use crate::jobs::{Controls, Done, JobKind, JobLink, JobProgress, JobResult, JobSpec};
use crate::live;
use crate::profile::ProfileDoc;
use crate::windows::{read, write, write_reporting, CurrentWindow};
use crate::{err, CmdResult};
use imt_core::events::SubscriberMetrics;
use imt_core::plugin::annotate::{AnnotationDto, AnnotationsRequest};
use imt_core::project::ProjectFile;
use imt_core::workspace::{
    ChartRequest, ContextRequest, ExportRequest, ExtractPreviewRequest, FacetRequest, PageRequest, TimelineRequest, WorkspaceState,
};
use imt_core::Profile;
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Runtime};

#[tauri::command]
pub(crate) async fn workspace_summary(state: CurrentWindow) -> CmdResult<Value> {
    read(&state, |ws| serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)).await
}

/// Runs an import (open, reload, project) as the window's import job: one at a time per window, with the
/// import's progress and Cancel; View opens the workspace.
pub(crate) async fn import_job<R: Runtime, T: Send + 'static>(
    app: &AppHandle<R>,
    state: &CurrentWindow,
    spec: JobSpec,
    f: impl FnOnce(&mut dyn imt_core::LogStore) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let (cancel, probe) = (state.progress.clone(), state.progress.clone());
    let controls = Controls::cancel(move || cancel.cancel()).with_probe(move || {
        let p = probe.snapshot();
        p.running.then(|| JobProgress {
            fraction: (p.bytes_total > 0).then(|| (p.bytes_done as f64 / p.bytes_total as f64) as f32),
            done: Some(p.done as u64),
            total: Some(p.total as u64),
            eta_ms: p.eta_ms,
        })
    });
    let (handle, win) = (app.clone(), state.arc().clone());
    crate::jobs::run(app, spec.window(state.label()), controls, move |ctx| {
        ctx.message("Reading the files");
        crate::windows::write_reporting_blocking(&handle, &win, f)
            .map(|v| Done::new(v).with_result(JobResult::Workspace))
            .map_err(err)
    })
    .await
}

/// The spec of an import of `paths` (Retry opens them again).
fn open_spec(paths: &[String]) -> JobSpec {
    let title = match paths {
        [one] => format!(
            "Open {}",
            std::path::Path::new(one)
                .file_name()
                .map_or(one.clone(), |n| n.to_string_lossy().into_owned())
        ),
        many => format!("Open {} files and folders", many.len()),
    };
    let mut spec = JobSpec::new(JobKind::Import, title).retry_with(serde_json::json!({"op": "open", "paths": paths}));
    if let Some(first) = paths.first() {
        spec = spec.link(JobLink::Path { path: first.clone() });
    }
    spec
}

/// Retry of an import: the window opens the paths again (through its UI, as after a launch).
pub(crate) fn retry_import<R: Runtime>(app: &AppHandle<R>, job: &crate::jobs::model::Job) -> Result<Option<crate::jobs::JobId>, String> {
    let label = job.window().ok_or("this import has no window")?;
    let paths: Vec<String> = job
        .request
        .as_ref()
        .and_then(|r| r.get("paths"))
        .and_then(|p| serde_json::from_value(p.clone()).ok())
        .filter(|p: &Vec<String>| !p.is_empty() && p.len() <= 1000 && p.iter().all(|s| !s.is_empty() && s.len() <= 4096))
        .ok_or("this import cannot be run again")?;
    let existing = crate::instance::existing(&paths.iter().map(std::path::PathBuf::from).collect::<Vec<_>>());
    if existing.is_empty() {
        return Err("none of these files exist any more".into());
    }
    crate::windows::queue_open(app, label, existing);
    Ok(None)
}

/// Files of an import that IMT could not use (not crashed: those are engine errors) become one message
/// with "Set up format".
fn report_not_imported<R: Runtime>(
    app: &AppHandle<R>,
    label: &str,
    files: &[imt_core::api::UnrecognizedFile],
    before: &std::collections::HashSet<String>,
) {
    use crate::notification_centre::centre::{NewNotice, NoticeAction, NoticeLink, NoticeScope, NoticeSeverity, NoticeSource};
    let new: Vec<_> = files
        .iter()
        .filter(|u| !before.contains(&u.path) && !u.reason.contains(" crashed: "))
        .collect();
    let Some(first) = new.first() else { return };
    let title = match new.len() {
        1 => format!("{} was not imported", first.name),
        n => format!("{n} files were not imported"),
    };
    let names: Vec<&str> = new.iter().take(5).map(|u| u.name.as_str()).collect();
    let notice = NewNotice::new(NoticeSource::Import, NoticeSeverity::Warning, title, NoticeScope::Project)
        .body(format!("IMT does not know their format yet: {}.", names.join(", ")))
        .primary(NoticeAction::SetUpFormat { path: first.path.clone() })
        .dedupe(format!("import:not-imported:{}", first.path))
        .link(NoticeLink {
            path: Some(first.path.clone()),
            ..NoticeLink::default()
        });
    crate::notification_centre::post(app, Some(label), notice, None);
}

#[tauri::command]
pub(crate) async fn open_paths<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, paths: Vec<String>) -> CmdResult<Value> {
    let paths = crate::activity::report::open::divert(&app, state.label(), paths).await;
    if paths.is_empty() {
        return write_reporting(&app, &state, |ws| serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)).await;
    }
    let (handle, label) = (app.clone(), state.label().to_string());
    let summary = import_job(&app, &state, open_spec(&paths), move |ws| {
        let before = unrecognized_paths(&ws.summary());
        ws.open(&paths.into_iter().map(PathBuf::from).collect::<Vec<_>>())?;
        let summary = ws.summary();
        log_unrecognized(&summary.unrecognized, &before);
        report_not_imported(&handle, &label, &summary.unrecognized, &before);
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
    .await?;
    crate::window_manager::import_finished(&app, state.label(), &summary);
    Ok(summary)
}

/// Files logged per import as not imported (the rest are counted).
const MAX_LOGGED_FILES: usize = 20;

fn unrecognized_paths(summary: &imt_core::api::WorkspaceSummary) -> std::collections::HashSet<String> {
    summary.unrecognized.iter().map(|u| u.path.clone()).collect()
}

/// Logs each file an import could not use (not in `before`), with why: a crashed importer at Error, an
/// unknown format or unreadable file at Warning. The path as the privacy settings allow; the reason is
/// redacted and capped (it never carries file contents, but may quote a path or a value).
pub(crate) fn log_unrecognized(files: &[imt_core::api::UnrecognizedFile], before: &std::collections::HashSet<String>) {
    let new: Vec<_> = files.iter().filter(|u| !before.contains(&u.path)).collect();
    for u in new.iter().take(MAX_LOGGED_FILES) {
        let file = privacy::safe_path(std::path::Path::new(&u.path));
        let reason: String = privacy::redact(&u.reason).chars().take(300).collect();
        if u.reason.contains(" crashed: ") {
            log::error!(File = file.as_str(), Reason = reason.as_str(); "Importer crashed on a file");
        } else {
            log::warn!(File = file.as_str(), Reason = reason.as_str(); "File not imported");
        }
    }
    if new.len() > MAX_LOGGED_FILES {
        log::warn!(More = (new.len() - MAX_LOGGED_FILES) as u64; "More files not imported");
    }
}

#[tauri::command]
pub(crate) async fn remove_source(state: CurrentWindow, source_id: u32) -> CmdResult<Value> {
    write(&state, move |ws| {
        ws.remove_source(source_id);
        serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn clear_workspace(state: CurrentWindow) -> CmdResult<Value> {
    state.set_project(None);
    write(&state, |ws| {
        ws.clear();
        serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn reload_workspace<R: Runtime>(app: AppHandle<R>, state: CurrentWindow) -> CmdResult<Value> {
    let summary = import_job(&app, &state, JobSpec::new(JobKind::Import, "Reload the files"), |ws| {
        ws.reload()?;
        log_unrecognized(&ws.summary().unrecognized, &Default::default());
        serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)
    })
    .await?;
    crate::window_manager::import_finished(&app, state.label(), &summary);
    Ok(summary)
}

#[tauri::command]
pub(crate) async fn query_page(state: CurrentWindow, request: PageRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.page(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn query_timeline(state: CurrentWindow, request: TimelineRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.timeline(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn query_facet(state: CurrentWindow, request: FacetRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.facet(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn query_chart(state: CurrentWindow, request: ChartRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.chart(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn preview_extractor(state: CurrentWindow, request: ExtractPreviewRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.preview_extractor(&request)).map_err(anyhow::Error::from)
    })
    .await
}

/// Progress of the running import (readable while the import holds the workspace).
#[tauri::command]
pub(crate) fn import_progress(state: CurrentWindow) -> imt_core::workspace::ProgressSnapshot {
    state.progress.snapshot()
}

/// Stops the running import after the files in flight; files not read are not added.
#[tauri::command]
pub(crate) fn cancel_import(state: CurrentWindow) {
    log::info!("Import cancelled by the user");
    state.progress.cancel();
}

/// The automatic health check: findings, most severe first.
#[tauri::command]
pub(crate) async fn overview(state: CurrentWindow, request: imt_core::api::QueryRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.overview(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

/// Ranges and points found by plugin annotators (reboots, service uptime, ...)
/// overlapping the request, by start time.
#[tauri::command]
pub(crate) async fn annotations(state: CurrentWindow, request: AnnotationsRequest) -> CmdResult<Vec<AnnotationDto>> {
    read(&state, move |ws| Ok(ws.annotations(&request))).await
}

/// Runs between lifecycle markers (`[[span]]`): a sorted page for the Runs
/// view plus every span kind's summary over the selection. The request is
/// bounded in the engine (page size, lane list).
#[tauri::command]
pub(crate) async fn query_runs(state: CurrentWindow, request: imt_core::spans::RunsRequest) -> CmdResult<imt_core::spans::RunsReply> {
    read(&state, move |ws| Ok(ws.runs(&request))).await
}

/// The Runs editor's preview of a candidate rule over the loaded data: field
/// errors, warnings, counts, the first runs and anomalies, and what saving writes.
#[tauri::command]
pub(crate) async fn preview_spans(
    state: CurrentWindow,
    request: imt_core::spans::SpanPreviewRequest,
) -> CmdResult<imt_core::spans::SpanPreview> {
    read(&state, move |ws| ws.preview_spans(&request)).await
}

/// Delivery counters of the plugins' event subscribers (plugin health).
/// Reads the event bus only, so it answers while an import runs.
#[tauri::command]
pub(crate) fn plugin_events(state: CurrentWindow) -> Vec<SubscriberMetrics> {
    state.event_metrics()
}

/// Writes the events matching the query to a file chosen by the user.
#[tauri::command]
pub(crate) async fn export_events<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    op: OpId,
    request: ExportRequest,
) -> CmdResult<Value> {
    let target = PathBuf::from(&request.path);
    crate::jobs::export(&app, &state, "Export events to", &target, move |ws| {
        let result = ws.export(&request)?;
        log::info!(Format = result.format.as_str(), Events = result.events as u64; "Events exported");
        let path = privacy::safe_path(std::path::Path::new(&result.path));
        let rows = result.events.to_string();
        domain_event(
            "export.written",
            op.as_deref(),
            &[("Export", "events"), ("Format", &result.format), ("Path", &path), ("Rows", &rows)],
        );
        serde_json::to_value(result).map_err(anyhow::Error::from)
    })
    .await
}

#[tauri::command]
pub(crate) async fn source_context(state: CurrentWindow, request: ContextRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        serde_json::to_value(ws.source_context(&request)?).map_err(anyhow::Error::from)
    })
    .await
}

/// The calling window's open files, profile and UI state as a project file.
pub(crate) fn project_of(state: &CurrentWindow, name: String, ui: Value) -> ProjectFile {
    let profile = state.profile_doc().toml;
    let ws = state.store_read().state();
    ProjectFile {
        version: imt_core::project::PROJECT_VERSION,
        name,
        sources: ws.roots,
        removed: ws.removed,
        profile,
        ui,
        clock: ws.offsets,
        reports: Vec::new(),
    }
}

/// Saves the open files, the active profile and the UI's state to a `.imtproj`.
#[tauri::command]
pub(crate) async fn save_project(
    state: CurrentWindow,
    op: OpId,
    path: String,
    name: String,
    ui: Value,
    reports: Option<Vec<Value>>,
    overwrite: Option<bool>,
) -> CmdResult<Value> {
    crate::share::refuse_bundle_folder(&path)?;
    // "Save" to the project's own file passes `overwrite`; "Save As" over another file asks first.
    imt_core::overwrite::check_overwrite(std::path::Path::new(&path), overwrite.unwrap_or(false)).map_err(err)?;
    let mut project = project_of(&state, name, ui);
    project.reports = crate::share::parse_reports(reports.unwrap_or_default())?;
    project.save(std::path::Path::new(&path)).map_err(err)?;
    state.set_project(Some(path.clone()));
    log::info!(Sources = project.sources.len() as u64; "Project saved");
    let (safe, sources) = (privacy::safe_path(std::path::Path::new(&path)), project.sources.len().to_string());
    domain_event("project.saved", op.as_deref(), &[("Path", &safe), ("Sources", &sources)]);
    Ok(serde_json::json!({ "path": path, "name": project.name }))
}

/// Opens a `.imtproj`: applies its profile, reopens its files and returns the UI state.
#[tauri::command]
pub(crate) async fn open_project<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, op: OpId, path: String) -> CmdResult<Value> {
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
        offsets: project.clock.clone(),
    };
    let spec = JobSpec::new(JobKind::Import, format!("Open project {}", project.name)).link(JobLink::Path { path: path.clone() });
    let (summary, missing) = import_job(&app, &state, spec, move |ws| {
        let missing = ws.restore(profile, &ws_state)?;
        Ok((serde_json::to_value(ws.summary())?, missing))
    })
    .await?;
    state.set_profile_doc(doc.clone());
    state.set_project(Some(path.clone()));
    log::info!(Sources = project.sources.len() as u64, Missing = missing.len() as u64; "Project opened");
    let safe = privacy::safe_path(std::path::Path::new(&path));
    let (sources, missing_n) = (project.sources.len().to_string(), missing.len().to_string());
    domain_event(
        "project.opened",
        op.as_deref(),
        &[("Path", &safe), ("Sources", &sources), ("Missing", &missing_n)],
    );
    crate::window_manager::import_finished(&app, state.label(), &summary);
    Ok(serde_json::json!({
        "path": path,
        "name": project.name,
        "summary": summary,
        "profile": doc,
        "ui": project.ui,
        "reports": project.reports,
        "missing": missing,
    }))
}

/// Turns live tail on or off for the calling window. While on, the opened
/// files and folders are watched (OS events, or polling for network shares
/// and after a watcher error); appended lines are read incrementally,
/// rotations and truncations followed, and `imt://workspace-changed` is
/// emitted to this window with a [`imt_core::workspace::TailReport`] payload
/// (at most about four per second); live alerts are emitted as
/// `imt://live-alert`. Call again after opening more files to watch them too.
/// Returns how many folders are watched.
#[tauri::command]
pub(crate) fn set_live<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    enabled: bool,
    options: Option<live::LiveOptions>,
) -> CmdResult<usize> {
    live::set(&app, state.arc(), enabled, options.unwrap_or_default())
}

/// Paths waiting to be opened in the calling window (launch arguments, files
/// opened from the OS, a restored session). `.imtproj` files should be opened
/// with `open_project`, others with `open_paths`.
#[tauri::command]
pub(crate) fn take_pending_open(state: CurrentWindow) -> CmdResult<Vec<String>> {
    Ok(state.take_pending())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, samples, test_app, window, TempDir};
    use crate::windows::{queue_open, Windows};
    use imt_core::query::Query;
    use imt_core::DEFAULT_PROFILE_TOML;
    use serde_json::json;
    use tauri::test::MockRuntime;
    use tauri::Manager;

    /// An app whose main workspace has `paths` opened, as the UI does after startup.
    fn opened(dir: Option<PathBuf>, paths: &[PathBuf]) -> tauri::App<MockRuntime> {
        let app = test_app(dir);
        main_window(&app).queue(crate::instance::existing(paths));
        let pending = take_pending_open(main_window(&app)).unwrap();
        block(open_paths(app.handle().clone(), main_window(&app), pending)).unwrap();
        app
    }

    fn page(limit: usize, query: Query) -> PageRequest {
        PageRequest {
            query,
            offset: 0,
            limit,
            descending: false,
            anchor: None,
        }
    }

    #[test]
    fn commands_round_trip() {
        let dir = TempDir::new("cmds");
        let app = opened(
            Some(dir.0.clone()),
            &[samples().join("serilog-clef"), PathBuf::from("/does/not/exist")],
        );
        let st = || main_window(&app);

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

        let p = block(query_page(st(), page(5, Query::default()))).unwrap();
        assert_eq!(p["rows"].as_array().unwrap().len(), 5);
        assert!(p["rows"][0]["sourceName"].is_string());
        let bad = Query {
            text: "(".into(),
            regex: true,
            ..Default::default()
        };
        assert!(block(query_page(st(), page(5, bad))).is_err());

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
    fn runs_and_their_preview_of_the_calling_window() {
        let dir = TempDir::new("runs");
        let line = |t: &str, msg: &str, pid: u32| {
            format!("{{\"@t\":\"2026-10-14T{t}Z\",\"@mt\":\"{msg}\",\"ProcessId\":{pid},\"MachineName\":\"WEB01\"}}\n")
        };
        let text = [
            line("10:00:00", "Application started", 1),
            line("10:01:00", "Application started", 2),
            line("10:05:00", "working", 1),
            line("10:09:00", "Application stopped", 1),
            line("10:10:00", "working", 2),
        ]
        .concat();
        std::fs::write(dir.0.join("app.clef"), text).unwrap();
        let app = opened(None, std::slice::from_ref(&dir.0));

        let req: imt_core::spans::RunsRequest = serde_json::from_value(json!({"sort": "duration", "descending": true})).unwrap();
        let reply = serde_json::to_value(block(query_runs(main_window(&app), req)).unwrap()).unwrap();
        // The wire shape the UI reads (api.types.ts RunsReply / RunInfo).
        assert_eq!(reply["total"], 2);
        assert_eq!(reply["summaries"][0]["kind"], "run");
        assert_eq!(reply["summaries"][0]["closed"], 1);
        assert_eq!(reply["summaries"][0]["open"], 1);
        let first = &reply["runs"][0];
        assert_eq!(first["run"]["status"], "closed");
        assert_eq!(first["run"]["durationMs"], 9 * 60_000);
        assert_eq!(first["run"]["start"]["message"], "Application started");
        assert_eq!(first["instance"], "1");
        assert_eq!(reply["runs"][1]["run"]["status"], "open");
        // Runs are annotations too.
        let anns = block(annotations(main_window(&app), AnnotationsRequest::default())).unwrap();
        assert_eq!(anns.iter().filter(|a| a.run.is_some()).count(), 2);

        let draft: imt_core::spans::SpanPreviewRequest = serde_json::from_value(json!({"draft": {
            "kind": "work", "label": "Work",
            "start": {"type": "pattern", "pattern": "working"},
            "stop": {"type": "markers", "kinds": ["stop"]},
            "per": "field:ProcessId"
        }}))
        .unwrap();
        let preview = serde_json::to_value(block(preview_spans(main_window(&app), draft)).unwrap()).unwrap();
        assert_eq!(preview["errors"], json!([]));
        assert_eq!(preview["summary"]["runs"], 2);
        assert_eq!(preview["plan"]["span"]["start"], "work-start");
        assert_eq!(preview["plan"]["markers"][0]["kind"], "work-start");
        let bad: imt_core::spans::SpanPreviewRequest = serde_json::from_value(
            json!({"draft": {"kind": "a b", "start": {"type": "markers", "kinds": []}, "stop": {"type": "pattern", "pattern": "("}}}),
        )
        .unwrap();
        let preview = serde_json::to_value(block(preview_spans(main_window(&app), bad)).unwrap()).unwrap();
        let fields: Vec<&str> = preview["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["field"].as_str().unwrap())
            .collect();
        assert_eq!(fields, ["kind", "start", "stop"]);
        assert!(preview["plan"].is_null());
        // Another window has its own (empty) workspace.
        window(&app, "w-2");
        let other = crate::windows::CurrentWindow::of(app.handle(), "w-2").unwrap();
        assert_eq!(block(query_runs(other, Default::default())).unwrap().total, 0);
    }
    #[test]
    fn annotations_and_event_metrics_of_the_calling_window() {
        let dir = TempDir::new("annotations");
        let line = |t: &str, provider: &str, id: u32| {
            format!("{{\"@t\":\"2026-10-14T{t}Z\",\"@mt\":\"{provider} {id}\",\"Provider\":\"{provider}\",\"EventId\":{id},\"Computer\":\"WEB01\"}}\n")
        };
        let text = [
            line("10:00:00", "App", 1),
            line("10:05:00", "EventLog", 6006),
            line("10:08:05", "EventLog", 6005),
            line("11:00:00", "App", 2),
        ]
        .concat();
        std::fs::write(dir.0.join("system.clef"), text).unwrap();
        let app = opened(None, std::slice::from_ref(&dir.0));

        let all = block(annotations(main_window(&app), AnnotationsRequest::default())).unwrap();
        let reboot = all.iter().find(|a| a.annotator == "windows.reboots").expect("a reboot span");
        assert_eq!(reboot.owner, "net.decryptology.windows");
        // The wire shape the UI reads (api.types.ts AnnotationDto).
        let json = serde_json::to_value(reboot).unwrap();
        for key in [
            "annotator",
            "owner",
            "kind",
            "from",
            "to",
            "instance",
            "on",
            "label",
            "severity",
            "evidenceQuery",
        ] {
            assert!(json.get(key).is_some(), "missing {key} in {json}");
        }
        assert_eq!(json["on"], json!({"field": "Computer", "value": "WEB01"}));
        assert!(json["severity"].is_string());

        let req: AnnotationsRequest = serde_json::from_value(json!({"to": 0})).unwrap();
        assert!(
            block(annotations(main_window(&app), req)).unwrap().is_empty(),
            "nothing before the epoch"
        );
        // Another window has its own (empty) workspace.
        window(&app, "w-2");
        let other = crate::windows::CurrentWindow::of(app.handle(), "w-2").unwrap();
        assert!(block(annotations(other, AnnotationsRequest::default())).unwrap().is_empty());

        // The built-in plugins subscribe to nothing yet; metrics are a list either way.
        let metrics = plugin_events(main_window(&app));
        assert!(metrics.iter().all(|m| !m.plugin.is_empty()));
    }

    #[test]
    fn plugins_and_export() {
        let dir = TempDir::new("export");
        let app = opened(
            Some(dir.0.clone()),
            &[samples().join("windows").join("registry-export.reg"), samples().join("csv")],
        );
        let st = || main_window(&app);
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
        let res = block(export_events(app.handle().clone(), st(), OpId::default(), req)).unwrap();
        assert_eq!(res["events"], summary["total"]);
        assert!(std::fs::read_to_string(&out).unwrap().lines().count() > 0);
        let bad: ExportRequest = serde_json::from_value(json!({"format": "pdf", "path": "x"})).unwrap();
        assert!(block(export_events(app.handle().clone(), st(), OpId::default(), bad))
            .unwrap_err()
            .contains("unknown export format"));
        assert_eq!(
            crate::lifecycle::plugins(false, &[]).unwrap().plugins().len(),
            crate::lifecycle::builtin_plugins().len()
        );
    }

    #[test]
    fn projects_and_context() {
        let dir = TempDir::new("project");
        let app = opened(Some(dir.0.clone()), &[samples().join("log4net")]);
        let st = || main_window(&app);
        let shared = || app.state::<crate::AppState>();
        block(crate::profile::save_profile(
            shared(),
            st(),
            OpId::default(),
            "name = 'Project profile'".into(),
        ))
        .unwrap();
        let file = dir.0.join("incident.imtproj").to_string_lossy().into_owned();
        let saved = block(save_project(
            st(),
            OpId::default(),
            file.clone(),
            "Incident".into(),
            json!({"bookmarks": [3]}),
            Some(vec![json!({"id": "r1", "title": "R"})]),
            None,
        ))
        .unwrap();
        assert_eq!(saved["name"], "Incident");
        // "Save As" over an existing file is refused until confirmed; "Save" to it passes `overwrite`.
        let again = block(save_project(
            st(),
            OpId::default(),
            file.clone(),
            "Incident".into(),
            json!({"bookmarks": [3]}),
            None,
            None,
        ));
        assert_eq!(again.unwrap_err(), format!("{file} already exists; confirm to replace it"));
        block(save_project(
            st(),
            OpId::default(),
            file.clone(),
            "Incident".into(),
            json!({"bookmarks": [3]}),
            Some(vec![json!({"id": "r1", "title": "R"})]),
            Some(true),
        ))
        .unwrap();
        assert_eq!(st().project().as_deref(), Some(file.as_str()));

        let ctx: ContextRequest = serde_json::from_value(json!({"recordId": 5, "before": 1, "after": 1})).unwrap();
        assert_eq!(block(source_context(st(), ctx)).unwrap()["lines"].as_array().unwrap().len(), 3);

        block(clear_workspace(st())).unwrap();
        assert!(st().project().is_none());
        block(crate::profile::save_profile(
            shared(),
            st(),
            OpId::default(),
            "name = 'Other'".into(),
        ))
        .unwrap();
        let opened = block(open_project(app.handle().clone(), st(), OpId::default(), file.clone())).unwrap();
        assert_eq!(opened["name"], "Incident");
        assert_eq!(opened["ui"]["bookmarks"][0], 3);
        assert_eq!(opened["reports"][0]["title"], "R");
        assert_eq!(opened["profile"]["name"], "Project profile");
        assert_eq!(opened["summary"]["profileName"], "Project profile");
        assert!(opened["summary"]["total"].as_u64().unwrap() > 0);
        assert_eq!(opened["missing"].as_array().unwrap().len(), 0);
        assert_eq!(crate::profile::get_profile(st()).unwrap().name, "Project profile");
        let missing = dir.0.join("nope.imtproj").to_string_lossy().into_owned();
        assert!(block(open_project(app.handle().clone(), st(), OpId::default(), missing)).is_err());
        std::fs::write(
            dir.0.join("bad.imtproj"),
            r#"{"version":1,"name":"x","sources":[],"profile":"[parse"}"#,
        )
        .unwrap();
        let bad = dir.0.join("bad.imtproj").to_string_lossy().into_owned();
        assert!(block(open_project(app.handle().clone(), st(), OpId::default(), bad)).is_err());
        assert!(block(save_project(
            st(),
            OpId::default(),
            "/no/such/dir/x.imtproj".into(),
            "x".into(),
            Value::Null,
            None,
            None
        ))
        .is_err());
        assert!(block(save_project(
            st(),
            OpId::default(),
            file.clone(),
            "x".into(),
            Value::Null,
            Some(vec![json!({})]),
            Some(true)
        ))
        .is_err());
    }

    #[test]
    fn exports_and_projects_are_domain_events_with_the_operation_id() {
        let _g = crate::activity::test_capture();
        let dir = TempDir::new("domain-events");
        let app = opened(Some(dir.0.clone()), &[samples().join("log4net")]);
        let st = || main_window(&app);
        let op = |id: &str| OpId(Some(id.into()));
        let out = dir.0.join("events.clef");
        let req: ExportRequest = serde_json::from_value(json!({"format": "clef", "path": out.to_string_lossy()})).unwrap();
        block(export_events(app.handle().clone(), st(), op("op-export"), req)).unwrap();
        let e = crate::tools::tests::domain_event("export.written", "op-export");
        assert_eq!((e.props["Export"].as_str(), e.props["Format"].as_str()), ("events", "clef"));
        assert!(
            e.props["Path"].ends_with("/events.clef") && e.props["Path"].starts_with("<dir#"),
            "{:?}",
            e.props
        );
        assert!(e.props["Rows"].parse::<u64>().unwrap() > 0);
        let file = dir.0.join("p.imtproj").to_string_lossy().into_owned();
        block(save_project(st(), op("op-save"), file.clone(), "P".into(), json!({}), None, None)).unwrap();
        assert_eq!(crate::tools::tests::domain_event("project.saved", "op-save").props["Sources"], "1");
        block(open_project(app.handle().clone(), st(), op("op-open"), file)).unwrap();
        let opened = crate::tools::tests::domain_event("project.opened", "op-open");
        assert_eq!(
            (opened.props["Path"].starts_with("<dir#"), opened.props["Missing"].as_str()),
            (true, "0")
        );
        // Nothing of the files' content or folders is logged.
        let all = format!("{:?}", crate::activity::captured());
        assert!(!all.contains(&dir.0.to_string_lossy().to_string()));
    }

    #[test]
    fn files_that_were_not_imported_are_logged_one_by_one() {
        let _g = crate::activity::test_capture();
        let dir = TempDir::new("unrecognized-log");
        std::fs::write(dir.0.join("mystery.bin"), [0u8, 159, 146, 150, 1, 2, 3]).unwrap();
        let files = vec![
            imt_core::api::UnrecognizedFile {
                path: "/secret/a.bin".into(),
                name: "a.bin".into(),
                reason: "not a recognised format".into(),
            },
            imt_core::api::UnrecognizedFile {
                path: "/secret/b.log".into(),
                name: "b.log".into(),
                reason: "importer x crashed: index out of bounds".into(),
            },
            imt_core::api::UnrecognizedFile {
                path: "/secret/old.log".into(),
                name: "old.log".into(),
                reason: "not a recognised format".into(),
            },
        ];
        let before: std::collections::HashSet<String> = ["/secret/old.log".to_string()].into();
        log_unrecognized(&files, &before);
        let logged: Vec<_> = crate::activity::captured()
            .into_iter()
            .filter(|c| c.props.contains_key("File"))
            .collect();
        assert_eq!(logged.len(), 2, "{logged:?}");
        assert_eq!(
            (logged[0].level, logged[0].message.as_str()),
            (log::Level::Warn, "File not imported")
        );
        assert_eq!(
            (logged[1].level, logged[1].message.as_str()),
            (log::Level::Error, "Importer crashed on a file")
        );
        assert!(logged.iter().all(|c| !c.props["File"].contains("secret")));
        let many: Vec<_> = (0..MAX_LOGGED_FILES + 3)
            .map(|i| imt_core::api::UnrecognizedFile {
                path: format!("/x/{i}"),
                name: i.to_string(),
                reason: "r".into(),
            })
            .collect();
        log_unrecognized(&many, &Default::default());
        assert!(crate::activity::captured()
            .iter()
            .any(|c| c.message == "More files not imported" && c.props["More"] == "3"));
        let _ = dir;
    }

    #[test]
    fn pending_open_queue() {
        let app = test_app(None);
        main_window(&app).queue(crate::instance::existing(&[
            samples().join("csv"),
            PathBuf::from("/does/not/exist"),
        ]));
        assert_eq!(
            take_pending_open(main_window(&app)).unwrap().len(),
            1,
            "missing paths are not queued"
        );
        assert!(take_pending_open(main_window(&app)).unwrap().is_empty());
        queue_open(app.handle(), "main", vec!["/a/b.imtproj".into()]);
        queue_open(app.handle(), "main", vec![]);
        assert_eq!(take_pending_open(main_window(&app)).unwrap(), ["/a/b.imtproj"]);
        // A closed window's queue cannot be recreated.
        app.state::<Windows>().remove("w-9");
        queue_open(app.handle(), "w-9", vec!["/x".into()]);
        assert!(app.state::<Windows>().get("w-9").is_none());
    }

    /// Two windows, one process: each command runs on the calling window's
    /// own workspace, resolved from the window label through the real IPC path.
    #[test]
    fn commands_are_routed_to_the_calling_window() {
        use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
        use tauri::test::{get_ipc_response, INVOKE_KEY};
        use tauri::webview::InvokeRequest;

        let app = crate::test_support::test_app_with_commands(None);
        let a = window(&app, "main");
        let b = window(&app, "w-2");
        let invoke = |w: &tauri::WebviewWindow<MockRuntime>, cmd: &str, body: Value| {
            let res = get_ipc_response(
                w,
                InvokeRequest {
                    cmd: cmd.into(),
                    callback: CallbackFn(0),
                    error: CallbackFn(1),
                    url: crate::test_support::ipc_url(),
                    body: InvokeBody::Json(body),
                    headers: Default::default(),
                    invoke_key: INVOKE_KEY.to_string(),
                },
            );
            match res.unwrap() {
                InvokeResponseBody::Json(s) => serde_json::from_str::<Value>(&s).unwrap(),
                InvokeResponseBody::Raw(_) => Value::Null,
            }
        };
        let csv = samples().join("csv").to_string_lossy().into_owned();
        let log4net = samples().join("log4net").to_string_lossy().into_owned();
        let sa = invoke(&a, "open_paths", json!({ "paths": [csv] }));
        let sb = invoke(&b, "open_paths", json!({ "paths": [log4net] }));
        assert_ne!(sa["total"], sb["total"]);
        let summary_a = invoke(&a, "workspace_summary", json!({}));
        let summary_b = invoke(&b, "workspace_summary", json!({}));
        assert_eq!(summary_a["total"], sa["total"]);
        assert_eq!(summary_b["total"], sb["total"]);
        assert_eq!(summary_a["sources"][0]["importer"], "csv");
        assert_ne!(summary_b["sources"][0]["importer"], "csv");
        // Clearing one window leaves the other alone.
        invoke(&b, "clear_workspace", json!({}));
        assert_eq!(invoke(&a, "workspace_summary", json!({}))["total"], sa["total"]);
        assert_eq!(invoke(&b, "workspace_summary", json!({}))["total"], 0);
        let windows = app.state::<Windows>();
        assert_eq!(windows.all().len(), 2);
    }
}
