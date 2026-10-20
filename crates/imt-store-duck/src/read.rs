//! The `Query` port: summary, page, timeline and facet as SQL where the filter compiles exactly, else a
//! scan of the SQL-prefiltered events in time order through the engine's `CompiledQuery`.

use crate::sql::{self, group_key_expr, order_by, text_expr, SqlFilter, FROM, ROWS, TIME_KEY};
use crate::{engine, DuckStore, Inner};
use duckdb::arrow::array::{BooleanArray, RecordBatch, StructArray};
use duckdb::arrow::compute::filter_record_batch;
use duckdb::Connection;
use imt_core::analysis::FieldStats;
use imt_core::api::{
    ChartData, ChartRequest, CompletionRequest, ContextRequest, ExportRequest, ExportResult, Facet, FacetRequest, FacetValue, FieldInfo,
    FieldStatsRequest, FileKind, Lane, MarkerCluster, QueryRequest, SourceContext, SourceInfo, Timeline, TimelineRequest, MAX_LANE_SOURCES,
};
use imt_core::batch::{BatchRow, EventBatch, EventSchema, BATCH_ROWS};
use imt_core::ingest::SourceReport;
use imt_core::model::Level;
use imt_core::ports::{EventView, Filter, PageQuery, PageReply, Query, StoreError, StoreResult, SummaryReply};
use imt_core::ql::{Completion, QlError, QlResult};
use imt_core::query::{group_key, CompiledQuery};
use imt_core::row::{to_record, ArrowRow, BatchSlots, FieldKeys, RowRef};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::Arc;

/// Distinct values counted per field (as the in-memory field index caps them).
const MAX_DISTINCT: usize = imt_core::analysis::field_stats::MAX_DISTINCT;

/// Most timeline lanes returned.
const MAX_LANES: usize = 500;

/// Field list and instance key, computed once per generation of the events.
pub(crate) struct Derived {
    fields: Vec<FieldInfo>,
    default_group_by: Vec<String>,
}

fn file_kind(s: &str) -> FileKind {
    match s {
        "Binary" => FileKind::Binary,
        "Compressed" => FileKind::Compressed,
        "ArchiveMember" => FileKind::ArchiveMember,
        _ => FileKind::Text,
    }
}

/// The summary's source entry of an import report.
pub(crate) fn report_source(r: &SourceReport) -> SourceInfo {
    SourceInfo {
        id: r.id.0,
        path: r.path.clone(),
        group: imt_core::workspace::group_of(&r.name),
        name: r.name.clone(),
        importer: r.importer.clone(),
        records: r.events,
        skipped: r.skipped,
        dropped: r.dropped,
        step_stats: r.step_stats.clone(),
        error: None,
        from: r.from,
        to: r.to,
        offset_ms: 0,
        file_kind: r.file_kind,
        timestamp_issue: None,
    }
}

/// What every read needs besides the filter.
pub(crate) struct Ctx {
    pub(crate) names: HashMap<u32, String>,
    /// The same names, shared by the rows streamed to analyses.
    pub(crate) shared_names: HashMap<u32, Arc<str>>,
    pub(crate) marker_kinds: Vec<String>,
    pub(crate) visible: u64,
    pub(crate) lifecycle: u64,
    pub(crate) default_group_by: Vec<String>,
}

impl Ctx {
    pub(crate) fn name(&self, source: u32) -> &str {
        self.names.get(&source).map_or("", String::as_str)
    }

    /// Calls `f` with each row of `batch` that matches `cq` (every row when the SQL was `exact`).
    pub(crate) fn each_match(&self, batch: &EventBatch, cq: &CompiledQuery, exact: bool, f: &mut dyn FnMut(&ArrowRow<'_>)) {
        let slots = if exact {
            BatchSlots::default()
        } else {
            BatchSlots::resolve(cq.keys(), batch.schema())
        };
        for r in batch.rows() {
            let row = ArrowRow {
                row: r,
                source: self.name(r.source().0),
                slots: &slots,
            };
            if exact || cq.matches_row(&row) {
                f(&row);
            }
        }
    }

    fn marker_names(&self, mask: u64) -> Vec<String> {
        let mask = mask & self.visible;
        (0..self.marker_kinds.len())
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| self.marker_kinds[i].clone())
            .collect()
    }
}

