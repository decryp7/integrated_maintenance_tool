//! The `Analysis` port and the analyses of the `Query` port (charts, field statistics): the engine's
//! pure analyses fed with events streamed from the database in time order ([`EventStream`]), so no
//! result needs every event in memory at once. Results of the costly analyses are cached in the
//! database per data version and canonical query (engine review finding 18).

use crate::read::{count, Ctx};
use crate::sql::{expr_sql, group_key_expr, lit, number_expr, order_by, row_columns, rows_sql, text_expr, Columns, SqlFilter, FROM};
use crate::{engine, DuckStore, Inner};
use duckdb::arrow::array::{RecordBatch, StructArray};
use duckdb::{params, Connection, Statement};
use imt_core::analysis::moment::{findings, moment_queries, trace_summaries, MetricCollector, MomentBuilder, OverviewInputs};
use imt_core::analysis::{self, Anomaly, AnomalyConfig, CompareConfig, Comparison, FieldStats, RequestSpan, WaterfallRow};
use imt_core::api::{
    ChartData, ChartRequest, CompareRequest, CorrelateRequest, FieldStatsRequest, Finding, LifecycleEvent, MomentRequest, MomentSummary,
    QueryRequest, RequestsRequest, TraceRequest, TraceSummary,
};
use imt_core::batch::{EventBatch, EventId, EventSchema, BATCH_ROWS};
use imt_core::chart::{chart_defs, chart_value, parse_y, Agg, Buckets, SeriesBuilder};
use imt_core::exceptions::{self, ExceptionGroup};
use imt_core::patterns::{MinerConfig, Pattern, PatternMiner, MAX_CLUSTERS};
use imt_core::ports::{Analysis, Analyzed, Coverage, Filter, StoreError, StoreResult};
use imt_core::profile::compile_conditions;
use imt_core::ql::{Expr, QueryControl};
use imt_core::query::{group_key_row, CompiledQuery};
use imt_core::row::{to_record, ArrowRow, BatchSlots, FieldKeys, OwnedRow, RowRef};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::sync::Arc;

/// Most events one analysis looks at; the coverage says when it stopped early.
pub const MAX_ANALYSED: usize = 2_000_000;

/// Most events of its window the Moment (an interactive summary) reads, in time order; the coverage says when
/// it stopped early. 2M events took 15-30 s and 300 MB at 10M events.
pub const MOMENT_EVENTS: usize = 250_000;

/// How an analysis reads its events.
pub(crate) struct Read<'a> {
    /// An extra SQL condition.
    pub extra: Option<&'a str>,
    /// In time order (else in storage order: cheaper, for analyses that do not depend on order).
    pub ordered: bool,
    pub columns: Columns,
    /// Most events read.
    pub limit: usize,
}

impl Read<'_> {
    fn ordered(columns: Columns) -> Read<'static> {
        Read {
            extra: None,
            ordered: true,
            columns,
            limit: MAX_ANALYSED,
        }
    }

    fn unordered(columns: Columns) -> Read<'static> {
        Read {
            extra: None,
            ordered: false,
            columns,
            limit: MAX_ANALYSED,
        }
    }
}

/// Most events a correlation lists.
const MAX_CORRELATED: usize = 10_000;

/// Lifecycle markers drawn on a chart.
const MAX_LIFECYCLE: usize = 500;

/// Events read in time order a window of time at a time (see [`EventStream::ordered`]).
struct Chunks<'c> {
    conn: &'c Connection,
    /// `SELECT <columns> FROM <events> WHERE (<filter>)`; a window adds its time range and the order.
    select: String,
    /// The windows' conditions, in time order (untimed events last).
    windows: Vec<String>,
    next: usize,
    /// How many events the stream's statement selects.
    total: u64,
    /// Newest first: the windows run backwards, each sorted descending.
    descending: bool,
}

/// Events per window of a time-ordered stream (from the minute histogram: whole minutes, a busy minute
/// split into equal parts), so DuckDB sorts a window within its memory limit.
const WINDOW_EVENTS: u64 = 100_000;

impl<'c> Chunks<'c> {
    /// The windows of time over every event: `e.ts` ranges with about `per` of the `matched` events each
    /// (assuming they spread like all events), then the untimed events.
    fn windows(conn: &Connection, per: u64, matched: u64) -> StoreResult<Vec<String>> {
        let mut stmt = conn
            .prepare(
                "SELECT minute, sum(n) FROM histogram WHERE minute IS NOT NULL AND source IN (SELECT id FROM sources WHERE ready) GROUP BY minute ORDER BY minute",
            )
            .map_err(engine)?;
        let minutes = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u64>(1)?)))
            .map_err(engine)?
            .collect::<duckdb::Result<Vec<_>>>()
            .map_err(engine)?;
        let all: u64 = minutes.iter().map(|m| m.1).sum();
        let per = (per as u128 * all as u128 / matched.max(1) as u128).clamp(per as u128, u64::MAX as u128) as u64;
        Ok(time_windows(&minutes, per))
    }

    /// The statement of the next window, or `None` after the last.
    fn next_statement(&mut self) -> Option<duckdb::Result<Statement<'c>>> {
        let index = if self.descending {
            self.windows.len().checked_sub(self.next + 1)?
        } else {
            self.next
        };
        let window = self.windows.get(index)?;
        self.next += 1;
        let sql = format!("{} AND {window} {}", self.select, order_by(self.descending));
        Some(self.conn.prepare(&sql).and_then(|mut stmt| {
            let _ = stmt.stream_arrow([])?;
            Ok(stmt)
        }))
    }
}

/// Conditions on `e.ts` that split time into windows of about `per` events each from per-minute counts
/// (minutes in order), then `e.ts IS NULL`; together they cover every event exactly once.
fn time_windows(minutes: &[(i64, u64)], per: u64) -> Vec<String> {
    const MINUTE: i64 = 60_000;
    let per = per.max(1);
    // Window starts (ms); the first is open below, the last open above.
    let mut starts: Vec<i64> = Vec::new();
    let mut acc = 0u64;
    for &(minute, n) in minutes {
        let start = minute.saturating_mul(MINUTE);
        if n > per {
            // A busy minute: equal parts of it, and the next window starts after it.
            let parts = n.div_ceil(per).min(MINUTE as u64) as i64;
            starts.extend((0..parts).map(|k| start + k * MINUTE / parts));
            starts.push(start + MINUTE);
            acc = 0;
        } else {
            if acc + n > per && acc > 0 {
                starts.push(start);
                acc = 0;
            }
            acc += n;
        }
    }
    starts.sort_unstable();
    starts.dedup();
    let mut windows = Vec::with_capacity(starts.len() + 2);
    let mut lower: Option<i64> = None;
    for &s in &starts {
        windows.push(match lower {
            None => format!("e.ts < {s}"),
            Some(l) => format!("e.ts >= {l} AND e.ts < {s}"),
        });
        lower = Some(s);
    }
    windows.push(match lower {
        None => "e.ts IS NOT NULL".to_string(),
        Some(l) => format!("e.ts >= {l}"),
    });
    windows.push("e.ts IS NULL".to_string());
    windows
}

