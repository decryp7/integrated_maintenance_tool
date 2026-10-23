//! Cancellable query-language runs with progress (`run_query_stream`).
//!
//! Each run is an analysis job: Jobs are the one cancel authority, so the UI stops a run with
//! `jobs_cancel` and the job's stop action flips its [`QueryControl`]; the engine stops at its next
//! batch check. The run's [`Channel`] first says which job it is ([`QueryStreamEvent::Started`]), then
//! carries progress (sampled every [`PROGRESS_EVERY`] by a small ticker thread, so the engine never waits
//! on IPC). A cancelled run answers `{ "cancelled": true }`: a normal outcome, not an error. `run_query`
//! stays as it was for callers that need neither.

use crate::jobs::{Controls, JobProgress};
use crate::windows::CurrentWindow;
use crate::CmdResult;
use debrief_core::api::QueryRequest;
use debrief_core::ports::StoreError;
use debrief_core::ql::{QlError, QlResult, QueryControl, QueryProgress};
use serde::Serialize;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Runtime};

/// How often progress is sent while a query runs.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(150);

/// What a run's channel carries (`kind`-tagged on the wire; mirrored as `QueryStreamEvent` in api.types.ts).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum QueryStreamEvent {
    /// The run is job `jobId`: cancel it with `jobs_cancel`.
    #[serde(rename_all = "camelCase")]
    Started { job_id: u64 },
    /// How far the run is.
    Progress(QueryProgress),
}

/// Sends `control`'s progress to `channel` every [`PROGRESS_EVERY`] (when it changed) until dropped.
struct Ticker(Option<mpsc::Sender<()>>);

impl Ticker {
    fn start(control: QueryControl, channel: Channel<QueryStreamEvent>) -> Ticker {
        let (stop, stopped) = mpsc::channel::<()>();
        let spawned = std::thread::Builder::new().name("debrief-query-progress".into()).spawn(move || {
            let mut last = None;
            while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(PROGRESS_EVERY) {
                let now = control.progress();
                if last != Some(now) {
                    last = Some(now);
                    // A closed webview cannot receive it; the run itself carries on.
                    if channel.send(QueryStreamEvent::Progress(now)).is_err() {
                        break;
                    }
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("Query progress is unavailable: {e}");
        }
        Ticker(Some(stop))
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        // Dropping the sender wakes the ticker at once.
        self.0.take();
    }
}

/// The outcome as the UI expects it (`StreamedQueryOutcome`): `{ result }`, `{ error }` (with the span to
/// underline) or `{ cancelled: true }`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<QlResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<QlError>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
}

/// A run's outcome; a failure of the store (not of the query) stays an error of the command.
pub fn outcome(out: Result<QlResult, StoreError>) -> CmdResult<QueryOutcome> {
    let none = QueryOutcome {
        result: None,
        error: None,
        cancelled: false,
    };
    match out {
        Ok(result) => Ok(QueryOutcome {
            result: Some(result),
            ..none
        }),
        Err(StoreError::Query(error)) => Ok(QueryOutcome {
            error: Some(error),
            ..none
        }),
        Err(StoreError::Cancelled) => Ok(QueryOutcome { cancelled: true, ..none }),
        Err(e) => Err(e.into()),
    }
}

/// Runs `request` as an analysis job (Cancel in the Jobs pane, or `jobs_cancel` with the id sent first on
/// `on_progress`, cancels the run), streaming progress to `on_progress`.
pub async fn run_controlled<R: Runtime>(
    app: &AppHandle<R>,
    state: &CurrentWindow,
    control: QueryControl,
    request: QueryRequest,
    on_progress: Channel<QueryStreamEvent>,
) -> CmdResult<QueryOutcome> {
    let _ticker = Ticker::start(control.clone(), on_progress.clone());
    let watched = control.clone();
    let (stop, probe) = (control.clone(), control.clone());
    let controls = Controls::cancel(move || stop.cancel()).with_probe(move || {
        let p = probe.progress();
        Some(JobProgress::counts(p.done, p.total))
    });
    let ql = request.query.ql.clone();
    let view = state.host.redacted_view();
    let started = on_progress.clone();
    let announce = move |id: crate::jobs::JobId| {
        let _ = started.send(QueryStreamEvent::Started { job_id: id.0 });
    };
    let out = crate::jobs::analysis_announced(app, state, "Run a query", "query", Some(ql), controls, announce, move |store, _| {
        let mut out = store.run_query_with(&request, &watched);
        if let (Ok(result), Some(r)) = (&mut out, &view) {
            r.view_ql(result);
        }
        Ok(out)
    })
    .await?;
    if matches!(out, Err(StoreError::Cancelled)) {
        log::info!(Stage = format!("{:?}", control.progress().stage).as_str(); "Query cancelled by the user");
    } else {
        // The final count, so the UI's progress ends where the run did.
        let _ = on_progress.send(QueryStreamEvent::Progress(control.progress()));
    }
    outcome(out)
}

/// Like `run_query`, as a job the UI cancels with `jobs_cancel` (the id is the channel's first event),
/// with progress on `onProgress`. See `api.types.ts` (`StreamedQueryOutcome`, `QueryStreamEvent`).
#[tauri::command]
pub async fn run_query_stream<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    request: QueryRequest,
    on_progress: Channel<QueryStreamEvent>,
) -> CmdResult<QueryOutcome> {
    use tracing::Instrument;
    let span = tracing::info_span!("query", Window = state.label());
    run_controlled(&app, &state, QueryControl::new(), request, on_progress)
        .instrument(span)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Json as _;
    use crate::test_support::Now as _;
    use crate::test_support::{block, main_window, samples, test_app};
    use debrief_core::query::Query;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex as StdMutex};
    use tauri::ipc::InvokeResponseBody;
    use tauri::Manager;

