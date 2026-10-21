//! Commands for analyses over the loaded events (query language, patterns, exceptions, anomalies,
//! comparisons, traces, the Moment view, parse preview). Each runs in the window's store (the store worker);
//! the slow ones are cancellable analysis jobs (Cancel interrupts the store's statements), and every analysis
//! says how much of the matching data it covered (`Analyzed`).

use crate::engine::with_store;
use crate::windows::CurrentWindow;
use crate::CmdResult;
use imt_core::analysis::{Anomaly, Comparison, FieldStats, RequestSpan, WaterfallRow};
use imt_core::api::{
    CompareRequest, CompletionRequest, CorrelateRequest, FieldStatsRequest, MomentRequest, MomentSummary, ParsePreview,
    ParsePreviewRequest, QueryRequest, RequestsRequest, TraceRequest, TraceSummary,
};
use imt_core::batch::EventId;
use imt_core::exceptions::ExceptionGroup;
use imt_core::patterns::Pattern;
use imt_core::ports::{Analyzed, Coverage, Store, StoreError, StoreResult};
use imt_core::ql::{Completion, QlError, QlResult};
use serde::Serialize;
use tauri::{AppHandle, Runtime};

/// An analysis view's computation as an analysis job (visible in the Jobs pane when it takes a while; Cancel
/// stops it in the store).
async fn analysis<R: Runtime, T: Send + 'static>(
    app: &AppHandle<R>,
    state: &CurrentWindow,
    title: &str,
    view: &str,
    ql: &str,
    f: impl FnOnce(&dyn Store) -> StoreResult<T> + Send + 'static,
) -> CmdResult<T> {
    crate::jobs::analysis(
        app,
        state,
        title,
        view,
        Some(ql.to_string()),
        Default::default(),
        move |store, _| f(store).map_err(crate::CmdError::from),
    )
    .await
}

/// The answer of `run_query`: the result, or the error to underline in the editor.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueryReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<QlResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<QlError>,
}

/// `{ result }` or `{ error: { message, offset, len } }` so the editor can underline the error.
#[tauri::command]
pub async fn run_query(state: CurrentWindow, request: QueryRequest) -> CmdResult<QueryReply> {
    with_store(&state, move |s| match s.run_query(&request) {
        Ok(result) => Ok(QueryReply {
            result: Some(result),
            error: None,
        }),
        Err(StoreError::Query(error)) => Ok(QueryReply {
            result: None,
            error: Some(error),
        }),
        Err(e) => Err(e),
    })
    .await
}

#[tauri::command]
pub async fn check_query(state: CurrentWindow, text: String) -> CmdResult<Option<QlError>> {
    with_store(&state, move |s| s.check_query(&text)).await
}

#[tauri::command]
pub async fn complete_query(state: CurrentWindow, request: CompletionRequest) -> CmdResult<Vec<Completion>> {
    with_store(&state, move |s| s.complete_query(&request)).await
}

#[tauri::command]
pub async fn query_patterns<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    request: QueryRequest,
) -> CmdResult<Analyzed<Vec<Pattern>>> {
    let ql = request.query.ql.clone();
    analysis(&app, &state, "Find patterns", "patterns", &ql, move |s| s.patterns(&request)).await
}

#[tauri::command]
pub async fn query_exceptions<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    request: QueryRequest,
) -> CmdResult<Analyzed<Vec<ExceptionGroup>>> {
    let ql = request.query.ql.clone();
    analysis(&app, &state, "Group exceptions", "exceptions", &ql, move |s| {
        s.exception_groups(&request)
    })
    .await
}

#[tauri::command]
pub async fn query_anomalies<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    request: QueryRequest,
) -> CmdResult<Analyzed<Vec<Anomaly>>> {
    let ql = request.query.ql.clone();
    analysis(&app, &state, "Detect anomalies", "anomalies", &ql, move |s| s.anomalies(&request)).await
}

#[tauri::command]
pub async fn compare_windows<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    request: CompareRequest,
) -> CmdResult<Analyzed<Comparison>> {
    analysis(&app, &state, "Compare two time windows", "compare", "", move |s| {
        s.compare(&request)
    })
    .await
}

#[tauri::command]
pub async fn field_stats(state: CurrentWindow, request: FieldStatsRequest) -> CmdResult<FieldStats> {
    with_store(&state, move |s| s.field_stats(&request)).await
}

#[tauri::command]
pub async fn correlate<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    request: CorrelateRequest,
) -> CmdResult<Analyzed<Vec<EventId>>> {
    analysis(&app, &state, "Correlate events", "correlate", "", move |s| s.correlate(&request)).await
}

/// The trace list, or one trace's waterfall when `traceId` is set, with the coverage.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TracesReply {
    traces: Vec<TraceSummary>,
    waterfall: Vec<WaterfallRow>,
    coverage: Coverage,
}

#[tauri::command]
pub async fn query_traces<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, request: TraceRequest) -> CmdResult<TracesReply> {
    analysis(&app, &state, "Find traces", "traces", "", move |s| {
        let a = s.traces(&request)?;
        let (traces, waterfall) = a.result;
        Ok(TracesReply {
            traces,
            waterfall,
            coverage: a.coverage,
        })
    })
    .await
}

#[tauri::command]
pub async fn query_requests<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    request: RequestsRequest,
) -> CmdResult<Analyzed<Vec<RequestSpan>>> {
    analysis(&app, &state, "Find requests", "requests", "", move |s| s.requests(&request)).await
}

#[tauri::command]
pub async fn query_moment<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    request: MomentRequest,
) -> CmdResult<Analyzed<MomentSummary>> {
    analysis(&app, &state, "Analyse a moment", "moment", "", move |s| s.moment(&request)).await
}

#[tauri::command]
pub async fn preview_parse(state: CurrentWindow, request: ParsePreviewRequest) -> CmdResult<ParsePreview> {
    with_store(&state, move |s| s.preview_parse(&request)).await
}

/// Infers a regex with named, typed groups from example lines (with explanation).
#[tauri::command]
pub async fn infer_regex(lines: Vec<String>) -> CmdResult<imt_core::regex_assist::InferredRegex> {
    crate::error::lift(imt_core::regex_assist::infer(&lines).map_err(crate::err))
}

/// Explains a regex in plain English as an indented outline.
#[tauri::command]
pub async fn explain_regex(pattern: String) -> CmdResult<Vec<imt_core::regex_assist::ExplainNode>> {
    crate::error::lift(imt_core::regex_assist::explain(&pattern).map_err(crate::err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Now as _;

    #[test]
    fn regex_assist_commands() {
        let r = infer_regex(vec!["order 1 ok".into(), "order 22 ok".into()]).now().unwrap();
        assert_eq!(serde_json::to_value(r).unwrap()["pattern"], r"order (?P<order>\d+) ok");
        assert!(infer_regex(vec![]).now().is_err());
        assert!(explain_regex(r"\d+".into()).now().unwrap().len() >= 2);
        assert!(explain_regex("(".into()).now().is_err());
    }
}
