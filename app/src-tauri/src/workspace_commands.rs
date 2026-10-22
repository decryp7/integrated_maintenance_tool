//! Workspace commands: open/reload/clear/remove, queries, export, projects,
//! import progress and live tail. Each acts on the calling window's store
//! ([`CurrentWindow`], in the store worker); the names and payloads are documented in
//! app/src/app/core/api.types.ts.

use crate::activity::{domain_event, privacy, OpId};
use crate::engine::{with_store, StoreKind};
use crate::jobs::{JobKind, JobLink, JobSpec};
use crate::live;
use crate::profile::ProfileDoc;
use crate::services::project::OpenedProject;
use crate::windows::CurrentWindow;
use crate::{err, CmdError, CmdResult};
use imt_core::api::{
    ChartData, ChartRequest, ContextRequest, ExportRequest, ExtractPreview, ExtractPreviewRequest, Facet, FacetRequest, Finding,
    SourceContext, Timeline, TimelineRequest, WorkspaceSummary,
};
use imt_core::batch::SourceId;
use imt_core::events::SubscriberMetrics;
use imt_core::package::ProjectManifest;
use imt_core::plugin::annotate::{AnnotationDto, AnnotationsRequest};
use imt_core::ports::{Analyzed, ImportReport, PageQuery, PageReply};
use imt_core::Profile;
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Runtime};

/// The window's summary: the store's (when one is open) with the host's profile and plugins.
pub(crate) async fn summary_of(state: &CurrentWindow) -> CmdResult<WorkspaceSummary> {
    let win = state.arc().clone();
    let reply = match win.engine.opened() {
        Some(store) => crate::sync::blocking("the log store", move || store.summary().map_err(CmdError::from)).await?,
        // Nothing opened yet: an empty summary without starting the store worker.
        None => imt_core::ports::SummaryReply {
            total: 0,
            untimed: 0,
            from: None,
            to: None,
            levels: imt_core::model::Level::ALL.iter().map(|l| (*l, 0)).collect(),
            sources: vec![],
            unrecognized: vec![],
            fields: vec![],
            default_group_by: vec![],
            marker_kinds: win.host.compiled().markers.iter().map(|m| m.kind.clone()).collect(),
            profile_name: win.host.compiled().profile.name.clone(),
        },
    };
    Ok(crate::engine::summary::compose(
        reply,
        &state.host,
        state.engine.last_report().as_ref(),
    ))
}

