//! Cancellable query-language runs with progress (`run_query_stream`, `cancel_query`).
//!
//! The UI names each run with a `queryId` it generates. While the run is in flight its
//! [`QueryControl`] is registered here under (window, id); `cancel_query` from the same window flips it,
//! and the engine stops at its next batch check. Progress goes to the UI through a Tauri [`Channel`]
//! (sampled every [`PROGRESS_EVERY`] by a small ticker thread, so the engine never waits on IPC). A
//! cancelled run answers `{ "cancelled": true }`: a normal outcome, not an error. `run_query` stays as
//! it was for callers that need neither.

use crate::windows::{read, CurrentWindow};
use crate::{CmdResult, State};
use imt_core::api::QueryRequest;
use imt_core::ql::{QueryControl, QueryProgress, RunError};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::time::Duration;
use tauri::ipc::Channel;

/// How often progress is sent while a query runs.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(150);
/// Most queries one window may have in flight (the UI runs one at a time; this bounds a misbehaving webview).
pub const MAX_PER_WINDOW: usize = 8;

/// The queries in flight, by (window, query id). App-wide managed state.
#[derive(Debug, Default)]
pub struct QueryJobs(Mutex<HashMap<(String, String), QueryControl>>);

/// Removes its job when the run ends, however it ends.
#[derive(Debug)]
pub struct JobGuard<'a> {
    jobs: &'a QueryJobs,
    key: (String, String),
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        self.jobs.lock().remove(&self.key);
    }
}

impl QueryJobs {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), QueryControl>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Registers a new run of `window` named `id`. Fails for a malformed or duplicate id, or when the
    /// window already has [`MAX_PER_WINDOW`] runs.
    pub fn start(&self, window: &str, id: &str) -> CmdResult<(QueryControl, JobGuard<'_>)> {
        let id = crate::activity::valid_op_id(id).ok_or("invalid query id: use 1-64 letters, digits or '-'")?;
        let mut jobs = self.lock();
        if jobs.keys().filter(|(w, _)| w == window).count() >= MAX_PER_WINDOW {
            return Err(format!(
                "too many queries running in this window (at most {MAX_PER_WINDOW}); cancel one first"
            ));
        }
        let key = (window.to_string(), id);
        if jobs.contains_key(&key) {
            return Err("a query with this id is already running".into());
        }
        let control = QueryControl::new();
        jobs.insert(key.clone(), control.clone());
        Ok((control, JobGuard { jobs: self, key }))
    }

    /// Cancels the run `id` of `window`. False when there is none (finished already, or never started).
    pub fn cancel(&self, window: &str, id: &str) -> bool {
        match self.lock().get(&(window.to_string(), id.to_string())) {
            Some(control) => {
                control.cancel();
                true
            }
            None => false,
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lock().len()
    }
}

/// Sends `control`'s progress to `channel` every [`PROGRESS_EVERY`] (when it changed) until dropped.
struct Ticker(Option<mpsc::Sender<()>>);

