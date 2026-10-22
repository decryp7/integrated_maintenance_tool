//! Anomaly detection where the data is: the detectors' inputs ([`AnomalyInputs`]) answered by aggregate SQL
//! over the matching events, so detecting needs memory for per-instance counts and a few sample ids, not
//! per event. Instances are grouped by the instance key's SQL ([`crate::sql::group_key_expr`]); "time order"
//! is the stores' order (time, source, line, id).
//!
//! The key is built once per event, when the instances are found: it also gives each combination of a source and
//! the values the key is made of (its parts) its instance, and every later statement finds an event's instance
//! by those values (an equi-join on a few short columns) instead of building its key again. Building the key
//! (a few dozen `CASE`s and map lookups per event) made each of the ~20 statements of a detection need 57-59 MB
//! of DuckDB's memory with 4 threads, at the edge of the analysis memory test's 64 MB limit (out of memory in a
//! third of its Linux runs); the join needs 44-46 MB and runs 40% faster.

use crate::engine;
use crate::sql::{double, lit, number_expr, text_expr, Fields, FROM};
use duckdb::arrow::array::{Array, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray, StructArray, UInt64Array};
use duckdb::Connection;
use imt_core::analysis::anomaly::{
    sample_id, says_stopped, BucketCount, CountedMedian, Ending, Gaps, Instance, InstanceRange, Latency, Layout, OutlierBucket, Trigger,
};
use imt_core::analysis::trend::counter_place;
use imt_core::analysis::{AnomalyInputs, GrowthSeries, TrendConfig};
use imt_core::model::Level;
use imt_core::ports::{StoreError, StoreResult};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Ids looked up by message at once.
const IDS_PER_STATEMENT: usize = 2_000;

static NEXT_TABLE: AtomicU64 = AtomicU64::new(0);

/// What identifies a PerfMon counter series: machine, process, process index and counter.
type CounterKey = (String, String, Option<f64>, String);

/// The inputs of one detection over the events of `filter`, on one connection (the instances are kept in a
/// temporary table of it, dropped at the end).
pub(crate) struct SqlInputs<'c> {
    conn: &'c Connection,
    /// `<filter> AND e.ts IS NOT NULL`.
    filter: String,
    /// The instance key of an event.
    key: String,
    /// The SQL of the values the key is made of (besides the source).
    parts: Vec<String>,
    fields: &'c Fields,
    /// The temporary table of instances: `i` (index, in key order), `k`, `n`, `first`, `last`.
    table: String,
    /// The temporary table giving each combination of a source and key parts its instance: `src`, `p0`.., `i`,
    /// `last` (the instance's).
    of: String,
    /// Each instance's key and events (by index).
    names: Vec<String>,
    events: Vec<u64>,
    /// The keys the segments' field statistics list (`None`: not read yet, or not reliable: every key may exist).
    keys: Option<Option<std::collections::HashSet<String>>>,
}

impl Drop for SqlInputs<'_> {
    fn drop(&mut self) {
        let _ = self.conn.execute_batch(&format!(
            "DROP TABLE IF EXISTS {0}; DROP TABLE IF EXISTS {0}_parts; DROP TABLE IF EXISTS {1}",
            self.table, self.of
        ));
    }
}

/// The gaps between one instance's consecutive timestamps (fed in ascending order): counts per distinct gap,
/// for the exact median, and the long gaps. The distinct gaps of an instance spanning `s` ms are fewer than
/// `sqrt(2 s)` (about 250,000 for a year), whatever its number of events.
struct GapCounter {
    at_least: f64,
    previous: Option<i64>,
    counts: HashMap<i64, u64>,
    gaps: u64,
    long: Vec<(i64, i64)>,
}

impl GapCounter {
    fn new(at_least: f64) -> GapCounter {
        GapCounter {
            at_least,
            previous: None,
            counts: HashMap::new(),
            gaps: 0,
            long: Vec::new(),
        }
    }

    fn add(&mut self, ts: i64) {
        if let Some(p) = self.previous {
            let gap = ts.saturating_sub(p);
            *self.counts.entry(gap).or_default() += 1;
            self.gaps += 1;
            if gap as f64 >= self.at_least {
                self.long.push((p, ts));
            }
        }
        self.previous = Some(ts);
    }

