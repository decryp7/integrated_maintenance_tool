//! The database layout. `meta` records the format version and the declared columns, so a store is
//! reopened with the schema it was written with, and a newer format is refused.
//!
//! Events live in Parquet segments (see [`crate::segments`]) listed in `segments`; `events` is a view over
//! the segments of the sources that are ready (completely imported and not removed), recreated whenever
//! the list changes and when the store is opened (it names the files by absolute path). It is read-only:
//! changing events means writing new segments and swapping them in the manifest. `events_empty` has the
//! same columns and stands in while there is no segment. `ord` maps time-order positions to ids.

use crate::sql::ident;
use imt_core::batch::{ColumnType, EventSchema};

/// Version of the database layout this build writes and reads.
pub const FORMAT_VERSION: u32 = 2;

/// The DuckDB type of a declared column.
fn sql_type(t: ColumnType) -> &'static str {
    match t {
        ColumnType::Utf8 => "VARCHAR",
        ColumnType::Int64 => "BIGINT",
        ColumnType::Float64 => "DOUBLE",
        ColumnType::Boolean => "BOOLEAN",
    }
}

pub fn type_name(t: ColumnType) -> &'static str {
    match t {
        ColumnType::Utf8 => "utf8",
        ColumnType::Int64 => "int64",
        ColumnType::Float64 => "float64",
        ColumnType::Boolean => "bool",
    }
}

pub fn parse_type(name: &str) -> Option<ColumnType> {
    Some(match name {
        "utf8" => ColumnType::Utf8,
        "int64" => ColumnType::Int64,
        "float64" => ColumnType::Float64,
        "bool" => ColumnType::Boolean,
        _ => return None,
    })
}

/// `CREATE TABLE` statements of a new store whose events have `schema`.
pub fn ddl(schema: &EventSchema) -> String {
    let declared: String = schema
        .declared()
        .iter()
        .map(|c| format!(", {} {}", ident(&c.column), sql_type(c.ty)))
        .collect();
    format!(
        "CREATE TABLE meta (key VARCHAR PRIMARY KEY, value VARCHAR NOT NULL);
CREATE TABLE sources (
  id UINTEGER PRIMARY KEY, path VARCHAR NOT NULL, name VARCHAR NOT NULL, importer VARCHAR, file_kind VARCHAR,
  events UBIGINT NOT NULL DEFAULT 0, skipped UBIGINT NOT NULL DEFAULT 0, dropped UBIGINT NOT NULL DEFAULT 0,
  ts_from BIGINT, ts_to BIGINT, bytes UBIGINT NOT NULL DEFAULT 0, ready BOOLEAN NOT NULL DEFAULT FALSE);
CREATE TABLE diagnostics (source UINTEGER NOT NULL, kind VARCHAR NOT NULL, severity VARCHAR NOT NULL, count UBIGINT NOT NULL, samples VARCHAR NOT NULL);
CREATE TABLE marker_kinds (generation UBIGINT NOT NULL, bit UTINYINT NOT NULL, kind VARCHAR NOT NULL, owner VARCHAR NOT NULL);
CREATE TABLE segments (file VARCHAR PRIMARY KEY, first_id UBIGINT NOT NULL, end_id UBIGINT NOT NULL, rows UBIGINT NOT NULL, live UBIGINT NOT NULL, indexed BOOLEAN NOT NULL DEFAULT FALSE);
CREATE TABLE trash (path VARCHAR PRIMARY KEY);
CREATE TABLE ord (pos UBIGINT NOT NULL, id UBIGINT NOT NULL);
CREATE TABLE events_empty (
  id UBIGINT NOT NULL, ts BIGINT, ts_state UTINYINT NOT NULL, level UTINYINT NOT NULL, source UINTEGER NOT NULL,
  line UINTEGER NOT NULL, byte_offset UBIGINT NOT NULL, message VARCHAR NOT NULL, template VARCHAR, exception VARCHAR,
  markers UBIGINT NOT NULL, raw VARCHAR NOT NULL{declared}, extra MAP(VARCHAR, VARCHAR) NOT NULL);"
    )
}

/// `CREATE OR REPLACE VIEW events` over the segment files `files` (absolute paths).
pub fn events_view(files: &[String]) -> String {
    let from = if files.is_empty() {
        "events_empty".to_string()
    } else {
        let list = files
            .iter()
            .map(|f| format!("'{}'", f.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ");
        format!("read_parquet([{list}])")
    };
    format!("CREATE OR REPLACE VIEW events AS SELECT * FROM {from} WHERE source IN (SELECT id FROM sources WHERE ready)")
}

/// The declared columns as stored in `meta` (`name\ttype` lines).
pub fn encode_declared(schema: &EventSchema) -> String {
    schema
        .declared()
        .iter()
        .map(|c| format!("{}\t{}", c.name.replace(['\t', '\n'], " "), type_name(c.ty)))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn decode_declared(text: &str) -> Option<EventSchema> {
    let mut cols = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let (name, t) = line.split_once('\t')?;
        cols.push((name.to_string(), parse_type(t)?));
    }
    EventSchema::new(cols).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_columns_round_trip() {
        let s = EventSchema::new([
            ("A", ColumnType::Utf8),
            ("B", ColumnType::Int64),
            ("C", ColumnType::Float64),
            ("D", ColumnType::Boolean),
        ])
        .unwrap();
        let back = decode_declared(&encode_declared(&s)).unwrap();
        assert!(back.same_columns(&s));
        assert!(decode_declared("A\tnope").is_none());
        assert!(decode_declared("no tab").is_none());
        assert!(decode_declared("").unwrap().declared().is_empty());
        let ddl = ddl(&s);
        assert!(ddl.contains("\"f_C\" DOUBLE") && ddl.contains("\"f_D\" BOOLEAN"), "{ddl}");
        assert!(events_view(&[]).contains("FROM events_empty"));
        assert!(events_view(&["C:/it's/a.parquet".into()]).contains("read_parquet(['C:/it''s/a.parquet'])"));
    }
}