    fn recorder(on: impl Fn(&Value) + Send + Sync + 'static) -> (Channel<QueryStreamEvent>, Arc<StdMutex<Vec<Value>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let sink = seen.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                let v: Value = serde_json::from_str(&text).unwrap();
                on(&v);
                sink.lock().unwrap().push(v);
            }
            Ok(())
        });
        (channel, seen)
    }

    fn req(ql: &str) -> QueryRequest {
        QueryRequest {
            query: Query {
                ql: ql.into(),
                ..Default::default()
            },
        }
    }

    fn opened() -> tauri::App<tauri::test::MockRuntime> {
        let app = test_app(None);
        main_window(&app).queue(crate::instance::existing(&[samples().join("serilog-clef")]));
        let pending = crate::workspace_commands::take_pending_open(main_window(&app), crate::paths::WindowGrants::of(app.handle(), "main"))
            .now()
            .unwrap();
        crate::test_support::finished(
            &app,
            block(crate::workspace_commands::open_paths(
                app.handle().clone(),
                main_window(&app),
                pending,
            )),
        )
        .unwrap()
        .json();
        app
    }

    #[test]
    fn a_run_says_its_job_streams_progress_and_answers_like_run_query() {
        let app = opened();
        let (channel, seen) = recorder(|_| {});
        let ql = "level >= information | stats count() as n";
        let out = block(run_query_stream(app.handle().clone(), main_window(&app), req(ql), channel))
            .unwrap()
            .json();
        assert_eq!(out["result"]["kind"], "table");
        let plain = block(crate::insights::run_query(main_window(&app), req(ql))).unwrap().json();
        assert_eq!(out, plain);
        let events = seen.lock().unwrap().clone();
        assert_eq!(events[0]["kind"], "started");
        let job = events[0]["jobId"].as_u64().unwrap();
        assert!(job > 0, "the run is a job");
        let last = events.last().cloned().expect("final progress");
        assert_eq!(last["kind"], "progress");

        let (channel, _) = recorder(|_| {});
        let out = block(run_controlled(
            app.handle(),
            &main_window(&app),
            QueryControl::new(),
            req("level >"),
            channel,
        ))
        .unwrap()
        .json();
        assert!(out["error"]["offset"].as_u64().unwrap() > 0, "{out}");
    }

    #[test]
    fn jobs_cancel_the_run_and_the_window_stays_usable() {
        let app = opened();
        // Cancelling the announced job stops the run's control.
        let control = QueryControl::new();
        let jobs = app.state::<crate::jobs::Jobs>().inner().clone();
        let (channel, _) = recorder(move |e| {
            if e["kind"] == "started" {
                let _ = jobs.cancel(crate::jobs::JobId(e["jobId"].as_u64().unwrap()));
            }
        });
        let _ = block(run_controlled(
            app.handle(),
            &main_window(&app),
            control.clone(),
            req("level >= debug"),
            channel,
        ))
        .unwrap();
        assert!(control.is_cancelled());
        // A cancelled run answers cancelled.
        let cancelled = QueryControl::new();
        cancelled.cancel();
        let (channel, _) = recorder(|_| {});
        let out = block(run_controlled(
            app.handle(),
            &main_window(&app),
            cancelled,
            req("level >= debug"),
            channel,
        ))
        .unwrap()
        .json();
        assert_eq!(out, json!({ "cancelled": true }));
        // The next run is complete.
        let (channel, _) = recorder(|_| {});
        let out = block(run_controlled(
            app.handle(),
            &main_window(&app),
            QueryControl::new(),
            req("level >= debug"),
            channel,
        ))
        .unwrap()
        .json();
        assert_eq!(out["result"]["kind"], "events");
    }

    #[test]
    fn outcomes_are_typed() {
        assert_eq!(outcome(Err(StoreError::Cancelled)).unwrap().json(), json!({ "cancelled": true }));
        let e = outcome(Err(StoreError::Query(debrief_core::ql::QlError::new("bad", 1, 2))))
            .unwrap()
            .json();
        assert_eq!(
            (e["error"]["message"].as_str(), e["error"]["offset"].as_u64()),
            (Some("bad"), Some(1))
        );
        // A failure of the store is an error of the command, not an outcome.
        assert!(outcome(Err(StoreError::Engine("gone".into()))).is_err());
    }

    #[test]
    fn the_ticker_reports_changes_until_stopped() {
        let control = QueryControl::new();
        let (channel, seen) = recorder(|_| {});
        let ticker = Ticker::start(control.clone(), channel);
        control.begin(debrief_core::ql::QueryStage::Matching, 10);
        control.advance(5);
        let start = std::time::Instant::now();
        while seen.lock().unwrap().is_empty() && start.elapsed() < Duration::from_secs(10) {
            std::thread::yield_now();
        }
        drop(ticker);
        let got = seen.lock().unwrap().clone();
        assert_eq!(got[0], json!({ "kind": "progress", "stage": "matching", "done": 5, "total": 10 }));
        // Unchanged progress is not resent.
        std::thread::sleep(PROGRESS_EVERY * 3);
        assert!(seen.lock().unwrap().len() <= 2);
    }
}