/// Runs `sql` (selecting `e.*`) and hands its rows to `f` as validated batches of at most
/// [`BATCH_ROWS`] events, streaming (the result is never materialised whole).
pub(crate) fn for_each_batch(
    conn: &Connection,
    sql: &str,
    schema: &Arc<EventSchema>,
    f: &mut dyn FnMut(EventBatch) -> ControlFlow<()>,
) -> StoreResult<()> {
    let mut stmt = conn.prepare(sql).map_err(engine)?;
    // Executes the query; the chunks are fetched below without the iterator, which panics on errors.
    let _ = stmt.stream_arrow([]).map_err(engine)?;
    while let Some(chunk) = stmt.step().map_err(engine)? {
        let rb = RecordBatch::from(&chunk as &StructArray);
        let mut offset = 0;
        while offset < rb.num_rows() {
            let len = (rb.num_rows() - offset).min(BATCH_ROWS);
            let batch = EventBatch::try_new(schema.clone(), rb.slice(offset, len)).map_err(engine)?;
            offset += len;
            if f(batch).is_break() {
                return Ok(());
            }
        }
    }
    Ok(())
}

pub(crate) fn count(conn: &Connection, sql: &str) -> StoreResult<usize> {
    conn.query_row(sql, [], |r| r.get::<_, u64>(0)).map(|n| n as usize).map_err(engine)
}

