//! The `Analysis` port and the analyses of the `Query` port (charts, field statistics): the engine's
//! pure analyses fed with events streamed from the database in time order ([`EventStream`]), so no
//! result needs every event in memory at once. Results of the costly analyses are cached in the
//! database per data version and canonical query (engine review finding 18).

use crate::read::{count, Ctx};
use crate::sql::{expr_sql, group_key_expr, lit, number_expr, order_by, row_columns, Columns, SqlFilter, FROM, ROWS};
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

/// How an analysis reads its events.
pub(crate) struct Read<'a> {
    /// An extra SQL condition.
    pub extra: Option<&'a str>,
    /// In time order (else in storage order: cheaper, for analyses that do not depend on order).
    pub ordered: bool,
    pub columns: Columns,
}

impl Read<'_> {
    fn ordered(columns: Columns) -> Read<'static> {
        Read {
            extra: None,
            ordered: true,
            columns,
        }
    }

    fn unordered(columns: Columns) -> Read<'static> {
        Read {
            extra: None,
            ordered: false,
            columns,
        }
    }
}

/// Most events a correlation lists.
const MAX_CORRELATED: usize = 10_000;

/// Lifecycle markers drawn on a chart.
const MAX_LIFECYCLE: usize = 500;

/// Matching events of one statement, in its order, one owned row at a time (the wide `raw` column is
/// not read). Rows past `limit` are counted but not yielded; an engine error or a cancel ends the
/// stream and is returned by [`EventStream::finish`].
pub(crate) struct EventStream<'c> {
    stmt: Statement<'c>,
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
        Ok(EventStream {
            stmt,
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
        })
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
                Ok(None) => return None,
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

    /// Drains the rest (counting matches) and returns how much was analysed.
    pub(crate) fn finish(mut self) -> StoreResult<Coverage> {
        while self.next().is_some() {}
        if let Some(e) = self.error.take() {
            return Err(e);
        }
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
fn fnv1a(text: &str) -> String {
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
    fn data_version(&self, conn: &Connection) -> StoreResult<u64> {
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
        let sql = format!(
            "SELECT {} FROM {FROM} WHERE {} AND {} {}",
            self.select_list(&sf, &cq, &read.columns),
            sf.all_sql(),
            read.extra.unwrap_or("TRUE"),
            if read.ordered { order_by(false) } else { String::new() }
        );
        let mut stream = EventStream::new(&conn, &sql, self.schema.clone(), &ctx, &cq, sf.exact, MAX_ANALYSED)?;
        let result = f(&mut stream, &cq);
        let coverage = stream.finish()?;
        Ok(Analyzed { result, coverage })
    }

    /// Calls `f` with each event matching `q` in time order, at most `limit` of them.
    pub(crate) fn stream_rows(&self, q: &Filter, limit: usize, f: &mut dyn FnMut(OwnedRow)) -> StoreResult<Coverage> {
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (sf, cq) = self.compile(&ctx, q)?;
        let sql = format!("SELECT {ROWS} FROM {FROM} WHERE {} {}", sf.all_sql(), order_by(false));
        let mut stream = EventStream::new(&conn, &sql, self.schema.clone(), &ctx, &cq, sf.exact, limit)?;
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

    fn anomalies(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Anomaly>>> {
        self.cached("anomalies", &req.query, || {
            let cfg = AnomalyConfig::default();
            let fields = cfg
                .latency_fields
                .iter()
                .chain(&cfg.growth.fields)
                .cloned()
                .chain(["Counter", "Value", "MachineName", "ProcessName", "Instance", "InstanceIndex"].map(String::from));
            // The detectors read the message of error events (crash triggers) and whether an instance's last
            // event says it stopped: other messages are not read.
            let cols = Columns {
                message_when: Some(
                    "e.level >= 4 OR contains(lower(e.message), 'stop') OR contains(lower(e.message), 'shutting down')".into(),
                ),
                ..Columns::only(true, fields)
            };
            self.analyse(&req.query, Read::ordered(cols), |rows, cq| {
                analysis::detect(
                    rows.map(|r| {
                        let key = cq.group_key_of(&r);
                        (r, key)
                    }),
                    &AnomalyConfig::default(),
                )
            })
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
            message_when: None,
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

    fn moment(&self, req: &MomentRequest) -> StoreResult<Analyzed<MomentSummary>> {
        if req.query.window.is_some() {
            return Err(StoreError::unsupported("live time windows", "6c"));
        }
        let (from, to) = (req.from.min(req.to), req.from.max(req.to));
        let [window, previous, all] = moment_queries(&req.query, from, to);
        let profile = self.profile();
        let cfg = &profile.profile.activity;
        let metric_fields = || {
            cfg.metrics
                .iter()
                .cloned()
                .chain(["Counter", "Value", "MachineName", "Instance"].map(String::from))
        };
        let fields = cfg.user.iter().chain(&cfg.action).cloned().chain(metric_fields());
        let builder = self.analyse(&window, Read::ordered(Columns::only(true, fields)), |rows, cq| {
            let mut b = MomentBuilder::new(cfg, from);
            for r in rows {
                b.add(&r, &cq.group_key_of(&r));
            }
            b
        })?;
        let before = self.analyse(&previous, Read::unordered(Columns::only(false, metric_fields())), |rows, cq| {
            let mut m = MetricCollector::new(&cfg.metrics);
            for r in rows {
                m.add(&r, &cq.group_key_of(&r));
            }
            m
        })?;
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
            let sql = format!("SELECT {select} FROM {FROM} WHERE e.ts IS NOT NULL {}", order_by(false));
            let mut stream = EventStream::new(&conn, &sql, self.schema.clone(), &ctx, &cq, true, usize::MAX)?;
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
        Ok(self
            .analyse(
                &req.query,
                Read::unordered(Columns::only(req.field == "@message", [req.field.clone()])),
                |rows, _| analysis::field_stats(rows, &req.field, req.buckets.clamp(1, 200)),
            )?
            .result)
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
        let order = if ordered { order_by(false) } else { String::new() };
        let sql = format!("SELECT {select} FROM {FROM} WHERE {filter} {order}");
        let mut stream = EventStream::new(conn, &sql, self.schema.clone(), ctx, cq, exact, usize::MAX)?;
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
        if req.query.window.is_some() {
            return Err(StoreError::unsupported("live time windows", "6c"));
        }
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (sf, cq) = self.compile(&ctx, &req.query)?;
        // The chart's own conditions: in SQL where they compile, else checked per event.
        let mut when_sql = vec!["e.ts IS NOT NULL".to_string()];
        let mut exact = sf.exact;
        for c in &when {
            match expr_sql(c.expr(), &self.schema, &cq.group_by) {
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
            group_key_expr(&def.split_by, &self.schema)
        };
        let value_sql = match &field {
            None => Some("CAST(1 AS DOUBLE)".to_string()),
            Some(f) => number_expr(f, &self.schema),
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
