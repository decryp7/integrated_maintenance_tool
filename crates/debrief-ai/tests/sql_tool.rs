//! The assistant's guarded SQL fallback (`run_sql`): it goes through the Store pane's SQL console, so each guardrail of
//! the console is triggered here through the tool, the views are pinned and match docs/ai/sql-views.md, and a scripted
//! model uses SQL for a percentile of gaps but not for a simple filter. All data is invented.

#[path = "support/duck_store.rs"]
mod duck_store;

use debrief_ai::assistant::{Assistant, AssistantConfig, EngineTools, ToolHost, ToolLimits};
use debrief_ai::chat::*;
use debrief_ai::llm::{Reply, ScriptedProvider};
use debrief_core::ports::{ai_views_text, AI_VIEWS, SQL_MAX_ROWS, SQL_VIEW_TABLE};
use duck_store::{clef, open, Fixture};
use serde_json::{json, Value};
use std::time::Duration;

fn fixture() -> Fixture {
    let rows = [
        ("2026-10-14T09:00:00.000Z", "Information", "Started", "agent.exe"),
        ("2026-10-14T09:00:01.000Z", "Error", "Render failed", "CompanyA.Graphic.Builder.exe"),
        (
            "2026-10-14T09:00:03.000Z",
            "Information",
            "Render ok",
            "CompanyA.Graphic.Builder.exe",
        ),
        ("2026-10-14T09:00:10.000Z", "Warning", "Slow disk", "agent.exe"),
        (
            "2026-10-14T09:00:30.000Z",
            "Error",
            "Render failed again",
            "CompanyA.Graphic.Builder.exe",
        ),
    ];
    open(&[("host.log", clef(&rows))])
}

fn call(f: &Fixture, sql: &str) -> debrief_ai::assistant::ToolOutcome {
    let mut t = EngineTools::new(f.store(), ToolLimits::for_context(8192));
    t.call("run_sql", &json!({"sql": sql}))
}

fn ok(f: &Fixture, sql: &str) -> Value {
    let o = call(f, sql);
    assert!(!o.is_error, "{sql}: {}", o.text);
    assert_eq!(o.evidence.sql, [sql.to_string()], "the SQL text rides along for the chip");
    serde_json::from_str(&o.text).unwrap()
}

#[test]
fn the_tool_is_described_from_the_one_table_and_is_a_last_resort() {
    let specs = EngineTools::tool_specs();
    let sql = specs.iter().find(|s| s.name == "run_sql").expect("run_sql is offered");
    assert!(
        sql.description.contains("LAST RESORT") && sql.description.contains("cannot express"),
        "{}",
        sql.description
    );
    assert!(sql.description.contains(&ai_views_text()), "generated from the table");
    for (name, _, cols) in SQL_VIEW_TABLE.iter() {
        let listed = AI_VIEWS.contains(name);
        assert_eq!(sql.description.contains(&format!("{name}:")), listed, "{name}");
        for (c, t, _) in cols.iter().filter(|_| listed) {
            assert!(sql.description.contains(&format!("{c} {t}")), "{name}.{c}");
        }
    }
    assert!(!sql.description.contains("notes"), "the user's notes never go to the model");
}

/// Parses `## view` sections and their `| `column` | `type` |` rows from the docs page.
fn documented() -> Vec<(String, Vec<(String, String)>)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ai/sql-views.md");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("## ") {
            out.push((name.trim().to_string(), Vec::new()));
        } else if let (Some(cur), true) = (out.last_mut(), line.starts_with("| `")) {
            let cells: Vec<&str> = line.split('|').map(|c| c.trim().trim_matches('`')).collect();
            cur.1.push((cells[1].to_string(), cells[2].to_string()));
        }
    }
    out.retain(|(_, cols)| !cols.is_empty());
    out
}

#[test]
fn the_docs_page_and_the_table_agree() {
    let docs = documented();
    let table: Vec<(String, Vec<(String, String)>)> = SQL_VIEW_TABLE
        .iter()
        .filter(|(n, _, _)| AI_VIEWS.contains(n))
        .map(|(n, _, cols)| (n.to_string(), cols.iter().map(|(c, t, _)| (c.to_string(), t.to_string())).collect()))
        .collect();
    assert_eq!(docs, table, "docs/ai/sql-views.md and SQL_VIEW_TABLE disagree: edit both");
}

