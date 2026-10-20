//! The database layout. `meta` records the format version and the declared columns, so a store is
//! reopened with the schema it was written with, and a newer format is refused.

use crate::sql::ident;
use imt_core::batch::{ColumnType, EventSchema};

/// Version of the database layout this build writes and reads.
pub const FORMAT_VERSION: u32 = 1;

/// The analysis cache (engine review finding 18): results per data version, analysis kind and
/// canonical query (its FNV-1a hash and text).
pub const CACHE_DDL: &str = "CREATE TABLE IF NOT EXISTS analysis_cache (
  version UBIGINT NOT NULL, kind VARCHAR NOT NULL, hash VARCHAR NOT NULL, query VARCHAR NOT NULL, value VARCHAR NOT NULL,
  PRIMARY KEY (version, kind, hash));";

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
CREATE TABLE events (
  id UBIGINT NOT NULL, ts BIGINT, ts_state UTINYINT NOT NULL, level UTINYINT NOT NULL, source UINTEGER NOT NULL,
  line UINTEGER NOT NULL, byte_offset UBIGINT NOT NULL, message VARCHAR NOT NULL, template VARCHAR, exception VARCHAR,
  markers UBIGINT NOT NULL, raw VARCHAR NOT NULL{declared}, extra MAP(VARCHAR, VARCHAR) NOT NULL);"
    )
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
    }
}