/// Matching events of one statement, in its order, one owned row at a time (the wide `raw` column is
/// not read). Rows past `limit` are counted but not yielded; an engine error or a cancel ends the
/// stream and is returned by [`EventStream::finish`].
pub(crate) struct EventStream<'c> {
    stmt: Statement<'c>,
    /// The chunks still to read of a time-ordered stream.
    chunks: Option<Chunks<'c>>,
    /// Matching events a time-ordered stream did not read (past its limit; counted by SQL).
    unread: u64,
    schema: Arc<EventSchema>,
    ctx: &'c Ctx,
    cq: &'c CompiledQuery,
    exact: bool,
    control: Option<&'c QueryControl>,
    chunk: Option<RecordBatch>,
    offset: usize,
    batch: Option<(EventBatch, BatchSlots)>,
    row: usize,
    limit: usize,
    yielded: u64,
    matched: u64,
    error: Option<StoreError>,
    done: bool,
}

impl<'c> EventStream<'c> {
    pub(crate) fn new(
        conn: &'c Connection,
        sql: &str,
        schema: Arc<EventSchema>,
        ctx: &'c Ctx,
        cq: &'c CompiledQuery,
        exact: bool,
        limit: usize,
    ) -> StoreResult<EventStream<'c>> {
        let mut stmt = conn.prepare(sql).map_err(engine)?;
        // Executes the query; chunks are fetched with `step`.
        let _ = stmt.stream_arrow([]).map_err(engine)?;
        Ok(EventStream::from_statement(stmt, schema, ctx, cq, exact, limit))
    }

    fn from_statement(
        stmt: Statement<'c>,
        schema: Arc<EventSchema>,
        ctx: &'c Ctx,
        cq: &'c CompiledQuery,
        exact: bool,
        limit: usize,
    ) -> EventStream<'c> {
        EventStream {
            stmt,
            chunks: None,
            unread: 0,
            schema,
            ctx,
            cq,
            exact,
            control: None,
            chunk: None,
            offset: 0,
            batch: None,
            row: 0,
            limit,
            yielded: 0,
            matched: 0,
            error: None,
            done: false,
        }
    }

    /// The events of `FROM .. WHERE filter` with the columns `select`, in time order, without sorting all of
    /// them at once (wide rows of 10M events need more memory than DuckDB is given): a window of time
    /// at a time, each sorted on its own. An `exact` stream stops reading at its limit (the rest are
    /// counted); otherwise every candidate is read, for the engine to check.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ordered(
        conn: &'c Connection,
        select: &str,
        filter: &str,
        schema: Arc<EventSchema>,
        ctx: &'c Ctx,
        cq: &'c CompiledQuery,
        exact: bool,
        limit: usize,
    ) -> StoreResult<EventStream<'c>> {
        EventStream::windowed(conn, select, filter, schema, ctx, cq, exact, limit, WINDOW_EVENTS, false)
    }

    /// [`EventStream::ordered`], newest first (untimed events first, as the grid shows them descending).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ordered_desc(
        conn: &'c Connection,
        select: &str,
        filter: &str,
        schema: Arc<EventSchema>,
        ctx: &'c Ctx,
        cq: &'c CompiledQuery,
        exact: bool,
        limit: usize,
    ) -> StoreResult<EventStream<'c>> {
        EventStream::windowed(conn, select, filter, schema, ctx, cq, exact, limit, WINDOW_EVENTS, true)
    }

    /// [`EventStream::ordered`] with windows of about `per` events.
    #[allow(clippy::too_many_arguments)]
    fn windowed(
        conn: &'c Connection,
        select: &str,
        filter: &str,
        schema: Arc<EventSchema>,
        ctx: &'c Ctx,
        cq: &'c CompiledQuery,
        exact: bool,
        limit: usize,
        per: u64,
        descending: bool,
    ) -> StoreResult<EventStream<'c>> {
        // Few events (a narrow query) sort at once.
        let total = count(conn, &format!("SELECT count(*) FROM {FROM} WHERE {filter}"))? as u64;
        let windows = if total <= per {
            vec!["TRUE".to_string()]
        } else {
            Chunks::windows(conn, per, total)?
        };
        let mut chunks = Chunks {
            conn,
            select: format!("SELECT {select} FROM {FROM} WHERE ({filter})"),
            windows,
            next: 0,
            total,
            descending,
        };
        let first = match chunks.next_statement() {
            Some(stmt) => stmt.map_err(engine)?,
            None => {
                let mut stmt = conn.prepare(&format!("SELECT {select} FROM {FROM} WHERE FALSE")).map_err(engine)?;
                let _ = stmt.stream_arrow([]).map_err(engine)?;
                stmt
            }
        };
        let mut stream = EventStream::from_statement(first, schema, ctx, cq, exact, limit);
        stream.chunks = Some(chunks);
        Ok(stream)
    }

    pub(crate) fn controlled(mut self, control: &'c QueryControl) -> EventStream<'c> {
        self.control = Some(control);
        self
    }

    /// The next batch of the statement, or `None` at the end (or on an error, kept for `finish`).
    fn next_batch(&mut self) -> Option<EventBatch> {
        loop {
            if let Some(rb) = &self.chunk {
                if self.offset < rb.num_rows() {
                    let len = (rb.num_rows() - self.offset).min(BATCH_ROWS);
                    let slice = rb.slice(self.offset, len);
                    self.offset += len;
                    return match EventBatch::try_new(self.schema.clone(), slice) {
                        Ok(b) => Some(b),
                        Err(e) => {
                            self.error = Some(engine(e));
                            None
                        }
                    };
                }
            }
            if self.control.is_some_and(QueryControl::is_cancelled) {
                self.error = Some(StoreError::Cancelled);
                return None;
            }
            match self.stmt.step() {
                Ok(Some(chunk)) => {
                    self.chunk = Some(RecordBatch::from(&chunk as &StructArray));
                    self.offset = 0;
                }
                Ok(None) => match self.chunks.as_mut().and_then(Chunks::next_statement) {
                    Some(Ok(stmt)) => self.stmt = stmt,
                    Some(Err(e)) => {
                        self.error = Some(engine(e));
                        return None;
                    }
                    None => return None,
                },
                Err(e) => {
                    self.error = Some(match self.control {
                        Some(c) if c.is_cancelled() => StoreError::Cancelled,
                        _ => engine(e),
                    });
                    return None;
                }
            }
        }
    }

    /// Ends a windowed exact stream at its limit: the matching events not read are counted in SQL.
    fn stop_counting(&mut self) {
        self.done = true;
        if let Some(chunks) = self.chunks.take() {
            // The event that hit the limit is counted with the rest.
            self.matched -= 1;
            self.unread = chunks.total.saturating_sub(self.matched);
        }
    }

    /// Drains the rest (counting matches) and returns how much was analysed.
    pub(crate) fn finish(mut self) -> StoreResult<Coverage> {
        while self.next().is_some() {}
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        self.matched += self.unread;
        let mut limits = Vec::new();
        if self.matched > self.yielded {
            limits.push(format!(
                "analysed the first {} of {} matching events; narrow the query to see the rest",
                self.yielded, self.matched
            ));
        }
        Ok(Coverage {
            matched: self.matched,
            analysed: self.yielded,
            limits,
        })
    }
}