impl Inner {
    pub(crate) fn ctx(&self, conn: &Connection) -> StoreResult<Ctx> {
        let profile = self.profile();
        let mut names = HashMap::new();
        let mut stmt = conn.prepare("SELECT id, name FROM sources").map_err(engine)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))
            .map_err(engine)?;
        for row in rows {
            let (id, name) = row.map_err(engine)?;
            names.insert(id, name);
        }
        Ok(Ctx {
            shared_names: names.iter().map(|(k, v)| (*k, Arc::from(v.as_str()))).collect(),
            names,
            marker_kinds: profile.markers.iter().map(|m| m.kind.clone()).collect(),
            visible: self.registry().visible_markers(&profile.markers),
            lifecycle: profile.lifecycle_mask,
            default_group_by: self.derived(conn)?.default_group_by.clone(),
        })
    }

    /// The field list and default instance key of the current events (cached per generation).
    fn derived(&self, conn: &Connection) -> StoreResult<Arc<Derived>> {
        let generation = self.generation.load(std::sync::atomic::Ordering::SeqCst);
        if let Some((g, d)) = crate::lock(&self.derived).as_ref() {
            if *g == generation {
                return Ok(d.clone());
            }
        }
        let profile = self.profile();
        let isnum = |x: &str| format!("coalesce(isfinite(TRY_CAST(trim({x}) AS DOUBLE)), false)");
        let mut fields = Vec::new();
        let declared_names: HashSet<&str> = self.schema.declared().iter().map(|c| c.name.as_str()).collect();
        for c in self.schema.declared() {
            let Some(v) = text_expr(&c.name, &self.schema) else { continue };
            let typed = if c.ty == imt_core::batch::ColumnType::Utf8 {
                "FALSE".to_string()
            } else {
                format!("e.{} IS NOT NULL", sql::ident(&c.column))
            };
            let row = conn
                .query_row(
                    &format!(
                        "SELECT count(v), count(DISTINCT v), count(*) FILTER (WHERE v IS NOT NULL AND NOT t AND NOT {}) FROM (SELECT {v} AS v, {typed} AS t FROM events e)",
                        isnum("v")
                    ),
                    [],
                    |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?, r.get::<_, u64>(2)?)),
                )
                .map_err(engine)?;
            if row.0 > 0 {
                fields.push((c.name.clone(), row));
            }
        }
        let mut stmt = conn
            .prepare(&format!(
                "SELECT k, count(*), count(DISTINCT v), count(*) FILTER (WHERE NOT {}) FROM (SELECT unnest(map_keys(e.extra)) AS k, unnest(map_values(e.extra)) AS v FROM events e) GROUP BY k",
                isnum("v")
            ))
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    (r.get::<_, u64>(1)?, r.get::<_, u64>(2)?, r.get::<_, u64>(3)?),
                ))
            })
            .map_err(engine)?;
        for row in rows {
            let (k, stats) = row.map_err(engine)?;
            if !declared_names.contains(k.as_str()) {
                fields.push((k, stats));
            }
        }
        let derived: HashSet<&str> = profile
            .extractors
            .iter()
            .flat_map(|e| e.fields.iter().map(|(n, _)| n.as_str()))
            .chain(profile.pipeline.calc_fields())
            .collect();
        let mut infos: Vec<FieldInfo> = fields
            .into_iter()
            .map(|(name, (count, distinct, non_numeric))| FieldInfo {
                derived: derived.contains(name.as_str()),
                count: count as usize,
                distinct: (distinct as usize).min(MAX_DISTINCT),
                numeric: non_numeric == 0,
                name,
            })
            .collect();
        infos.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        let total = count(conn, "SELECT count(*) FROM events")?;
        let sources = count(conn, "SELECT count(*) FROM sources WHERE ready")?;
        let default_group_by = imt_core::query::default_group_by(&profile.profile.instance, &infos, total, sources);
        let d = Arc::new(Derived {
            fields: infos,
            default_group_by,
        });
        *crate::lock(&self.derived) = Some((generation, d.clone()));
        Ok(d)
    }

    pub(crate) fn compile(&self, ctx: &Ctx, q: &Filter) -> StoreResult<(SqlFilter, CompiledQuery)> {
        sql::compile(q, &ctx.default_group_by, &ctx.marker_kinds, &self.schema)
    }

    fn view(&self, ctx: &Ctx, row: &BatchRow<'_>, group_by: &[String]) -> EventView {
        let rec = to_record(row);
        let source_name = ctx.name(row.source().0).to_string();
        EventView {
            id: row.id(),
            ts: row.ts(),
            ts_state: row.ts_state(),
            level: rec.level,
            group: group_key(group_by, &rec, &source_name),
            markers: ctx.marker_names(rec.markers),
            message: rec.message,
            template: rec.template,
            exception: rec.exception,
            fields: rec.fields,
            source: row.source(),
            source_name,
            line: row.line(),
            byte_offset: row.byte_offset(),
        }
    }

    /// Ids of the matching events in time order (the scan fallback).
    fn matching_ids(&self, conn: &Connection, ctx: &Ctx, f: &SqlFilter, cq: &CompiledQuery) -> StoreResult<Vec<u64>> {
        let mut ids = Vec::new();
        let sql = format!("SELECT {ROWS} FROM {FROM} WHERE {} {}", f.all_sql(), order_by(false));
        for_each_batch(conn, &sql, &self.schema, &mut |batch| {
            ctx.each_match(&batch, cq, f.exact, &mut |row| ids.push(row.id()));
            ControlFlow::Continue(())
        })?;
        Ok(ids)
    }

    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (f, cq) = self.compile(&ctx, &req.query)?;
        let limit = req.rows();
        let display = |pos: usize, total: usize| if req.descending { total - 1 - pos } else { pos };
        if f.exact {
            let w = f.all_sql();
            let total = count(&conn, &format!("SELECT count(*) FROM {FROM} WHERE {w}"))?;
            let mut offset = req.offset;
            let mut anchor_index = None;
            if let Some(a) = req.anchor {
                let key: Option<(i64, u32, u32)> = conn
                    .query_row(
                        &format!("SELECT {TIME_KEY}, e.source, e.line FROM {FROM} WHERE {w} AND e.id = {}", a.0),
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .ok();
                if let Some((k, s, l)) = key {
                    let before = format!(
                        "{TIME_KEY} < {k} OR ({TIME_KEY} = {k} AND (e.source < {s} OR (e.source = {s} AND (e.line < {l} OR (e.line = {l} AND e.id < {})))))",
                        a.0
                    );
                    let pos = count(&conn, &format!("SELECT count(*) FROM {FROM} WHERE {w} AND ({before})"))?;
                    let idx = display(pos, total);
                    anchor_index = Some(idx);
                    offset = idx - idx % limit;
                }
            }
            let offset = offset.min(total);
            // Two phases: the page's ids by the sort key (narrow columns only), then their rows.
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT e.id FROM {FROM} WHERE {w} {} LIMIT {limit} OFFSET {offset}",
                    order_by(req.descending)
                ))
                .map_err(engine)?;
            let wanted = stmt
                .query_map([], |r| r.get::<_, u64>(0))
                .map_err(engine)?
                .collect::<Result<Vec<u64>, _>>()
                .map_err(engine)?;
            let rows = self.rows_by_id(&conn, &ctx, &wanted, &cq.group_by)?;
            return Ok(PageReply {
                total,
                offset,
                rows,
                anchor_index,
            });
        }
        let ids = self.matching_ids(&conn, &ctx, &f, &cq)?;
        let total = ids.len();
        let mut offset = req.offset;
        let mut anchor_index = None;
        if let Some(a) = req.anchor {
            if let Some(pos) = ids.iter().position(|id| *id == a.0) {
                let idx = display(pos, total);
                anchor_index = Some(idx);
                offset = idx - idx % limit;
            }
        }
        let offset = offset.min(total);
        let end = (offset + limit).min(total);
        let wanted: Vec<u64> = (offset..end).map(|i| ids[display(i, total)]).collect();
        let rows = self.rows_by_id(&conn, &ctx, &wanted, &cq.group_by)?;
        Ok(PageReply {
            total,
            offset,
            rows,
            anchor_index,
        })
    }

    /// The grid rows of `ids` (at most a page), in that order.
    fn rows_by_id(&self, conn: &Connection, ctx: &Ctx, ids: &[u64], group_by: &[String]) -> StoreResult<Vec<EventView>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let list: Vec<String> = ids.iter().map(u64::to_string).collect();
        let mut by_id: HashMap<u64, EventView> = HashMap::new();
        let sql = format!("SELECT {ROWS} FROM events e WHERE e.id IN ({})", list.join(", "));
        for_each_batch(conn, &sql, &self.schema, &mut |batch| {
            for r in batch.rows() {
                by_id.insert(r.id().0, self.view(ctx, &r, group_by));
            }
            ControlFlow::Continue(())
        })?;
        Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }

    pub(crate) fn summary(&self) -> StoreResult<SummaryReply> {
        let conn = self.conn()?;
        let derived = self.derived(&conn)?;
        let (total, untimed, from, to) = conn
            .query_row(
                "SELECT count(*), count(*) FILTER (WHERE ts IS NULL), min(ts), max(ts) FROM events",
                [],
                |r| {
                    Ok((
                        r.get::<_, u64>(0)?,
                        r.get::<_, u64>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .map_err(engine)?;
        let mut levels: BTreeMap<Level, usize> = Level::ALL.iter().map(|l| (*l, 0)).collect();
        let mut stmt = conn.prepare("SELECT level, count(*) FROM events GROUP BY level").map_err(engine)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, u8>(0)?, r.get::<_, u64>(1)?)))
            .map_err(engine)?;
        for row in rows {
            let (l, n) = row.map_err(engine)?;
            if let Some(level) = Level::ALL.get(l as usize) {
                levels.insert(*level, n as usize);
            }
        }
        let mut sources = Vec::new();
        let mut stmt = conn
            .prepare("SELECT id, path, name, importer, file_kind, events, skipped, dropped, ts_from, ts_to FROM sources WHERE ready ORDER BY name")
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                let name: String = r.get(2)?;
                Ok(SourceInfo {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    group: imt_core::workspace::group_of(&name),
                    name,
                    importer: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    file_kind: file_kind(&r.get::<_, Option<String>>(4)?.unwrap_or_default()),
                    records: r.get::<_, u64>(5)? as usize,
                    skipped: r.get::<_, u64>(6)? as usize,
                    dropped: r.get::<_, u64>(7)? as usize,
                    step_stats: Vec::new(),
                    error: None,
                    from: r.get(8)?,
                    to: r.get(9)?,
                    offset_ms: 0,
                    timestamp_issue: None,
                })
            })
            .map_err(engine)?;
        for row in rows {
            sources.push(row.map_err(engine)?);
        }
        let profile = self.profile();
        Ok(SummaryReply {
            total: total as usize,
            untimed: untimed as usize,
            from,
            to,
            levels,
            sources,
            unrecognized: self.unrecognized(),
            fields: derived.fields.clone(),
            default_group_by: derived.default_group_by.clone(),
            marker_kinds: profile.markers.iter().map(|m| m.kind.clone()).collect(),
            profile_name: profile.profile.name.clone(),
        })
    }

    fn facet(&self, req: &FacetRequest) -> StoreResult<Facet> {
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (f, cq) = self.compile(&ctx, &req.query)?;
        let limit = req.limit.clamp(1, 1000);
        let expr = if req.field == "@group" {
            group_key_expr(&cq.group_by, &self.schema)
        } else {
            text_expr(&req.field, &self.schema)
        };
        if let (true, Some(expr)) = (f.exact, expr) {
            let inner = format!(
                "SELECT {expr} AS v FROM events e JOIN sources s ON s.id = e.source WHERE {}",
                f.all_sql()
            );
            let (total, distinct) = conn
                .query_row(&format!("SELECT count(v), count(DISTINCT v) FROM ({inner})"), [], |r| {
                    Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?))
                })
                .map_err(engine)?;
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT v, count(*) AS n FROM ({inner}) WHERE v IS NOT NULL GROUP BY v ORDER BY n DESC, v LIMIT {limit}"
                ))
                .map_err(engine)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(FacetValue {
                        value: r.get(0)?,
                        count: r.get::<_, u64>(1)? as usize,
                    })
                })
                .map_err(engine)?;
            let values = rows.collect::<Result<Vec<_>, _>>().map_err(engine)?;
            return Ok(Facet {
                field: req.field.clone(),
                values,
                distinct: distinct as usize,
                total: total as usize,
            });
        }
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut total = 0;
        let sql = format!("SELECT {ROWS} FROM {FROM} WHERE {} {}", f.all_sql(), order_by(false));
        let key = FieldKeys::new().key(&req.field);
        for_each_batch(&conn, &sql, &self.schema, &mut |batch| {
            ctx.each_match(&batch, &cq, f.exact, &mut |row| {
                let value = if req.field == "@group" {
                    Some(cq.group_key_of(row))
                } else {
                    row.text(&key).map(|c| c.into_owned())
                };
                if let Some(v) = value {
                    total += 1;
                    *counts.entry(v).or_default() += 1;
                }
            });
            ControlFlow::Continue(())
        })?;
        let distinct = counts.len();
        let mut values: Vec<FacetValue> = counts.into_iter().map(|(value, count)| FacetValue { value, count }).collect();
        values.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
        values.truncate(limit);
        Ok(Facet {
            field: req.field.clone(),
            values,
            distinct,
            total,
        })
    }

    fn scan(&self, q: &Filter, visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (f, cq) = self.compile(&ctx, q)?;
        let sql = format!("SELECT {ROWS} FROM {FROM} WHERE {} {}", f.all_sql(), order_by(false));
        let mut failed = None;
        for_each_batch(&conn, &sql, &self.schema, &mut |batch| {
            if f.exact {
                return visit(&batch);
            }
            let mut keep = vec![false; batch.num_rows()];
            ctx.each_match(&batch, &cq, false, &mut |row| keep[row.row.index()] = true);
            let keep = BooleanArray::from(keep);
            if keep.true_count() == 0 {
                return ControlFlow::Continue(());
            }
            match filter_record_batch(batch.record_batch(), &keep)
                .map_err(engine)
                .and_then(|rb| EventBatch::try_new(batch.schema().clone(), rb).map_err(engine))
            {
                Ok(b) => visit(&b),
                Err(e) => {
                    failed = Some(e);
                    ControlFlow::Break(())
                }
            }
        })?;
        failed.map_or(Ok(()), Err)
    }
}

