//! The query language over owned rows (what a store streams out) and over a table the store aggregated itself:
//! filters, `where` stages and `stats` give the same answers as over records, and a pre-aggregated table takes
//! the remaining stages.

use imt_core::model::{Level, Record};
use imt_core::ql::{self, Expr, QlResult, QueryControl};
use imt_core::row::OwnedRow;
use serde_json::{json, Value};
use std::sync::Arc;

fn row(id: u64, level: Level, msg: &str, ms: i64, host: &str) -> OwnedRow {
    let mut rec = Record::new(id as u32 + 1);
    rec.id = id as u32;
    rec.level = level;
    rec.message = msg.into();
    rec.fields.insert("ms".into(), json!(ms));
    rec.fields.insert("host".into(), json!(host));
    OwnedRow {
        id,
        rec,
        source: Arc::from("a.log"),
    }
}

fn rows() -> Vec<OwnedRow> {
    vec![
        row(0, Level::Information, "started", 10, "web1"),
        row(1, Level::Warning, "slow request", 250, "web1"),
        row(2, Level::Error, "request failed", 900, "web2"),
        row(3, Level::Error, "request failed again", 700, "web2"),
        row(4, Level::Warning, "slow again", 400, "web2"),
    ]
}

fn run(text: &str) -> QlResult {
    let program = ql::parse(text).unwrap();
    ql::run_rows(&program, &mut rows().into_iter(), &QueryControl::new()).unwrap()
}

#[test]
fn filters_and_leading_where_stages_apply_to_owned_rows() {
    assert_eq!(
        run("level >= warning | where ms > 300"),
        QlResult::Events {
            ids: vec![2, 3, 4],
            fields: None
        }
    );
    assert!(matches!(run("level = fatal"), QlResult::Events { ref ids, .. } if ids.is_empty()));
    // The ids are the store's, whatever the row's own position.
    assert!(matches!(run("request | sort ms desc | head 2"), QlResult::Events { ref ids, .. } if *ids == [2, 3]));
}

#[test]
fn a_streaming_stats_counts_the_rows_that_pass_the_filter_and_where_stages() {
    let QlResult::Table { columns, rows } = run("level >= warning | where ms > 300 | stats count() as n, avg(ms) as avg by host") else {
        panic!("a table")
    };
    assert_eq!(columns, ["host", "n", "avg"]);
    assert_eq!(rows, vec![vec![json!("web2"), json!(3), json!(666.6666666666666)]]);
}

#[test]
fn a_table_a_store_aggregated_takes_the_remaining_stages() {
    let program = ql::parse("level >= warning | stats count() as n by host | sort n desc | head 1").unwrap();
    let table = vec![
        vec![json!("web1"), json!(1)],
        vec![json!("web2"), json!(3)],
        vec![json!("web3"), json!(2)],
    ];
    // Only the stages after the `stats` are applied to the table.
    let after = &program.stages[1..];
    let result = ql::run_table(vec!["host".into(), "n".into()], table, after, &QueryControl::new()).unwrap();
    assert_eq!(
        result,
        QlResult::Table {
            columns: vec!["host".into(), "n".into()],
            rows: vec![vec![json!("web2"), json!(3)]]
        }
    );
    // A cancelled control stops it with the cancelled error, not a partial table.
    let control = QueryControl::new();
    control.cancel();
    assert!(ql::run_table(vec!["n".into()], vec![vec![Value::from(1)]], after, &control).is_err());
}

#[test]
fn filter_expressions_combine_without_ceremony() {
    assert!(matches!(Expr::all(vec![]), Expr::Const(true)));
    assert!(matches!(Expr::all(vec![Expr::Const(true), Expr::Const(true)]), Expr::Const(true)));
    assert!(matches!(Expr::all(vec![Expr::Const(true), Expr::Const(false)]), Expr::Const(false)));
    assert!(matches!(Expr::all(vec![Expr::Const(false), Expr::Const(false)]), Expr::And(ref v) if v.len() == 2));
}

#[test]
fn a_streaming_stats_can_group_by_what_only_the_event_knows() {
    let table = |text: &str| match run(text) {
        QlResult::Table { columns, rows } => (columns, rows),
        other => panic!("not a table: {other:?}"),
    };
    let (columns, rows) = table("level >= information | stats count() as n by level");
    assert_eq!(columns[0], "level");
    assert_eq!(rows.len(), 3, "information, warning and error: {rows:?}");
    // A full-text filter and a where stage see the same rows when the stats streams.
    let (_, rows) = table("request | stats count() as n by host");
    assert_eq!(rows, vec![vec![json!("web1"), json!(1)], vec![json!("web2"), json!(2)]]);
    let (_, rows) = table("level >= information | stats count() as n by source");
    assert_eq!(rows, vec![vec![json!("a.log"), json!(5)]]);
}