impl Iterator for EventStream<'_> {
    type Item = OwnedRow;

    fn next(&mut self) -> Option<OwnedRow> {
        if self.done {
            return None;
        }
        loop {
            if let Some((b, slots)) = &self.batch {
                while self.row < b.num_rows() {
                    let r = b.row(self.row);
                    self.row += 1;
                    let source = self.ctx.shared_names.get(&r.source().0).cloned().unwrap_or_else(|| Arc::from(""));
                    let row = ArrowRow {
                        row: r,
                        source: &source,
                        slots,
                    };
                    if !self.exact && !self.cq.matches_row(&row) {
                        continue;
                    }
                    self.matched += 1;
                    if self.exact && self.yielded == self.limit as u64 && self.chunks.is_some() {
                        // The rest of a windowed exact stream is counted, not read.
                        self.stop_counting();
                        return None;
                    }
                    if self.yielded < self.limit as u64 {
                        self.yielded += 1;
                        return Some(OwnedRow {
                            id: r.id().0,
                            rec: to_record(&r),
                            source,
                        });
                    }
                }
            }
            match self.next_batch() {
                Some(b) => {
                    let slots = if self.exact {
                        BatchSlots::default()
                    } else {
                        BatchSlots::resolve(self.cq.keys(), b.schema())
                    };
                    self.batch = Some((b, slots));
                    self.row = 0;
                }
                None => {
                    self.done = true;
                    return None;
                }
            }
        }
    }
}