/// One timed event that belongs to a visible instance, as the timeline aggregates it.
struct Candidate {
    ts: i64,
    lane: usize,
    level: usize,
    matched: bool,
    mask: u64,
    source: u32,
    line: u32,
    id: u64,
    message: Option<String>,
}

/// Time order of an event: (ts, source, line, id).
type OrderKey = (i64, u32, u32, u64);

/// A marker cluster being collected, with the order key of its first event.
struct Cluster {
    first: OrderKey,
    cluster: MarkerCluster,
}

fn cluster_message(m: &str) -> String {
    m.chars().take(240).collect()
}

/// Lanes from their parts, sorted and truncated as the in-memory timeline does.
fn finish_lanes(mut lanes: Vec<Lane>, clusters: Vec<HashMap<(usize, usize), Cluster>>) -> (Vec<Lane>, usize) {
    for (lane, c) in lanes.iter_mut().zip(clusters) {
        lane.markers = c.into_values().map(|c| c.cluster).collect();
        lane.markers.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.kind.cmp(&b.kind)));
        lane.sources.sort_unstable();
    }
    lanes.sort_by(|a, b| a.first.cmp(&b.first).then_with(|| a.key.cmp(&b.key)));
    let truncated = lanes.len().saturating_sub(MAX_LANES);
    lanes.truncate(MAX_LANES);
    (lanes, truncated)
}