#[test]
fn the_view_schema_is_pinned_in_the_real_store() {
    let f = fixture();
    let pinned: [(&str, &[(&str, &str)]); 3] = [
        (
            "events",
            &[
                ("id", "UBIGINT"),
                ("ts", "TIMESTAMP"),
                ("level", "VARCHAR"),
                ("source", "UINTEGER"),
                ("line", "UINTEGER"),
                ("byte_offset", "UBIGINT"),
                ("message", "VARCHAR"),
                ("template", "VARCHAR"),
                ("exception", "VARCHAR"),
                ("fields", "MAP(VARCHAR, VARCHAR)"),
            ],
        ),
        (
            "sources",
            &[
                ("id", "UINTEGER"),
                ("name", "VARCHAR"),
                ("path", "VARCHAR"),
                ("importer", "VARCHAR"),
                ("parser", "VARCHAR"),
                ("events", "UBIGINT"),
                ("skipped", "UBIGINT"),
                ("first_ts", "TIMESTAMP"),
                ("last_ts", "TIMESTAMP"),
                ("bytes", "UBIGINT"),
            ],
        ),
        (
            "fields",
            &[
                ("key", "VARCHAR"),
                ("type", "VARCHAR"),
                ("storage", "VARCHAR"),
                ("events", "UBIGINT"),
                ("fill_rate", "DOUBLE"),
                ("approx_distinct", "UBIGINT"),
            ],
        ),
    ];
    for (view, cols) in pinned {
        let v = ok(&f, &format!("DESCRIBE {view}"));
        let got: Vec<(String, String)> = v["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r[0].as_str().unwrap().to_string(), r[1].as_str().unwrap().to_string()))
            .collect();
        let want: Vec<(String, String)> = cols.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        assert_eq!(got, want, "{view}");
    }
}

#[test]
fn internal_tables_are_not_reachable_from_the_tool() {
    let f = fixture();
    for t in [
        "segments",
        "segment_fields",
        "events_wide",
        "sources_internal",
        "duckdb_tables()",
        "information_schema.tables",
    ] {
        let o = call(&f, &format!("SELECT * FROM {t}"));
        // The sandbox only holds the documented views (and system catalogs show nothing of the store's tables).
        if !o.is_error {
            let v: Value = serde_json::from_str(&o.text).unwrap();
            let text = v.to_string();
            assert!(!text.contains("segment") && !text.contains("events_wide"), "{t}: {text}");
        }
    }
    for t in ["segments", "segment_fields", "events_wide"] {
        assert!(call(&f, &format!("SELECT * FROM {t}")).is_error, "{t}");
    }
}

#[test]
fn every_console_guardrail_refuses_through_the_tool() {
    let f = fixture();
    for (sql, says) in [
        ("DELETE FROM events", "DELETE"),
        ("INSERT INTO sources VALUES (1)", "INSERT"),
        ("UPDATE sources SET name = 'x'", "UPDATE"),
        ("CREATE TABLE t AS SELECT 1", "CREATE"),
        ("DROP VIEW events", "DROP"),
        ("ATTACH 'other.duckdb' AS o", "ATTACH"),
        ("COPY (SELECT * FROM events) TO 'out.csv'", "COPY"),
        ("INSTALL httpfs", "INSTALL"),
        ("LOAD httpfs", "LOAD"),
        ("SET enable_external_access = true", "SET"),
        ("PRAGMA database_list", "PRAGMA"),
        ("SELECT * FROM read_csv('/etc/hosts')", "read_csv"),
        ("SELECT * FROM read_parquet('x.parquet')", "read_parquet"),
        ("SELECT * FROM 'https://example.com/x.csv'", "Files cannot be read"),
        ("SELECT 1; SELECT 2", "one statement"),
        ("SELECT 1; DROP VIEW events", "one statement"),
    ] {
        let o = call(&f, sql);
        assert!(o.is_error, "{sql}: {}", o.text);
        assert!(o.text.contains(says), "{sql}: {}", o.text);
        assert!(o.text.contains("SQL refused or invalid"), "{sql}: {}", o.text);
        assert_eq!(
            o.evidence.sql,
            [sql.to_string()],
            "even a refused statement is shown, so the user sees what was tried"
        );
    }
    // The store is untouched.
    assert_eq!(ok(&f, "SELECT count(*) AS n FROM events")["rows"][0][0], 5);
}

#[test]
fn a_slow_statement_is_stopped_at_the_time_limit() {
    let f = fixture();
    f.store.set_sql_time_limit(Duration::from_millis(200));
    let o = call(
        &f,
        "SELECT count(*) FROM range(3000000000) a, range(3000000000) b WHERE a.range + b.range = -1",
    );
    assert!(o.is_error && o.text.contains("Stopped after"), "{}", o.text);
}

#[test]
fn the_result_is_capped_at_the_console_rows_and_the_rows_shown() {
    let f = fixture();
    let v = ok(&f, "SELECT range AS n FROM range(25000)");
    assert_eq!(
        (v["totalRows"].as_u64(), v["truncated"].as_bool()),
        (Some(SQL_MAX_ROWS as u64), Some(true))
    );
    assert!(v["rows"].as_array().unwrap().len() <= 20, "the model sees a page, not the result");
    let mut t = EngineTools::new(f.store(), ToolLimits::for_context(8192));
    let o = t.call("run_sql", &json!({"sql": "SELECT range AS n FROM range(500)", "limit": 9999}));
    let v: Value = serde_json::from_str(&o.text).unwrap();
    assert!(v["rows"].as_array().unwrap().len() <= 50);
}

#[test]
fn selecting_event_ids_makes_them_citable() {
    let f = fixture();
    let o = call(&f, "SELECT id, message FROM events WHERE level = 'Error' ORDER BY ts");
    assert!(!o.is_error, "{}", o.text);
    assert_eq!(o.evidence.record_ids.len(), 2, "{:?}", o.evidence);
}

// ---- the assistant: SQL only when the query language cannot say it ----

fn run_turn(f: &Fixture, script: Vec<Reply>, question: &str) -> (Vec<ChatEvent>, String) {
    let a = Assistant::new(ScriptedProvider::new(script), AssistantConfig::default());
    let env = ChatEnv::new(&a, f.store().into());
    let mut session = ChatSession::new("sql");
    let mut events = Vec::new();
    session
        .send(&env, question, &ChatContext::default(), &mut |e| events.push(e.event.clone()))
        .unwrap();
    let text = events
        .iter()
        .find_map(|e| match e {
            ChatEvent::Done { text, .. } => Some(text.clone()),
            _ => None,
        })
        .expect("the turn is done");
    (events, text)
}

fn finished(events: &[ChatEvent]) -> Vec<(String, bool, Vec<String>, Vec<String>)> {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::ToolFinished { name, error, evidence, .. } => {
                Some((name.clone(), *error, evidence.queries.clone(), evidence.sql.clone()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_percentile_of_gaps_uses_sql_and_shows_it() {
    let f = fixture();
    let sql = "SELECT quantile_cont(gap, 0.95) AS p95 FROM (SELECT epoch(ts) - epoch(lag(ts) OVER (ORDER BY ts)) AS gap FROM events) WHERE gap IS NOT NULL";
    let (events, text) = run_turn(
        &f,
        vec![
            Reply::Tools(String::new(), vec![("run_sql".into(), json!({"sql": sql}))]),
            Reply::Text("I used SQL: the 95th percentile of the gaps between events is 17 seconds.".into()),
        ],
        "what is the 95th percentile of the gap between events?",
    );
    // The SQL really computes the answer: gaps are 1, 2, 7 and 20 seconds.
    let p95 = ok(&f, sql)["rows"][0][0].as_f64().unwrap();
    assert!((p95 - 18.05).abs() < 0.01, "{p95}");
    assert_eq!(finished(&events), [("run_sql".to_string(), false, vec![], vec![sql.to_string()])]);
    assert!(text.contains("used SQL"));
}

#[test]
fn a_simple_filter_does_not_use_sql() {
    let f = fixture();
    let (events, _) = run_turn(
        &f,
        vec![
            Reply::Tools(String::new(), vec![("run_query".into(), json!({"ql": "level >= error"}))]),
            Reply::Text("There are 2 errors.".into()),
        ],
        "show me the errors",
    );
    let done = finished(&events);
    assert_eq!(done, [("run_query".to_string(), false, vec!["level >= error".to_string()], vec![])]);
    assert!(done.iter().all(|d| d.3.is_empty()), "no SQL for a filter");
}
