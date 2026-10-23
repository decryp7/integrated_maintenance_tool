//! The Store pane' commands (ADR 0030): read-only reports about the window's store (overview, segments, fields,
//! full-text index, memory), its maintenance as confirmed jobs, and the read-only SQL console. Everything is
//! computed by the store (in the store worker); the shell adds what only it knows: the window's running jobs and
//! its notes. The names and payloads are documented in app/src/app/core/api.types.ts.

use crate::engine::with_store;
use crate::jobs::{Controls, Done, JobKind, JobSpec, PhaseScale};
use crate::query_cancel::QueryStreamEvent;
use crate::windows::CurrentWindow;
use crate::{CmdError, CmdResult};
use debrief_core::ports::{
    MaintenanceAction, RunningJob, SegmentsPage, SegmentsRequest, SqlExportRequest, SqlIds, SqlIdsRequest, SqlNote, SqlPage,
    SqlPageRequest, SqlRequest, SqlResult, SqlSchema, StoreError, StoreFields, StoreIndex, StoreMemory, StoreOverview, MAX_SQL_LEN,
    SQL_MAX_NOTES, SQL_NOTE_CHARS,
};
use debrief_core::ql::{QlError, QueryControl};
use debrief_notes::{Anchor, Note};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime};

#[tauri::command]
pub(crate) async fn store_overview(state: CurrentWindow) -> CmdResult<StoreOverview> {
    with_store(&state, |s| s.store_overview()).await
}

#[tauri::command]
pub(crate) async fn store_segments(state: CurrentWindow, request: SegmentsRequest) -> CmdResult<SegmentsPage> {
    with_store(&state, move |s| s.store_segments(&request)).await
}

#[tauri::command]
pub(crate) async fn store_fields(state: CurrentWindow) -> CmdResult<StoreFields> {
    with_store(&state, |s| s.store_fields()).await
}

#[tauri::command]
pub(crate) async fn store_index(state: CurrentWindow) -> CmdResult<StoreIndex> {
    with_store(&state, |s| s.store_index()).await
}

/// The window's queued and running jobs (and the app-wide ones), as the Memory section lists them.
fn running_jobs<R: Runtime>(app: &AppHandle<R>, window: &str) -> Vec<RunningJob> {
    let Some(jobs) = app.try_state::<crate::jobs::Jobs>() else {
        return Vec::new();
    };
    jobs.find_active(|j| j.window().is_none_or(|w| w == window))
        .into_iter()
        .filter_map(|id| jobs.get(id))
        .map(|j| RunningJob {
            kind: j.kind.as_str().to_string(),
            title: j.title,
        })
        .collect()
}

/// The store's memory against its budgets, with the worker process and the window's running jobs.
#[tauri::command]
pub(crate) async fn store_memory<R: Runtime>(app: AppHandle<R>, state: CurrentWindow) -> CmdResult<StoreMemory> {
    let mut m = with_store(&state, |s| s.store_memory()).await?;
    m.running_jobs = running_jobs(&app, state.label());
    // The budgets the user chose in Settings ▸ Performance (ADR 0031), not the defaults the store knows.
    if let Some(app_state) = app.try_state::<crate::app_state::AppState>() {
        let s = app_state.performance.settings();
        m.import_budget_bytes = u64::from(s.import_budget_mb) << 20;
        m.query_budget_bytes = u64::from(s.query_budget_mb) << 20;
    }
    Ok(m)
}