fn add_cluster(
    clusters: &mut HashMap<(usize, usize), Cluster>,
    kinds: &[String],
    mask: u64,
    bucket: usize,
    n: u32,
    first: OrderKey,
    message: &str,
) {
    let kind = mask.trailing_zeros() as usize;
    let Some(name) = kinds.get(kind) else { return };
    match clusters.get_mut(&(kind, bucket)) {
        Some(c) => {
            c.cluster.count += n;
            if first < c.first {
                c.first = first;
                c.cluster.ts = first.0;
                c.cluster.id = u32::try_from(first.3).unwrap_or(u32::MAX);
                c.cluster.message = cluster_message(message);
            }
        }
        None => {
            clusters.insert(
                (kind, bucket),
                Cluster {
                    first,
                    cluster: MarkerCluster {
                        kind: name.clone(),
                        bucket,
                        ts: first.0,
                        id: u32::try_from(first.3).unwrap_or(u32::MAX),
                        count: n,
                        message: cluster_message(message),
                    },
                },
            );
        }
    }
}

impl Inner {
    fn timeline(&self, req: &TimelineRequest) -> StoreResult<Timeline> {
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let (f, cq) = self.compile(&ctx, &req.query)?;
        let buckets = req.buckets.clamp(10, 2000);
        let lifecycle = if req.always_show_lifecycle {
            ctx.lifecycle & ctx.visible
        } else {
            0
        };
        match group_key_expr(&cq.group_by, &self.schema) {
            Some(key) if f.exact => self.timeline_sql(&conn, &ctx, &f, &key, req, buckets, lifecycle),
            _ => self.timeline_scan(&conn, &ctx, &f, &cq, req, buckets, lifecycle),
        }
    }

