//! The database layout. `meta` records the format version and the declared columns, so a store is
//! reopened with the schema it was written with, and a newer format is refused.
//!
//! Events live in Parquet segments (see [`crate::segments`]) listed in `segments`; `events` is a view over
//! the segments of the sources that are ready (completely imported and not removed), recreated whenever
//! the list changes and when the store is opened (it names the files by absolute path). It is read-only:
//! changing events means writing new segments and swapping them in the manifest. `events_empty` has the
//! same columns and stands in while there is no segment. `ord` maps time-order positions to ids.

use crate::sql::ident;
use debrief_core::batch::{ColumnType, EventSchema};

/// Version of the database layout this build writes and reads.
/// 3: sources record their parser (ADR 0026 preparation) and the `parsers` table embeds each parser's TOML.
pub const FORMAT_VERSION: u32 = 3;

/// The analysis cache (engine review finding 18): results per data version, analysis kind and
/// canonical query (its FNV-1a hash and text).
pub const CACHE_DDL: &str = "CREATE TABLE IF NOT EXISTS analysis_cache (
  version UBIGINT NOT NULL, kind VARCHAR NOT NULL, hash VARCHAR NOT NULL, query VARCHAR NOT NULL, value VARCHAR NOT NULL,
  PRIMARY KEY (version, kind, hash));";

/// Tables added after the format was first written (created with a new store and, if missing, when a store opens):
/// `source_encodings` (how each source file was decoded, by path; a `manual` row is the user's choice and is read
/// with again on every re-import), `clock_offsets` (a source file's clock-skew correction in ms, already applied to its events) and `tail_offsets`
/// (where live tail reads each followed file next: the byte and the line it continues at, the size it saw, whether
/// it is read whole when it changes and why, whether its last event is held back and when it last grew).
pub const ADDED_DDL: &str = "CREATE TABLE IF NOT EXISTS source_encodings (path VARCHAR PRIMARY KEY, label VARCHAR NOT NULL, confidence UTINYINT NOT NULL, origin VARCHAR NOT NULL);
CREATE TABLE IF NOT EXISTS clock_offsets (path VARCHAR PRIMARY KEY, offset_ms BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS tail_offsets (
  source UINTEGER PRIMARY KEY, byte UBIGINT NOT NULL, line UBIGINT NOT NULL, size UBIGINT NOT NULL,
  whole BOOLEAN NOT NULL DEFAULT FALSE, held BOOLEAN NOT NULL DEFAULT FALSE, note VARCHAR, changed_ms BIGINT);
CREATE TABLE IF NOT EXISTS catalogue (
  source UINTEGER NOT NULL, scope VARCHAR NOT NULL, name VARCHAR NOT NULL, norm VARCHAR NOT NULL, n UBIGINT NOT NULL,
  first_ts BIGINT, last_ts BIGINT, first_id UBIGINT NOT NULL, last_id UBIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS catalogue_trigrams (source UINTEGER NOT NULL, tri VARCHAR NOT NULL, scope VARCHAR NOT NULL, name VARCHAR NOT NULL);
CREATE TABLE IF NOT EXISTS catalogue_sources (
  source UINTEGER PRIMARY KEY, names UBIGINT NOT NULL, dropped UBIGINT NOT NULL, guids UBIGINT NOT NULL, version UINTEGER NOT NULL);";

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
  ts_from BIGINT, ts_to BIGINT, bytes UBIGINT NOT NULL DEFAULT 0, ready BOOLEAN NOT NULL DEFAULT FALSE,
  parser_id VARCHAR, parser_version VARCHAR, parser_hash VARCHAR);
CREATE TABLE parsers (hash VARCHAR PRIMARY KEY, id VARCHAR NOT NULL, version VARCHAR NOT NULL, toml VARCHAR NOT NULL);
CREATE TABLE diagnostics (source UINTEGER NOT NULL, kind VARCHAR NOT NULL, severity VARCHAR NOT NULL, count UBIGINT NOT NULL, samples VARCHAR NOT NULL);
CREATE TABLE marker_kinds (generation UBIGINT NOT NULL, bit UTINYINT NOT NULL, kind VARCHAR NOT NULL, owner VARCHAR NOT NULL);
CREATE TABLE segments (file VARCHAR PRIMARY KEY, first_id UBIGINT NOT NULL, end_id UBIGINT NOT NULL, rows UBIGINT NOT NULL, live UBIGINT NOT NULL, indexed BOOLEAN NOT NULL DEFAULT FALSE, promoted VARCHAR NOT NULL DEFAULT '[]');
CREATE TABLE segment_fields (file VARCHAR NOT NULL, key VARCHAR NOT NULL, rows UBIGINT NOT NULL, numeric_rows UBIGINT NOT NULL, distinct_est UBIGINT NOT NULL, top_values VARCHAR NOT NULL);
CREATE TABLE histogram (file VARCHAR NOT NULL, minute BIGINT, level UTINYINT NOT NULL, source UINTEGER NOT NULL, n UBIGINT NOT NULL);
CREATE TABLE trash (path VARCHAR PRIMARY KEY);
CREATE TABLE ord (pos UBIGINT NOT NULL, id UBIGINT NOT NULL);
CREATE TABLE events_empty (
  id UBIGINT NOT NULL, ts BIGINT, ts_state UTINYINT NOT NULL, level UTINYINT NOT NULL, source UINTEGER NOT NULL,
  line UINTEGER NOT NULL, byte_offset UBIGINT NOT NULL, message VARCHAR NOT NULL, template VARCHAR, exception VARCHAR,
  markers UBIGINT NOT NULL, raw VARCHAR NOT NULL{declared}, extra MAP(VARCHAR, VARCHAR) NOT NULL);
{ADDED_DDL}"
    )
}

