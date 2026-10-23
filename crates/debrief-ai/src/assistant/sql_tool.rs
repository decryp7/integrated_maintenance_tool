//! `run_sql`: the guarded SQL fallback for questions the query language cannot express. It reuses the Store pane's SQL
//! console through the [`Console`](debrief_core::ports::Console) port, so every guardrail of that console applies as it
//! is (ADR 0030): one read-only statement, an allow-list of statements and table functions, no file, network or
//! extension access, a time limit, a row cap and a memory limit. The console's database holds only the documented
//! views (`debrief_core::ports::SQL_VIEW_TABLE`, documented in `docs/ai/sql-views.md`); the store's internal tables
//! are not in it, and this tool names no others. The `notes` view stays empty: notes are the user's and never go to the model.

use super::tools::{Evidence, ToolOutcome};
use debrief_core::ports::{ai_views_text, SqlRequest, SqlResult, Store, StoreError, SQL_MAX_ROWS};
use serde::Deserialize;
use serde_json::{json, Value};

/// The tool's name (also what the chat shows on its chip).
pub const RUN_SQL: &str = "run_sql";

/// Rows a result shows the model at most (the console keeps up to [`SQL_MAX_ROWS`]; `totalRows` says how many).
pub const MAX_ROWS_SHOWN: usize = 50;

#[derive(Deserialize)]
pub(super) struct SqlArg {
    pub sql: String,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// The model's description of the tool, generated from the one table of views.
pub fn description() -> String {
    format!(
        "LAST RESORT. Run one read-only SQL SELECT (DuckDB dialect) over the views below. Use it ONLY when the query language of \
         run_query cannot express the question: joins across sources, window functions, percentiles or gaps between events, \
         arbitrary grouping. A filter on a level, field or name is NOT a reason: use find_events / find_values / run_query. \
         One statement; no writes, files, settings or extensions (refused with a reason); results are cut at {SQL_MAX_ROWS} rows \
         and a time limit applies, so aggregate and add LIMIT. Select `id` from events to get citable event ids. \
         Tell the user you used SQL. The views (the only tables there are; `fields` of events is a MAP: fields['Key']):\n{}",
        ai_views_text()
    )
}

/// What the model sees of a console result: columns, the first rows, the event ids when `id` is selected, and the counts.
pub(super) fn result_json(r: &SqlResult, limit: usize) -> Value {
    let limit = limit.clamp(1, MAX_ROWS_SHOWN);
    let id_col = if r.event_ids {
        r.columns.iter().position(|c| c.name.eq_ignore_ascii_case("id"))
    } else {
        None
    };
    let rows: Vec<&Vec<Value>> = r.rows.iter().take(limit).collect();
    let mut out = json!({
        "kind": "table",
        "columns": r.columns.iter().map(|c| json!({"name": c.name, "type": c.ty})).collect::<Vec<_>>(),
        "rows": rows,
        "totalRows": r.total,
        "truncated": r.truncated,
    });
    if let Some(i) = id_col {
        let ids: Vec<u64> = r
            .rows
            .iter()
            .take(limit)
            .filter_map(|row| row.get(i).and_then(Value::as_u64))
            .collect();
        out["ids"] = json!(ids);
    }
    out
}

/// The refusal or failure as a message the model can act on.
pub(super) fn failure(e: &StoreError) -> ToolOutcome {
    match e {
        StoreError::Query(q) => ToolOutcome::err(format!(
            "SQL refused or invalid: {} (at byte {}); only one read-only SELECT over the views events, sources and fields is allowed",
            q.message, q.offset
        )),
        StoreError::Invalid(m) | StoreError::Busy(m) => ToolOutcome::err(m.clone()),
        other => ToolOutcome::err(format!("the SQL cannot run: {other}")),
    }
}

/// Runs `a.sql` through the console; the SQL text rides along in the evidence for the chat chip.
pub(super) fn run(s: &dyn Store, a: SqlArg, finish: impl FnOnce(Value, String) -> ToolOutcome) -> ToolOutcome {
    let sql = a.sql.trim().to_string();
    let req = SqlRequest {
        sql: sql.clone(),
        ..SqlRequest::default()
    };
    match s.sql_run(&req) {
        Err(e) => {
            let mut o = failure(&e);
            o.evidence = Evidence {
                sql: vec![sql],
                ..Evidence::default()
            };
            o
        }
        Ok(r) => {
            let mut o = finish(result_json(&r, a.limit.unwrap_or(20)), format!("SQL: {} rows", r.total));
            o.evidence.sql = vec![sql];
            o
        }
    }
}