    fn bounds(req: &TimelineRequest, min: Option<i64>, max: Option<i64>, buckets: usize) -> (i64, i64, f64) {
        let (from, to) = match (req.query.from, req.query.to) {
            (Some(f), Some(t)) => (f, t),
            (f, t) => (f.or(min).unwrap_or(0), t.or(max).unwrap_or(0)),
        };
        let span = (to - from).max(1);
        (from, to, span as f64 / buckets as f64)
    }

    #[allow(clippy::too_many_arguments)]
    fn timeline_sql(
        &self,
        conn: &Connection,
        ctx: &Ctx,
        f: &SqlFilter,
        key: &str,
        req: &TimelineRequest,
        buckets: usize,
        lifecycle: u64,
    ) -> StoreResult<Timeline> {
        let base = format!(
            "SELECT {key} AS k, e.ts, e.level, e.source, e.line, e.id, e.markers, e.message, ({}) AS m FROM events e JOIN sources s ON s.id = e.source WHERE e.ts IS NOT NULL AND {}",
            f.content_sql(),
            f.structural_sql()
        );
        let (min, max) = conn
            .query_row(&format!("SELECT min(ts), max(ts) FROM ({base})"), [], |r| {
                Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?))
            })
            .map_err(engine)?;
        let (from, to, bucket_ms) = Self::bounds(req, min, max, buckets);
        let bucket = format!(
            "least(greatest(CAST(trunc(CAST(ts - {from} AS DOUBLE) / CAST('{bucket_ms:?}' AS DOUBLE)) AS BIGINT), 0), {})",
            buckets - 1
        );
        let mut lanes: Vec<Lane> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT k, min(ts), max(ts), count(*), count(*) FILTER (WHERE m) FROM ({base}) GROUP BY k"
            ))
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Lane {
                    key: r.get(0)?,
                    first: r.get(1)?,
                    last: r.get(2)?,
                    total: r.get::<_, u64>(3)? as usize,
                    matched: r.get::<_, u64>(4)? as usize,
                    density: vec![0; buckets],
                    markers: Vec::new(),
                    sources: Vec::new(),
                })
            })
            .map_err(engine)?;
        for lane in rows {
            let lane = lane.map_err(engine)?;
            index.insert(lane.key.clone(), lanes.len());
            lanes.push(lane);
        }
        let mut histogram = vec![[0u32; 6]; buckets];
        let mut stmt = conn
            .prepare(&format!(
                "SELECT k, {bucket} AS b, level, count(*) FROM ({base}) WHERE m GROUP BY ALL"
            ))
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, u8>(2)?, r.get::<_, u64>(3)?))
            })
            .map_err(engine)?;
        for row in rows {
            let (k, b, level, n) = row.map_err(engine)?;
            let b = (b.max(0) as usize).min(buckets - 1);
            histogram[b][(level as usize).min(5)] += n as u32;
            if let Some(&i) = index.get(&k) {
                lanes[i].density[b] += n as u32;
            }
        }
        // The first sources of each lane, in the order their first events appear.
        let mut firsts: Vec<Vec<(OrderKey, u32)>> = vec![Vec::new(); lanes.len()];
        let mut stmt = conn
            .prepare(&format!(
                "SELECT k, source, ts, line, id FROM (SELECT k, source, ts, line, id, row_number() OVER (PARTITION BY k, source ORDER BY ts, line, id) AS rn FROM ({base})) WHERE rn = 1"
            ))
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, u32>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            })
            .map_err(engine)?;
        for row in rows {
            let (k, source, ts, line, id) = row.map_err(engine)?;
            if let Some(&i) = index.get(&k) {
                firsts[i].push(((ts, source, line, id), source));
            }
        }
        for (lane, mut f) in lanes.iter_mut().zip(firsts) {
            f.sort_unstable();
            lane.sources = f.into_iter().take(MAX_LANE_SOURCES).map(|(_, s)| s).collect();
        }
        let mut clusters: Vec<HashMap<(usize, usize), Cluster>> = (0..lanes.len()).map(|_| HashMap::new()).collect();
        let visible = ctx.visible;
        let mut stmt = conn
            .prepare(&format!(
                "SELECT k, mask, b, n, ts, source, line, id, message FROM (
                   SELECT k, mask, b, ts, source, line, id, message, count(*) OVER (PARTITION BY k, mask, b) AS n,
                          row_number() OVER (PARTITION BY k, mask, b ORDER BY ts, source, line, id) AS rn
                   FROM (SELECT *, CASE WHEN m THEN markers & {visible}::UBIGINT ELSE markers & {lifecycle}::UBIGINT END AS mask, {bucket} AS b FROM ({base}))
                   WHERE mask <> 0)
                 WHERE rn = 1"
            ))
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, u64>(3)?,
                    (r.get::<_, i64>(4)?, r.get::<_, u32>(5)?, r.get::<_, u32>(6)?, r.get::<_, u64>(7)?),
                    r.get::<_, String>(8)?,
                ))
            })
            .map_err(engine)?;
        for row in rows {
            let (k, mask, b, n, first, message) = row.map_err(engine)?;
            if let Some(&i) = index.get(&k) {
                let b = (b.max(0) as usize).min(buckets - 1);
                add_cluster(&mut clusters[i], &ctx.marker_kinds, mask, b, n as u32, first, &message);
            }
        }
        let (lanes, truncated_lanes) = finish_lanes(lanes, clusters);
        Ok(Timeline {
            from,
            to,
            bucket_ms,
            histogram,
            lanes,
            truncated_lanes,
        })
    }

    /// The timeline by scanning the structurally prefiltered events (field filters, instance selection,
    /// regex text, query language), as the in-memory store computes it.
    #[allow(clippy::too_many_arguments)]
    fn timeline_scan(
        &self,
        conn: &Connection,
        ctx: &Ctx,
        f: &SqlFilter,
        cq: &CompiledQuery,
        req: &TimelineRequest,
        buckets: usize,
        lifecycle: u64,
    ) -> StoreResult<Timeline> {
        let mut keys: Vec<String> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut cands: Vec<Candidate> = Vec::new();
        let sql = format!(
            "SELECT {ROWS} FROM {FROM} WHERE e.ts IS NOT NULL AND {} {}",
            f.structural_sql(),
            order_by(false)
        );
        for_each_batch(conn, &sql, &self.schema, &mut |batch| {
            let slots = BatchSlots::resolve(cq.keys(), batch.schema());
            for r in batch.rows() {
                let row = ArrowRow {
                    row: r,
                    source: ctx.name(r.source().0),
                    slots: &slots,
                };
                let key = cq.group_key_of(&row);
                if !cq.structural_row(&row, Some(&key)) {
                    continue;
                }
                let lane = *index.entry(key).or_insert_with_key(|k| {
                    keys.push(k.clone());
                    keys.len() - 1
                });
                let matched = cq.content_row(&row);
                let mask = if matched {
                    r.markers() & ctx.visible
                } else {
                    r.markers() & lifecycle
                };
                cands.push(Candidate {
                    ts: r.ts().unwrap_or_default(),
                    lane,
                    level: row.level().index(),
                    matched,
                    mask,
                    source: r.source().0,
                    line: r.line(),
                    id: r.id().0,
                    message: (mask != 0).then(|| r.message().to_string()),
                });
            }
            ControlFlow::Continue(())
        })?;
        let (from, to, bucket_ms) = Self::bounds(req, cands.iter().map(|c| c.ts).min(), cands.iter().map(|c| c.ts).max(), buckets);
        let bucket_of = |ts: i64| (((ts - from) as f64 / bucket_ms) as usize).min(buckets - 1);
        let mut histogram = vec![[0u32; 6]; buckets];
        let mut lanes: Vec<Lane> = keys
            .into_iter()
            .map(|key| Lane {
                key,
                first: i64::MAX,
                last: i64::MIN,
                total: 0,
                matched: 0,
                density: vec![0; buckets],
                markers: Vec::new(),
                sources: Vec::new(),
            })
            .collect();
        let mut clusters: Vec<HashMap<(usize, usize), Cluster>> = (0..lanes.len()).map(|_| HashMap::new()).collect();
        for c in &cands {
            let lane = &mut lanes[c.lane];
            lane.first = lane.first.min(c.ts);
            lane.last = lane.last.max(c.ts);
            lane.total += 1;
            if lane.sources.len() < MAX_LANE_SOURCES && !lane.sources.contains(&c.source) {
                lane.sources.push(c.source);
            }
            let b = bucket_of(c.ts);
            if c.matched {
                lane.matched += 1;
                lane.density[b] += 1;
                histogram[b][c.level] += 1;
            }
            if c.mask != 0 {
                add_cluster(
                    &mut clusters[c.lane],
                    &ctx.marker_kinds,
                    c.mask,
                    b,
                    1,
                    (c.ts, c.source, c.line, c.id),
                    c.message.as_deref().unwrap_or(""),
                );
            }
        }
        let (lanes, truncated_lanes) = finish_lanes(lanes, clusters);
        Ok(Timeline {
            from,
            to,
            bucket_ms,
            histogram,
            lanes,
            truncated_lanes,
        })
    }
}