/// Runs a maintenance action as a job of the window (the UI asks to confirm first) and returns its id at once;
/// the job's message says what it did. Refused (busy) while an import runs.
#[tauri::command]
pub(crate) async fn store_maintenance<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, action: MaintenanceAction) -> CmdResult<u64> {
    let spec = JobSpec::new(JobKind::Maintenance, action.title()).window(state.label());
    let win = state.arc().clone();
    let (controls, control) = crate::jobs::store_cancel(Controls::default());
    let id = crate::jobs::spawn(&app, spec, controls, move |ctx| -> CmdResult<Done<()>> {
        let store = win.engine.store()?;
        ctx.phase(action.phase(), PhaseScale::Indeterminate, 0, 0);
        let report = match control.scope(|| store.maintain(action)) {
            Ok(r) => r,
            // Cancel interrupted the store's statements: nothing was half applied (compaction swaps in one transaction).
            Err(_) if ctx.is_cancelled() => return Ok(Done::new(()).cancelled_if(true)),
            Err(e) => return Err(CmdError::from(e)),
        };
        if action == MaintenanceAction::RebuildIndex {
            // The store returns at once and indexes in the background: the job lasts until the index covers the segments.
            let cancelled = follow_indexing(&*store, ctx);
            let msg = if cancelled {
                "Stopped waiting; the full-text index keeps catching up in the background."
            } else {
                "The full-text index is rebuilt."
            };
            ctx.message(msg);
            log::info!(Action = action.title(); "Store maintenance done");
            return Ok(if cancelled {
                Done::new(()).cancelled_if(true)
            } else {
                Done::new(())
            });
        }
        log::info!(Action = action.title(); "Store maintenance done");
        ctx.message(&report.message);
        Ok(Done::new(()))
    });
    crate::error::lift(id.map(|id| id.0))
}

/// Waits for the full-text index to cover every segment, reporting how many are done; true when Cancel stopped the wait.
fn follow_indexing(store: &dyn debrief_core::ports::Store, ctx: &crate::jobs::JobCtx) -> bool {
    let mut total = 0u64;
    loop {
        if ctx.is_cancelled() {
            return true;
        }
        let Ok(index) = store.store_index() else { return false };
        total = total.max(index.pending_segments);
        if index.pending_segments == 0 || index.error.is_some() {
            return false;
        }
        ctx.phase("Indexing segments", PhaseScale::Count, total - index.pending_segments, total);
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

#[tauri::command]
pub(crate) async fn sql_schema(state: CurrentWindow) -> CmdResult<SqlSchema> {
    with_store(&state, |s| s.sql_schema()).await
}

#[tauri::command]
pub(crate) async fn sql_page(state: CurrentWindow, request: SqlPageRequest) -> CmdResult<SqlPage> {
    with_store(&state, move |s| s.sql_page(&request)).await
}

/// The event ids of a kept result: *Open in Events* filters the grid to them (`@id in (...)`).
#[tauri::command]
pub(crate) async fn sql_ids(state: CurrentWindow, request: SqlIdsRequest) -> CmdResult<SqlIds> {
    with_store(&state, move |s| s.sql_ids(&request)).await
}

/// The window's notes as `debrief.notes` sees them.
fn window_notes(state: &CurrentWindow) -> Vec<SqlNote> {
    state.notes.snapshot().notes().iter().take(SQL_MAX_NOTES).map(sql_note).collect()
}

/// Writes all rows of a console query as CSV to a file the user chose: an export job (Cancel stops it, the job's
/// message says how many rows were written and whether the limits cut it). Returns the job's id at once.
#[tauri::command]
pub(crate) async fn sql_export<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    sql: String,
    path: String,
    overwrite: bool,
) -> CmdResult<u64> {
    if sql.len() > MAX_SQL_LEN {
        return Err(CmdError::from(format!("The SQL text is longer than {} KB.", MAX_SQL_LEN >> 10)));
    }
    let target = crate::paths::WindowGrants::of(&app, state.label())
        .write(&path, &[])?
        .path()
        .to_path_buf();
    // Checked now, so the dialog can ask to replace the file; the store checks again when it writes.
    debrief_core::overwrite::check_overwrite(std::path::Path::new(&path), overwrite)?;
    let request = SqlExportRequest {
        sql,
        notes: window_notes(&state),
        path: target.to_string_lossy().into_owned(),
        overwrite,
    };
    crate::jobs::export(&app, &state, "Export SQL result to", &target, move |store| {
        let r = store.sql_export(&request).map_err(CmdError::from)?;
        log::info!(Rows = r.rows, Bytes = r.bytes, Truncated = r.truncated; "SQL result exported");
        Ok(exported_rows(r.rows, r.truncated))
    })
}

/// The line the export job's message shows.
fn exported_rows(rows: u64, truncated: bool) -> String {
    match (rows, truncated) {
        (1, false) => "Exported 1 row.".into(),
        (_, false) => format!("Exported {rows} rows."),
        (_, true) => format!("Exported the first {rows} rows (the export's limit); narrow the query for the rest."),
    }
}

/// A console run's outcome as the UI expects it (`SqlOutcome`): `{ result }`, `{ error }` (the span to underline)
/// or `{ cancelled: true }`; a failure of the store (or a limit) stays an error of the command.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<SqlResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<QlError>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
}

