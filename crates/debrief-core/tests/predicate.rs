//! Differential tests of the one predicate language (redesign R2): conditions and query filters are
//! lowered into the query language's `Expr`; on every sample event they decide as the evaluator they
//! replaced (kept below as `legacy`), and an event reads the same as an in-memory record or as a row of
//! an Arrow batch.

use debrief_core::batch::{EventBatchBuilder, EventId, EventSchema, EventTime, FieldValue, LevelCode, NewEvent, SourceId};
use debrief_core::condition::Condition;
use debrief_core::model::{value_text, Record};
use debrief_core::query::{CompiledQuery, FieldFilter, Query};
use debrief_core::row::{ArrowRow, BatchSlots, RecordRow};
use debrief_core::{Profile, Workspace};
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

/// The condition evaluator before the lowering (engine review finding 7), as it was.
mod legacy {
    use debrief_core::model::Record;
    use regex::Regex;

    #[derive(Clone, Copy, PartialEq)]
    enum Op {
        Eq,
        Ne,
        Contains,
        NotContains,
        Matches,
        Exists,
        Missing,
        Gt,
        Lt,
        In,
    }

    pub struct Condition {
        field: String,
        op: Op,
        value: String,
        lower: String,
        regex: Option<Regex>,
        number: Option<f64>,
        set: Vec<String>,
    }

    impl Condition {
        pub fn new(field: &str, op: &str, value: Option<&str>) -> Condition {
            let op = match op {
                "eq" => Op::Eq,
                "ne" => Op::Ne,
                "contains" => Op::Contains,
                "notContains" => Op::NotContains,
                "matches" => Op::Matches,
                "exists" => Op::Exists,
                "missing" => Op::Missing,
                "gt" => Op::Gt,
                "lt" => Op::Lt,
                _ => Op::In,
            };
            let value = value.unwrap_or_default().to_string();
            Condition {
                field: field.to_string(),
                lower: value.to_lowercase(),
                regex: (op == Op::Matches).then(|| Regex::new(&value).unwrap()),
                number: matches!(op, Op::Gt | Op::Lt).then(|| value.trim().parse().unwrap()),
                set: if op == Op::In {
                    value.split('|').map(|s| s.trim().to_string()).collect()
                } else {
                    Vec::new()
                },
                value,
                op,
            }
        }

        pub fn eval(&self, rec: &Record, source_name: &str) -> bool {
            let numbers = || {
                let n = match self.field.as_str() {
                    "@level" => Some(rec.level.index() as f64),
                    f => rec.field_number(f),
                };
                n.zip(self.number)
            };
            let v = || rec.field_text(&self.field, source_name);
            match self.op {
                Op::Gt => numbers().is_some_and(|(n, t)| n > t),
                Op::Lt => numbers().is_some_and(|(n, t)| n < t),
                Op::Exists => v().is_some_and(|s| !s.is_empty()),
                Op::Missing => v().is_none_or(|s| s.is_empty()),
                Op::Eq => v().is_some_and(|s| s == self.value || (self.field == "@level" && s.eq_ignore_ascii_case(&self.value))),
                Op::Ne => v().is_none_or(|s| s != self.value),
                Op::Contains => v().is_some_and(|s| s.to_lowercase().contains(&self.lower)),
                Op::NotContains => v().is_none_or(|s| !s.to_lowercase().contains(&self.lower)),
                Op::Matches => v().is_some_and(|s| self.regex.as_ref().is_some_and(|r| r.is_match(&s))),
                Op::In => v().is_some_and(|s| self.set.iter().any(|x| *x == *s)),
            }
        }
    }
}

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn workspace() -> Workspace {
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    let dirs = ["serilog-json", "serilog-clef", "serilog-text", "log4net", "sqlserver", "csv"];
    ws.open(&dirs.iter().map(|d| samples().join(d)).collect::<Vec<_>>()).unwrap();
    assert!(ws.records().len() > 5000, "{} records", ws.records().len());
    ws
}

const CONDITIONS: &[(&str, &str, Option<&str>)] = &[
    ("MachineName", "eq", Some("WEB01")),
    ("MachineName", "ne", Some("WEB01")),
    ("@level", "eq", Some("warning")),
    ("@level", "eq", Some("Error")),
    ("@level", "ne", Some("Warning")),
    ("@message", "contains", Some("order")),
    ("@message", "notContains", Some("ORDER")),
    ("@exception", "contains", Some("timeout")),
    ("Path", "matches", Some("^/api/.*s$")),
    ("@template", "matches", Some("\\{\\w+\\}")),
    ("StatusCode", "exists", None),
    ("StatusCode", "missing", None),
    ("@exception", "exists", None),
    ("ElapsedMs", "gt", Some("250")),
    ("ElapsedMs", "lt", Some("20")),
    ("Application", "eq", Some("OrderService")),
    ("@level", "gt", Some("2")),
    ("@level", "lt", Some("2")),
    ("@line", "lt", Some("100")),
    ("@line", "eq", Some("7")),
    ("MachineName", "in", Some("WEB01|WEB02")),
    ("@level", "in", Some("Warning|Error")),
    ("@source", "contains", Some("web")),
    ("@source", "ne", Some("x")),
    ("Nope", "ne", Some("x")),
    ("Nope", "missing", None),
];

