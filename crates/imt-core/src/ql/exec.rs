//! Runs a [`Program`] over events (any [`RowRef`]).

use super::control::{QueryControl, QueryStage, RunError, BATCH};
use super::error::QlError;
use super::expr::{EventRow, TableRow};
use super::parser::{FieldRef, Program, SortKeySpec, Stage};
use super::stats::aggregate;
use super::QlResult;
use crate::model::Record;
use crate::row::{RecordRow, RowRef};
use serde_json::Value;
use std::cmp::Ordering;

enum State<R> {
    Events { items: Vec<R>, fields: Option<Vec<String>> },
    Table { columns: Vec<String>, rows: Vec<Vec<Value>> },
}

enum Key {
    Num(f64),
    Text(String),
    Missing,
}

fn key_order(a: &Key, b: &Key, descending: bool) -> Ordering {
    let base = match (a, b) {
        (Key::Missing, Key::Missing) => return Ordering::Equal,
        // Missing values sort last in both directions.
        (Key::Missing, _) => return Ordering::Greater,
        (_, Key::Missing) => return Ordering::Less,
        (Key::Num(x), Key::Num(y)) => x.total_cmp(y),
        (Key::Num(_), Key::Text(_)) => Ordering::Less,
        (Key::Text(_), Key::Num(_)) => Ordering::Greater,
        (Key::Text(x), Key::Text(y)) => x.cmp(y),
    };
    if descending {
        base.reverse()
    } else {
        base
    }
}

fn event_key(row: &dyn RowRef, field: &FieldRef) -> Key {
    if field.name == "@level" {
        return Key::Num(row.level().index() as f64);
    }
    if let Some(n) = row.number(&field.key) {
        return Key::Num(n);
    }
    row.text(&field.key).map_or(Key::Missing, |t| Key::Text(t.into_owned()))
}

fn cell_key(cell: &Value) -> Key {
    match cell {
        Value::Null => Key::Missing,
        Value::Number(n) => n.as_f64().map_or(Key::Missing, Key::Num),
        Value::String(s) => Key::Text(s.clone()),
        other => Key::Text(other.to_string()),
    }
}

fn column_index(columns: &[String], field: &FieldRef) -> Result<usize, QlError> {
    TableRow::column(columns, &field.name).ok_or_else(|| {
        QlError::new(
            format!("no column '{}' in the result (columns: {})", field.label, columns.join(", ")),
            field.offset,
            field.len,
        )
    })
}

impl<R: RowRef> State<R> {
    fn apply(self, stage: &Stage, control: &QueryControl) -> Result<State<R>, RunError> {
        Ok(match (self, stage) {
            (State::Events { mut items, fields }, Stage::Where(f)) => {
                let mut seen = 0usize;
                items.retain(|row| {
                    seen += 1;
                    // Once cancelled, the rest is dropped quickly; the caller discards the state.
                    !(seen.is_multiple_of(BATCH) && control.is_cancelled()) && f.matches_event(row)
                });
                control.check()?;
                State::Events { items, fields }
            }
            (State::Table { columns, mut rows }, Stage::Where(f)) => {
                rows.retain(|cells| f.matches_row(&TableRow { columns: &columns, cells }));
                State::Table { columns, rows }
            }
            (State::Events { items, fields }, Stage::Sort(specs)) => {
                let keyed = control
                    .guard(items.into_iter())
                    .map(|row| (specs.iter().map(|s| event_key(&row, &s.field)).collect(), row))
                    .collect();
                control.check()?;
                State::Events {
                    items: sort_keyed(keyed, specs),
                    fields,
                }
            }
            (State::Table { columns, rows }, Stage::Sort(specs)) => {
                let idx = specs
                    .iter()
                    .map(|s| column_index(&columns, &s.field))
                    .collect::<Result<Vec<_>, _>>()?;
                let keyed = rows
                    .into_iter()
                    .map(|r| (idx.iter().map(|i| cell_key(&r[*i])).collect(), r))
                    .collect();
                let rows = sort_keyed(keyed, specs);
                State::Table { columns, rows }
            }
            (State::Events { mut items, fields }, Stage::Head(n)) => {
                items.truncate(*n);
                State::Events { items, fields }
            }
            (State::Table { columns, mut rows }, Stage::Head(n)) => {
                rows.truncate(*n);
                State::Table { columns, rows }
            }
            (State::Events { items, .. }, Stage::Fields(f)) => State::Events {
                items,
                fields: Some(f.iter().map(|f| f.label.clone()).collect()),
            },
            (State::Table { columns, rows }, Stage::Fields(f)) => {
                let idx = f.iter().map(|f| column_index(&columns, f)).collect::<Result<Vec<_>, _>>()?;
                let rows = rows.iter().map(|r| idx.iter().map(|i| r[*i].clone()).collect()).collect();
                State::Table {
                    columns: idx.iter().map(|i| columns[*i].clone()).collect(),
                    rows,
                }
            }
            (State::Events { items, .. }, Stage::Stats { aggs, by }) => {
                let rows = control.guard(items.iter()).map(|row| EventRow(row));
                let (columns, rows) = aggregate(rows, aggs, by)?;
                control.check()?;
                State::Table { columns, rows }
            }
            (State::Table { columns, rows }, Stage::Stats { aggs, by }) => {
                let (cols, out) = aggregate(rows.iter().map(|cells| TableRow { columns: &columns, cells }), aggs, by)?;
                State::Table { columns: cols, rows: out }
            }
        })
    }
}