/// FNV-1a, a stable hash of the canonical query text (cache keys must not change between builds).
pub(crate) fn fnv1a(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The canonical text of a query: its JSON with set-like lists sorted, so equal queries share a key.
fn canonical(q: &Filter) -> String {
    let mut q = q.clone();
    q.levels.sort();
    q.levels.dedup();
    q.sources.sort_unstable();
    q.sources.dedup();
    q.markers.sort();
    q.groups.sort();
    serde_json::to_string(&q).unwrap_or_default()
}

impl Inner {
    /// The data version (bumped by every change to the events, persisted, so cached results of an
    /// earlier session are never mistaken for current ones).
    pub(crate) fn data_version(&self, conn: &Connection) -> StoreResult<u64> {
        let v: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = 'data_version'", [], |r| r.get(0))
            .ok();
        Ok(v.and_then(|v| v.parse().ok()).unwrap_or(0))
    }

    /// The cached value of `kind` for the canonical text `key` in the current data version.
    pub(crate) fn cache_get(&self, conn: &Connection, kind: &str, key: &str) -> StoreResult<Option<String>> {
        let version = self.data_version(conn)?;
        let hit: Option<(String, String)> = conn
            .query_row(
                "SELECT query, value FROM analysis_cache WHERE version = ? AND kind = ? AND hash = ?",
                params![version, kind, fnv1a(key)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        Ok(hit.filter(|(query, _)| query == key).map(|(_, value)| value))
    }

    /// Stores `value` for `kind` and `key` in the current data version (not while an import runs; a
    /// failed write only costs a recomputation later).
    pub(crate) fn cache_put(&self, conn: &Connection, kind: &str, key: &str, value: &str) -> StoreResult<()> {
        if self.is_importing() {
            return Ok(());
        }
        let version = self.data_version(conn)?;
        if let Err(e) = conn.execute(
            "INSERT OR REPLACE INTO analysis_cache VALUES (?, ?, ?, ?, ?)",
            params![version, kind, fnv1a(key), key, value],
        ) {
            log::warn!("caching {kind} failed: {e}");
        }
        Ok(())
    }

    /// A cached analysis result, or `compute`'s (stored for next time).
    fn cached<T: Serialize + DeserializeOwned>(
        &self,
        kind: &str,
        q: &Filter,
        compute: impl FnOnce() -> StoreResult<Analyzed<T>>,
    ) -> StoreResult<Analyzed<T>> {
        let conn = self.conn()?;
        let text = canonical(q);
        if let Some(value) = self.cache_get(&conn, kind, &text)? {
            if let Ok((result, matched, analysed, limits)) = serde_json::from_str::<(T, u64, u64, Vec<String>)>(&value) {
                return Ok(Analyzed {
                    result,
                    coverage: Coverage { matched, analysed, limits },
                });
            }
        }
        let out = compute()?;
        let value = serde_json::to_string(&(&out.result, out.coverage.matched, out.coverage.analysed, &out.coverage.limits))
            .map_err(|e| StoreError::Engine(e.to_string()))?;
        self.cache_put(&conn, kind, &text, &value)?;
        Ok(out)
    }

    /// The select list of a read over `q`: `cols` plus the instance key's fields, everything when part of
    /// the filter is decided on the scan (it may need any field).
    pub(crate) fn select_list(&self, sf: &SqlFilter, cq: &CompiledQuery, cols: &Columns) -> String {
        if !sf.exact {
            return row_columns(&self.schema, &Columns::all());
        }
        let mut cols = cols.clone();
        if let Some(f) = &mut cols.fields {
            f.extend(cq.group_by.iter().cloned());
        }
        row_columns(&self.schema, &cols)
    }

    /// Streams the events matching `q` (and `read.extra`, an SQL condition) to `f`, in time order when
    /// `read.ordered`, at most [`MAX_ANALYSED`] of them, with the columns `read.columns`.
    fn analyse<T>(
        &self,
        q: &Filter,
        read: Read<'_>,
        f: impl FnOnce(&mut EventStream<'_>, &CompiledQuery) -> T,
    ) -> StoreResult<Analyzed<T>> {
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (sf, cq) = self.compile(&ctx, q)?;
        let select = self.select_list(&sf, &cq, &read.columns);
        let filter = format!("{} AND {}", sf.all_sql(), read.extra.unwrap_or("TRUE"));
        let mut stream = if read.ordered {
            EventStream::ordered(&conn, &select, &filter, self.schema.clone(), &ctx, &cq, sf.exact, read.limit)?
        } else {
            let sql = format!("SELECT {select} FROM {FROM} WHERE {filter}");
            EventStream::new(&conn, &sql, self.schema.clone(), &ctx, &cq, sf.exact, read.limit)?
        };
        let result = f(&mut stream, &cq);
        let coverage = stream.finish()?;
        Ok(Analyzed { result, coverage })
    }

    /// Writes every event matching the request's query, in time order (or newest first), with one of the
    /// registered exporters; streams (nothing is kept per event). With `redact`, every event passes the
    /// profile's redaction rules with a fresh salt.
    pub(crate) fn export(&self, req: &imt_core::api::ExportRequest) -> StoreResult<imt_core::api::ExportResult> {
        let exporter = self
            .registry()
            .exporter(&req.format)
            .cloned()
            .ok_or_else(|| StoreError::Invalid(format!("unknown export format \"{}\"", req.format)))?;
        let path = std::path::Path::new(&req.path);
        imt_core::overwrite::check_overwrite(path, req.overwrite).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let redactor = if req.redact {
            let r = imt_core::redact::Redactor::from_config(&self.profile().profile.redact, imt_core::redact::Salt::random())
                .map_err(|e| StoreError::Invalid(format!("the profile's redaction rules: {e:#}")))?;
            Some(r)
        } else {
            None
        };
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (sf, cq) = self.compile(&ctx, &req.query)?;
        let open = if req.descending {
            EventStream::ordered_desc
        } else {
            EventStream::ordered
        };
        let mut stream = open(
            &conn,
            &rows_sql(&self.schema),
            &sf.all_sql(),
            self.schema.clone(),
            &ctx,
            &cq,
            sf.exact,
            usize::MAX,
        )?;
        let file = std::fs::File::create(path).map_err(|e| StoreError::Io(format!("cannot create {}: {e}", req.path)))?;
        let mut out = std::io::BufWriter::new(file);
        let mut counts = imt_core::redact::Counts::new();
        let names = &ctx.names;
        let mut rows = stream.by_ref().map(|r| {
            let group = cq.group_key_of(&r);
            let source_name = names.get(&r.rec.source).map_or("", String::as_str);
            let record = match &redactor {
                Some(x) => std::borrow::Cow::Owned(x.record(&r.rec, &mut counts).into_owned()),
                None => std::borrow::Cow::Owned(r.rec),
            };
            imt_core::export::ExportRow {
                record,
                source_name,
                group,
            }
        });
        let written = exporter.export(&mut rows, &mut out);
        drop(rows);
        stream.finish()?;
        let events = written.map_err(|e| StoreError::Io(format!("cannot write {}: {e:#}", req.path)))?;
        std::io::Write::flush(&mut out).map_err(|e| StoreError::Io(format!("cannot write {}: {e}", req.path)))?;
        Ok(imt_core::api::ExportResult {
            format: exporter.id().into(),
            path: req.path.clone(),
            events,
            redactions: counts,
        })
    }

    /// Calls `f` with each event matching `q` in time order, at most `limit` of them.
    pub(crate) fn stream_rows(&self, q: &Filter, limit: usize, f: &mut dyn FnMut(OwnedRow)) -> StoreResult<Coverage> {
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (sf, cq) = self.compile(&ctx, q)?;
        let mut stream = EventStream::ordered(
            &conn,
            &rows_sql(&self.schema),
            &sf.all_sql(),
            self.schema.clone(),
            &ctx,
            &cq,
            sf.exact,
            limit,
        )?;
        for r in stream.by_ref() {
            f(r);
        }
        stream.finish()
    }

    fn patterns(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Pattern>>> {
        self.cached("patterns", &req.query, || {
            let mut overflow = 0;
            let mut out = self.analyse(&req.query, Read::ordered(Columns::only(true, Vec::<String>::new())), |rows, _| {
                let mut miner = PatternMiner::new(MinerConfig::default());
                for r in rows {
                    miner.add(&r);
                }
                overflow = miner.overflowed();
                miner.clusters()
            })?;
            if overflow > 0 {
                out.coverage.limits.push(format!(
                    "patterns: the first {MAX_CLUSTERS} distinct patterns are kept; {overflow} events are counted as other messages"
                ));
            }
            Ok(out)
        })
    }

    fn exception_groups(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<ExceptionGroup>>> {
        self.cached("exceptions", &req.query, || {
            let read = Read {
                extra: Some("e.exception IS NOT NULL"),
                ..Read::ordered(Columns::only(false, Vec::<String>::new()))
            };
            self.analyse(&req.query, read, |rows, cq| {
                exceptions::group(rows.map(|r| {
                    let key = cq.group_key_of(&r);
                    (r, key)
                }))
            })
        })
    }

    /// The SQL condition that selects exactly the events matching a compiled filter: the filter's own SQL when
    /// it is exact, else the ids of the events that pass the scan's check, written to a file (no limit, bounded
    /// memory). The returned files must be kept while the condition is used.
    fn exact_selection(
        &self,
        conn: &Connection,
        ctx: &Ctx,
        sf: &SqlFilter,
        cq: &CompiledQuery,
    ) -> StoreResult<(String, Vec<Arc<crate::hits::HitsFile>>)> {
        if sf.exact {
            return Ok((sf.all_sql(), sf.keep.clone()));
        }
        let sql = format!(
            "SELECT {} FROM {FROM} WHERE {}",
            self.select_list(sf, cq, &Columns::all()),
            sf.all_sql()
        );
        let mut stream = EventStream::new(conn, &sql, self.schema.clone(), ctx, cq, false, usize::MAX)?;
        let mut ids = self.hits.writer().map_err(|e| StoreError::Io(format!("{e:#}")))?;
        for r in stream.by_ref() {
            ids.push(r.id).map_err(|e| StoreError::Io(format!("{e:#}")))?;
        }
        stream.finish()?;
        let hits = ids.finish().map_err(|e| StoreError::Io(format!("{e:#}")))?;
        let (condition, file) = hits
            .condition()
            .ok_or_else(|| StoreError::Engine("the selected events' file has an unusable path".into()))?;
        Ok((condition, file.into_iter().chain(sf.keep.iter().cloned()).collect()))
    }

    /// Anomalies detected in SQL (see `crate::anomalies`): memory for per-instance aggregates, not per event,
    /// and every matching event analysed.
    fn anomalies_sql(&self, q: &Filter, cfg: &AnomalyConfig) -> StoreResult<Analyzed<Vec<Anomaly>>> {
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (sf, cq) = self.compile(&ctx, q)?;
        let ungroupable = || StoreError::Invalid("the instance fields cannot be grouped (a name contains a NUL character)".into());
        let key = group_key_expr(&cq.group_by, &ctx.fields).ok_or_else(ungroupable)?;
        // What the key is made of besides the source (its name): the fields' text.
        let parts: Vec<String> = cq
            .group_by
            .iter()
            .map(|f| text_expr(f, &ctx.fields))
            .collect::<Option<_>>()
            .ok_or_else(ungroupable)?;
        let (filter, _keep) = self.exact_selection(&conn, &ctx, &sf, &cq)?;
        let matched = count(&conn, &format!("SELECT count(*) FROM {FROM} WHERE {filter}"))? as u64;
        let mut inputs = crate::anomalies::SqlInputs::new(&conn, &filter, &key, parts, &ctx.fields);
        let result = analysis::detect_with(&mut inputs, cfg)?;
        Ok(Analyzed {
            result,
            coverage: Coverage {
                matched,
                analysed: matched,
                limits: Vec::new(),
            },
        })
    }

    fn anomalies(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Anomaly>>> {
        self.cached("anomalies", &req.query, || {
            self.anomalies_sql(&req.query, &AnomalyConfig::default())
        })
    }

    fn compare(&self, req: &CompareRequest) -> StoreResult<Analyzed<Comparison>> {
        // Two statements stream at once: one connection each.
        let (conn_a, conn_b) = (self.conn()?, self.conn()?);
        let ctx = self.ctx(&conn_a)?;
        let (fa, qa) = self.compile(&ctx, &req.a)?;
        let (fb, qb) = self.compile(&ctx, &req.b)?;
        // Every field is compared, the message is not; the order does not matter.
        let cols = Columns {
            message: false,
            fields: None,
        };
        let sql = |f: &SqlFilter, q: &CompiledQuery| format!("SELECT {} FROM {FROM} WHERE {}", self.select_list(f, q, &cols), f.all_sql());
        let mut a = EventStream::new(&conn_a, &sql(&fa, &qa), self.schema.clone(), &ctx, &qa, fa.exact, MAX_ANALYSED)?;
        let mut b = EventStream::new(&conn_b, &sql(&fb, &qb), self.schema.clone(), &ctx, &qb, fb.exact, MAX_ANALYSED)?;
        let result = analysis::compare_windows(&mut a, &mut b, &CompareConfig::default());
        let coverage = a.finish()?.and(b.finish()?);
        Ok(Analyzed { result, coverage })
    }

    fn correlate(&self, req: &CorrelateRequest) -> StoreResult<Analyzed<Vec<EventId>>> {
        let conn = self.conn()?;
        let mut keys = FieldKeys::new();
        let expr = Expr::TextEq {
            field: keys.key(&req.field),
            value: req.value.clone(),
            ignore_case: false,
        };
        let cond = expr_sql(&expr, &self.schema, &[])
            .ok_or_else(|| StoreError::Invalid("the field or value cannot be searched (it contains a NUL character)".into()))?;
        let matched = count(&conn, &format!("SELECT count(*) FROM {FROM} WHERE {cond}"))? as u64;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT e.id FROM {FROM} WHERE {cond} ORDER BY coalesce(e.ts, 9223372036854775807), e.id LIMIT {MAX_CORRELATED}"
            ))
            .map_err(engine)?;
        let ids = stmt
            .query_map([], |r| r.get::<_, u64>(0).map(EventId))
            .map_err(engine)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(engine)?;
        let mut limits = Vec::new();
        if matched > ids.len() as u64 {
            limits.push(format!("the first {MAX_CORRELATED} of {matched} events are listed"));
        }
        Ok(Analyzed {
            coverage: Coverage {
                matched,
                analysed: matched,
                limits,
            },
            result: ids,
        })
    }

    fn traces(&self, req: &TraceRequest) -> StoreResult<Analyzed<(Vec<TraceSummary>, Vec<WaterfallRow>)>> {
        let fields = analysis::trace::SPAN_INPUT_FIELDS.map(String::from);
        self.analyse(&req.query, Read::ordered(Columns::only(true, fields)), |rows, _| {
            let spans = analysis::spans(rows);
            if req.trace_id.is_empty() {
                (trace_summaries(&spans), Vec::new())
            } else {
                (Vec::new(), analysis::waterfall(&spans, &req.trace_id))
            }
        })
    }

    fn requests(&self, req: &RequestsRequest) -> StoreResult<Analyzed<Vec<RequestSpan>>> {
        let fields: Vec<String> = req.id_field.split('|').map(|f| f.trim().to_string()).collect();
        self.analyse(&req.query, Read::ordered(Columns::only(false, fields)), |rows, cq| {
            let mut out = analysis::request_durations(
                rows.map(|r| {
                    let key = cq.group_key_of(&r);
                    (r, key)
                }),
                &req.id_field,
            );
            analysis::trace::rank_requests(&mut out);
            out
        })
    }

    /// The metrics of the events of `q` as `MetricCollector` collects them, aggregated in SQL (only their
    /// averages are used: the Moment's window before): PerfMon counters per machine and instance, the other
    /// events' metric fields per instance.
    fn metric_averages<'f>(&self, q: &Filter, metrics: &'f [String]) -> StoreResult<Analyzed<MetricCollector<'f>>> {
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (sf, cq) = self.compile(&ctx, q)?;
        let (filter, _keep) = self.exact_selection(&conn, &ctx, &sf, &cq)?;
        let unreadable = |f: &str| StoreError::Invalid(format!("the field name {f:?} cannot be read"));
        let text = |f: &str| text_expr(f, &ctx.fields).ok_or_else(|| unreadable(f));
        let number = |f: &str| number_expr(f, &ctx.fields).ok_or_else(|| unreadable(f));
        let key = group_key_expr(&cq.group_by, &ctx.fields)
            .ok_or_else(|| StoreError::Invalid("the instance fields cannot be grouped (a name contains a NUL character)".into()))?;
        let (machine, instance, counter, value) = (text("MachineName")?, text("Instance")?, text("Counter")?, number("Value")?);
        let is_counter = format!("(({counter}) IS NOT NULL AND ({value}) IS NOT NULL)");
        let mut collector = MetricCollector::new(metrics);
        let mut add = |sql: &str| -> StoreResult<()> {
            let mut stmt = conn.prepare(sql).map_err(engine)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, u64>(2)?,
                        r.get::<_, f64>(3)?,
                    ))
                })
                .map_err(engine)?;
            for row in rows {
                let (series, metric, n, sum) = row.map_err(engine)?;
                collector.add_summary(series, metric, n as usize, sum);
            }
            Ok(())
        };
        add(&format!(
            "SELECT CASE WHEN coalesce({instance}, '') <> '' THEN concat(coalesce({machine}, ''), ' · ', {instance}) ELSE coalesce({machine}, '') END,
                    {counter}, count(*), sum({value})
             FROM {FROM} WHERE ({filter}) AND {is_counter} GROUP BY 1, 2"
        ))?;
        // Every metric field in one pass over the other events.
        if !metrics.is_empty() {
            let mut sums = Vec::new();
            for m in metrics {
                let v = number(m)?;
                sums.push(format!("count({v}), sum({v})"));
            }
            let sql = format!(
                "SELECT ({key}), {} FROM {FROM} WHERE ({filter}) AND NOT {is_counter} GROUP BY 1",
                sums.join(", ")
            );
            let mut stmt = conn.prepare(&sql).map_err(engine)?;
            let mut rows = stmt.query([]).map_err(engine)?;
            while let Some(r) = rows.next().map_err(engine)? {
                let series: String = r.get(0).map_err(engine)?;
                for (i, m) in metrics.iter().enumerate() {
                    let n: u64 = r.get(1 + 2 * i).map_err(engine)?;
                    let sum: Option<f64> = r.get(2 + 2 * i).map_err(engine)?;
                    collector.add_summary(series.clone(), m.clone(), n as usize, sum.unwrap_or(0.0));
                }
            }
        }
        let matched = count(&conn, &format!("SELECT count(*) FROM {FROM} WHERE {filter}"))? as u64;
        Ok(Analyzed {
            result: collector,
            coverage: Coverage {
                matched,
                analysed: matched,
                limits: Vec::new(),
            },
        })
    }

    fn moment(&self, req: &MomentRequest) -> StoreResult<Analyzed<MomentSummary>> {
        let query = self.resolve_window(&req.query)?;
        let (from, to) = (req.from.min(req.to), req.from.max(req.to));
        let [window, previous, all] = moment_queries(&query, from, to);
        let profile = self.profile();
        let cfg = &profile.profile.activity;
        let metric_fields = || {
            cfg.metrics
                .iter()
                .cloned()
                .chain(["Counter", "Value", "MachineName", "Instance"].map(String::from))
        };
        let fields = cfg.user.iter().chain(&cfg.action).cloned().chain(metric_fields());
        let read = Read {
            limit: MOMENT_EVENTS,
            ..Read::ordered(Columns::only(true, fields))
        };
        let builder = self.analyse(&window, read, |rows, cq| {
            let mut b = MomentBuilder::new(cfg, from);
            for r in rows {
                b.add(&r, &cq.group_key_of(&r));
            }
            b
        })?;
        let before = self.metric_averages(&previous, &cfg.metrics)?;
        // Detected over every event once per data version and query (cached), whatever the window.
        let anomalies = self.anomalies(&QueryRequest { query: all })?;
        let overlapping = anomalies.result.into_iter().filter(|a| a.from <= to && a.to >= from).collect();
        Ok(Analyzed {
            result: builder.result.finish(to, before.result, overlapping),
            coverage: builder.coverage.and(before.coverage).and(anomalies.coverage),
        })
    }

    fn overview(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Finding>>> {
        let anomalies = self.anomalies(req)?;
        let groups = self.exception_groups(req)?;
        let summary = self.summary()?;
        let quarantined = self
            .registry()
            .plugins()
            .into_iter()
            .filter(|p| p.status == imt_core::plugin::PluginStatus::Quarantined)
            .map(|p| (p.name, p.last_error.unwrap_or_default()))
            .collect();
        let inputs = OverviewInputs {
            sources: &summary.sources,
            unrecognized: &summary.unrecognized,
            quarantined,
        };
        Ok(Analyzed {
            result: findings(&anomalies.result, &groups.result, &inputs),
            coverage: anomalies.coverage.and(groups.coverage),
        })
    }

    /// Runs between lifecycle markers: every timed event, in time order, fed to the profile's span rules.
    fn runs(&self, req: &imt_core::spans::RunsRequest) -> StoreResult<imt_core::spans::RunsReply> {
        use imt_core::spans::book;
        let profile = self.profile();
        let rules = book::enabled_rules(&profile);
        let mut pairers = book::pairers(&rules);
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        if !rules.is_empty() {
            let cq = CompiledQuery::new(&Filter::default(), &ctx.default_group_by, &ctx.marker_kinds)
                .map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
            // The message (for regex rules and run texts), the per-field rules' fields and the lanes' fields.
            let per = rules.iter().filter_map(|(r, _)| match &r.per {
                imt_core::spans::Per::Field(f) => Some(f.clone()),
                imt_core::spans::Per::Instance => None,
            });
            let select = row_columns(&self.schema, &Columns::only(true, per.chain(ctx.default_group_by.iter().cloned())));
            let mut stream = EventStream::ordered(&conn, &select, "e.ts IS NOT NULL", self.schema.clone(), &ctx, &cq, true, usize::MAX)?;
            for r in stream.by_ref() {
                let lane = imt_core::query::group_key(&ctx.default_group_by, &r.rec, &r.source);
                book::feed_one(&r.rec, &r.source, &lane, &rules, &mut pairers);
            }
            stream.finish()?;
        }
        let anns = book::annotations(&rules, &pairers, &ctx.default_group_by);
        Ok(book::runs_reply(&profile, &rules, &pairers, &anns, req))
    }

    pub(crate) fn field_stats(&self, req: &FieldStatsRequest) -> StoreResult<FieldStats> {
        let buckets = req.buckets.clamp(1, 200);
        {
            let conn = self.conn()?;
            let ctx = self.ctx(&conn)?;
            let (sf, _) = self.compile(&ctx, &req.query)?;
            let exprs = (text_expr(&req.field, &ctx.fields), number_expr(&req.field, &ctx.fields));
            if let (true, (Some(t), Some(n))) = (sf.exact, exprs) {
                return self.field_stats_sql(&conn, &sf, &t, &n, buckets);
            }
        }
        Ok(self
            .analyse(
                &req.query,
                Read::unordered(Columns::only(req.field == "@message", [req.field.clone()])),
                |rows, _| analysis::field_stats(rows, &req.field, req.buckets.clamp(1, 200)),
            )?
            .result)
    }

    /// Field statistics as SQL aggregates (no row leaves the database): counts, the numeric summary with the
    /// engine's percentile definition (`quantile_cont` interpolates as `percentile_sorted` does), the top
    /// values and the histogram over the engine's bin layout. Means and deviations may differ from a
    /// sequential sum in the last digits.
    fn field_stats_sql(&self, conn: &Connection, sf: &SqlFilter, t: &str, n: &str, buckets: usize) -> StoreResult<FieldStats> {
        use imt_core::analysis::{HistogramBin, NumericStats, ValueCount};
        let base = format!("SELECT {t} AS t, {n} AS n FROM {FROM} WHERE {}", sf.all_sql());
        let row = conn
            .query_row(
                &format!(
                    "SELECT count(*), count(t), count(n) FILTER (WHERE t IS NOT NULL), min(n), max(n), avg(n), stddev_pop(n),
                            quantile_cont(n, 0.5), quantile_cont(n, 0.9), quantile_cont(n, 0.95), quantile_cont(n, 0.99)
                     FROM ({base})"
                ),
                [],
                |r| {
                    Ok((
                        r.get::<_, u64>(0)?,
                        r.get::<_, u64>(1)?,
                        r.get::<_, u64>(2)?,
                        r.get::<_, Option<f64>>(3)?,
                        r.get::<_, Option<f64>>(4)?,
                        r.get::<_, Option<f64>>(5)?,
                        r.get::<_, Option<f64>>(6)?,
                        [
                            r.get::<_, Option<f64>>(7)?,
                            r.get::<_, Option<f64>>(8)?,
                            r.get::<_, Option<f64>>(9)?,
                            r.get::<_, Option<f64>>(10)?,
                        ],
                    ))
                },
            )
            .map_err(engine)?;
        let (total, count, numbers, min, max, mean, stddev, q) = row;
        let mut top = Vec::new();
        let mut distinct = 0usize;
        {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT t, count(*) AS c, count(*) OVER () FROM ({base}) WHERE t IS NOT NULL GROUP BY t ORDER BY c DESC, t LIMIT {}",
                    imt_core::analysis::field_stats::TOP
                ))
                .map_err(engine)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?, r.get::<_, u64>(2)?)))
                .map_err(engine)?;
            for row in rows {
                let (value, c, d) = row.map_err(engine)?;
                distinct = (d as usize).min(imt_core::analysis::field_stats::MAX_DISTINCT);
                top.push(ValueCount { value, count: c });
            }
        }
        let numeric = match (count > 0 && numbers == count, min, max) {
            (true, Some(min), Some(max)) => {
                let histogram = match imt_core::analysis::field_stats::bin_layout(min, max, buckets) {
                    None => vec![HistogramBin {
                        from: min,
                        to: max,
                        count: numbers,
                    }],
                    Some((start, step, bins)) => {
                        let mut out: Vec<HistogramBin> = (0..bins)
                            .map(|i| HistogramBin {
                                from: start + i as f64 * step,
                                to: start + (i + 1) as f64 * step,
                                count: 0,
                            })
                            .collect();
                        let (s0, st) = (
                            crate::sql::double(start).unwrap_or_else(|| "0".into()),
                            crate::sql::double(step).unwrap_or_else(|| "1".into()),
                        );
                        let mut stmt = conn
                            .prepare(&format!(
                                "SELECT greatest(0, least(CAST(trunc((n - {s0}) / {st}) AS BIGINT), {})) AS i, count(*) FROM ({base}) WHERE n IS NOT NULL GROUP BY i",
                                bins - 1
                            ))
                            .map_err(engine)?;
                        let rows = stmt
                            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, u64>(1)?)))
                            .map_err(engine)?;
                        for row in rows {
                            let (i, c) = row.map_err(engine)?;
                            if let Some(b) = out.get_mut(i as usize) {
                                b.count += c;
                            }
                        }
                        out
                    }
                };
                Some(NumericStats {
                    min,
                    max,
                    mean: mean.unwrap_or(0.0),
                    stddev: stddev.unwrap_or(0.0),
                    p50: q[0].unwrap_or(0.0),
                    p90: q[1].unwrap_or(0.0),
                    p95: q[2].unwrap_or(0.0),
                    p99: q[3].unwrap_or(0.0),
                    histogram,
                })
            }
            _ => None,
        };
        Ok(FieldStats {
            count,
            missing: total - count,
            distinct,
            numeric,
            top,
        })
    }

    /// Streams a chart's events (`select` columns matching `filter`), in time order when `ordered`.
    #[allow(clippy::too_many_arguments)]
    fn chart_rows(
        &self,
        conn: &Connection,
        ctx: &Ctx,
        exact: bool,
        cq: &CompiledQuery,
        select: &str,
        filter: &str,
        ordered: bool,
        f: &mut dyn FnMut(OwnedRow),
    ) -> StoreResult<()> {
        let mut stream = if ordered {
            EventStream::ordered(conn, select, filter, self.schema.clone(), ctx, cq, exact, usize::MAX)?
        } else {
            EventStream::new(
                conn,
                &format!("SELECT {select} FROM {FROM} WHERE {filter}"),
                self.schema.clone(),
                ctx,
                cq,
                exact,
                usize::MAX,
            )?
        };
        for r in stream.by_ref() {
            f(r);
        }
        stream.finish()?;
        Ok(())
    }

    pub(crate) fn chart(&self, req: &ChartRequest) -> StoreResult<ChartData> {
        let profile = self.profile();
        let (def, when) = match (&req.def, &req.chart_id) {
            (Some(def), _) => (def.clone(), Vec::new()),
            (None, Some(id)) => {
                let defs = chart_defs(&profile.profile);
                let i = defs
                    .iter()
                    .position(|d| &d.id == id)
                    .ok_or_else(|| StoreError::NotFound(format!("chart \"{id}\"")))?;
                let when = compile_conditions(&profile.profile.chart[i].when).map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
                (defs[i].clone(), when)
            }
            (None, None) => return Err(StoreError::Invalid("a chart request needs chartId or def".into())),
        };
        let (agg, field) = parse_y(&def.y).map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (sf, cq) = self.compile(&ctx, &req.query)?;
        // The chart's own conditions: in SQL where they compile, else checked per event.
        let mut when_sql = vec!["e.ts IS NOT NULL".to_string()];
        let mut exact = sf.exact;
        for c in &when {
            match expr_sql(c.expr(), &ctx.fields, &cq.group_by) {
                Some(s) => when_sql.push(s),
                None => exact = false,
            }
        }
        let filter = format!("{} AND {}", sf.all_sql(), when_sql.join(" AND "));
        let keep = |r: &OwnedRow| when.iter().all(|c| c.eval_row(r));
        // Only the value, the split and the conditions' fields; in time order only for `last()`.
        let mut names: Vec<String> = def.split_by.iter().cloned().chain(field.clone()).collect();
        names.extend(when.iter().map(|c| c.check().field().to_string()));
        let cols = Columns::only(names.iter().any(|n| n == "@message"), names);
        let select = self.select_list(&sf, &cq, &cols);
        let ordered = agg == Agg::Last;
        let (from, to) = match (req.query.from, req.query.to) {
            (Some(f), Some(t)) => (f, t),
            (qf, qt) => {
                let (min, max) = if exact {
                    conn.query_row(&format!("SELECT min(e.ts), max(e.ts) FROM {FROM} WHERE {filter}"), [], |r| {
                        Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?))
                    })
                    .map_err(engine)?
                } else {
                    let sql = format!("SELECT {select} FROM {FROM} WHERE {filter}");
                    let mut stream = EventStream::new(&conn, &sql, self.schema.clone(), &ctx, &cq, sf.exact, usize::MAX)?;
                    let (mut min, mut max): (Option<i64>, Option<i64>) = (None, None);
                    for ts in stream.by_ref().filter(|r| keep(r)).filter_map(|r| r.ts()) {
                        min = Some(min.map_or(ts, |m| m.min(ts)));
                        max = Some(max.map_or(ts, |m| m.max(ts)));
                    }
                    stream.finish()?;
                    (min, max)
                };
                let from = qf.or(min).unwrap_or(0);
                (from, qt.or(max).unwrap_or(from))
            }
        };
        let buckets = Buckets::of(&def, from, to, req.buckets).map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
        let mut builder = SeriesBuilder::new(buckets);
        // count(), min() and max() of a filter SQL expresses are one GROUP BY (sums and averages stay in Rust,
        // so they add up exactly as the engine does).
        let key_sql = if def.split_by.is_empty() {
            lit(&def.title)
        } else {
            group_key_expr(&def.split_by, &ctx.fields)
        };
        let value_sql = match &field {
            None => Some("CAST(1 AS DOUBLE)".to_string()),
            Some(f) => number_expr(f, &ctx.fields),
        };
        if let (true, Agg::Count | Agg::Min | Agg::Max, Some(key_sql), Some(value_sql)) = (exact, agg, key_sql, value_sql) {
            let Buckets { start, bucket_ms, n } = buckets;
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT k, b, count(*), min(v), max(v) FROM (
                       SELECT {key_sql} AS k, (e.ts - {start}) // {bucket_ms} AS b, {value_sql} AS v FROM {FROM} WHERE {filter} AND e.ts >= {start})
                     WHERE v IS NOT NULL AND b < {n} GROUP BY k, b"
                ))
                .map_err(engine)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, u64>(2)?,
                        r.get::<_, f64>(3)?,
                        r.get::<_, f64>(4)?,
                    ))
                })
                .map_err(engine)?;
            for row in rows {
                let (k, b, count, min, max) = row.map_err(engine)?;
                let acc = imt_core::chart::Acc {
                    count,
                    sum: 0.0,
                    min,
                    max,
                    last: max,
                };
                builder.add_acc(b as usize, k, &acc);
            }
        } else {
            self.chart_rows(&conn, &ctx, sf.exact, &cq, &select, &filter, ordered, &mut |r| {
                if !exact && !keep(&r) {
                    return;
                }
                let Some(value) = chart_value(&r, field.as_deref()) else { return };
                let key = if def.split_by.is_empty() {
                    def.title.clone()
                } else {
                    imt_core::query::group_key(&def.split_by, &r.rec, &r.source)
                };
                builder.add(r.ts().unwrap_or_default(), value, key);
            })?;
        }
        let series = builder.finish(&def, agg);
        let fatal = ctx
            .marker_kinds
            .iter()
            .enumerate()
            .filter(|(_, k)| *k == "fatal")
            .fold(0u64, |m, (i, _)| m | (1 << i));
        let mask = (profile.lifecycle_mask | fatal) & ctx.visible;
        let mut lifecycle = Vec::new();
        if mask != 0 {
            let sql = format!(
                "SELECT {} FROM {FROM} WHERE {} AND (e.markers & {mask}::UBIGINT) <> 0 AND e.ts >= {from} AND e.ts <= {to} {}",
                self.select_list(&sf, &cq, &Columns::only(false, Vec::<String>::new())),
                sf.structural_sql(),
                order_by(false)
            );
            // Only the structural criteria apply to lifecycle markers.
            let mut stream = EventStream::new(&conn, &sql, self.schema.clone(), &ctx, &cq, true, usize::MAX)?;
            for r in stream.by_ref() {
                if !cq.structural_row(&r, None) {
                    continue;
                }
                let m = r.markers() & mask;
                lifecycle.push(LifecycleEvent {
                    kind: ctx.marker_kinds[m.trailing_zeros() as usize].clone(),
                    ts: r.ts().unwrap_or_default(),
                    group: group_key_row(cq.group_keys(), &r),
                    id: u32::try_from(r.id).unwrap_or(u32::MAX),
                });
                if lifecycle.len() >= MAX_LIFECYCLE {
                    break;
                }
            }
            drop(stream);
        }
        Ok(ChartData {
            def,
            from,
            to,
            bucket_ms: buckets.bucket_ms,
            series,
            lifecycle,
        })
    }
}

