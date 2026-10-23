//! Per-lane counts for timelines whose sources hold several instances (lanes): `lane_units` (events per instance
//! key, time unit and level) and `lane_sources` (each lane's sources with their first and last time). Large
//! stores answer such timelines from them instead of reading every event's instance fields (each pass over the
//! field map took about a second at 10M events, three passes per timeline). Both are small: lanes × units
//! (at most [`UNITS`] over the store's span) and lanes × sources.
//!
//! They are a cache: materialised on first use per data version and instance key (one pass over the events,
//! about the cost of one such timeline), dropped when the data version moves on.

use crate::analysis::fnv1a;
use crate::sql::{lit, FROM};
use crate::Inner;
use debrief_core::ports::StoreResult;
use duckdb::Connection;

/// Time units over the store's span: timelines with up to this many buckets over everything can use them.
pub const UNITS: usize = 2_000;

/// The shortest unit (ms).
const MIN_UNIT_MS: i64 = 1_000;

/// The tables (created with the analysis cache).
pub const DDL: &str = "CREATE TABLE IF NOT EXISTS lane_units (
  version UBIGINT NOT NULL, key_hash VARCHAR NOT NULL, k VARCHAR NOT NULL, unit BIGINT NOT NULL, level UTINYINT NOT NULL,
  n UBIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS lane_sources (
  version UBIGINT NOT NULL, key_hash VARCHAR NOT NULL, k VARCHAR NOT NULL, source UINTEGER NOT NULL, tmin BIGINT NOT NULL,
  tmax BIGINT NOT NULL);
CREATE TABLE IF NOT EXISTS lane_marks (
  version UBIGINT NOT NULL, key_hash VARCHAR NOT NULL, k VARCHAR NOT NULL, ts BIGINT NOT NULL, source UINTEGER NOT NULL,
  line UINTEGER NOT NULL, id UBIGINT NOT NULL, level UTINYINT NOT NULL, markers UBIGINT NOT NULL, message VARCHAR NOT NULL);";

/// The tables, each with the rows of one data version and instance key.
const TABLES: [&str; 3] = ["lane_units", "lane_sources", "lane_marks"];

/// Drops the rows of data versions before `version`.
pub fn prune_sql(version: u64) -> String {
    TABLES.map(|t| format!("DELETE FROM {t} WHERE version < {version};")).join(" ")
}

/// The rows of one instance key in the current data version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaneCounts {
    version: u64,
    hash: String,
    /// The length of a unit (ms).
    pub unit_ms: i64,
}

impl LaneCounts {
    /// The SQL condition selecting its rows (columns unqualified).
    pub(crate) fn condition(&self) -> String {
        format!("version = {} AND key_hash = '{}'", self.version, self.hash)
    }

    /// [`LaneCounts::condition`] on the table aliased `alias`.
    pub(crate) fn condition_of(&self, alias: &str) -> String {
        format!("{alias}.version = {} AND {alias}.key_hash = '{}'", self.version, self.hash)
    }
}

impl Inner {
    /// The lane counts of the instance key `key` (its SQL), materialised now if they are not; `None` while an
    /// import runs or when materialising failed (the caller then groups the events itself).
    pub(crate) fn lane_counts(&self, conn: &Connection, key: &str) -> StoreResult<Option<LaneCounts>> {
        let version = self.data_version(conn)?;
        let hash = fnv1a(key);
        if let Some(unit) = self.cache_get(conn, "lane-counts", key)?.and_then(|v| v.parse().ok()) {
            return Ok(Some(LaneCounts {
                version,
                hash,
                unit_ms: unit,
            }));
        }
        if self.is_importing() {
            return Ok(None);
        }
        let Some(h) = lit(&hash) else { return Ok(None) };
        let (from, to): (Option<i64>, Option<i64>) = conn
            .query_row("SELECT min(ts_from), max(ts_to) FROM sources WHERE ready", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(crate::engine)?;
        let span = to.zip(from).map_or(0, |(t, f)| t - f);
        let unit = debrief_core::chart::nice_bucket(span, UNITS).max(MIN_UNIT_MS);
        let clear = TABLES
            .map(|t| format!("DELETE FROM {t} WHERE version < {version} OR key_hash = {h};"))
            .join(" ");
        let sql = format!(
            "BEGIN TRANSACTION;
             {clear}
             INSERT INTO lane_marks
               SELECT {version}, {h}, ({key}) AS k, e.ts, e.source, e.line, e.id, e.level, e.markers, e.message
               FROM {FROM} WHERE e.ts IS NOT NULL AND e.markers <> 0;
             INSERT INTO lane_units
               SELECT {version}, {h}, ({key}) AS k, CAST(floor(e.ts / {unit}) AS BIGINT) AS unit, e.level, count(*)
               FROM {FROM} WHERE e.ts IS NOT NULL GROUP BY ALL;
             INSERT INTO lane_sources
               SELECT {version}, {h}, ({key}) AS k, e.source, min(e.ts), max(e.ts) FROM {FROM} WHERE e.ts IS NOT NULL GROUP BY ALL;
             COMMIT;"
        );
        if let Err(e) = conn.execute_batch(&sql) {
            let _ = conn.execute_batch("ROLLBACK");
            log::warn!("the timeline's lane counts could not be materialised: {e}");
            return Ok(None);
        }
        self.cache_put(conn, "lane-counts", key, &unit.to_string())?;
        Ok(Some(LaneCounts {
            version,
            hash,
            unit_ms: unit,
        }))
    }
}