fn sort_keyed<T>(mut keyed: Vec<(Vec<Key>, T)>, specs: &[SortKeySpec]) -> Vec<T> {
    keyed.sort_by(|(a, _), (b, _)| {
        a.iter()
            .zip(b)
            .zip(specs)
            .map(|((x, y), s)| key_order(x, y, s.descending))
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal)
    });
    keyed.into_iter().map(|(_, it)| it).collect()
}

/// Runs `program` over `records` (record, source name). See [`run_with`] for a cancellable run.
pub fn run<'a>(program: &Program, records: &mut dyn Iterator<Item = (&'a Record, &'a str)>) -> Result<QlResult, QlError> {
    // Nobody else holds this control, so it is never cancelled.
    run_with(program, records, &QueryControl::new()).map_err(RunError::into_ql_error)
}

/// Like [`run`], checking `control` between batches of records and between stages, and reporting
/// progress (stage [`QueryStage::Evaluating`]) through it. Returns [`RunError::Cancelled`] promptly
/// once the control is cancelled; the partial state is dropped.
pub fn run_with<'a>(
    program: &Program,
    records: &mut dyn Iterator<Item = (&'a Record, &'a str)>,
    control: &QueryControl,
) -> Result<QlResult, RunError> {
    run_rows(program, &mut records.map(|(rec, source)| RecordRow { rec, source }), control)
}

/// [`run_with`] over any events. The leading filter, the `where` stages right after it and a first
/// `stats` stream (nothing is kept per event); `sort`, `head` and `fields` keep the events they need.
pub fn run_rows<R: RowRef>(program: &Program, rows: &mut dyn Iterator<Item = R>, control: &QueryControl) -> Result<QlResult, RunError> {
    control.check()?;
    // The upper bound when there is one: a filtering iterator (a store's stream) knows at most how many it yields.
    let (low, high) = rows.size_hint();
    control.begin(QueryStage::Evaluating, high.unwrap_or(low));
    let leading = program.stages.iter().take_while(|s| matches!(s, Stage::Where(_))).count();
    let wheres: Vec<&super::Filter> = program.stages[..leading]
        .iter()
        .filter_map(|s| match s {
            Stage::Where(f) => Some(f),
            _ => None,
        })
        .collect();
    let keep = |row: &R| program.filter.matches_event(row) && wheres.iter().all(|f| f.matches_event(row));
    let mut stages = program.stages[leading..].iter().peekable();
    let state = match stages.peek() {
        Some(Stage::Stats { aggs, by }) => {
            stages.next();
            let matched = control.watch(rows).filter(|r| keep(r)).map(OwnedEvent);
            let (columns, out) = aggregate(matched, aggs, by)?;
            control.check()?;
            State::Table { columns, rows: out }
        }
        _ => {
            let items = control.watch(rows).filter(|r| keep(r)).collect();
            control.check()?;
            State::Events { items, fields: None }
        }
    };
    finish(state, stages, control)
}

/// Applies `stages` to a result table (e.g. one a store aggregated in SQL for the program's first
/// `stats`), as [`run_rows`] does after its `stats`.
pub fn run_table(columns: Vec<String>, rows: Vec<Vec<Value>>, stages: &[Stage], control: &QueryControl) -> Result<QlResult, RunError> {
    finish::<crate::row::OwnedRow>(State::Table { columns, rows }, stages.iter(), control)
}

/// Applies the remaining `stages` to `state` and returns the result.
fn finish<'a, R: RowRef>(
    mut state: State<R>,
    stages: impl Iterator<Item = &'a Stage>,
    control: &QueryControl,
) -> Result<QlResult, RunError> {
    for stage in stages {
        state = state.apply(stage, control)?;
        control.check()?;
    }
    Ok(match state {
        State::Events { items, fields } => QlResult::Events {
            ids: items.iter().map(|r| u32::try_from(r.id()).unwrap_or(u32::MAX)).collect(),
            fields,
        },
        State::Table { columns, rows } => QlResult::Table { columns, rows },
    })
}