fn outcome(out: Result<SqlResult, StoreError>) -> CmdResult<SqlOutcome> {
    let none = SqlOutcome {
        result: None,
        error: None,
        cancelled: false,
    };
    match out {
        Ok(result) => Ok(SqlOutcome {
            result: Some(result),
            ..none
        }),
        Err(StoreError::Query(error)) => Ok(SqlOutcome {
            error: Some(error),
            ..none
        }),
        Err(StoreError::Cancelled) => Ok(SqlOutcome { cancelled: true, ..none }),
        Err(e) => Err(e.into()),
    }
}

/// A note as `debrief.notes` sees it (text cut to [`SQL_NOTE_CHARS`]).
fn sql_note(n: &Note) -> SqlNote {
    let mut out = SqlNote {
        id: n.id.clone(),
        severity: serde_json::to_value(n.severity)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default(),
        tags: n.tags.clone(),
        text: n.text.chars().take(SQL_NOTE_CHARS).collect(),
        author: n.author.clone(),
        created: n.created_ms,
        updated: n.updated_ms,
        ..SqlNote::default()
    };
    let (anchor, from, to) = match &n.anchor {
        Anchor::Event { event_id, ts, .. } => {
            out.event_id = Some(*event_id);
            ("event", *ts, None)
        }
        Anchor::Instant { at } => ("instant", Some(*at), None),
        Anchor::Range { from, to } => ("range", Some(*from), Some(*to)),
        Anchor::Source { path } => {
            out.source_path = Some(path.clone());
            ("source", None, None)
        }
        Anchor::Marker { from, to, .. } => ("marker", Some(*from), *to),
        Anchor::ChartPoint { at, .. } => ("chartPoint", Some(*at), None),
        Anchor::Project => ("project", None, None),
        Anchor::Image { .. } => ("image", None, None),
    };
    out.anchor = anchor.to_string();
    out.from = from;
    out.to = to;
    out
}

