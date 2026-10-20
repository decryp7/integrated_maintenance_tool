//! Promoted fields: an `extra` key present in at least half of the rows of a segment's first batch also gets
//! a typed column `p_<key>` in that segment (profile-declared fields keep their own `f_` columns), so facets,
//! statistics and filters on it read one column instead of the whole `extra` map. The column repeats the
//! value; `extra` stays complete, so `SELECT e.*` costs nothing extra (rebuilding the map per row made reads
//! of whole rows 10-100x slower at 10M events). A value that does not fit the column's type is NULL there.
//! Integers and booleans are typed only when their text round-trips exactly; other values (floats included)
//! are text.
//!
//! Each segment records its promoted set in the manifest; `events_wide` reads the segments with
//! `union_by_name` (a segment without a column reads it as NULL: such rows still have the key in `extra`).

use anyhow::{Context, Result};
use duckdb::arrow::array::{Array, ArrayRef, AsArray, BooleanBuilder, Int64Builder, RecordBatch, StringBuilder};
use duckdb::arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use imt_core::batch::{EventBatch, EventSchema};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Prefix of promoted columns.
pub const PREFIX: &str = "p_";

/// Most promoted keys per segment.
pub const MAX_PROMOTED: usize = 32;

/// Keys tracked while deciding (bounds the decision's memory).
const MAX_CANDIDATES: usize = 256;

/// The type of a promoted column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PType {
    Int,
    Bool,
    Text,
}

impl PType {
    pub fn name(self) -> &'static str {
        match self {
            PType::Int => "int",
            PType::Bool => "bool",
            PType::Text => "text",
        }
    }

    fn parse(s: &str) -> Option<PType> {
        Some(match s {
            "int" => PType::Int,
            "bool" => PType::Bool,
            "text" => PType::Text,
            _ => return None,
        })
    }

    pub fn sql(self) -> &'static str {
        match self {
            PType::Int => "BIGINT",
            PType::Bool => "BOOLEAN",
            PType::Text => "VARCHAR",
        }
    }

    fn arrow(self) -> DataType {
        match self {
            PType::Int => DataType::Int64,
            PType::Bool => DataType::Boolean,
            PType::Text => DataType::Utf8,
        }
    }

    /// Whether `v` is stored in a column of this type (and reads back as the same text).
    fn fits(self, v: &str) -> bool {
        match self {
            PType::Int => v.parse::<i64>().is_ok_and(|n| n.to_string() == v),
            PType::Bool => v == "true" || v == "false",
            PType::Text => true,
        }
    }
}

/// The promoted keys of a segment, in column order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Promotion {
    pub keys: Vec<(String, PType)>,
}

/// The column name of a promoted key.
pub fn column(key: &str) -> String {
    format!("{PREFIX}{key}")
}

impl Promotion {
    /// Decides from a segment's first batch.
    pub fn decide(batch: &EventBatch) -> Promotion {
        let schema = batch.schema();
        let declared: HashSet<String> = schema.declared().iter().map(|d| d.name.to_lowercase()).collect();
        let rows = batch.num_rows();
        let Some(map) = batch.record_batch().column(schema.extra_index()).as_map_opt() else {
            return Promotion::default();
        };
        let (Some(keys), Some(values)) = (map.keys().as_string_opt::<i32>(), map.values().as_string_opt::<i32>()) else {
            return Promotion::default();
        };
        // key -> (rows with it, every value an integer, every value a boolean)
        let mut seen: HashMap<&str, (usize, bool, bool)> = HashMap::new();
        let offsets = map.value_offsets();
        for row in 0..rows {
            for i in offsets[row] as usize..offsets[row + 1] as usize {
                if !values.is_valid(i) {
                    continue;
                }
                let (k, v) = (keys.value(i), values.value(i));
                if !seen.contains_key(k) && seen.len() >= MAX_CANDIDATES {
                    continue;
                }
                let e = seen.entry(k).or_insert((0, true, true));
                e.0 += 1;
                e.1 &= PType::Int.fits(v);
                e.2 &= PType::Bool.fits(v);
            }
        }
        let mut chosen: Vec<(&str, usize, PType)> = seen
            .into_iter()
            .filter(|(k, (n, _, _))| !k.is_empty() && n * 2 >= rows && rows > 0 && !declared.contains(&k.to_lowercase()))
            .map(|(k, (n, int, b))| {
                let t = if int {
                    PType::Int
                } else if b {
                    PType::Bool
                } else {
                    PType::Text
                };
                (k, n, t)
            })
            .collect();
        chosen.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let mut lower = HashSet::new();
        let keys = chosen
            .into_iter()
            .filter(|(k, _, _)| lower.insert(k.to_lowercase()))
            .take(MAX_PROMOTED)
            .map(|(k, _, t)| (k.to_string(), t))
            .collect();
        Promotion { keys }
    }