#[tauri::command]
pub(crate) async fn workspace_summary(state: CurrentWindow) -> CmdResult<WorkspaceSummary> {
    summary_of(&state).await
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
fn report_not_imported<R: Runtime>(app: &AppHandle<R>, label: &str, files: &[imt_core::api::UnrecognizedFile]) {
    use crate::notification_centre::centre::{NewNotice, NoticeAction, NoticeLink, NoticeScope, NoticeSeverity, NoticeSource};
    let new: Vec<_> = files
        .iter()
        .filter(|u| !u.crashed && u.reason != imt_core::workspace::NOT_IMPORTED)
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

/// A source whose timestamps could not be read for many events becomes a message with "Fix timestamps"
/// (the format wizard, pre-filled with the format the engine verified). Once per file: a reload or a
/// second import of the same file does not repeat it. The body carries counts and the format, never
/// lines of the file (messages are saved with the project).
pub(crate) fn timestamp_notice(source: &imt_core::api::SourceInfo) -> Option<crate::notification_centre::centre::NewNotice> {
    use crate::notification_centre::centre::{NewNotice, NoticeAction, NoticeLink, NoticeScope, NoticeSeverity, NoticeSource};
    let issue = source.timestamp_issue.as_ref()?;
    let format = issue.suggestion.as_ref().map(|s| s.format.clone());
    let mut body = format!(
        "{} of {} events have no readable timestamp, so they are missing from the timeline.",
        issue.unparsed_lines, issue.total_lines
    );
    if let Some(f) = &format {
        body.push_str(&format!(" The format {f} reads them."));
    }
    let notice = NewNotice::new(
        NoticeSource::Import,
        NoticeSeverity::Warning,
        format!("Timestamps not recognised in {}", source.name),
        NoticeScope::Project,
    )
    .body(body)
    .primary(NoticeAction::FixTimestamps {
        path: source.path.clone(),
        format,
    })
    .dedupe(format!("import:timestamps:{}", source.path))
    .once()
    .link(NoticeLink {
        path: Some(source.path.clone()),
        ..NoticeLink::default()
    });
    Some(notice)
}

fn report_timestamp_issues<R: Runtime>(app: &AppHandle<R>, label: &str, sources: &[imt_core::api::SourceInfo]) {
    for notice in sources.iter().filter_map(timestamp_notice) {
        crate::notification_centre::post(app, Some(label), notice, None);
    }
}

/// Most paths one open request may name (a folder opens everything in it).
pub(crate) const MAX_OPEN_PATHS: usize = 1024;

/// The paths of an open request, bounded and checked: each one granted to the window (chosen in a
/// dialog, dropped, from the command line or IMT's own output) or an entry of the recent list (granted
/// again now), canonicalised. A recent entry that no longer exists keeps its spelling, so the import
/// reports it as not found.
pub(crate) fn open_request<R: Runtime>(app: &AppHandle<R>, window: &str, paths: Vec<String>) -> CmdResult<Vec<PathBuf>> {
    if paths.len() > MAX_OPEN_PATHS {
        return Err(crate::CmdError::from(format!(
            "{} files at once are too many: open at most {MAX_OPEN_PATHS}, or open their folder",
            paths.len()
        )));
    }
    let grants = crate::paths::WindowGrants::of(app, window);
    let recents = crate::recent::recents(app);
    paths
        .iter()
        .map(|raw| match grants.read_or_listed(raw, &[], |p| recents.has_file(p)) {
            Ok(g) => Ok(g.path().to_path_buf()),
            Err(_) if recents.has_file(raw) => Ok(crate::paths::parse(raw)?.to_path_buf()),
            Err(e) => Err(e),
        })
        .collect()
}

#[tauri::command]
pub(crate) async fn open_paths<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, paths: Vec<String>) -> CmdResult<WorkspaceSummary> {
    let paths: Vec<String> = open_request(&app, state.label(), paths)?
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let summary = open_granted(&app, &state, paths.clone()).await?;
    crate::recent::opened_files(&app, &paths);
    Ok(summary)
}

/// What an import reported to the user and the logs: files not imported, crashes (typed engine errors),
/// unreadable timestamps.
fn after_import<R: Runtime>(
    app: &AppHandle<R>,
    state: &CurrentWindow,
    before: &crate::engine_errors::HealthSnapshot,
    report: &ImportReport,
) {
    let after = crate::engine_errors::HealthSnapshot::new(&state.host.registry.plugins(), &report.unrecognized);
    for e in crate::engine_errors::new_errors(before, &after) {
        crate::snapshots::emit_engine_error(app, state.label(), &e);
    }
    log_unrecognized(&report.unrecognized);
    report_not_imported(app, state.label(), &report.unrecognized);
    report_timestamp_issues(app, state.label(), &report.sources);
    // Counts only: file names and contents stay out of the logs.
    log::info!(
        Sources = report.sources.len() as u64,
        Unrecognized = report.unrecognized.len() as u64,
        Events = report.events as u64,
        ElapsedMs = report.elapsed_ms;
        "Workspace opened"
    );
    state.set_loaded(true);
}

/// Opens paths that were checked already (granted, or IMT's own): see [`open_request`].
pub(crate) async fn open_granted<R: Runtime>(app: &AppHandle<R>, state: &CurrentWindow, paths: Vec<String>) -> CmdResult<WorkspaceSummary> {
    let paths = crate::activity::report::open::divert(app, state.label(), paths).await;
    if paths.is_empty() {
        return summary_of(state).await;
    }
    let before = crate::engine_errors::HealthSnapshot::new(&state.host.registry.plugins(), &[]);
    let files: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let report = crate::engine::import::run(app, state, open_spec(&paths), Box::new(move |store| store.open(&files))).await?;
    after_import(app, state, &before, &report);
    let summary = summary_of(state).await?;
    crate::window_manager::import_finished(app, state.label(), summary.total as u64);
    Ok(summary)
}

/// Files logged per import as not imported (the rest are counted).
const MAX_LOGGED_FILES: usize = 20;

/// Logs each file an import could not use, with why: a crashed importer at Error, an unknown format or
/// unreadable file at Warning. The path as the privacy settings allow; the reason is redacted and capped (it
/// never carries file contents, but may quote a path or a value).
pub(crate) fn log_unrecognized(files: &[imt_core::api::UnrecognizedFile]) {
    for u in files.iter().take(MAX_LOGGED_FILES) {
        let file = privacy::safe_path(std::path::Path::new(&u.path));
        let reason: String = privacy::redact(&u.reason).chars().take(300).collect();
        if u.crashed {
            log::error!(File = file.as_str(), Reason = reason.as_str(); "Importer crashed on a file");
        } else {
            log::warn!(File = file.as_str(), Reason = reason.as_str(); "File not imported");
        }
    }
    if files.len() > MAX_LOGGED_FILES {
        log::warn!(More = (files.len() - MAX_LOGGED_FILES) as u64; "More files not imported");
    }
}

#[tauri::command]
pub(crate) async fn remove_source(state: CurrentWindow, source_id: u32) -> CmdResult<WorkspaceSummary> {
    let win = state.arc().clone();
    with_store(&state, move |s| {
        s.remove_source(SourceId(source_id))?;
        crate::engine::annotate::run(&win, s, &|_| {});
        Ok(())
    })
    .await?;
    summary_of(&state).await
}

#[tauri::command]
pub(crate) async fn clear_workspace(state: CurrentWindow) -> CmdResult<WorkspaceSummary> {
    state.set_project(None);
    if state.engine.opened().is_some() {
        with_store(&state, |s| s.clear()).await?;
    }
    state.host.registry.clear_annotations();
    state.set_loaded(false);
    summary_of(&state).await
}

#[tauri::command]
pub(crate) async fn reload_workspace<R: Runtime>(app: AppHandle<R>, state: CurrentWindow) -> CmdResult<WorkspaceSummary> {
    let before = crate::engine_errors::HealthSnapshot::new(&state.host.registry.plugins(), &[]);
    let spec = JobSpec::new(JobKind::Import, "Reload the files");
    let report = crate::engine::import::run(&app, &state, spec, Box::new(|store| store.reload())).await?;
    after_import(&app, &state, &before, &report);
    let summary = summary_of(&state).await?;
    crate::window_manager::import_finished(&app, state.label(), summary.total as u64);
    Ok(summary)
}

#[tauri::command]
pub(crate) async fn query_page(state: CurrentWindow, request: PageQuery) -> CmdResult<PageReply> {
    let view = state.host.redacted_view();
    with_store(&state, move |s| {
        let mut page = s.page(&request)?;
        if let Some(r) = &view {
            r.view_page(&mut page);
        }
        Ok(page)
    })
    .await
}

#[tauri::command]
pub(crate) async fn query_timeline(state: CurrentWindow, request: TimelineRequest) -> CmdResult<Timeline> {
    with_store(&state, move |s| s.timeline(&request)).await
}

#[tauri::command]
pub(crate) async fn query_facet(state: CurrentWindow, request: FacetRequest) -> CmdResult<Facet> {
    let view = state.host.redacted_view();
    with_store(&state, move |s| {
        let mut facet = s.facet(&request)?;
        if let Some(r) = &view {
            r.view_facet(&mut facet);
        }
        Ok(facet)
    })
    .await
}

#[tauri::command]
pub(crate) async fn query_chart(state: CurrentWindow, request: ChartRequest) -> CmdResult<ChartData> {
    with_store(&state, move |s| s.chart(&request)).await
}

#[tauri::command]
pub(crate) async fn preview_extractor(state: CurrentWindow, request: ExtractPreviewRequest) -> CmdResult<ExtractPreview> {
    with_store(&state, move |s| s.preview_extractor(&request)).await
}

/// Progress of the running import (the import job's, which the Jobs pane also shows).
#[tauri::command]
pub(crate) async fn import_progress(state: CurrentWindow) -> imt_core::workspace::ProgressSnapshot {
    state.engine.progress()
}

/// The automatic health check: findings, most severe first, with how much of the data it covered.
#[tauri::command]
pub(crate) async fn overview(state: CurrentWindow, request: imt_core::api::QueryRequest) -> CmdResult<Analyzed<Vec<Finding>>> {
    with_store(&state, move |s| s.overview(&request)).await
}

/// Ranges and points found by plugin annotators (reboots, service uptime, ...) overlapping the request, by
/// start time: what they found when they last ran over the store (after each import, see `engine::annotate`).
#[tauri::command]
pub(crate) async fn annotations(state: CurrentWindow, request: AnnotationsRequest) -> CmdResult<Vec<AnnotationDto>> {
    Ok(state.host.registry.annotations(&request))
}

/// Runs between lifecycle markers (`[[span]]`): a sorted page for the Runs view plus every span kind's
/// summary over the selection. The request is bounded in the engine (page size, lane list).
#[tauri::command]
pub(crate) async fn query_runs(state: CurrentWindow, request: imt_core::spans::RunsRequest) -> CmdResult<imt_core::spans::RunsReply> {
    with_store(&state, move |s| s.runs(&request)).await
}

/// The Runs editor's preview of a candidate rule over the loaded data: field errors, warnings, counts, the
/// first runs and anomalies, and what saving writes (the store pairs it like the Runs table).
#[tauri::command]
pub(crate) async fn preview_spans(
    state: CurrentWindow,
    request: imt_core::spans::SpanPreviewRequest,
) -> CmdResult<imt_core::spans::SpanPreview> {
    with_store(&state, move |s| s.preview_spans(&request)).await
}

/// Import problems of the window's sources (ADR 0014 §5): unreadable timestamps, lines too long, files no
/// importer reads... counted per source and kind, with sample lines and a checked fix where there is one.
#[tauri::command]
pub(crate) async fn import_problems(state: CurrentWindow) -> CmdResult<Vec<imt_core::ports::DiagnosticRow>> {
    if state.engine.opened().is_none() {
        return Ok(Vec::new());
    }
    with_store(&state, |s| s.problems()).await
}

/// How many affected events a proposed fix (a profile override, from the engine or the AI) would repair,
/// checked on the stored events before the user applies it.
#[tauri::command]
pub(crate) async fn evaluate_fix(state: CurrentWindow, fix: imt_core::ports::QuickFix) -> CmdResult<imt_core::ports::FixEvaluation> {
    with_store(&state, move |s| s.evaluate_fix(&fix)).await
}

/// How far the full-text index is (search covers indexed events only).
#[tauri::command]
pub(crate) async fn index_status(state: CurrentWindow) -> CmdResult<imt_core::ports::IndexStatus> {
    if state.engine.opened().is_none() {
        return Ok(imt_core::ports::IndexStatus {
            indexed: 0,
            total: 0,
            ready: true,
        });
    }
    with_store(&state, |s| s.index_status()).await
}

/// Delivery counters of the plugins' event subscribers (plugin health).
#[tauri::command]
pub(crate) async fn plugin_events(state: CurrentWindow) -> Vec<SubscriberMetrics> {
    state.event_metrics()
}

/// Writes the events matching the query to a file chosen by the user (an export job; Cancel stops it).
#[tauri::command]
pub(crate) async fn export_events<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    op: OpId,
    mut request: ExportRequest,
) -> CmdResult<u64> {
    let target = crate::paths::WindowGrants::of(&app, state.label())
        .write(&request.path, &[])?
        .path()
        .to_path_buf();
    // Checked now, so the dialog can ask to replace the file; the store checks again when it writes.
    imt_core::overwrite::check_overwrite(std::path::Path::new(&request.path), request.overwrite)?;
    request.path = target.to_string_lossy().into_owned();
    let redact = request.redact;
    crate::jobs::export(&app, &state, "Export events to", &target, move |store| {
        let result = store.export(&request).map_err(CmdError::from)?;
        log::info!(Format = result.format.as_str(), Events = result.events as u64; "Events exported");
        let path = privacy::safe_path(std::path::Path::new(&result.path));
        let rows = result.events.to_string();
        domain_event(
            "export.written",
            op.as_deref(),
            &[("Export", "events"), ("Format", &result.format), ("Path", &path), ("Rows", &rows)],
        );
        Ok(exported_line(&result, redact))
    })
}