impl Ticker {
    fn start(control: QueryControl, channel: Channel<QueryProgress>) -> Ticker {
        let (stop, stopped) = mpsc::channel::<()>();
        let spawned = std::thread::Builder::new().name("imt-query-progress".into()).spawn(move || {
            let mut last = None;
            while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(PROGRESS_EVERY) {
                let now = control.progress();
                if last != Some(now) {
                    last = Some(now);
                    // A closed webview cannot receive it; the run itself carries on.
                    if channel.send(now).is_err() {
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

/// The outcome as the UI expects it: `{ result }`, `{ error }` (with the span to underline) or
/// `{ cancelled: true }`.
pub fn outcome(out: Result<imt_core::ql::QlResult, RunError>) -> Value {
    match out {
        Ok(result) => json!({ "result": result }),
        Err(RunError::Invalid(error)) => json!({ "error": error }),
        Err(RunError::Cancelled) => json!({ "cancelled": true }),
    }
}

/// Runs `request` under `control`, streaming progress to `on_progress`.
pub async fn run_controlled(
    state: &CurrentWindow,
    control: QueryControl,
    request: QueryRequest,
    on_progress: Channel<QueryProgress>,
) -> CmdResult<Value> {
    let _ticker = Ticker::start(control.clone(), on_progress.clone());
    let watched = control.clone();
    let out = read(state, move |ws| Ok(ws.run_query_with(&request, &watched))).await?;
    if matches!(out, Err(RunError::Cancelled)) {
        log::info!(Stage = format!("{:?}", control.progress().stage).as_str(); "Query cancelled by the user");
    } else {
        // The final count, so the UI's progress ends where the run did.
        let _ = on_progress.send(control.progress());
    }
    Ok(outcome(out))
}

/// Like `run_query`, cancellable with [`cancel_query`] (same `queryId`, same window), with progress on
/// `onProgress`. See `api.types.ts` (`StreamedQueryOutcome`).
#[tauri::command]
pub async fn run_query_stream(
    state: CurrentWindow,
    jobs: State<'_, QueryJobs>,
    request: QueryRequest,
    query_id: String,
    on_progress: Channel<QueryProgress>,
) -> CmdResult<Value> {
    use tracing::Instrument;
    let (control, _job) = jobs.start(state.label(), &query_id)?;
    let span = tracing::info_span!("query", QueryId = query_id.as_str(), Window = state.label());
    run_controlled(&state, control, request, on_progress).instrument(span).await
}

/// Cancels the window's query `queryId`; false when it is not running (any more).
#[tauri::command]
pub fn cancel_query(state: CurrentWindow, jobs: State<'_, QueryJobs>, query_id: String) -> bool {
    jobs.cancel(state.label(), &query_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, samples, test_app};
    use crate::windows::MAIN;
    use imt_core::query::Query;
    use std::sync::{Arc, Mutex as StdMutex};
    use tauri::ipc::InvokeResponseBody;

    fn recorder() -> (Channel<QueryProgress>, Arc<StdMutex<Vec<Value>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let sink = seen.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                sink.lock().unwrap().push(serde_json::from_str(&text).unwrap());
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
        let pending = crate::workspace_commands::take_pending_open(main_window(&app)).unwrap();
        block(crate::workspace_commands::open_paths(
            app.handle().clone(),
            main_window(&app),
            pending,
        ))
        .unwrap();
        app
    }

    #[test]
    fn a_run_streams_progress_and_answers_like_run_query() {
        let app = opened();
        let jobs = QueryJobs::default();
        let (channel, seen) = recorder();
        let (control, job) = jobs.start(MAIN, "q-1").unwrap();
        let out = block(run_controlled(
            &main_window(&app),
            control,
            req("level >= information | stats count() as n"),
            channel,
        ))
        .unwrap();
        drop(job);
        assert_eq!(out["result"]["kind"], "table");
        let plain = block(crate::insights::run_query(
            main_window(&app),
            req("level >= information | stats count() as n"),
        ))
        .unwrap();
        assert_eq!(out, plain);
        let last = seen.lock().unwrap().last().cloned().expect("final progress");
        assert_eq!(last["stage"], "evaluating");
        assert_eq!(last["done"], last["total"]);
        assert_eq!(jobs.len(), 0);

        let (channel, _) = recorder();
        let (control, _job) = jobs.start(MAIN, "q-2").unwrap();
        let out = block(run_controlled(&main_window(&app), control, req("level >"), channel)).unwrap();
        assert!(out["error"]["offset"].as_u64().unwrap() > 0, "{out}");
    }

    #[test]
    fn a_cancelled_run_answers_cancelled_and_the_window_stays_usable() {
        let app = opened();
        let jobs = QueryJobs::default();
        let (control, job) = jobs.start(MAIN, "q-c").unwrap();
        assert!(!jobs.cancel("other-window", "q-c"), "another window cannot cancel it");
        assert!(jobs.cancel(MAIN, "q-c"));
        let (channel, _) = recorder();
        let out = block(run_controlled(&main_window(&app), control, req("level >= debug"), channel)).unwrap();
        assert_eq!(out, json!({ "cancelled": true }));
        drop(job);
        assert!(!jobs.cancel(MAIN, "q-c"), "finished runs are gone");
        // The next run is complete.
        let (control, _job) = jobs.start(MAIN, "q-c").unwrap();
        let (channel, _) = recorder();
        let out = block(run_controlled(&main_window(&app), control, req("level >= debug"), channel)).unwrap();
        assert_eq!(out["result"]["kind"], "events");
    }

    #[test]
    fn ids_are_validated_unique_and_bounded() {
        let jobs = QueryJobs::default();
        assert!(jobs.start(MAIN, "").is_err());
        assert!(jobs.start(MAIN, "bad id; drop").unwrap_err().contains("invalid query id"));
        let _a = jobs.start(MAIN, "a").unwrap();
        assert!(jobs.start(MAIN, "a").unwrap_err().contains("already running"));
        let _b = jobs.start("w-2", "a").expect("ids are per window");
        let mut held = Vec::new();
        for i in 1..MAX_PER_WINDOW {
            held.push(jobs.start(MAIN, &format!("q{i}")).unwrap());
        }
        assert!(jobs.start(MAIN, "one-more").unwrap_err().contains("too many"));
        held.clear();
        assert!(jobs.start(MAIN, "one-more").is_ok());
    }

    #[test]
    fn outcomes_are_typed() {
        assert_eq!(outcome(Err(RunError::Cancelled)), json!({ "cancelled": true }));
        let e = outcome(Err(RunError::Invalid(imt_core::ql::QlError::new("bad", 1, 2))));
        assert_eq!(
            (e["error"]["message"].as_str(), e["error"]["offset"].as_u64()),
            (Some("bad"), Some(1))
        );
    }

    #[test]
    fn the_ticker_reports_changes_until_stopped() {
        let control = QueryControl::new();
        let (channel, seen) = recorder();
        let ticker = Ticker::start(control.clone(), channel);
        control.begin(imt_core::ql::QueryStage::Matching, 10);
        control.advance(5);
        let start = std::time::Instant::now();
        while seen.lock().unwrap().is_empty() && start.elapsed() < Duration::from_secs(10) {
            std::thread::yield_now();
        }
        drop(ticker);
        let got = seen.lock().unwrap().clone();
        assert_eq!(got[0], json!({ "stage": "matching", "done": 5, "total": 10 }));
        // Unchanged progress is not resent.
        std::thread::sleep(PROGRESS_EVERY * 3);
        assert!(seen.lock().unwrap().len() <= 2);
    }
}