    /// The Parquet schema of a segment: the event columns, then the promoted ones.
    pub fn schema(&self, base: &EventSchema) -> SchemaRef {
        let mut fields: Vec<Field> = base.arrow().fields().iter().map(|f| f.as_ref().clone()).collect();
        fields.extend(self.keys.iter().map(|(k, t)| Field::new(column(k), t.arrow(), true)));
        Arc::new(Schema::new(fields))
    }

    /// `batch` in the segment's layout: the event columns, then the promoted columns.
    pub fn apply(&self, batch: &EventBatch) -> Result<RecordBatch> {
        let rb = batch.record_batch();
        if self.keys.is_empty() {
            return Ok(rb.clone());
        }
        let xi = batch.schema().extra_index();
        let map = rb.column(xi).as_map_opt().context("extra is not a map")?;
        let keys = map.keys().as_string_opt::<i32>().context("extra keys")?;
        let values = map.values().as_string_opt::<i32>().context("extra values")?;
        let index: HashMap<&str, usize> = self.keys.iter().enumerate().map(|(i, (k, _))| (k.as_str(), i)).collect();
        enum B {
            Int(Int64Builder),
            Bool(BooleanBuilder),
            Text(StringBuilder),
        }
        let mut cols: Vec<B> = self
            .keys
            .iter()
            .map(|(_, t)| match t {
                PType::Int => B::Int(Int64Builder::with_capacity(rb.num_rows())),
                PType::Bool => B::Bool(BooleanBuilder::with_capacity(rb.num_rows())),
                PType::Text => B::Text(StringBuilder::new()),
            })
            .collect();
        let offsets = map.value_offsets();
        let mut slot: Vec<Option<&str>> = vec![None; self.keys.len()];
        for row in 0..rb.num_rows() {
            slot.iter_mut().for_each(|s| *s = None);
            for i in offsets[row] as usize..offsets[row + 1] as usize {
                if let Some(&c) = index.get(keys.value(i)) {
                    if values.is_valid(i) && slot[c].is_none() && self.keys[c].1.fits(values.value(i)) {
                        slot[c] = Some(values.value(i));
                    }
                }
            }
            for (c, v) in slot.iter().enumerate() {
                match &mut cols[c] {
                    B::Int(b) => b.append_option(v.and_then(|v| v.parse::<i64>().ok())),
                    B::Bool(b) => b.append_option(v.map(|v| v == "true")),
                    B::Text(b) => b.append_option(*v),
                }
            }
        }
        let mut columns: Vec<ArrayRef> = rb.columns().to_vec();
        for b in cols {
            columns.push(match b {
                B::Int(mut b) => Arc::new(b.finish()),
                B::Bool(mut b) => Arc::new(b.finish()),
                B::Text(mut b) => Arc::new(b.finish()),
            });
        }
        let mut fields: Vec<Field> = rb.schema().fields().iter().map(|f| f.as_ref().clone()).collect();
        fields.extend(self.keys.iter().map(|(k, t)| Field::new(column(k), t.arrow(), true)));
        RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).context("building a promoted batch")
    }

    /// As stored in the manifest (JSON `[[key, type], ...]`).
    pub fn encode(&self) -> String {
        let v: Vec<[&str; 2]> = self.keys.iter().map(|(k, t)| [k.as_str(), t.name()]).collect();
        serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
    }

    /// From the manifest; unknown types are left out.
    pub fn decode(text: &str) -> Promotion {
        let v: Vec<[String; 2]> = serde_json::from_str(text).unwrap_or_default();
        Promotion {
            keys: v.into_iter().filter_map(|[k, t]| PType::parse(&t).map(|t| (k, t))).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::batch::{EventBatchBuilder, EventId, EventTime, FieldValue, LevelCode, NewEvent, SourceId};

    fn batch(rows: &[&[(&str, &str)]]) -> EventBatch {
        let schema = Arc::new(EventSchema::new([("Declared", imt_core::batch::ColumnType::Utf8)]).unwrap());
        let mut b = EventBatchBuilder::new(schema);
        for (i, fields) in rows.iter().enumerate() {
            let ev = NewEvent {
                time: EventTime::Parsed(i as i64),
                level: LevelCode::new(2).unwrap(),
                source: SourceId(0),
                line: i as u32 + 1,
                byte_offset: 0,
                message: "m",
                template: None,
                exception: None,
                markers: 0,
                raw: "r",
            };
            b.push(&ev, fields.iter().map(|(k, v)| (*k, FieldValue::Text(v)))).unwrap();
        }
        b.finish().assign_ids(EventId(0)).unwrap()
    }

    #[test]
    fn frequent_keys_are_promoted_with_exact_types() {
        let b = batch(&[
            &[
                ("Machine", "WEB01"),
                ("Status", "200"),
                ("Ok", "true"),
                ("Rare", "x"),
                ("declared", "d"),
            ],
            &[("Machine", "WEB02"), ("Status", "007"), ("Ok", "false")],
            &[("Machine", "WEB02"), ("Status", "500"), ("Ok", "true"), ("machine", "dup")],
            &[("Other", "1")],
        ]);
        let p = Promotion::decide(&b);
        assert_eq!(
            p.keys,
            [
                ("Machine".to_string(), PType::Text),
                ("Ok".to_string(), PType::Bool),
                ("Status".to_string(), PType::Text)
            ],
            "007 does not round-trip as an integer; the declared name and rare keys stay"
        );
        assert_eq!(Promotion::decode(&p.encode()), p);
        assert_eq!(Promotion::decode("garbage"), Promotion::default());
        let rb = p.apply(&b).unwrap();
        assert_eq!(rb.schema().fields().len(), b.record_batch().num_columns() + 3);
        let machine = rb.column_by_name("p_Machine").unwrap().as_string::<i32>();
        assert_eq!((machine.value(0), machine.is_null(3)), ("WEB01", true));
        let ok = rb.column_by_name("p_Ok").unwrap().as_boolean();
        assert!(ok.value(0) && !ok.value(1));
        // extra keeps every value.
        let xi = b.schema().extra_index();
        assert_eq!(rb.column(xi).as_ref(), b.record_batch().column(xi).as_ref());
        let ints = Promotion::decide(&batch(&[&[("N", "1")], &[("N", "-20")]]));
        assert_eq!(ints.keys, [("N".to_string(), PType::Int)]);
        let rb = ints.apply(&batch(&[&[("N", "5")], &[("N", "x")]])).unwrap();
        let n = rb
            .column_by_name("p_N")
            .unwrap()
            .as_primitive::<duckdb::arrow::datatypes::Int64Type>();
        assert_eq!(
            (n.value(0), n.is_null(1)),
            (5, true),
            "a value that does not fit is NULL in the column"
        );
        assert!(Promotion::default().apply(&b).unwrap().num_columns() == b.record_batch().num_columns());
    }
}