/// `read_parquet` over segment files (absolute paths), columns matched by name (segments promote
/// different fields).
pub fn parquet_from(files: &[String]) -> String {
    let list = files
        .iter()
        .map(|f| format!("'{}'", f.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    format!("read_parquet([{list}], union_by_name = true)")
}

/// The event columns in [`EventSchema`] order (what `SELECT e.*` on `events` returns); promoted `p_`
/// columns are left out (they repeat values that `extra` holds too).
pub fn canonical_columns(schema: &EventSchema) -> String {
    schema
        .arrow()
        .fields()
        .iter()
        .map(|f| ident(f.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Rows of ready sources only.
pub const READY: &str = "source IN (SELECT id FROM sources WHERE ready)";

/// `CREATE OR REPLACE VIEW` statements of `events` (the event schema; read-only) and `events_wide` (every
/// stored column, the promoted `p_<key>` ones included, for fast access to promoted fields) over the
/// segment files `files` (absolute paths).
pub fn events_view(schema: &EventSchema, files: &[String]) -> String {
    if files.is_empty() {
        return "CREATE OR REPLACE VIEW events AS SELECT * FROM events_empty;
CREATE OR REPLACE VIEW events_wide AS SELECT * FROM events_empty;"
            .to_string();
    }
    let from = parquet_from(files);
    format!(
        "CREATE OR REPLACE VIEW events AS SELECT {} FROM {from} WHERE {READY};
CREATE OR REPLACE VIEW events_wide AS SELECT * FROM {from} WHERE {READY};",
        canonical_columns(schema)
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
        let plain = EventSchema::plain();
        assert!(events_view(&plain, &[]).contains("FROM events_empty"));
        let v = events_view(&plain, &["C:/it's/a.parquet".into()]);
        assert!(v.contains("read_parquet(['C:/it''s/a.parquet'], union_by_name = true)"), "{v}");
        assert!(v.contains("CREATE OR REPLACE VIEW events_wide"), "{v}");
        assert!(canonical_columns(&plain).ends_with("\"raw\", \"extra\""));
    }
}