/// "Exported 1204 events (redacted, 3 values masked)": the line the export's message shows.
fn exported_line(result: &imt_core::api::ExportResult, redacted: bool) -> String {
    let masked: usize = result.redactions.values().sum();
    match (redacted, masked) {
        (true, 1) => format!("Exported {} events (redacted, 1 value masked).", result.events),
        (true, n) => format!("Exported {} events (redacted, {n} values masked).", result.events),
        (false, _) => format!("Exported {} events.", result.events),
    }
}

#[tauri::command]
pub(crate) async fn source_context(state: CurrentWindow, request: ContextRequest) -> CmdResult<SourceContext> {
    let view = state.host.redacted_view();
    with_store(&state, move |s| {
        let mut context = s.source_context(&request)?;
        if let Some(r) = &view {
            r.view_context(&mut context);
        }
        Ok(context)
    })
    .await
}

/// The calling window's workspace as a workspace file (its session): the store folder to reopen, the opened
/// roots, the profile and the UI state.
pub(crate) async fn session_of(state: &CurrentWindow, name: String, ui: Value) -> CmdResult<imt_core::project::ProjectFile> {
    let (roots, store) = match state.engine.opened() {
        Some(s) => {
            let dir = s.store_dir().to_string_lossy().into_owned();
            let saved = crate::sync::blocking("the log store", move || {
                s.save_project(&ProjectManifest::default()).map_err(CmdError::from)
            })
            .await?;
            (saved.roots, Some(dir))
        }
        None => (Vec::new(), None),
    };
    Ok(crate::services::project::session(name, state.profile_doc().toml, roots, store, ui))
}

/// Saves the window's store as a project: `<name>.imtz`, self-contained (events, sources, diagnostics, the
/// profile, the UI state and reports) and shareable; it opens without the original log files. Written by a
/// file job (it can take a while for a large store); returns the path and name at once.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // the command's arguments are its wire contract
pub(crate) async fn save_project<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    grants: crate::paths::WindowGrants,
    op: OpId,
    path: String,
    name: String,
    ui: Value,
    reports: Option<Vec<Value>>,
    overwrite: Option<bool>,
) -> CmdResult<crate::services::project::SavedProject> {
    crate::share::refuse_bundle_folder(&path)?;
    let target = grants.write(&path, &[imt_core::package::ARCHIVE_EXTENSION])?;
    let reports = crate::share::parse_reports(reports.unwrap_or_default())?;
    let file = target.path().to_path_buf();
    // "Save" to the project's own file passes `overwrite`; "Save As" over another file asks first.
    imt_core::overwrite::check_overwrite(&file, overwrite.unwrap_or(false))?;
    let store = state.engine.store()?;
    let manifest = ProjectManifest {
        name: name.clone(),
        profile_overrides: state.profile_doc().toml,
        ..ProjectManifest::default()
    };
    let (n, out) = (name.clone(), file.clone());
    crate::sync::blocking("saving the project", move || {
        crate::services::project::write_ui(&store.store_dir(), &ui, &reports)?;
        let bytes = store.export_project(&manifest, &out).map_err(CmdError::from)?;
        log::info!(Bytes = bytes; "Project saved");
        Ok::<_, CmdError>(())
    })
    .await?;
    // "Save" writes here again without a dialog.
    grants.grant(target.path(), crate::paths::Access::Write, crate::paths::GrantSource::Project)?;
    state.set_project(Some(path.clone()));
    crate::recent::opened_project(&app, &path, &n);
    let safe = privacy::safe_path(std::path::Path::new(&path));
    domain_event("project.saved", op.as_deref(), &[("Path", &safe)]);
    Ok(crate::services::project::SavedProject { path, name })
}