/// Every operator decides as before on every sample event (the one intended change: `ne` on a level
/// now ignores case like `eq`, which no case below depends on).
#[test]
fn lowered_conditions_decide_as_the_old_evaluator() {
    let ws = workspace();
    let names: std::collections::HashMap<u32, String> = ws.summary().sources.into_iter().map(|s| (s.id, s.name)).collect();
    for (field, op, value) in CONDITIONS {
        let new = Condition::new(field, op, *value).unwrap();
        let old = legacy::Condition::new(field, op, *value);
        let mut hits = 0;
        for r in ws.records() {
            let source = names.get(&r.source).map_or("", String::as_str);
            let got = new.eval(r, source);
            assert_eq!(
                got,
                old.eval(r, source),
                "{field} {op} {value:?} on record {} ({})",
                r.id,
                r.message
            );
            hits += usize::from(got);
        }
        eprintln!("{field} {op} {value:?}: {hits} of {}", ws.records().len());
    }
}

/// A record as one row of a batch with only the fixed columns and `extra` (every field as text).
fn batch_of(r: &Record) -> debrief_core::batch::EventBatch {
    let mut b = EventBatchBuilder::new(Arc::new(EventSchema::plain()));
    let texts: Vec<(&str, Cow<str>)> = r.fields.iter().map(|(k, v)| (k.as_str(), value_text(v))).collect();
    let ev = NewEvent {
        time: EventTime::parsed(r.ts),
        level: LevelCode::new(r.level.index() as u8).unwrap(),
        source: SourceId(r.source),
        line: r.line,
        byte_offset: 0,
        message: &r.message,
        template: r.template.as_deref(),
        exception: r.exception.as_deref(),
        markers: r.markers,
        raw: "",
    };
    b.push(&ev, texts.iter().map(|(k, v)| (*k, FieldValue::Text(v)))).unwrap();
    b.finish().assign_ids(EventId(u64::from(r.id))).unwrap()
}

/// Query filters (conditions, the search box, the query language, time, levels, sources and
/// instances) decide the same on a record and on its Arrow row.
#[test]
fn queries_decide_the_same_on_records_and_arrow_rows() {
    let ws = workspace();
    let names: std::collections::HashMap<u32, String> = ws.summary().sources.into_iter().map(|s| (s.id, s.name)).collect();
    let summary = ws.summary();
    let mid = (summary.from.unwrap() + summary.to.unwrap()) / 2;
    let mut queries: Vec<Query> = CONDITIONS
        .iter()
        .map(|(field, op, value)| Query {
            filters: vec![FieldFilter {
                field: field.to_string(),
                op: op.to_string(),
                value: value.map(str::to_string),
            }],
            ..Query::default()
        })
        .collect();
    for ql in [
        "level >= warning and not message contains order",
        "ElapsedMs > 100 or StatusCode in (500, 404)",
        "Path ~ 'orders$' | where MachineName != WEB01",
        "timeout",
        "missing(Upstream)",
    ] {
        queries.push(Query {
            ql: ql.into(),
            ..Query::default()
        });
    }
    queries.push(Query {
        text: "time[ds]? ?out".into(),
        regex: true,
        ..Query::default()
    });
    queries.push(Query {
        text: "Order".into(),
        case_sensitive: true,
        from: Some(mid - 86_400_000),
        to: Some(mid + 86_400_000),
        levels: vec![debrief_core::model::Level::Information],
        sources: (0..20).collect(),
        ..Query::default()
    });
    queries.push(Query {
        group_by: vec!["MachineName".into()],
        groups: vec!["WEB02".into()],
        ..Query::default()
    });
    // Converting a record to a batch is slow; every 7th record covers the samples well.
    let rows: Vec<(&Record, debrief_core::batch::EventBatch)> = ws.records().iter().step_by(7).map(|r| (r, batch_of(r))).collect();
    for (i, q) in queries.iter().enumerate() {
        let cq = CompiledQuery::new(q, &["@source".into()], &[]).unwrap();
        let mut hits = 0;
        for (r, batch) in &rows {
            let source = names.get(&r.source).map_or("", String::as_str);
            let slots = BatchSlots::resolve(cq.keys(), batch.schema());
            let arrow = ArrowRow {
                row: batch.row(0),
                source,
                slots: &slots,
            };
            let record = RecordRow { rec: r, source };
            let want = cq.matches_row(&record);
            assert_eq!(cq.matches_row(&arrow), want, "{q:?} on {}", r.message);
            assert_eq!(cq.group_key_of(&arrow), cq.group_key_of(&record));
            hits += usize::from(want);
        }
        eprintln!("query {i}: {hits} of {}", rows.len());
        assert!(hits > 0, "query {i} matches some events: {q:?}");
    }
}