    fn finish(self) -> Gaps {
        let mut runs: Vec<(i64, u64)> = self.counts.into_iter().collect();
        runs.sort_unstable();
        let mut median = CountedMedian::new(self.gaps);
        for (gap, n) in runs {
            median.push(gap as f64, n);
        }
        Gaps {
            median: median.median(),
            long: self.long,
        }
    }
}

/// `(VALUES (..), ..) AS v(<columns>)` of rows already in SQL.
fn values(rows: impl IntoIterator<Item = String>, columns: &str) -> Option<String> {
    let rows: Vec<String> = rows.into_iter().map(|r| format!("({r})")).collect();
    (!rows.is_empty()).then(|| format!("(VALUES {}) AS v({columns})", rows.join(", ")))
}

/// Comma-separated ids (as `string_agg` gives them) parsed.
fn ids(text: &str) -> Vec<u32> {
    text.split(',')
        .filter_map(|s| s.trim().parse::<u64>().ok())
        .map(sample_id)
        .collect()
}

fn column<T: 'static>(rb: &RecordBatch, i: usize) -> StoreResult<&T> {
    rb.column(i)
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| StoreError::Engine(format!("anomaly detection: unexpected type of column {i}")))
}

impl<'c> SqlInputs<'c> {
    /// The inputs over the events of `filter`, whose instance key is `key`, made of the source and the values
    /// `parts` (see [`crate::sql::group_key_expr`]: the SQL of its fields' text).
    pub(crate) fn new(conn: &'c Connection, filter: &str, key: &str, parts: Vec<String>, fields: &'c Fields) -> SqlInputs<'c> {
        let n = NEXT_TABLE.fetch_add(1, Ordering::Relaxed);
        SqlInputs {
            conn,
            filter: format!("({filter}) AND e.ts IS NOT NULL"),
            key: key.to_string(),
            parts,
            fields,
            table: format!("anomaly_instances_{n}"),
            of: format!("anomaly_instance_of_{n}"),
            names: Vec::new(),
            events: Vec::new(),
            keys: None,
        }
    }

    /// `JOIN <instance of> t ON ...`: an event's instance `t.i` (and its `t.last`), by its source and key parts.
    fn join(&self) -> String {
        let mut on = vec!["t.src = e.source".to_string()];
        on.extend(
            self.parts
                .iter()
                .enumerate()
                .map(|(j, p)| format!("t.p{j} IS NOT DISTINCT FROM ({p})")),
        );
        format!("JOIN {} t ON {}", self.of, on.join(" AND "))
    }

    /// `FROM <events> JOIN <instance of> t ... WHERE <filter>`: the events with their instance `t.i`.
    fn from(&self) -> String {
        format!("{FROM} {} WHERE {}", self.join(), self.filter)
    }

    fn bucket(layout: &Layout) -> String {
        format!(
            "least((e.ts - ({})) // {}, {})",
            layout.start,
            layout.bucket.max(1),
            layout.buckets.saturating_sub(1)
        )
    }

    fn number(&self, field: &str) -> StoreResult<String> {
        number_expr(field, self.fields).ok_or_else(|| StoreError::Invalid(format!("the field name {field:?} cannot be read")))
    }

    fn text(&self, field: &str) -> StoreResult<String> {
        text_expr(field, self.fields).ok_or_else(|| StoreError::Invalid(format!("the field name {field:?} cannot be read")))
    }

    /// Whether some event may have `field`: false only when the segments' field statistics, which list every key
    /// of every segment, do not know it (a scan over a field nobody has is skipped). A segment with as many keys as
    /// the statistics track may have more, so then every field may exist.
    fn may_have(&mut self, field: &str) -> bool {
        if field.starts_with('@') {
            return true;
        }
        if self.keys.is_none() {
            let read = || -> duckdb::Result<Option<std::collections::HashSet<String>>> {
                let widest: i64 = self.conn.query_row(
                    "SELECT coalesce(max(c), 0) FROM (SELECT count(*) AS c FROM segment_fields GROUP BY file)",
                    [],
                    |r| r.get(0),
                )?;
                if widest >= crate::stats::MAX_FIELDS as i64 {
                    return Ok(None);
                }
                let mut stmt = self.conn.prepare("SELECT DISTINCT key FROM segment_fields")?;
                let keys = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<duckdb::Result<_>>()?;
                Ok(Some(keys))
            };
            self.keys = Some(read().unwrap_or(None));
        }
        match &self.keys {
            Some(Some(keys)) => keys.contains(field) || self.fields.schema.declared_index(field).is_some(),
            _ => true,
        }
    }

    /// Rows of `sql`, each mapped by `f`.
    fn rows<T>(&self, sql: &str, f: impl FnMut(&duckdb::Row<'_>) -> duckdb::Result<T>) -> StoreResult<Vec<T>> {
        let mut stmt = self.conn.prepare(sql).map_err(engine)?;
        let rows = stmt.query_map([], f).map_err(engine)?;
        rows.collect::<duckdb::Result<Vec<T>>>().map_err(engine)
    }

    /// The result of `sql` as Arrow batches, streamed (never materialised whole).
    fn stream(&self, sql: &str, f: &mut dyn FnMut(&RecordBatch) -> StoreResult<()>) -> StoreResult<()> {
        let mut stmt = self.conn.prepare(sql).map_err(engine)?;
        let _ = stmt.stream_arrow([]).map_err(engine)?;
        while let Some(chunk) = stmt.step().map_err(engine)? {
            f(&RecordBatch::from(&chunk as &StructArray))?;
        }
        Ok(())
    }

    /// For each of `count` requests, the first `n` ids of `events` ordered by the columns `order` (the last ones
    /// when `last`), where `events` selects the request `r`, `id` and those columns. A top-n per request (no sort
    /// of the events, no window).
    fn first_ids(&self, count: usize, events: &str, order: &[&str], last: bool, n: usize) -> StoreResult<Vec<Vec<u32>>> {
        let mut out = vec![Vec::new(); count];
        if count == 0 {
            return Ok(out);
        }
        let key: Vec<String> = order.iter().map(|c| format!("{c} := {c}")).collect();
        let pick = if last { "arg_max" } else { "arg_min" };
        let sql = format!(
            "SELECT r, array_to_string({pick}(id, struct_pack({}), {n}), ',') FROM ({events}) GROUP BY r",
            key.join(", ")
        );
        for (r, list) in self.rows(&sql, |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))? {
            if let Some(slot) = usize::try_from(r).ok().and_then(|r| out.get_mut(r)) {
                *slot = ids(&list);
            }
        }
        Ok(out)
    }

    /// The messages of events by id.
    fn messages(&self, wanted: &[u64]) -> StoreResult<HashMap<u64, String>> {
        let mut out = HashMap::new();
        for chunk in wanted.chunks(IDS_PER_STATEMENT) {
            let list: Vec<String> = chunk.iter().map(u64::to_string).collect();
            let sql = format!("SELECT e.id, e.message FROM {FROM} WHERE e.id IN ({})", list.join(", "));
            out.extend(self.rows(&sql, |r| Ok((r.get::<_, u64>(0)?, r.get::<_, String>(1)?)))?);
        }
        Ok(out)
    }
}

