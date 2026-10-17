//! Runs a [`Program`] over records.

use super::error::QlError;
use super::expr::{EventRow, TableRow};
use super::parser::{FieldRef, Program, SortKeySpec, Stage};
use super::stats::aggregate;
use super::QlResult;
use crate::model::Record;
use serde_json::Value;
use std::cmp::Ordering;

enum State<'a> {
    Events {
        items: Vec<(&'a Record, &'a str)>,
        fields: Option<Vec<String>>,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
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

fn event_key(rec: &Record, source: &str, field: &FieldRef) -> Key {
    if field.name == "@level" {
        return Key::Num(rec.level.index() as f64);
    }
    if let Some(n) = rec.field_number(&field.name) {
        return Key::Num(n);
    }
    rec.field_text(&field.name, source)
        .map_or(Key::Missing, |t| Key::Text(t.into_owned()))
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

impl<'a> State<'a> {
    fn apply(self, stage: &Stage) -> Result<State<'a>, QlError> {
        Ok(match (self, stage) {
            (State::Events { mut items, fields }, Stage::Where(f)) => {
                items.retain(|(rec, src)| f.matches(rec, src));
                State::Events { items, fields }
            }
            (State::Table { columns, mut rows }, Stage::Where(f)) => {
                rows.retain(|cells| f.matches_row(&TableRow { columns: &columns, cells }));
                State::Table { columns, rows }
            }
            (State::Events { items, fields }, Stage::Sort(specs)) => {
                let keyed = items
                    .into_iter()
                    .map(|(rec, src)| (specs.iter().map(|s| event_key(rec, src, &s.field)).collect(), (rec, src)))
                    .collect();
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
                let (columns, rows) = aggregate(items.iter().map(|(rec, source)| EventRow { rec, source }), aggs, by)?;
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

pub fn run<'a>(program: &Program, records: &mut dyn Iterator<Item = (&'a Record, &'a str)>) -> Result<QlResult, QlError> {
    let items = records.filter(|(rec, src)| program.filter.matches(rec, src)).collect();
    let mut state = State::Events { items, fields: None };
    for stage in &program.stages {
        state = state.apply(stage)?;
    }
    Ok(match state {
        State::Events { items, fields } => QlResult::Events {
            ids: items.iter().map(|(r, _)| r.id).collect(),
            fields,
        },
        State::Table { columns, rows } => QlResult::Table { columns, rows },
    })
}