/// An owned event as a filter row (for the streaming `stats`).
struct OwnedEvent<R>(R);

impl<R: RowRef> super::Row for OwnedEvent<R> {
    fn text(&self, key: &crate::row::FieldKey) -> Option<std::borrow::Cow<'_, str>> {
        self.0.text(key)
    }
    fn number(&self, key: &crate::row::FieldKey) -> Option<f64> {
        self.0.number(key)
    }
    fn level(&self) -> Option<crate::model::Level> {
        Some(self.0.level())
    }
    fn full_text(&self, needle: &str) -> bool {
        super::expr::full_text_match(&self.0, needle)
    }
    fn value(&self, key: &crate::row::FieldKey) -> Option<Value> {
        self.0.value(key)
    }
    fn event(&self) -> Option<&dyn RowRef> {
        Some(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_notice_a_cancel_during_their_pass() {
        let recs: Vec<Record> = (0..BATCH * 3)
            .map(|i| {
                let mut r = Record::new(i as u32);
                r.fields.insert("N".into(), serde_json::json!(i % 10));
                r
            })
            .collect();
        let control = QueryControl::new();
        control.cancel();
        for ql in ["| where N < 5", "| sort N", "| stats count() by N"] {
            let program = super::super::parse(ql).unwrap();
            let state = State::Events {
                items: recs.iter().map(|rec| RecordRow { rec, source: "s" }).collect(),
                fields: None,
            };
            assert!(
                matches!(state.apply(&program.stages[0], &control), Err(RunError::Cancelled)),
                "{ql}"
            );
        }
    }

    #[test]
    fn run_table_applies_the_stages_after_a_first_stats() {
        let program = super::super::parse("| stats count() as n by N | sort n desc | head 2").unwrap();
        let rows = vec![
            vec![serde_json::json!("a"), serde_json::json!(1)],
            vec![serde_json::json!("b"), serde_json::json!(3)],
            vec![serde_json::json!("c"), serde_json::json!(2)],
        ];
        let columns = vec!["N".to_string(), "n".to_string()];
        let control = QueryControl::new();
        let out = super::run_table(columns.clone(), rows.clone(), &program.stages[1..], &control).unwrap();
        assert_eq!(
            out,
            QlResult::Table {
                columns: columns.clone(),
                rows: vec![rows[1].clone(), rows[2].clone()],
            }
        );
        // No further stages: the table as given; a cancel stops between stages.
        assert_eq!(
            super::run_table(columns.clone(), rows.clone(), &[], &control).unwrap(),
            QlResult::Table {
                columns: columns.clone(),
                rows: rows.clone()
            }
        );
        control.cancel();
        assert!(matches!(
            super::run_table(columns, rows, &program.stages[1..], &control),
            Err(RunError::Cancelled)
        ));
    }

    #[test]
    fn an_owned_event_row_delegates_everything_to_its_event() {
        use super::super::Row;
        let mut rec = Record::new(1);
        rec.message = "Disk Full here".into();
        rec.level = crate::model::Level::Error;
        rec.fields.insert("N".into(), serde_json::json!(7));
        let row = OwnedEvent(RecordRow { rec: &rec, source: "s" });
        let key = crate::row::FieldKeys::new().key("N");
        assert_eq!(row.level(), Some(crate::model::Level::Error));
        assert!(row.full_text("disk full"));
        assert!(!row.full_text("absent"));
        assert_eq!(row.number(&key), Some(7.0));
        assert_eq!(row.value(&key), Some(serde_json::json!(7)));
        assert_eq!(
            row.text(&crate::row::FieldKeys::new().key("N")).map(|t| t.into_owned()),
            Some("7".to_string())
        );
        assert_eq!(row.event().map(|e| e.message().to_string()), Some("Disk Full here".to_string()));
    }

    #[test]
    fn a_row_that_is_not_an_event_has_no_event() {
        use super::super::Row;
        struct Plain;
        impl Row for Plain {
            fn text(&self, _: &crate::row::FieldKey) -> Option<std::borrow::Cow<'_, str>> {
                None
            }
            fn number(&self, _: &crate::row::FieldKey) -> Option<f64> {
                None
            }
            fn level(&self) -> Option<crate::model::Level> {
                None
            }
            fn full_text(&self, _: &str) -> bool {
                false
            }
            fn value(&self, _: &crate::row::FieldKey) -> Option<Value> {
                None
            }
        }
        assert!(Plain.event().is_none());
    }
}