impl AnomalyInputs for SqlInputs<'_> {
    type Error = StoreError;

    fn instances(&mut self) -> StoreResult<Vec<Instance>> {
        // The one statement that builds the key: per source and key parts (a few per source), then per key.
        let part_values: String = self.parts.iter().enumerate().map(|(j, p)| format!(", ({p}) AS p{j}")).collect();
        let part_columns: String = (0..self.parts.len()).map(|j| format!(", p.p{j}")).collect();
        self.conn
            .execute_batch(&format!(
                "CREATE OR REPLACE TEMP TABLE {t}_parts AS
                   SELECT e.source AS src{part_values}, ({key}) AS k, count(*) AS n, min(e.ts) AS first, max(e.ts) AS last
                   FROM {FROM} WHERE {filter} GROUP BY ALL;
                 CREATE OR REPLACE TEMP TABLE {t} AS SELECT CAST(row_number() OVER (ORDER BY k) - 1 AS INTEGER) AS i, k, n, first, last FROM (
                   SELECT k, CAST(sum(n) AS BIGINT) AS n, min(first) AS first, max(last) AS last FROM {t}_parts GROUP BY k);
                 CREATE OR REPLACE TEMP TABLE {of} AS SELECT p.src{part_columns}, t.i, t.last FROM {t}_parts p JOIN {t} t ON t.k = p.k;
                 DROP TABLE {t}_parts;",
                t = self.table,
                of = self.of,
                key = self.key,
                filter = self.filter,
            ))
            .map_err(engine)?;
        let mut instances = self.rows(&format!("SELECT k, n, first, last FROM {} ORDER BY i", self.table), |r| {
            Ok(Instance {
                key: r.get(0)?,
                events: r.get(1)?,
                first: r.get(2)?,
                last: r.get(3)?,
            })
        })?;
        // The detectors expect Rust's order of keys (bytes); DuckDB's is the same, which this keeps true.
        if !instances.windows(2).all(|w| w[0].key < w[1].key) {
            return Err(StoreError::Engine("anomaly detection: instances are not in key order".into()));
        }
        instances.shrink_to_fit();
        self.events = instances.iter().map(|x| x.events).collect();
        self.names = instances.iter().map(|x| x.key.clone()).collect();
        Ok(instances)
    }

    fn bucket_counts(&mut self, layout: &Layout) -> StoreResult<Vec<BucketCount>> {
        let sql = format!(
            "SELECT t.i, {} AS b, count(*), count(*) FILTER (WHERE e.level >= {}) FROM {} GROUP BY ALL",
            Self::bucket(layout),
            Level::Error.index(),
            self.from()
        );
        self.rows(&sql, |r| {
            Ok(BucketCount {
                instance: r.get::<_, i32>(0)?.max(0) as usize,
                bucket: r.get::<_, i64>(1)?.max(0) as usize,
                events: r.get(2)?,
                errors: r.get(3)?,
            })
        })
    }

    fn error_ids(&mut self, ranges: &[InstanceRange], n: usize) -> StoreResult<Vec<Vec<u32>>> {
        let Some(v) = values(
            ranges
                .iter()
                .enumerate()
                .map(|(r, x)| format!("{r}, {}, {}, {}", x.instance, x.from, x.to)),
            "r, i, a, z",
        ) else {
            return Ok(Vec::new());
        };
        let events = format!(
            "SELECT v.r AS r, e.id AS id, e.ts AS ts, e.source AS source, e.line AS line FROM {FROM} {}
             JOIN {v} ON v.i = t.i AND e.ts BETWEEN v.a AND v.z WHERE {} AND e.level >= {}",
            self.join(),
            self.filter,
            Level::Error.index()
        );
        self.first_ids(ranges.len(), &events, &["ts", "source", "line", "id"], false, n)
    }

    fn gaps(&mut self, instances: &[usize], at_least: f64) -> StoreResult<Vec<Gaps>> {
        let Some(v) = values(instances.iter().map(|i| i.to_string()), "i") else {
            return Ok(Vec::new());
        };
        // The instance and time of each event, sorted (a sort spills to disk; DuckDB's window operator does not,
        // and ran out of memory on large instances): the gaps are counted here, one instance at a time.
        let sql = format!(
            "SELECT t.i AS i, e.ts AS ts FROM {FROM} {} JOIN {v} ON v.i = t.i WHERE {} ORDER BY i, ts",
            self.join(),
            self.filter
        );
        let wanted: HashMap<usize, usize> = instances.iter().enumerate().map(|(r, i)| (*i, r)).collect();
        let mut out: Vec<Gaps> = vec![Gaps::default(); instances.len()];
        let mut current: Option<(usize, GapCounter)> = None;
        self.stream(&sql, &mut |rb| {
            let (i, ts) = (column::<Int32Array>(rb, 0)?, column::<Int64Array>(rb, 1)?);
            for row in 0..rb.num_rows() {
                let Some(&r) = wanted.get(&(i.value(row).max(0) as usize)) else {
                    continue;
                };
                if current.as_ref().is_none_or(|c| c.0 != r) {
                    if let Some((done, counter)) = current.take() {
                        out[done] = counter.finish();
                    }
                    current = Some((r, GapCounter::new(at_least)));
                }
                if let Some((_, counter)) = &mut current {
                    counter.add(ts.value(row));
                }
            }
            Ok(())
        })?;
        if let Some((done, counter)) = current {
            out[done] = counter.finish();
        }
        Ok(out)
    }

    fn endings(&mut self, crash_window: i64) -> StoreResult<Vec<Ending>> {
        let last_of = |extra: &str| {
            format!(
                "SELECT i, struct_extract(f, 'ts'), struct_extract(f, 'id'), struct_extract(f, 'level') FROM (
                   SELECT t.i AS i, max(struct_pack(ts := e.ts, source := e.source, line := e.line, id := e.id, level := e.level)) AS f
                   FROM {} {extra} GROUP BY t.i)",
                self.from()
            )
        };
        let last = self.rows(&last_of(""), |r| Ok((r.get::<_, i32>(0)?, r.get::<_, u64>(2)?)))?;
        let triggers = self.rows(
            &last_of(&format!(
                "AND (e.level = {} OR (e.exception IS NOT NULL AND e.level >= {})) AND e.ts >= t.last - ({crash_window})",
                Level::Fatal.index(),
                Level::Error.index()
            )),
            |r| Ok((r.get::<_, i32>(0)?, r.get::<_, i64>(1)?, r.get::<_, u64>(2)?, r.get::<_, u8>(3)?)),
        )?;
        let wanted: Vec<u64> = last.iter().map(|l| l.1).chain(triggers.iter().map(|t| t.2)).collect();
        let messages = self.messages(&wanted)?;
        let message = |id: u64| messages.get(&id).cloned().unwrap_or_default();
        let mut out: Vec<Ending> = (0..self.events.len())
            .map(|_| Ending {
                last_id: 0,
                stopped: false,
                trigger: None,
            })
            .collect();
        for (i, id) in last {
            if let Some(e) = usize::try_from(i).ok().and_then(|i| out.get_mut(i)) {
                e.last_id = sample_id(id);
                e.stopped = says_stopped(&message(id));
            }
        }
        for (i, ts, id, level) in triggers {
            if let Some(e) = usize::try_from(i).ok().and_then(|i| out.get_mut(i)) {
                e.trigger = Some(Trigger {
                    ts,
                    id: sample_id(id),
                    level: Level::ALL[usize::from(level).min(5)],
                    message: message(id),
                });
            }
        }
        Ok(out)
    }

    fn latest_error_ids(&mut self, instances: &[usize], n: usize) -> StoreResult<Vec<Vec<u32>>> {
        let Some(v) = values(instances.iter().enumerate().map(|(r, i)| format!("{r}, {i}")), "r, i") else {
            return Ok(Vec::new());
        };
        let events = format!(
            "SELECT v.r AS r, e.id AS id, e.ts AS ts, e.source AS source, e.line AS line FROM {FROM} {} JOIN {v} ON v.i = t.i WHERE {} AND e.level >= {}",
            self.join(),
            self.filter,
            Level::Error.index()
        );
        self.first_ids(instances.len(), &events, &["ts", "source", "line", "id"], true, n)
    }

    fn latency(&mut self, field: &str) -> StoreResult<Latency> {
        if !self.may_have(field) {
            return Ok(Latency::default());
        }
        // One pass: the count and the exact (interpolated) median and 99th percentile.
        let sql = format!(
            "SELECT count(v), quantile_cont(v, 0.5), quantile_cont(v, 0.99) FROM (SELECT {} AS v FROM {FROM} WHERE {})",
            self.number(field)?,
            self.filter
        );
        let row = self.rows(&sql, |r| {
            Ok((r.get::<_, u64>(0)?, r.get::<_, Option<f64>>(1)?, r.get::<_, Option<f64>>(2)?))
        })?;
        Ok(match row.first() {
            Some((n, Some(median), Some(p99))) => Latency {
                count: *n as usize,
                median: *median,
                p99: *p99,
            },
            _ => Latency::default(),
        })
    }

    fn value_count(&mut self, field: &str) -> StoreResult<usize> {
        let sql = format!("SELECT count({}) FROM {FROM} WHERE {}", self.number(field)?, self.filter);
        Ok(self.rows(&sql, |r| r.get::<_, u64>(0))?.first().copied().unwrap_or(0) as usize)
    }

    fn values_at(&mut self, field: &str, ranks: &[usize]) -> StoreResult<Vec<f64>> {
        if ranks.is_empty() {
            return Ok(Vec::new());
        }
        // The distinct values with their counts, ascending (an aggregate and a sort, which spill; not a window
        // over every value), walked to the ranks.
        let sql = format!(
            "SELECT v, count(*) FROM (SELECT {} AS v FROM {FROM} WHERE {}) WHERE v IS NOT NULL GROUP BY v ORDER BY v",
            self.number(field)?,
            self.filter
        );
        let mut out = Vec::with_capacity(ranks.len());
        let (mut seen, mut next) = (0u64, 0);
        self.stream(&sql, &mut |rb| {
            let (v, n) = (column::<Float64Array>(rb, 0)?, column::<Int64Array>(rb, 1)?);
            for row in 0..rb.num_rows() {
                seen += n.value(row).max(0) as u64;
                while ranks.get(next).is_some_and(|r| (*r as u64) < seen) {
                    out.push(v.value(row));
                    next += 1;
                }
            }
            Ok(())
        })?;
        Ok(out)
    }

    fn outliers(&mut self, field: &str, above: f64, at_least: f64, layout: &Layout) -> StoreResult<Vec<OutlierBucket>> {
        let (Some(above), Some(at_least)) = (double(above), double(at_least)) else {
            return Ok(Vec::new());
        };
        let sql = format!(
            "SELECT b, count(*), max(v), min(i), count(DISTINCT i),
                    array_to_string(list(id ORDER BY i, ts, source, line, id)[1:{}], ',')
             FROM (SELECT t.i AS i, {} AS b, {} AS v, e.id AS id, e.ts AS ts, e.source AS source, e.line AS line FROM {})
             WHERE v > {above} AND v >= {at_least} GROUP BY b ORDER BY b",
            imt_core::analysis::anomaly::MAX_SAMPLES,
            Self::bucket(layout),
            self.number(field)?,
            self.from()
        );
        let rows = self.rows(&sql, |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, f64>(2)?,
                r.get::<_, i32>(3)?,
                r.get::<_, u64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        Ok(rows
            .into_iter()
            .map(|(b, count, peak, first, distinct, list)| OutlierBucket {
                bucket: b.max(0) as usize,
                count: count as usize,
                peak,
                first_instance: first.max(0) as usize,
                single: distinct == 1,
                ids: ids(&list),
            })
            .collect())
    }

    fn bucket_ids(&mut self, layout: &Layout, runs: &[(usize, usize)], n: usize) -> StoreResult<Vec<Vec<u32>>> {
        let Some(v) = values(runs.iter().enumerate().map(|(r, (a, z))| format!("{r}, {a}, {z}")), "r, a, z") else {
            return Ok(Vec::new());
        };
        let events = format!(
            "SELECT v.r AS r, x.* FROM (SELECT t.i AS i, {} AS b, e.id AS id, e.ts AS ts, e.source AS source, e.line AS line FROM {}) x
             JOIN {v} ON x.b BETWEEN v.a AND v.z",
            Self::bucket(layout),
            self.from()
        );
        self.first_ids(runs.len(), &events, &["b", "i", "ts", "source", "line", "id"], false, n)
    }

    fn growth_series(&mut self, cfg: &TrendConfig, f: &mut dyn FnMut(GrowthSeries)) -> StoreResult<()> {
        // The instances' growth fields, one field at a time, in instance then time order.
        for field in &cfg.fields {
            if !self.may_have(field) {
                continue;
            }
            let sql = format!(
                "SELECT i, ts, v, CAST(id AS UBIGINT) FROM (
                   SELECT t.i AS i, e.ts AS ts, {} AS v, e.id AS id, e.source AS source, e.line AS line FROM {})
                 WHERE v IS NOT NULL ORDER BY i, ts, source, line, id",
                self.number(field)?,
                self.from()
            );
            let names = &self.names;
            let mut current: Option<(i32, GrowthSeries)> = None;
            self.stream(&sql, &mut |rb| {
                let (k, ts, v, id) = (
                    column::<Int32Array>(rb, 0)?,
                    column::<Int64Array>(rb, 1)?,
                    column::<Float64Array>(rb, 2)?,
                    column::<UInt64Array>(rb, 3)?,
                );
                for row in 0..rb.num_rows() {
                    let instance = k.value(row);
                    if current.as_ref().is_none_or(|s| s.0 != instance) {
                        if let Some((_, done)) = current.take() {
                            f(done);
                        }
                        let place = usize::try_from(instance)
                            .ok()
                            .and_then(|i| names.get(i))
                            .cloned()
                            .unwrap_or_default();
                        current = Some((
                            instance,
                            GrowthSeries {
                                place,
                                name: field.clone(),
                                counter: false,
                                points: Vec::new(),
                            },
                        ));
                    }
                    if let Some((_, s)) = &mut current {
                        s.points.push((ts.value(row), v.value(row), sample_id(id.value(row))));
                    }
                }
                Ok(())
            })?;
            if let Some((_, done)) = current {
                f(done);
            }
        }
        // PerfMon counters, one series per machine, process and counter, in time then instance order.
        let counters: Vec<String> = cfg.counters.iter().filter_map(|c| lit(c)).collect();
        if counters.is_empty() || !self.may_have("Counter") || !self.may_have("Value") {
            return Ok(());
        }
        let sql = format!(
            "SELECT m, p, idx, c, ts, v, CAST(id AS UBIGINT) FROM (
               SELECT coalesce({}, '') AS m, coalesce({}, {}, '') AS p, {} AS idx, {} AS c, {} AS v,
                      t.i AS i, e.ts AS ts, e.id AS id, e.source AS source, e.line AS line
               FROM {})
             WHERE c IN ({}) AND v IS NOT NULL
             ORDER BY m, p, idx NULLS FIRST, c, ts, i, source, line, id",
            self.text("MachineName")?,
            self.text("ProcessName")?,
            self.text("Instance")?,
            self.number("InstanceIndex")?,
            self.text("Counter")?,
            self.number("Value")?,
            self.from(),
            counters.join(", ")
        );
        // The current series and its (machine, process, index, counter).
        let mut current: Option<(CounterKey, GrowthSeries)> = None;
        self.stream(&sql, &mut |rb| {
            let (m, p, c) = (
                column::<StringArray>(rb, 0)?,
                column::<StringArray>(rb, 1)?,
                column::<StringArray>(rb, 3)?,
            );
            let (idx, ts, v, id) = (
                column::<Float64Array>(rb, 2)?,
                column::<Int64Array>(rb, 4)?,
                column::<Float64Array>(rb, 5)?,
                column::<UInt64Array>(rb, 6)?,
            );
            for row in 0..rb.num_rows() {
                let index = (!idx.is_null(row)).then(|| idx.value(row));
                let same = current.as_ref().is_some_and(|((cm, cp, ci, cc), _)| {
                    cm == m.value(row) && cp == p.value(row) && ci.map(f64::to_bits) == index.map(f64::to_bits) && cc == c.value(row)
                });
                if !same {
                    if let Some((_, done)) = current.take() {
                        f(done);
                    }
                    let key = (m.value(row).to_string(), p.value(row).to_string(), index, c.value(row).to_string());
                    let series = GrowthSeries {
                        place: counter_place(&key.0, &key.1, index),
                        name: key.3.clone(),
                        counter: true,
                        points: Vec::new(),
                    };
                    current = Some((key, series));
                }
                if let Some((_, s)) = &mut current {
                    s.points.push((ts.value(row), v.value(row), sample_id(id.value(row))));
                }
            }
            Ok(())
        })?;
        if let Some((_, done)) = current {
            f(done);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_build_values_and_parse_id_lists() {
        assert_eq!(values(Vec::<String>::new(), "a"), None);
        assert_eq!(values(["1, 2".to_string()], "a, b").as_deref(), Some("(VALUES (1, 2)) AS v(a, b)"));
        assert_eq!(ids("3,4, x,99999999999"), vec![3, 4, u32::MAX]);
        assert!(ids("").is_empty());
    }

    #[test]
    fn gaps_are_counted_per_distinct_gap_with_the_long_ones() {
        let mut c = GapCounter::new(10.0);
        for ts in [0, 0, 5, 20, 21, 25, 40, 44] {
            c.add(ts);
        }
        // Gaps 0, 5, 15, 1, 4, 15, 4: five distinct ones, median 4.
        assert_eq!((c.counts.len(), c.gaps), (5, 7));
        let g = c.finish();
        assert_eq!(
            g,
            Gaps {
                median: 4.0,
                long: vec![(5, 20), (25, 40)]
            }
        );
        assert_eq!(GapCounter::new(1.0).finish(), Gaps::default());
    }
}
