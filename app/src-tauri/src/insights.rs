//! Commands for analyses over the loaded events (query language, patterns,
//! exceptions, anomalies, comparisons, traces, the Moment view, parse preview).

use crate::windows::{read, CurrentWindow};
use crate::CmdResult;
use imt_core::api::{
    CompareRequest, CompletionRequest, CorrelateRequest, FieldStatsRequest, MomentRequest, ParsePreviewRequest, QueryRequest,
    RequestsRequest, TraceRequest,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Runtime};

/// An analysis view's computation as an analysis job (visible in the Jobs pane when it takes a while).
async fn analysis<R: Runtime>(
    app: &AppHandle<R>,
    state: &CurrentWindow,
    title: &str,
    view: &str,
    ql: &str,
    f: impl FnOnce(&dyn imt_core::LogStore) -> anyhow::Result<Value> + Send + 'static,
) -> CmdResult<Value> {
    crate::jobs::analysis(app, state, title, view, Some(ql.to_string()), Default::default(), move |ws, _| {
        f(ws)
    })
    .await
}

fn to_value(v: impl serde::Serialize) -> anyhow::Result<Value> {
    Ok(serde_json::to_value(v)?)
}

/// `{ "result": QlResult }` or `{ "error": { message, offset, len } }` so the
/// editor can underline the error.
#[tauri::command]
pub async fn run_query(state: CurrentWindow, request: QueryRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        Ok(match ws.run_query(&request) {
            Ok(result) => json!({ "result": result }),
            Err(error) => json!({ "error": error }),
        })
    })
    .await
}

#[tauri::command]
pub async fn check_query(state: CurrentWindow, text: String) -> CmdResult<Value> {
    read(&state, move |ws| to_value(ws.check_query(&text))).await
}

#[tauri::command]
pub async fn complete_query(state: CurrentWindow, request: CompletionRequest) -> CmdResult<Value> {
    read(&state, move |ws| to_value(ws.complete_query(&request))).await
}

#[tauri::command]
pub async fn query_patterns<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, request: QueryRequest) -> CmdResult<Value> {
    let ql = request.query.ql.clone();
    analysis(&app, &state, "Find patterns", "patterns", &ql, move |ws| {
        to_value(ws.patterns(&request)?)
    })
    .await
}

/// How many of the matching records the analyses look at, of how many: the UI notes partial results.
#[tauri::command]
pub async fn analysis_coverage(state: CurrentWindow, request: QueryRequest) -> CmdResult<imt_core::api::AnalysisCoverage> {
    read(&state, move |ws| ws.analysis_coverage(&request)).await
}

#[tauri::command]
pub async fn query_exceptions<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, request: QueryRequest) -> CmdResult<Value> {
    let ql = request.query.ql.clone();
    analysis(&app, &state, "Group exceptions", "exceptions", &ql, move |ws| {
        to_value(ws.exception_groups(&request)?)
    })
    .await
}

#[tauri::command]
pub async fn query_anomalies<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, request: QueryRequest) -> CmdResult<Value> {
    let ql = request.query.ql.clone();
    analysis(&app, &state, "Detect anomalies", "anomalies", &ql, move |ws| {
        to_value(ws.anomalies(&request)?)
    })
    .await
}

#[tauri::command]
pub async fn compare_windows<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, request: CompareRequest) -> CmdResult<Value> {
    analysis(&app, &state, "Compare two time windows", "compare", "", move |ws| {
        to_value(ws.compare(&request)?)
    })
    .await
}

#[tauri::command]
pub async fn field_stats(state: CurrentWindow, request: FieldStatsRequest) -> CmdResult<Value> {
    read(&state, move |ws| to_value(ws.field_stats(&request)?)).await
}

#[tauri::command]
pub async fn correlate(state: CurrentWindow, request: CorrelateRequest) -> CmdResult<Value> {
    read(&state, move |ws| to_value(ws.correlate(&request))).await
}

/// `{ traces, waterfall }`: the trace list, or one trace's waterfall when `traceId` is set.
#[tauri::command]
pub async fn query_traces(state: CurrentWindow, request: TraceRequest) -> CmdResult<Value> {
    read(&state, move |ws| {
        let (traces, waterfall) = ws.traces(&request)?;
        Ok(json!({ "traces": traces, "waterfall": waterfall }))
    })
    .await
}

#[tauri::command]
pub async fn query_requests(state: CurrentWindow, request: RequestsRequest) -> CmdResult<Value> {
    read(&state, move |ws| to_value(ws.requests(&request)?)).await
}

#[tauri::command]
pub async fn query_moment<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, request: MomentRequest) -> CmdResult<Value> {
    analysis(&app, &state, "Analyse a moment", "moment", "", move |ws| {
        to_value(ws.moment(&request)?)
    })
    .await
}

#[tauri::command]
pub async fn preview_parse(state: CurrentWindow, request: ParsePreviewRequest) -> CmdResult<Value> {
    read(&state, move |ws| to_value(ws.preview_parse(&request))).await
}

/// Infers a regex with named, typed groups from example lines (with explanation).
#[tauri::command]
pub fn infer_regex(lines: Vec<String>) -> CmdResult<Value> {
    serde_json::to_value(imt_core::regex_assist::infer(&lines).map_err(crate::err)?).map_err(crate::err)
}

/// Explains a regex in plain English as an indented outline.
#[tauri::command]
pub fn explain_regex(pattern: String) -> CmdResult<Value> {
    serde_json::to_value(imt_core::regex_assist::explain(&pattern).map_err(crate::err)?).map_err(crate::err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_assist_commands() {
        let r = infer_regex(vec!["order 1 ok".into(), "order 22 ok".into()]).unwrap();
        assert_eq!(r["pattern"], r"order (?P<order>\d+) ok");
        assert!(infer_regex(vec![]).is_err());
        assert!(explain_regex(r"\d+".into()).unwrap().as_array().unwrap().len() >= 2);
        assert!(explain_regex("(".into()).is_err());
    }
}