/// Runs a console query as an analysis job (Cancel in the Jobs pane, or `jobs_cancel` with the id sent first on
/// `on_event`, interrupts it in the store). The window's notes go with it (`debrief.notes`).
#[tauri::command]
pub(crate) async fn sql_run<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    sql: String,
    explain: bool,
    on_event: Channel<QueryStreamEvent>,
) -> CmdResult<SqlOutcome> {
    // The store checks the text again; this keeps an oversized request from going further.
    if sql.len() > MAX_SQL_LEN {
        return Ok(SqlOutcome {
            error: Some(QlError::new(format!("The SQL text is longer than {} KB.", MAX_SQL_LEN >> 10), 0, 0)),
            result: None,
            cancelled: false,
        });
    }
    let notes = window_notes(&state);
    let request = SqlRequest { sql, explain, notes };
    let control = QueryControl::new();
    let stop = control.clone();
    let controls = Controls::cancel(move || stop.cancel());
    let started = on_event.clone();
    let announce = move |id: crate::jobs::JobId| {
        let _ = started.send(QueryStreamEvent::Started { job_id: id.0 });
    };
    let out = crate::jobs::analysis_announced(&app, &state, "SQL query", "store", None, controls, announce, move |store, _| {
        Ok(store.sql_run(&request))
    })
    .await?;
    outcome(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Json as _;
    use crate::test_support::Now as _;
    use crate::test_support::{block, main_window, samples, test_app};
    use serde_json::Value;
    use std::sync::{Arc, Mutex};
    use tauri::ipc::InvokeResponseBody;

    fn opened() -> tauri::App<tauri::test::MockRuntime> {
        let app = test_app(None);
        main_window(&app).queue(crate::instance::existing(&[samples().join("serilog-clef")]));
        let pending = crate::workspace_commands::take_pending_open(main_window(&app), crate::paths::WindowGrants::of(app.handle(), "main"))
            .now()
            .unwrap();
        block(crate::workspace_commands::open_paths(
            app.handle().clone(),
            main_window(&app),
            pending,
        ))
        .unwrap()
        .json();
        app
    }

    fn recorder() -> (Channel<QueryStreamEvent>, Arc<Mutex<Vec<Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                sink.lock().unwrap().push(serde_json::from_str(&text).unwrap());
            }
            Ok(())
        });
        (channel, seen)
    }

    #[test]
    fn the_reports_and_the_console_answer_for_the_window() {
        let app = opened();
        let o = block(store_overview(main_window(&app))).unwrap();
        assert!(o.events > 0 && o.segments > 0);
        let page = block(store_segments(main_window(&app), SegmentsRequest::default())).unwrap();
        assert_eq!(page.total, o.segments);
        assert!(!block(store_fields(main_window(&app))).unwrap().fields.is_empty());
        assert_eq!(block(store_index(main_window(&app))).unwrap().events, o.events);
        let m = block(store_memory(app.handle().clone(), main_window(&app))).unwrap();
        assert!(m.query_budget_bytes > 0);
        assert!(block(sql_schema(main_window(&app))).unwrap().views.len() == 4);

        let (channel, seen) = recorder();
        let out = block(sql_run(
            app.handle().clone(),
            main_window(&app),
            "SELECT count(*) AS n FROM events".into(),
            false,
            channel,
        ))
        .unwrap();
        let result = out.result.expect("rows");
        assert_eq!(result.rows[0][0], o.events);
        let events = seen.lock().unwrap().clone();
        assert_eq!(events[0]["kind"], "started", "the run is a job: {events:?}");
        let more = block(sql_page(
            main_window(&app),
            SqlPageRequest {
                result: result.result,
                offset: 0,
            },
        ))
        .unwrap();
        assert_eq!(more.rows.len(), 1);

        let (channel, _) = recorder();
        let refused = block(sql_run(
            app.handle().clone(),
            main_window(&app),
            "DROP VIEW events".into(),
            false,
            channel,
        ))
        .unwrap();
        let e = refused.error.clone().expect("a refusal");
        assert!(e.message.contains("DROP"), "{e:?}");
        let (channel, _) = recorder();
        let long = block(sql_run(
            app.handle().clone(),
            main_window(&app),
            "x".repeat(MAX_SQL_LEN + 1),
            false,
            channel,
        ))
        .unwrap();
        assert!(long.error.unwrap().message.contains("longer"));
        let wire = serde_json::to_value(&refused).unwrap();
        assert!(wire.get("result").is_none() && wire.get("cancelled").is_none());
    }

    #[test]
    fn outcomes_and_notes_have_their_wire_shape() {
        assert!(outcome(Err(StoreError::Cancelled)).unwrap().cancelled);
        assert!(outcome(Err(StoreError::Busy("x".into()))).is_err());
        let note = |anchor: Anchor| Note {
            id: "n".into(),
            anchor,
            text: "é".repeat(SQL_NOTE_CHARS + 10),
            tags: vec!["t".into()],
            severity: debrief_notes::NoteSeverity::Problem,
            author: "me".into(),
            created_ms: 1,
            updated_ms: 2,
        };
        let e = sql_note(&note(Anchor::Event {
            event_id: 7,
            ts: Some(9),
            source: String::new(),
            excerpt: String::new(),
        }));
        assert_eq!(
            (e.anchor.as_str(), e.event_id, e.from, e.severity.as_str()),
            ("event", Some(7), Some(9), "problem")
        );
        assert_eq!(e.text.chars().count(), SQL_NOTE_CHARS);
        let s = sql_note(&note(Anchor::Source { path: "/x/a.log".into() }));
        assert_eq!((s.anchor.as_str(), s.source_path.as_deref()), ("source", Some("/x/a.log")));
        let r = sql_note(&note(Anchor::Range { from: 1, to: 5 }));
        assert_eq!((r.from, r.to), (Some(1), Some(5)));
        assert_eq!(sql_note(&note(Anchor::Instant { at: 3 })).from, Some(3));
        assert_eq!(sql_note(&note(Anchor::Project)).anchor, "project");
        let m = sql_note(&note(Anchor::Marker {
            name: "m".into(),
            from: 1,
            to: Some(2),
        }));
        assert_eq!((m.anchor.as_str(), m.to), ("marker", Some(2)));
        let c = sql_note(&note(Anchor::ChartPoint {
            chart: "c".into(),
            series: None,
            at: 4,
            value: None,
        }));
        assert_eq!((c.anchor.as_str(), c.from), ("chartPoint", Some(4)));
    }
}