impl Analysis for DuckStore {
    fn patterns(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Pattern>>> {
        self.inner.patterns(req)
    }
    fn exception_groups(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<ExceptionGroup>>> {
        self.inner.exception_groups(req)
    }
    fn anomalies(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Anomaly>>> {
        self.inner.anomalies(req)
    }
    fn compare(&self, req: &CompareRequest) -> StoreResult<Analyzed<Comparison>> {
        self.inner.compare(req)
    }
    fn correlate(&self, req: &CorrelateRequest) -> StoreResult<Analyzed<Vec<EventId>>> {
        self.inner.correlate(req)
    }
    fn traces(&self, req: &TraceRequest) -> StoreResult<Analyzed<(Vec<TraceSummary>, Vec<WaterfallRow>)>> {
        self.inner.traces(req)
    }
    fn requests(&self, req: &RequestsRequest) -> StoreResult<Analyzed<Vec<RequestSpan>>> {
        self.inner.requests(req)
    }
    fn moment(&self, req: &MomentRequest) -> StoreResult<Analyzed<MomentSummary>> {
        self.inner.moment(req)
    }
    fn overview(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Finding>>> {
        self.inner.overview(req)
    }
    fn runs(&self, req: &imt_core::spans::RunsRequest) -> StoreResult<imt_core::spans::RunsReply> {
        self.inner.runs(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::ports::{Ingest, Query};

    /// Two CLEF files of `n` events each whose times interleave (a: even seconds, b: odd).
    fn interleaved(dir: &std::path::Path, n: usize) -> Vec<std::path::PathBuf> {
        ["a", "b"]
            .iter()
            .enumerate()
            .map(|(k, name)| {
                let text: String = (0..n)
                    .map(|i| {
                        let s = 2 * i + k;
                        // Only file a has a `Host`.
                        let host = if k == 0 {
                            format!(",\"Host\":\"h{}\"", i % 3)
                        } else {
                            String::new()
                        };
                        format!(
                            "{{\"@t\":\"2026-01-01T{:02}:{:02}:{:02}Z\",\"@mt\":\"event {s}\"{host}}}
",
                            s / 3600,
                            s / 60 % 60,
                            s % 60
                        )
                    })
                    .collect();
                let p = dir.join(format!("{name}.clef"));
                std::fs::write(&p, text).unwrap();
                p
            })
            .collect()
    }

    #[test]
    fn time_windows_cover_every_event_once() {
        assert_eq!(time_windows(&[], 10), vec!["e.ts IS NOT NULL", "e.ts IS NULL"]);
        // 6 + 5 > 10: a new window at minute 1; minute 2 (25 events) in three parts; minute 3 after it.
        let w = time_windows(&[(0, 6), (1, 5), (2, 25), (3, 1)], 10);
        assert_eq!(
            w,
            vec![
                "e.ts < 60000",
                "e.ts >= 60000 AND e.ts < 120000",
                "e.ts >= 120000 AND e.ts < 140000",
                "e.ts >= 140000 AND e.ts < 160000",
                "e.ts >= 160000 AND e.ts < 180000",
                "e.ts >= 180000",
                "e.ts IS NULL",
            ]
        );
    }

    #[test]
    fn ordered_streams_read_windows_in_time_order_and_count_the_rest() {
        let root = std::env::temp_dir().join(format!("imt-ordered-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("logs")).unwrap();
        let n = 3_000;
        let paths = interleaved(&root.join("logs"), n);
        let store = DuckStore::create(
            &root.join("store"),
            imt_core::Profile::default_profile(),
            imt_core::PluginRegistry::new(),
        )
        .unwrap();
        store.open(&paths).unwrap().wait().unwrap();
        let inner = &store.inner;
        let conn = inner.conn().unwrap();
        let ctx = inner.ctx(&conn).unwrap();
        let (sf, cq) = inner.compile(&ctx, &Filter::default()).unwrap();
        // A key only some segments' events have is still read from its promoted column.
        assert!(ctx.fields.promoted_text.contains("Host"), "{:?}", ctx.fields.promoted_text);
        let facet = store
            .facet(&imt_core::api::FacetRequest {
                query: Filter::default(),
                field: "Host".into(),
                limit: 10,
            })
            .unwrap();
        assert_eq!((facet.total, facet.distinct), (n, 3));
        let select = rows_sql(&inner.schema);
        let schema = || inner.schema.clone();
        // Exact: the first `limit` events, across windows of 500 events (about four minutes); the rest
        // only counted.
        let limit = 4_321;
        let mut stream = EventStream::windowed(&conn, &select, &sf.all_sql(), schema(), &ctx, &cq, true, limit, 500, false).unwrap();
        let messages: Vec<String> = stream.by_ref().map(|r| r.rec.message.clone()).collect();
        assert_eq!(messages.len(), limit);
        assert!(
            messages.iter().enumerate().all(|(i, m)| *m == format!("event {i}")),
            "time order across both files"
        );
        let coverage = stream.finish().unwrap();
        assert_eq!((coverage.matched, coverage.analysed), (2 * n as u64, limit as u64));
        assert_eq!(coverage.limits.len(), 1);
        // Exact under the limit: everything, nothing counted twice.
        let all = EventStream::windowed(&conn, &select, &sf.all_sql(), schema(), &ctx, &cq, true, usize::MAX, 500, false).unwrap();
        let coverage = all.finish().unwrap();
        assert_eq!((coverage.matched, coverage.analysed), (2 * n as u64, 2 * n as u64));
        // Newest first: the windows backwards, each sorted descending.
        let desc = EventStream::windowed(&conn, &select, &sf.all_sql(), schema(), &ctx, &cq, true, 3_000, 500, true).unwrap();
        let newest: Vec<String> = desc.map(|r| r.rec.message.clone()).collect();
        let total = 2 * n;
        assert!(
            newest.iter().enumerate().all(|(i, m)| *m == format!("event {}", total - 1 - i)),
            "descending across windows"
        );
        // Not exact: every candidate is read and checked by the engine (all match here). No match: empty.
        let stream = EventStream::ordered(&conn, &select, &sf.all_sql(), schema(), &ctx, &cq, false, 5).unwrap();
        let coverage = stream.finish().unwrap();
        assert_eq!((coverage.matched, coverage.analysed), (2 * n as u64, 5));
        let mut empty = EventStream::ordered(&conn, &select, "FALSE", schema(), &ctx, &cq, true, 5).unwrap();
        assert!(empty.next().is_none());
        assert_eq!(empty.finish().unwrap().matched, 0);
        drop(conn);
        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cache_keys_are_stable_and_canonical() {
        assert_eq!(fnv1a(""), "cbf29ce484222325");
        assert_eq!(fnv1a("a"), "af63dc4c8601ec8c");
        let a = Filter {
            levels: vec![imt_core::model::Level::Error, imt_core::model::Level::Warning],
            sources: vec![3, 1],
            ..Filter::default()
        };
        let b = Filter {
            levels: vec![imt_core::model::Level::Warning, imt_core::model::Level::Error],
            sources: vec![1, 3, 3],
            ..Filter::default()
        };
        assert_eq!(canonical(&a), canonical(&b));
        assert_ne!(canonical(&a), canonical(&Filter::default()));
    }
}