/// Opens a project the user chose (or one from the recent list): an `.imtz` (extracted into IMT's stores
/// folder) or a `.imt` project folder; its events open without the original log files. IMT's own workspace
/// files (`.imtproj`: the autosaved session, the activity log) reopen their store or import their files.
#[tauri::command]
pub(crate) async fn open_project<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    grants: crate::paths::WindowGrants,
    op: OpId,
    path: String,
) -> CmdResult<OpenedProject> {
    let recents = crate::recent::recents(&app);
    let file = grants.read_or_listed(
        &path,
        &[
            imt_core::package::ARCHIVE_EXTENSION,
            "imtproj",
            imt_core::package::PROJECT_EXTENSION,
        ],
        |p| recents.has_project(p),
    )?;
    let is_workspace_file = file.path().extension().is_some_and(|e| e == "imtproj");
    let session = crate::lifecycle::is_session_file(file.path());
    let opened = if is_workspace_file {
        open_project_file(&app, &state, op, file.path().to_path_buf(), path.clone()).await?
    } else {
        open_store_project(&app, &state, op, file.path().to_path_buf(), path.clone()).await?
    };
    if !session {
        // The project's own file: "Save" writes it back without a dialog.
        if let Err(e) = grants.grant(file.path(), crate::paths::Access::Write, crate::paths::GrantSource::Project) {
            log::debug!("the project file is not writable from this window: {e}");
        }
        state.set_project(Some(path.clone()));
        crate::recent::opened_project(&app, &path, &opened.name);
    }
    Ok(opened)
}

/// Opens a saved project (`.imtz` or `.imt`) as the window's store.
async fn open_store_project<R: Runtime>(
    app: &AppHandle<R>,
    state: &CurrentWindow,
    op: OpId,
    file: PathBuf,
    shown: String,
) -> CmdResult<OpenedProject> {
    let win = state.arc().clone();
    let spec = JobSpec::new(JobKind::Import, "Open the project")
        .window(state.label())
        .link(JobLink::Path { path: shown.clone() });
    let (store, manifest) = crate::jobs::run(app, spec, Default::default(), move |ctx: &crate::jobs::JobCtx| -> CmdResult<_> {
        ctx.message("Opening the project");
        let base = win.engine.profile();
        let dest = win.engine.unpack_root();
        let opened = win.engine.host().open_project(&file, &dest, &base).map_err(CmdError::from)?;
        crate::engine::annotate::run(&win, opened.0.as_ref(), &|m| ctx.message(m));
        Ok(crate::jobs::Done::new(opened).with_result(crate::jobs::JobResult::Workspace))
    })
    .await?;
    let (ui, reports) = crate::services::project::read_ui(&store.store_dir());
    // The project's profile: its overrides merged over the window's (what the store applied).
    let profile = {
        let s = store.clone();
        crate::sync::blocking("the log store", move || s.active_profile().map_err(CmdError::from)).await?
    };
    let doc = if manifest.profile_overrides.trim().is_empty() {
        state.profile_doc()
    } else {
        ProfileDoc {
            name: profile.name.clone(),
            path: None,
            toml: manifest.profile_overrides.clone(),
        }
    };
    apply_window_profile(state, profile)?;
    state.set_profile_doc(doc.clone());
    state.engine.replace(store, StoreKind::Project);
    state.set_loaded(true);
    let captures = crate::services::project::captures(&ui);
    crate::paths::grant_read_all(app, state.label(), &captures, crate::paths::GrantSource::Output);
    let summary = summary_of(state).await?;
    log::info!(Sources = summary.sources.len() as u64; "Project opened");
    let safe = privacy::safe_path(std::path::Path::new(&shown));
    domain_event("project.opened", op.as_deref(), &[("Path", &safe)]);
    crate::window_manager::import_finished(app, state.label(), summary.total as u64);
    Ok(OpenedProject {
        path: shown,
        name: manifest.name,
        summary,
        profile: doc,
        ui,
        reports,
        missing: Vec::new(),
        read_only: false,
        bundle: None,
        folder: None,
    })
}

/// The window's host and new stores follow `profile`.
fn apply_window_profile(state: &CurrentWindow, profile: Profile) -> CmdResult<()> {
    let compiled = state.host.compile(profile.clone()).map_err(err)?;
    state.host.set_compiled(compiled);
    state.engine.set_profile(profile);
    Ok(())
}

/// Opens a workspace file that was checked already (granted, or IMT's own: a session, an unpacked bundle's).
/// `shown` is the path as the UI names it. Its store folder is reopened when it is still there; otherwise its
/// files are imported. Grants the window the captures the project's UI state reopens.
pub(crate) async fn open_project_file<R: Runtime>(
    app: &AppHandle<R>,
    state: &CurrentWindow,
    op: OpId,
    file: PathBuf,
    shown: String,
) -> CmdResult<OpenedProject> {
    let project = crate::sync::blocking("reading the project", move || crate::services::project::load(&file)).await?;
    let profile = Profile::from_toml(&project.profile).map_err(|e| CmdError::invalid(e.message).line(e.line))?;
    let doc = ProfileDoc {
        name: profile.name.clone(),
        path: None,
        toml: project.profile.clone(),
    };
    apply_window_profile(state, profile)?;
    state.set_profile_doc(doc.clone());
    let reopened = match project.store.as_deref().map(PathBuf::from).filter(|d| d.is_dir()) {
        Some(dir) => {
            let win = state.arc().clone();
            let kind = if dir.starts_with(win.engine.scratch_root()) {
                StoreKind::Scratch
            } else {
                StoreKind::Project
            };
            let reopen = move || {
                let store = win.engine.reopen(&dir, kind)?;
                crate::engine::annotate::run(&win, store.as_ref(), &|_| {});
                Ok(store)
            };
            match crate::sync::blocking("reopening the session", reopen).await {
                Ok(_) => true,
                Err(e) => {
                    log::warn!("the session's store could not be reopened ({e}); importing its files again");
                    false
                }
            }
        }
        None => false,
    };
    let mut missing = Vec::new();
    if !reopened {
        let existing: Vec<String> = project.sources.iter().filter(|p| source_exists(p)).cloned().collect();
        missing = project.sources.iter().filter(|p| !source_exists(p)).cloned().collect();
        if !existing.is_empty() {
            let before = crate::engine_errors::HealthSnapshot::new(&state.host.registry.plugins(), &[]);
            let spec = JobSpec::new(JobKind::Import, format!("Open {}", project.name)).link(JobLink::Path { path: shown.clone() });
            let files: Vec<PathBuf> = existing.iter().map(PathBuf::from).collect();
            let report = crate::engine::import::run(app, state, spec, Box::new(move |store| store.open(&files))).await?;
            after_import(app, state, &before, &report);
        }
    }
    state.set_loaded(true);
    let captures = crate::services::project::captures(&project.ui);
    crate::paths::grant_read_all(app, state.label(), &captures, crate::paths::GrantSource::Output);
    let summary = summary_of(state).await?;
    log::info!(Sources = project.sources.len() as u64, Missing = missing.len() as u64; "Project opened");
    let safe = privacy::safe_path(std::path::Path::new(&shown));
    let (sources, missing_n) = (project.sources.len().to_string(), missing.len().to_string());
    domain_event(
        "project.opened",
        op.as_deref(),
        &[("Path", &safe), ("Sources", &sources), ("Missing", &missing_n)],
    );
    crate::window_manager::import_finished(app, state.label(), summary.total as u64);
    Ok(OpenedProject {
        path: shown,
        name: project.name,
        summary,
        profile: doc,
        ui: project.ui,
        reports: project.reports.into_iter().filter_map(|r| serde_json::to_value(r).ok()).collect(),
        missing,
        read_only: false,
        bundle: None,
        folder: None,
    })
}