impl Query for DuckStore {
    fn summary(&self) -> StoreResult<SummaryReply> {
        self.inner.summary()
    }

    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        self.inner.page(req)
    }

    fn timeline(&self, req: &TimelineRequest) -> StoreResult<Timeline> {
        self.inner.timeline(req)
    }

    fn facet(&self, req: &FacetRequest) -> StoreResult<Facet> {
        self.inner.facet(req)
    }

    fn scan(&self, filter: &Filter, visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
        self.inner.scan(filter, visit)
    }

    fn chart(&self, req: &ChartRequest) -> StoreResult<ChartData> {
        self.inner.chart(req)
    }

    fn field_stats(&self, req: &FieldStatsRequest) -> StoreResult<FieldStats> {
        self.inner.field_stats(req)
    }

    fn source_context(&self, req: &ContextRequest) -> StoreResult<SourceContext> {
        self.inner.source_context(req)
    }

    fn run_query_with(&self, req: &QueryRequest, control: &imt_core::ql::QueryControl) -> StoreResult<QlResult> {
        self.inner.run_query_with(req, control)
    }

    fn check_query(&self, text: &str) -> StoreResult<Option<QlError>> {
        self.inner.check_query(text)
    }

    fn complete_query(&self, req: &CompletionRequest) -> StoreResult<Vec<Completion>> {
        self.inner.complete_query(req)
    }

    fn export(&self, _req: &ExportRequest) -> StoreResult<ExportResult> {
        Err(StoreError::unsupported("export", "R2c"))
    }
}