/// Whether a project's source still exists (an archive member: its archive).
fn source_exists(p: &str) -> bool {
    let file = p.split("!/").next().unwrap_or(p);
    std::path::Path::new(file).exists()
}

/// Turns live tail on or off for the calling window. While on, the opened
/// files and folders are watched (OS events, or polling for network shares
/// and after a watcher error); appended lines are read by the store and
/// `imt://workspace-changed` is emitted to this window (at most about four per
/// second). Returns how many folders are watched.
#[tauri::command]
pub(crate) async fn set_live<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    enabled: bool,
    options: Option<live::LiveOptions>,
) -> CmdResult<usize> {
    let window = state.arc().clone();
    tauri::async_runtime::spawn_blocking(move || live::set(&app, &window, enabled, options.unwrap_or_default()))
        .await
        .map_err(err)?
}

/// Paths waiting to be opened in the calling window (launch arguments, files
/// opened from the OS, a restored session). Projects and workspace files should be opened
/// with `open_project`, others with `open_paths`. Only Rust queues them, so they are granted to the
/// window as they are handed over.
#[tauri::command]
pub(crate) async fn take_pending_open(state: CurrentWindow, grants: crate::paths::WindowGrants) -> CmdResult<Vec<String>> {
    let pending = state.take_pending();
    for p in &pending {
        let _ = grants.grant(
            std::path::Path::new(p),
            crate::paths::Access::Read,
            crate::paths::GrantSource::CommandLine,
        );
    }
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Json as _;
    use crate::test_support::Now as _;
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
        let pending = take_pending_open(main_window(&app), crate::paths::WindowGrants::of(app.handle(), "main"))
            .now()
            .unwrap();
        block(open_paths(app.handle().clone(), main_window(&app), pending)).unwrap().json();
        app
    }

    fn page(limit: usize, query: Query) -> PageQuery {
        PageQuery {
            query,
            offset: 0,
            limit,
            descending: false,
            anchor: None,
        }
    }

    #[test]
    fn the_export_line_counts_events_and_masked_values() {
        let r = |n: usize| imt_core::api::ExportResult {
            format: "csv".into(),
            path: "x".into(),
            events: 7,
            redactions: [("email".to_string(), n)].into_iter().collect(),
        };
        assert_eq!(exported_line(&r(0), false), "Exported 7 events.");
        assert_eq!(exported_line(&r(1), true), "Exported 7 events (redacted, 1 value masked).");
        assert_eq!(exported_line(&r(3), true), "Exported 7 events (redacted, 3 values masked).");
    }

    #[test]
    fn commands_round_trip() {
        let dir = TempDir::new("cmds");
        let app = opened(
            Some(dir.0.clone()),
            &[samples().join("serilog-clef"), PathBuf::from("/does/not/exist")],
        );
        let st = || main_window(&app);

        let summary = block(workspace_summary(st())).unwrap().json();
        assert_eq!(summary["sources"].as_array().unwrap().len(), 2);
        assert_eq!(summary["profileName"], "Default (Serilog)");

        let summary = block(open_paths(
            app.handle().clone(),
            st(),
            vec![samples().join("log4net").to_string_lossy().into_owned()],
        ))
        .unwrap()
        .json();
        assert_eq!(summary["sources"].as_array().unwrap().len(), 3);
        assert!(block(open_paths(app.handle().clone(), st(), vec!["/does/not/exist".into()]))
            .unwrap_err()
            .contains("does/not/exist"));

        let p = block(query_page(st(), page(5, Query::default()))).unwrap().json();
        assert_eq!(p["rows"].as_array().unwrap().len(), 5);
        assert!(p["rows"][0]["sourceName"].is_string());
        let bad = Query {
            text: "(".into(),
            regex: true,
            ..Default::default()
        };
        assert!(block(query_page(st(), page(5, bad))).is_err());

        let tl: TimelineRequest = serde_json::from_value(json!({"buckets": 50})).unwrap();
        assert!(!block(query_timeline(st(), tl)).unwrap().json()["lanes"]
            .as_array()
            .unwrap()
            .is_empty());
        let facet: FacetRequest = serde_json::from_value(json!({"field": "MachineName"})).unwrap();
        assert_eq!(block(query_facet(st(), facet)).unwrap().json()["distinct"], 2);
        let chart: ChartRequest = serde_json::from_value(json!({"chartId": "events-by-level"})).unwrap();
        assert!(block(query_chart(st(), chart)).unwrap().json()["series"].is_array());
        let preview: ExtractPreviewRequest = serde_json::from_value(json!({"toml": DEFAULT_PROFILE_TOML, "extractor": "x"})).unwrap();
        assert!(block(preview_extractor(st(), preview)).unwrap().json()["error"].is_string());

        let id = summary["sources"][0]["id"].as_u64().unwrap() as u32;
        let summary = block(remove_source(st(), id)).unwrap().json();
        assert_eq!(summary["sources"].as_array().unwrap().len(), 2);
        let summary = block(reload_workspace(app.handle().clone(), st())).unwrap().json();
        assert_eq!(summary["sources"].as_array().unwrap().len(), 2);
        let summary = block(clear_workspace(st())).unwrap().json();
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
        // Another window has its own (empty) workspace.
        window(&app, "w-2");
        let other = crate::windows::CurrentWindow::of(app.handle(), "w-2").unwrap();
        assert_eq!(block(query_runs(other, Default::default())).unwrap().total, 0);
    }
    #[test]
    fn problems_fixes_and_the_index_of_the_calling_window() {
        let dir = TempDir::new("problems");
        std::fs::write(
            dir.0.join("odd.log"),
            "14.10.2026 10:00:00 INFO first\n14.10.2026 10:00:01 INFO second\n",
        )
        .unwrap();
        let app = test_app(None);
        // Nothing opened: nothing to report, without starting the store.
        assert!(block(import_problems(main_window(&app))).unwrap().is_empty());
        assert!(block(index_status(main_window(&app))).unwrap().ready);
        crate::paths::allow_under(app.handle(), "main", &dir.0);
        block(open_paths(
            app.handle().clone(),
            main_window(&app),
            vec![dir.0.to_string_lossy().into_owned()],
        ))
        .unwrap();
        let problems = block(import_problems(main_window(&app))).unwrap();
        assert!(!problems.is_empty(), "the unreadable timestamps are a problem");
        let fix: imt_core::ports::QuickFix =
            serde_json::from_value(json!({"parserOverride": "[format]\ntimestamp_formats = ['%d.%m.%Y %H:%M:%S']\n"})).unwrap();
        let e = block(evaluate_fix(main_window(&app), fix)).unwrap();
        assert!(
            e.fixed <= e.checked && (e.checked > 0 || !e.notes.is_empty()),
            "the store says what it checked: {e:?}"
        );
        let bad: imt_core::ports::QuickFix = serde_json::from_value(json!({"parserOverride": "[format"})).unwrap();
        assert_eq!(
            block(evaluate_fix(main_window(&app), bad)).unwrap_err().code,
            crate::ErrorCode::Invalid
        );
        let index = block(index_status(main_window(&app))).unwrap();
        assert!(index.indexed <= index.total, "{index:?}");
    }

    #[test]
    fn event_metrics_and_annotations_of_the_calling_window() {
        let app = test_app(None);
        // The built-in plugins subscribe to nothing yet; metrics are a list either way.
        let metrics = plugin_events(main_window(&app)).now();
        assert!(metrics.iter().all(|m| !m.plugin.is_empty()));
        // Annotators do not run over the store yet: nothing is stored, nothing fails.
        assert!(block(annotations(main_window(&app), AnnotationsRequest::default()))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn plugins_and_export() {
        let dir = TempDir::new("export");
        let app = opened(
            Some(dir.0.clone()),
            &[samples().join("windows").join("registry-export.reg"), samples().join("csv")],
        );
        let st = || main_window(&app);
        crate::paths::allow_under(app.handle(), "main", &dir.0);
        let summary = block(workspace_summary(st())).unwrap().json();
        assert!(summary["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "net.decryptology.registry"));
        assert!(summary["sources"].as_array().unwrap().iter().any(|s| s["importer"] == "registry"));
        assert_eq!(summary["exporters"].as_array().unwrap().len(), 2);

        let out = dir.0.join("events.clef");
        let req: ExportRequest = serde_json::from_value(json!({"format": "clef", "path": out.to_string_lossy()})).unwrap();
        let job = block(export_events(app.handle().clone(), st(), OpId::default(), req)).unwrap();
        assert_eq!(
            crate::test_support::job_done(&app, job),
            format!("Exported {} events.", summary["total"])
        );
        assert!(std::fs::read_to_string(&out).unwrap().lines().count() > 0);
        let bad: ExportRequest = serde_json::from_value(json!({"format": "pdf", "path": dir.0.join("x.pdf").to_string_lossy()})).unwrap();
        // What only the engine knows (the format) fails the job, with the reason.
        let job = block(export_events(app.handle().clone(), st(), OpId::default(), bad)).unwrap();
        let end = crate::test_support::job_end(&app, job);
        assert_eq!(end.state, crate::jobs::JobState::Failed);
        assert!(end.error.unwrap_or_default().contains("unknown export format"));
    }

    fn save(
        app: &tauri::App<MockRuntime>,
        label: &str,
        path: &str,
        name: &str,
        ui: Value,
        reports: Option<Vec<Value>>,
        overwrite: Option<bool>,
    ) -> CmdResult<crate::services::project::SavedProject> {
        let st = crate::windows::CurrentWindow::of(app.handle(), label).unwrap();
        block(save_project(
            app.handle().clone(),
            st,
            crate::paths::WindowGrants::of(app.handle(), label),
            OpId::default(),
            path.to_string(),
            name.to_string(),
            ui,
            reports,
            overwrite,
        ))
    }

    fn open(app: &tauri::App<MockRuntime>, label: &str, path: &str) -> CmdResult<OpenedProject> {
        let st = crate::windows::CurrentWindow::of(app.handle(), label).unwrap();
        block(open_project(
            app.handle().clone(),
            st,
            crate::paths::WindowGrants::of(app.handle(), label),
            OpId::default(),
            path.to_string(),
        ))
    }

    /// A project is the store saved as an `.imtz`: it opens without the log files, with its profile, view and
    /// reports.
    #[test]
    fn projects_are_self_contained_and_open_without_their_files() {
        let dir = TempDir::new("project");
        let logs = dir.0.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for f in std::fs::read_dir(from).unwrap().flatten() {
                if f.path().is_dir() {
                    copy_dir(&f.path(), &to.join(f.file_name()));
                } else {
                    std::fs::copy(f.path(), to.join(f.file_name())).unwrap();
                }
            }
        }
        copy_dir(&samples().join("log4net"), &logs);
        let app = test_app(Some(dir.0.join("profiles")));
        crate::paths::allow_under(app.handle(), "main", &dir.0);
        let st = || main_window(&app);
        let opened = block(open_paths(app.handle().clone(), st(), vec![logs.to_string_lossy().into_owned()])).unwrap();
        assert!(opened.total > 0, "{:?}", opened.unrecognized);
        let shared = || app.state::<crate::AppState>();
        crate::test_support::job_done(
            &app,
            crate::profile::save_profile(
                app.handle().clone(),
                shared(),
                st(),
                OpId::default(),
                "name = 'Project profile'".into(),
            )
            .now()
            .unwrap(),
        );
        let total = block(workspace_summary(st())).unwrap().total;
        assert!(total > 0);
        let file = dir.0.join("incident.imtz").to_string_lossy().into_owned();
        let report = || Some(vec![json!({"id": "r1", "title": "R"})]);
        let saved = save(&app, "main", &file, "Incident", json!({"bookmarks": [3]}), report(), None).unwrap();
        assert_eq!(saved.name, "Incident");
        // "Save As" over an existing file is refused until confirmed; "Save" to it passes `overwrite`.
        let again = save(&app, "main", &file, "Incident", json!({}), None, None);
        assert_eq!(again.unwrap_err().code, crate::ErrorCode::FileExists);
        save(&app, "main", &file, "Incident", json!({"bookmarks": [3]}), report(), Some(true)).unwrap();
        assert_eq!(st().project().as_deref(), Some(file.as_str()));
        // Only projects are written, and reports must be valid.
        assert!(save(
            &app,
            "main",
            &dir.0.join("x.imtproj").to_string_lossy(),
            "x",
            Value::Null,
            None,
            None
        )
        .is_err());
        assert!(save(&app, "main", &file, "x", Value::Null, Some(vec![json!({})]), Some(true)).is_err());

        let ctx: ContextRequest = serde_json::from_value(json!({"recordId": 5, "before": 1, "after": 1})).unwrap();
        assert!(!block(source_context(st(), ctx)).unwrap().lines.is_empty());

        // The log files go away; the project opens in another window all the same.
        std::fs::remove_dir_all(&logs).unwrap();
        crate::paths::allow_under(app.handle(), "w-2", &dir.0);
        let opened = open(&app, "w-2", &file).unwrap();
        assert_eq!(opened.name, "Incident");
        assert_eq!(opened.ui["bookmarks"][0], 3);
        assert_eq!(opened.reports[0]["title"], "R");
        assert_eq!(opened.profile.name, "Project profile");
        assert_eq!(opened.summary.total, total);
        assert!(opened.missing.is_empty());
        let w2 = crate::windows::CurrentWindow::of(app.handle(), "w-2").unwrap();
        assert_eq!(crate::profile::get_profile(w2).now().unwrap().name, "Project profile");
        // Missing and damaged projects are refused with a message.
        assert!(open(&app, "w-2", &dir.0.join("nope.imtz").to_string_lossy()).is_err());
        std::fs::write(dir.0.join("bad.imtz"), "not a zip").unwrap();
        assert!(open(&app, "w-2", &dir.0.join("bad.imtz").to_string_lossy()).is_err());
        std::fs::write(
            dir.0.join("bad.imtproj"),
            r#"{"version":1,"name":"x","sources":[],"profile":"[parse"}"#,
        )
        .unwrap();
        assert!(open(&app, "w-2", &dir.0.join("bad.imtproj").to_string_lossy()).is_err());
        block(clear_workspace(st())).unwrap();
        assert!(st().project().is_none());
    }

    /// The autosaved session names the store folder: reopening it reopens the store instead of importing again.
    #[test]
    fn a_session_reopens_its_store() {
        let dir = TempDir::new("session-store");
        let app = opened(None, &[samples().join("log4net")]);
        let st = || main_window(&app);
        let total = block(workspace_summary(st())).unwrap().total;
        let session = block(session_of(&st(), "Last session".into(), json!({"zoom": 1}))).unwrap();
        assert!(session.store.is_some() && !session.sources.is_empty());
        let file = dir.0.join("session.imtproj");
        session.save(&file).unwrap();
        let w2 = crate::windows::CurrentWindow::of(app.handle(), "w-2").unwrap();
        let opened = block(open_project_file(app.handle(), &w2, OpId::default(), file.clone(), String::new())).unwrap();
        assert_eq!(opened.summary.total, total);
        assert_eq!(opened.ui["zoom"], 1);
        // Its store gone, the session imports its files again.
        let mut gone = session.clone();
        gone.store = Some(dir.0.join("no-store").to_string_lossy().into_owned());
        gone.save(&file).unwrap();
        let w3 = crate::windows::CurrentWindow::of(app.handle(), "w-3").unwrap();
        let opened = block(open_project_file(app.handle(), &w3, OpId::default(), file, String::new())).unwrap();
        assert_eq!(opened.summary.total, total);
    }

    #[test]
    fn exports_and_projects_are_domain_events_with_the_operation_id() {
        let _g = crate::activity::test_capture();
        let dir = TempDir::new("domain-events");
        let app = opened(Some(dir.0.clone()), &[samples().join("log4net")]);
        let st = || main_window(&app);
        crate::paths::allow_under(app.handle(), "main", &dir.0);
        let op = |id: &str| OpId(Some(id.into()));
        let out = dir.0.join("events.clef");
        let req: ExportRequest = serde_json::from_value(json!({"format": "clef", "path": out.to_string_lossy()})).unwrap();
        let job = block(export_events(app.handle().clone(), st(), op("op-export"), req)).unwrap();
        crate::test_support::job_done(&app, job);
        let e = crate::tools::tests::domain_event("export.written", "op-export");
        assert_eq!((e.props["Export"].as_str(), e.props["Format"].as_str()), ("events", "clef"));
        assert!(
            e.props["Path"].ends_with("/events.clef") && e.props["Path"].starts_with("<dir#"),
            "{:?}",
            e.props
        );
        assert!(e.props["Rows"].parse::<u64>().unwrap() > 0);
        let file = dir.0.join("p.imtz").to_string_lossy().into_owned();
        block(save_project(
            app.handle().clone(),
            st(),
            crate::paths::WindowGrants::of(app.handle(), "main"),
            op("op-save"),
            file.clone(),
            "P".into(),
            json!({}),
            None,
            None,
        ))
        .unwrap();
        assert!(crate::tools::tests::domain_event("project.saved", "op-save").props["Path"].starts_with("<dir#"));
        block(open_project(
            app.handle().clone(),
            st(),
            crate::paths::WindowGrants::of(app.handle(), "main"),
            op("op-open"),
            file,
        ))
        .unwrap();
        let opened = crate::tools::tests::domain_event("project.opened", "op-open");
        assert!(opened.props["Path"].starts_with("<dir#"));
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
                crashed: false,
            },
            imt_core::api::UnrecognizedFile {
                path: "/secret/b.log".into(),
                name: "b.log".into(),
                reason: "importer x failed: index out of bounds".into(),
                crashed: true,
            },
        ];
        log_unrecognized(&files);
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
                crashed: false,
            })
            .collect();
        log_unrecognized(&many);
        assert!(crate::activity::captured()
            .iter()
            .any(|c| c.message == "More files not imported" && c.props["More"] == "3"));
        let _ = dir;
    }

    #[test]
    fn unreadable_timestamps_become_one_message_per_file() {
        use crate::notification_centre::centre::{NoticeAction, NoticeSeverity};
        use imt_core::api::{SourceInfo, TimestampIssue, TimestampSuggestion};
        let mut source = SourceInfo {
            id: 0,
            path: "/logs/app.csv".into(),
            name: "app.csv".into(),
            group: "app.csv".into(),
            importer: "csv".into(),
            records: 4,
            skipped: 0,
            split_events: 0,
            dropped: 0,
            step_stats: Vec::new(),
            error: None,
            from: None,
            to: None,
            offset_ms: 0,
            file_kind: Default::default(),
            timestamp_issue: None,
            routed_from: None,
            problems: Vec::new(),
            identity: Vec::new(),
            parser: None,
        };
        assert_eq!(timestamp_notice(&source), None);
        source.timestamp_issue = Some(TimestampIssue {
            unparsed_lines: 4,
            total_lines: 4,
            examples: vec!["Sat Nov 08 00:04:37 2025,secret".into()],
            example_lines: vec![1],
            suggestion: Some(TimestampSuggestion {
                format: "%a %b %d %H:%M:%S %Y".into(),
                sample: "Sat Nov 08 00:04:37 2025".into(),
                parser_snippet: String::new(),
                parsed_samples: 4,
                checked_samples: 4,
            }),
        });
        let n = timestamp_notice(&source).unwrap();
        assert_eq!(
            (n.severity, n.title.as_str()),
            (NoticeSeverity::Warning, "Timestamps not recognised in app.csv")
        );
        assert!(
            n.body.contains("4 of 4 events") && n.body.contains("%a %b %d %H:%M:%S %Y"),
            "{}",
            n.body
        );
        // No line of the file goes into the message (it is saved with the project).
        assert!(!n.body.contains("secret"));
        assert_eq!(
            n.primary,
            Some(NoticeAction::FixTimestamps {
                path: "/logs/app.csv".into(),
                format: Some("%a %b %d %H:%M:%S %Y".into())
            })
        );
        assert!(n.once && n.dedupe_key.as_deref() == Some("import:timestamps:/logs/app.csv"));
        // Without a suggestion the wizard opens empty.
        source.timestamp_issue.as_mut().unwrap().suggestion = None;
        let n = timestamp_notice(&source).unwrap();
        assert!(matches!(n.primary, Some(NoticeAction::FixTimestamps { format: None, .. })));
        assert!(!n.body.contains("format"));
        // Posted twice (a reload), it is one message.
        let centre = crate::notification_centre::NotificationCentre::new(None, std::sync::Arc::new(crate::jobs::SystemClock));
        centre.post(Some("main"), timestamp_notice(&source).unwrap());
        centre.post(Some("main"), timestamp_notice(&source).unwrap());
        let snap = centre.snapshot("main");
        assert_eq!((snap.notices.len(), snap.notices[0].count), (1, 1));
    }

    #[test]
    fn pending_open_queue() {
        let app = test_app(None);
        main_window(&app).queue(crate::instance::existing(&[
            samples().join("csv"),
            PathBuf::from("/does/not/exist"),
        ]));
        assert_eq!(
            take_pending_open(main_window(&app), crate::paths::WindowGrants::of(app.handle(), "main"))
                .now()
                .unwrap()
                .len(),
            1,
            "missing paths are not queued"
        );
        assert!(
            take_pending_open(main_window(&app), crate::paths::WindowGrants::of(app.handle(), "main"))
                .now()
                .unwrap()
                .is_empty()
        );
        queue_open(app.handle(), "main", vec!["/a/b.imtproj".into()]);
        queue_open(app.handle(), "main", vec![]);
        assert_eq!(
            take_pending_open(main_window(&app), crate::paths::WindowGrants::of(app.handle(), "main"))
                .now()
                .unwrap(),
            ["/a/b.imtproj"]
        );
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
        crate::paths::allow_under(app.handle(), "w-2", &samples());
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

    /// Only granted paths open; the recent lists (filled by Rust after a granted open or save) grant
    /// their entries again; a project grants the captures its UI state reopens.
    #[test]
    fn opening_needs_a_grant_or_a_recent_entry() {
        let dir = TempDir::new("open-grants");
        let log = dir.0.join("a.log");
        std::fs::write(&log, "2026-10-14 10:00:00 INFO one\n").unwrap();
        let capture = dir.0.join("x.cpuprofile");
        std::fs::write(&capture, "{}").unwrap();
        let app = test_app(None);
        let st = || main_window(&app);
        let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let grants = || crate::paths::WindowGrants::of(app.handle(), "main");
        let e = block(open_paths(app.handle().clone(), st(), vec![s(&log)])).unwrap_err();
        assert!(e.contains("only reads files you choose"), "{e}");
        assert!(crate::recent::recents(app.handle()).list().files.is_empty());
        // Chosen (dropped, picked): opens and becomes a recent file.
        crate::paths::grant(
            app.handle(),
            "main",
            &log,
            crate::paths::Access::Read,
            crate::paths::GrantSource::Drop,
        )
        .unwrap();
        block(open_paths(app.handle().clone(), st(), vec![s(&log)])).unwrap().json();
        let recent = crate::recent::recents(app.handle()).list().files;
        assert_eq!(recent.len(), 1);
        // Another window may reopen it from the recent list (granted again there); a file that is not
        // listed stays refused.
        let w2 = crate::windows::CurrentWindow::of(app.handle(), "w-2").unwrap();
        block(open_paths(app.handle().clone(), w2.clone(), recent.clone())).unwrap().json();
        assert!(crate::paths::WindowGrants::of(app.handle(), "w-2")
            .read_file(&recent[0], &[])
            .is_ok());
        assert!(block(open_paths(app.handle().clone(), w2, vec![s(&capture)])).is_err());

        // A project: saved where the user chose, listed, reopened from the list by another window.
        let project = dir.0.join("p.imtz");
        crate::paths::grant(
            app.handle(),
            "main",
            &project,
            crate::paths::Access::Write,
            crate::paths::GrantSource::Dialog,
        )
        .unwrap();
        let ui = json!({ "profiling": { "captures": [s(&capture), "relative.cpuprofile"] } });
        block(save_project(
            app.handle().clone(),
            st(),
            grants(),
            OpId::default(),
            s(&project),
            "P".into(),
            ui,
            None,
            None,
        ))
        .unwrap();
        let listed = crate::recent::recents(app.handle()).list().projects;
        assert_eq!((listed[0].name.as_str(), listed.len()), ("P", 1));
        let w3 = crate::windows::CurrentWindow::of(app.handle(), "w-3").unwrap();
        let g3 = || crate::paths::WindowGrants::of(app.handle(), "w-3");
        assert!(g3().read_file(&s(&capture), &[]).is_err());
        let opened = block(open_project(app.handle().clone(), w3.clone(), g3(), OpId::default(), s(&project))).unwrap();
        assert_eq!(opened.name, "P");
        assert!(g3().read_file(&s(&capture), &[]).is_ok(), "the project's captures may be reopened");
        assert!(g3().write(&s(&project), &["imtz"]).is_ok(), "Save writes the project back");
        assert_eq!(w3.project().as_deref(), Some(s(&project).as_str()));
        // A project nobody chose is refused.
        let other = dir.0.join("other.imtz");
        std::fs::copy(&project, &other).unwrap();
        let e = block(open_project(app.handle().clone(), w3, g3(), OpId::default(), s(&other))).unwrap_err();
        assert!(e.contains("only reads files you choose"), "{e}");
        assert!(crate::services::project::captures(&json!(null)).is_empty());
    }
}
