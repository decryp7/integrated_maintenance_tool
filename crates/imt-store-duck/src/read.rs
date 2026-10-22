//! The `Query` port: summary, page, timeline and facet as SQL where the filter compiles exactly, else a
//! scan of the SQL-prefiltered events in time order through the engine's `CompiledQuery`.

use crate::sql::{self, group_key_expr, order_by, rows_sql, text_expr, Fields, SqlFilter, FROM, TIME_KEY};
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
use imt_core::ports::{EventView, Filter, PageQuery, PageReply, Query, StoreResult, SummaryReply};
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
        split_events: 0,
        step_stats: r.step_stats.clone(),
        error: None,
        from: r.from,
        to: r.to,
        offset_ms: 0,
        file_kind: r.file_kind,
        timestamp_issue: None,
        routed_from: None,
        problems: r.problems.clone(),
        identity: Vec::new(),
        parser: Some(r.parser.id.clone()),
    }
}

/// The import problems of each source (ADR 0020 §2: the Explorer lists them), from the diagnostics table, as the
/// in-memory store keeps them per source; kinds this build does not know are left out.
fn source_problems(conn: &Connection, sources: &mut [SourceInfo]) -> StoreResult<()> {
    use imt_core::ingest::{Problem, ProblemKind};
    let mut by_source: HashMap<u32, Vec<Problem>> = HashMap::new();
    let mut stmt = conn
        .prepare("SELECT source, kind, count, samples FROM diagnostics ORDER BY source, kind")
        .map_err(engine)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })
        .map_err(engine)?;
    for row in rows {
        let (source, kind, count, samples) = row.map_err(engine)?;
        let Some(kind) = ProblemKind::from_id(&kind) else { continue };
        let mut sample_lines: Vec<u32> = serde_json::from_str(&samples).unwrap_or_default();
        sample_lines.truncate(imt_core::ingest::MAX_SAMPLES);
        by_source.entry(source).or_default().push(Problem { kind, count, sample_lines });
    }
    for s in sources {
        if let Some(mut p) = by_source.remove(&s.id) {
            p.sort_by_key(|p| p.kind);
            s.problems = p;
        }
    }
    Ok(())
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
    /// Where fields are read from (declared and promoted columns).
    pub(crate) fields: Fields,
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
            fields: self.fields(conn)?,
        })
    }

    /// The schema and the keys every segment promotes to a text column.
    pub(crate) fn fields(&self, conn: &Connection) -> StoreResult<Fields> {
        use std::collections::BTreeSet;
        // Per segment file, the keys it promotes as text.
        let mut text: HashMap<String, BTreeSet<String>> = HashMap::new();
        let mut stmt = conn.prepare("SELECT file, promoted FROM segments").map_err(engine)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(engine)?;
        for row in rows {
            let (file, promoted) = row.map_err(engine)?;
            let p = crate::promote::Promotion::decode(&promoted);
            let keys = p
                .keys
                .into_iter()
                .filter(|(_, t)| *t == crate::promote::PType::Text)
                .map(|(k, _)| k)
                .collect();
            text.insert(file, keys);
        }
        // A key is read from its column when every segment that has the key promotes it as text (a
        // segment without the key reads the column as NULL, as the key is missing there).
        let mut promoted: BTreeSet<String> = text.values().flatten().cloned().collect();
        let mut stmt = conn.prepare("SELECT file, key FROM segment_fields").map_err(engine)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(engine)?;
        for row in rows {
            let (file, key) = row.map_err(engine)?;
            if promoted.contains(&key) && !text.get(&file).is_some_and(|t| t.contains(&key)) {
                promoted.remove(&key);
            }
        }
        Ok(Fields {
            schema: self.schema.clone(),
            promoted_text: promoted,
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
        // Computed once per data version and profile, and kept in the database (a reopened project does
        // not scan its fields again).
        let cache_key =
            serde_json::to_string(&(&profile.profile.instance, &profile.profile.extract, &profile.profile.pipeline)).unwrap_or_default();
        if let Some(value) = self.cache_get(conn, "fields", &cache_key)? {
            if let Ok((fields, default_group_by)) = serde_json::from_str::<(Vec<FieldInfo>, Vec<String>)>(&value) {
                let d = Arc::new(Derived { fields, default_group_by });
                *crate::lock(&self.derived) = Some((generation, d.clone()));
                return Ok(d);
            }
        }
        // From the per-segment field catalog (R2b): counts are exact; the distinct count is exact when every
        // segment has at most ten values (their top values are all listed).
        let mut acc: BTreeMap<String, (u64, u64, u64, HashSet<String>, bool)> = BTreeMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT key, rows, numeric_rows, distinct_est, top_values FROM segment_fields")
                .map_err(engine)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, u64>(1)?,
                        r.get::<_, u64>(2)?,
                        r.get::<_, u64>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })
                .map_err(engine)?;
            for row in rows {
                let (key, rows, numeric, distinct, top) = row.map_err(engine)?;
                let top: Vec<(String, u64)> = serde_json::from_str(&top).unwrap_or_default();
                let e = acc.entry(key).or_insert((0, 0, 0, HashSet::new(), true));
                e.0 += rows;
                e.1 += numeric;
                e.2 += distinct;
                e.4 &= distinct as usize <= top.len();
                e.3.extend(top.into_iter().map(|(v, _)| v));
            }
        }
        // Keys with more values than a segment lists: counted exactly in one statement while the store is
        // small, else estimated (the segments' estimates summed: high-cardinality values rarely repeat
        // across segments).
        let total = count(conn, "SELECT count(*) FROM events")?;
        let mut exact: HashMap<String, u64> = HashMap::new();
        if (total as u64) < self.preaggregate_above.load(std::sync::atomic::Ordering::Relaxed) {
            let fields = self.fields(conn)?;
            let unlisted: Vec<(&String, String)> = acc
                .iter()
                .filter(|(_, e)| !e.4)
                .filter_map(|(k, _)| text_expr(k, &fields).map(|x| (k, x)))
                .collect();
            // One key per statement: the distinct values of every key at once exceeded DuckDB's memory limit.
            for (k, x) in unlisted {
                let n = count(conn, &format!("SELECT count(DISTINCT {x}) FROM {FROM}"))?;
                exact.insert(k.clone(), n as u64);
            }
        }
        let fields: Vec<(String, (u64, u64, u64))> = acc
            .into_iter()
            .map(|(k, (rows, numeric, summed, values, listed))| {
                let distinct = match exact.get(&k) {
                    Some(n) => *n,
                    None if listed => values.len() as u64,
                    None => summed.min(rows).max(values.len() as u64),
                };
                (k, (rows, distinct, rows - numeric.min(rows)))
            })
            .collect();
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
        let default_group_by = self.instance_key(conn, &profile, &infos)?;
        let d = Arc::new(Derived {
            fields: infos,
            default_group_by,
        });
        if let Ok(value) = serde_json::to_string(&(&d.fields, &d.default_group_by)) {
            self.cache_put(conn, "fields", &cache_key, &value)?;
        }
        *crate::lock(&self.derived) = Some((generation, d.clone()));
        Ok(d)
    }

    /// The automatic instance key, its candidates checked per source as the workspace checks them
    /// (`imt_core::instance`, ADR 0020): one grouped count over the events of the candidate fields present.
    fn instance_key(
        &self,
        conn: &Connection,
        profile: &imt_core::profile::CompiledProfile,
        infos: &[FieldInfo],
    ) -> StoreResult<Vec<String>> {
        let cfg = &profile.profile.instance;
        if !cfg.key.is_empty() {
            return Ok(cfg.key.clone());
        }
        // The identity fields the sources' importers declare.
        let mut identity: Vec<String> = Vec::new();
        {
            let mut stmt = conn.prepare("SELECT DISTINCT importer FROM sources WHERE ready").map_err(engine)?;
            let ids = stmt.query_map([], |r| r.get::<_, Option<String>>(0)).map_err(engine)?;
            let importers = self.registry().importers();
            for id in ids {
                let Some(id) = id.map_err(engine)? else { continue };
                let declared = match importers.find(&id) {
                    Some(i) => i.identity_fields(),
                    None => importers
                        .binary_importers()
                        .find(|b| b.id() == id)
                        .map(|b| b.identity_fields())
                        .unwrap_or_default(),
                };
                for f in declared {
                    if !identity.contains(&f) {
                        identity.push(f);
                    }
                }
            }
            // And those of the sources' parsers (ADR 0026).
            let mut stmt = conn
                .prepare("SELECT DISTINCT parser_id FROM sources WHERE ready AND parser_id IS NOT NULL ORDER BY parser_id")
                .map_err(engine)?;
            let ids = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(engine)?;
            let set = self.parser_set();
            for id in ids {
                for f in set.identity_of(&id.map_err(engine)?) {
                    if !identity.contains(f) {
                        identity.push(f.clone());
                    }
                }
            }
        }
        let candidates = imt_core::instance::candidates(cfg, &identity);
        let fields = self.fields(conn)?;
        // Every field a candidate names (any case variant), at most 64.
        let counted: Vec<(&str, String)> = infos
            .iter()
            .filter(|f| candidates.iter().any(|c| c.eq_ignore_ascii_case(&f.name)))
            .filter_map(|f| text_expr(&f.name, &fields).map(|x| (f.name.as_str(), x)))
            .take(64)
            .collect();
        let mut events: Vec<usize> = Vec::new();
        let mut presence = Presence::default();
        if !counted.is_empty() {
            let select: String = counted.iter().map(|(_, x)| format!(", count(nullif({x}, ''))")).collect();
            let sql = format!("SELECT e.source, count(*){select} FROM {FROM} GROUP BY e.source ORDER BY e.source");
            let mut stmt = conn.prepare(&sql).map_err(engine)?;
            let mut rows = stmt.query([]).map_err(engine)?;
            while let Some(r) = rows.next().map_err(engine)? {
                let i = events.len();
                events.push(r.get::<_, u64>(1).map_err(engine)? as usize);
                for (k, (name, _)) in counted.iter().enumerate() {
                    let n = r.get::<_, u64>(k + 2).map_err(engine)? as usize;
                    presence.0.insert(((*name).to_string(), i), n);
                }
            }
        } else {
            events = vec![0; count(conn, "SELECT count(*) FROM sources WHERE ready")?];
        }
        Ok(imt_core::instance::instance_key(cfg, &identity, infos, &events, &presence))
    }

    pub(crate) fn compile(&self, ctx: &Ctx, q: &Filter) -> StoreResult<(SqlFilter, CompiledQuery)> {
        let resolved = self.resolve_window(q)?;
        let q = resolved.as_ref();
        let source = WithHits {
            fields: &ctx.fields,
            inner: self,
            keep: std::cell::RefCell::new(Vec::new()),
        };
        let (mut f, cq) = sql::compile(q, &ctx.default_group_by, &ctx.marker_kinds, &source)?;
        f.keep = source.keep.into_inner();
        Ok((f, cq))
    }

    /// The query with a live window ("the last 5 minutes") resolved into `from`: before the newest event, or
    /// the system clock with `[live] clock = "wall"`.
    pub(crate) fn resolve_window<'q>(&self, q: &'q Filter) -> StoreResult<std::borrow::Cow<'q, Filter>> {
        if q.window.is_none() {
            return Ok(std::borrow::Cow::Borrowed(q));
        }
        let now = if imt_core::query::live_clock_is_wall(&self.profile().profile) {
            imt_core::query::wall_ms()
        } else {
            let conn = self.conn()?;
            let newest: Option<i64> = conn.query_row("SELECT max(ts) FROM events", [], |r| r.get(0)).map_err(engine)?;
            newest.unwrap_or_else(imt_core::query::wall_ms)
        };
        Ok(imt_core::query::resolve_live_window(q, now))
    }

    /// The hits file of a word search, or `None` when the index cannot answer it exactly (unavailable, not
    /// caught up with the segments, or too many hits).
    fn word_hits(&self, words: &imt_core::words::WordQuery) -> anyhow::Result<Option<crate::hits::Hits>> {
        let Some(fts) = self.fts.as_ref() else { return Ok(None) };
        let conn = self.conn().map_err(|e| anyhow::anyhow!("{e}"))?;
        let pending: u64 = conn.query_row("SELECT count(*) FROM segments WHERE NOT indexed", [], |r| r.get(0))?;
        if pending > 0 {
            return Ok(None);
        }
        let index = &fts.shared.fts;
        let key = format!("{:?}|{:?}|{}", words.terms, words.prefixes, index.docs());
        let generation = self.generation.load(std::sync::atomic::Ordering::SeqCst);
        self.hits
            .get_or_write(&key, generation, || index.all_ids(words, crate::fts::MAX_FILTER_HITS))
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
            search_match: None,
        }
    }

    /// Ids of the matching events in time order (the scan fallback).
    fn matching_ids(&self, conn: &Connection, ctx: &Ctx, f: &SqlFilter, cq: &CompiledQuery) -> StoreResult<Vec<u64>> {
        let mut ids = Vec::new();
        let sql = format!(
            "SELECT {} FROM {FROM} WHERE {} {}",
            rows_sql(&self.schema),
            f.all_sql(),
            order_by(false)
        );
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
        // No criteria: the time-order positions give the page directly (no sort).
        if f.exact && f.structural.is_empty() && f.content.is_empty() && !self.is_importing() {
            let total = count(&conn, "SELECT count(*) FROM ord")?;
            let mut offset = req.offset;
            let mut anchor_index = None;
            if let Some(a) = req.anchor {
                let pos: Option<u64> = conn
                    .query_row("SELECT pos FROM ord WHERE id = ?", duckdb::params![a.0], |r| r.get(0))
                    .ok();
                if let Some(pos) = pos {
                    let idx = display(pos as usize, total);
                    anchor_index = Some(idx);
                    offset = idx - idx % limit;
                }
            }
            let offset = offset.min(total);
            let end = (offset + limit).min(total);
            let (lo, hi) = if req.descending {
                (total - end, total - offset)
            } else {
                (offset, end)
            };
            let mut stmt = conn
                .prepare("SELECT id FROM ord WHERE pos >= ? AND pos < ? ORDER BY pos")
                .map_err(engine)?;
            let mut wanted = stmt
                .query_map(duckdb::params![lo as u64, hi as u64], |r| r.get::<_, u64>(0))
                .map_err(engine)?
                .collect::<Result<Vec<u64>, _>>()
                .map_err(engine)?;
            if req.descending {
                wanted.reverse();
            }
            let rows = self.rows_by_id(&conn, &ctx, &wanted, &cq.group_by)?;
            return Ok(PageReply {
                total,
                offset,
                rows,
                anchor_index,
            });
        }
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

    /// [`Query::events`]: the grid rows of the ids, in the order asked.
    pub(crate) fn events(&self, req: &imt_core::ports::EventsRequest) -> StoreResult<Vec<EventView>> {
        if req.ids.len() > imt_core::ports::MAX_PAGE_ROWS {
            return Err(imt_core::ports::StoreError::Invalid(format!(
                "at most {} events at once",
                imt_core::ports::MAX_PAGE_ROWS
            )));
        }
        let conn = self.conn()?;
        let ctx = self.ctx(&conn)?;
        let q = Filter {
            group_by: req.group_by.clone(),
            ..Filter::default()
        };
        let (_, cq) = self.compile(&ctx, &q)?;
        let ids: Vec<u64> = req.ids.iter().map(|i| i.0).collect();
        self.rows_by_id(&conn, &ctx, &ids, &cq.group_by)
    }

    /// The grid rows of `ids` (at most a page), in that order.
    fn rows_by_id(&self, conn: &Connection, ctx: &Ctx, ids: &[u64], group_by: &[String]) -> StoreResult<Vec<EventView>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let list: Vec<String> = ids.iter().map(u64::to_string).collect();
        let mut by_id: HashMap<u64, EventView> = HashMap::new();
        // Only the segments whose id blocks hold the ids are read.
        let mut sorted: Vec<u64> = ids.to_vec();
        sorted.sort_unstable();
        let seg_dir = self.dir.join(crate::segments::SEGMENTS_DIR);
        let mut files = Vec::new();
        {
            let mut stmt = conn.prepare("SELECT file, first_id, end_id FROM segments").map_err(engine)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?, r.get::<_, u64>(2)?)))
                .map_err(engine)?;
            for row in rows {
                let (file, first, end) = row.map_err(engine)?;
                let at = sorted.partition_point(|i| *i < first);
                if sorted.get(at).is_some_and(|i| *i < end) {
                    files.push(seg_dir.join(file).to_string_lossy().into_owned());
                }
            }
        }
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT {} FROM {} e WHERE e.id IN ({}) AND {}",
            rows_sql(&self.schema),
            crate::schema::parquet_from(&files),
            list.join(", "),
            crate::schema::READY
        );
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
            .prepare("SELECT id, path, name, importer, file_kind, events, skipped, dropped, ts_from, ts_to, parser_id, coalesce((SELECT offset_ms FROM clock_offsets c WHERE c.path = sources.path), 0) FROM sources WHERE ready ORDER BY name")
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
                    split_events: 0,
                    step_stats: Vec::new(),
                    error: None,
                    from: r.get(8)?,
                    to: r.get(9)?,
                    offset_ms: r.get(11)?,
                    timestamp_issue: None,
                    routed_from: None,
                    problems: Vec::new(),
                    identity: Vec::new(),
                    parser: r.get(10)?,
                })
            })
            .map_err(engine)?;
        for row in rows {
            sources.push(row.map_err(engine)?);
        }
        source_problems(&conn, &mut sources)?;
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
            group_key_expr(&cq.group_by, &ctx.fields)
        } else {
            text_expr(&req.field, &ctx.fields)
        };
        if let (true, Some(expr)) = (f.exact, expr) {
            let inner = format!("SELECT {expr} AS v FROM {FROM} WHERE {}", f.all_sql());
            // One scan: the groups, with the totals over all groups alongside the top ones.
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT v, n, CAST(sum(n) OVER () AS UBIGINT) AS total, count(*) OVER () AS distinct_values FROM (SELECT v, count(*) AS n FROM ({inner}) WHERE v IS NOT NULL GROUP BY v) ORDER BY n DESC, v LIMIT {limit}"
                ))
                .map_err(engine)?;
            let (mut total, mut distinct) = (0u64, 0u64);
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        FacetValue {
                            value: r.get(0)?,
                            count: r.get::<_, u64>(1)? as usize,
                        },
                        r.get::<_, u64>(2)?,
                        r.get::<_, u64>(3)?,
                    ))
                })
                .map_err(engine)?;
            let mut values = Vec::new();
            for row in rows {
                let (v, t, d) = row.map_err(engine)?;
                (total, distinct) = (t, d);
                values.push(v);
            }
            return Ok(Facet {
                field: req.field.clone(),
                values,
                distinct: distinct as usize,
                total: total as usize,
            });
        }
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut total = 0;
        // Counts only: no order needed.
        let sql = format!("SELECT {} FROM {FROM} WHERE {}", rows_sql(&self.schema), f.all_sql());
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
        let sql = format!(
            "SELECT {} FROM {FROM} WHERE {} {}",
            rows_sql(&self.schema),
            f.all_sql(),
            order_by(false)
        );
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
        match group_key_expr(&cq.group_by, &ctx.fields) {
            Some(key) if f.exact => {
                if let Some(t) = self.timeline_histogram(&conn, &ctx, &cq, &key, req, buckets, lifecycle)? {
                    return Ok(t);
                }
                self.timeline_sql(&conn, &ctx, &f, &key, req, buckets, lifecycle)
            }
            _ => self.timeline_scan(&conn, &ctx, &f, &cq, req, buckets, lifecycle),
        }
    }

    /// The instance key of each source, when every timed event of a source has the same one (cached per data
    /// version and instance key); `None` when some source has several (then lanes need the events).
    fn source_keys(&self, conn: &Connection, key: &str) -> StoreResult<Option<HashMap<u32, String>>> {
        if let Some(v) = self.cache_get(conn, "source-keys", key)? {
            if let Ok(m) = serde_json::from_str::<Option<HashMap<u32, String>>>(&v) {
                return Ok(m);
            }
        }
        let mut stmt = conn
            .prepare(&format!(
                "SELECT e.source, {key} AS k FROM {FROM} WHERE e.ts IS NOT NULL GROUP BY ALL"
            ))
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))
            .map_err(engine)?;
        let mut out: Option<HashMap<u32, String>> = Some(HashMap::new());
        for row in rows {
            let (source, k) = row.map_err(engine)?;
            if let Some(m) = &mut out {
                if m.insert(source, k).is_some() {
                    out = None;
                }
            }
        }
        if let Ok(v) = serde_json::to_string(&out) {
            self.cache_put(conn, "source-keys", key, &v)?;
        }
        Ok(out)
    }

    /// The timeline from the per-segment minute × level × source histogram (R2b), for large stores when
    /// buckets are a minute or longer, the query only selects sources, a time range and levels, and each
    /// source belongs to one lane. Counts are those of whole minutes (a bucket edge inside a minute
    /// takes the whole minute), lane bounds are minutes. `None` when it does not apply.
    #[allow(clippy::too_many_arguments)]
    fn timeline_histogram(
        &self,
        conn: &Connection,
        ctx: &Ctx,
        cq: &CompiledQuery,
        key: &str,
        req: &TimelineRequest,
        buckets: usize,
        lifecycle: u64,
    ) -> StoreResult<Option<Timeline>> {
        let positioned = count(conn, "SELECT count(*) FROM ord")? as u64;
        if positioned < self.preaggregate_above.load(std::sync::atomic::Ordering::Relaxed) || self.is_importing() {
            return Ok(None);
        }
        let mut sources: Option<Vec<u32>> = None;
        let mut levels: u8 = 0x3f;
        let mut range: (Option<i64>, Option<i64>) = (None, None);
        for e in cq.structural_exprs() {
            match e {
                imt_core::ql::Expr::Sources(set) => sources = Some(set.iter().copied().collect()),
                imt_core::ql::Expr::Time { from, to } => range = (*from, *to),
                _ => return Ok(None),
            }
        }
        for e in cq.content_exprs() {
            match e {
                imt_core::ql::Expr::Levels(mask) => levels &= *mask,
                _ => return Ok(None),
            }
        }
        // Bounds as the event query computes them: the sources' first and last timed events.
        let source_filter = sources
            .as_ref()
            .map(|s| format!(" AND id IN ({})", s.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")))
            .unwrap_or_default();
        let (min, max): (Option<i64>, Option<i64>) = conn
            .query_row(
                &format!("SELECT min(ts_from), max(ts_to) FROM sources WHERE ready{source_filter}"),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(engine)?;
        let (from, to, bucket_ms) = Self::bounds(req, min.or(range.0), max.or(range.1), buckets);
        // Each source in one lane: the minute histogram mapped to lanes. Otherwise the per-lane counts (lanes ×
        // time units, and lanes × sources), materialised once per data version and instance key; they know no
        // sources per unit, so a source filter needs the events. A bucket counts whole units.
        let lane_of = self.source_keys(conn, key)?;
        let materialised = match &lane_of {
            Some(_) => None,
            None if sources.is_some() => return Ok(None),
            None => match self.lane_counts(conn, key)? {
                Some(m) => Some(m),
                None => return Ok(None),
            },
        };
        let unit = materialised.as_ref().map_or(60_000, |m| m.unit_ms);
        if bucket_ms < unit as f64 {
            return Ok(None);
        }
        let column = if materialised.is_some() { "h.unit" } else { "h.minute" };
        let mut conds = vec![format!("{column} IS NOT NULL")];
        if materialised.is_none() {
            conds.push("h.source IN (SELECT id FROM sources WHERE ready)".to_string());
        }
        if let Some(s) = &sources {
            conds.push(format!(
                "h.source IN ({})",
                s.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
            ));
        }
        if let Some(f) = range.0 {
            conds.push(format!("{column} >= {}", f.div_euclid(unit)));
        }
        if let Some(t) = range.1 {
            conds.push(format!("{column} <= {}", t.div_euclid(unit)));
        }
        let sql = match &materialised {
            None => format!(
                "SELECT NULL::VARCHAR, h.source, h.minute, h.level, sum(h.n) FROM histogram h WHERE {} GROUP BY ALL",
                conds.join(" AND ")
            ),
            Some(m) => format!(
                "SELECT h.k, NULL::UINTEGER, h.unit, h.level, sum(h.n) FROM lane_units h WHERE {} AND {} GROUP BY ALL",
                m.condition(),
                conds.join(" AND ")
            ),
        };
        let mut stmt = conn.prepare(&sql).map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<u32>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, u8>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            })
            .map_err(engine)?;
        let mut lanes: Vec<Lane> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut histogram = vec![[0u32; 6]; buckets];
        let mut firsts: HashMap<(usize, u32), i64> = HashMap::new();
        for row in rows {
            let (lane_key, source, minute, level, n) = row.map_err(engine)?;
            let Some(k) = lane_key
                .as_ref()
                .or_else(|| lane_of.as_ref().zip(source).and_then(|(l, s)| l.get(&s)))
            else {
                continue;
            };
            let i = *index.entry(k.clone()).or_insert_with_key(|k| {
                lanes.push(Lane {
                    key: k.clone(),
                    first: i64::MAX,
                    last: i64::MIN,
                    total: 0,
                    matched: 0,
                    density: vec![0; buckets],
                    markers: Vec::new(),
                    sources: Vec::new(),
                });
                lanes.len() - 1
            });
            let start = minute * unit;
            let lane = &mut lanes[i];
            lane.first = lane.first.min(start.max(from));
            lane.last = lane.last.max((start + unit - 1).min(to));
            lane.total += n as usize;
            let b = ((((start - from) as f64) / bucket_ms).floor().max(0.0) as usize).min(buckets - 1);
            if levels & (1 << level.min(5)) != 0 {
                lane.matched += n as usize;
                lane.density[b] += n as u32;
                histogram[b][(level as usize).min(5)] += n as u32;
            }
            if let Some(source) = source {
                firsts.entry((i, source)).and_modify(|m| *m = (*m).min(start)).or_insert(start);
            }
        }
        // The materialised lanes' sources: those with events in the time range (by their bounds there).
        if let Some(m) = &materialised {
            let mut overlap = Vec::new();
            if let Some(f) = range.0 {
                overlap.push(format!("tmax >= {f}"));
            }
            if let Some(t) = range.1 {
                overlap.push(format!("tmin <= {t}"));
            }
            overlap.push(m.condition());
            let mut stmt = conn
                .prepare(&format!("SELECT k, source, tmin FROM lane_sources WHERE {}", overlap.join(" AND ")))
                .map_err(engine)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?, r.get::<_, i64>(2)?)))
                .map_err(engine)?;
            for row in rows {
                let (k, source, first) = row.map_err(engine)?;
                if let Some(&i) = index.get(&k) {
                    firsts.insert((i, source), first);
                }
            }
        }
        // Without a time range a lane spans its sources' first and last timed events, which are known exactly.
        if range == (None, None) {
            // Per source, or per lane and source when sources have several lanes.
            let mut bounds: HashMap<(Option<usize>, u32), (i64, i64)> = HashMap::new();
            let sql = match &materialised {
                None => "SELECT NULL::VARCHAR, id, ts_from, ts_to FROM sources WHERE ready AND ts_from IS NOT NULL AND ts_to IS NOT NULL"
                    .to_string(),
                Some(m) => format!("SELECT k, source, tmin, tmax FROM lane_sources WHERE {}", m.condition()),
            };
            let mut stmt = conn.prepare(&sql).map_err(engine)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, u32>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .map_err(engine)?;
            for row in rows {
                let (k, id, a, b) = row.map_err(engine)?;
                let lane = match k {
                    Some(k) => match index.get(&k) {
                        Some(i) => Some(*i),
                        None => continue,
                    },
                    None => None,
                };
                bounds.insert((lane, id), (a, b));
            }
            let mut exact: Vec<Option<(i64, i64)>> = vec![None; lanes.len()];
            for &(i, source) in firsts.keys() {
                let lane = materialised.as_ref().map(|_| i);
                if let Some(&(a, b)) = bounds.get(&(lane, source)) {
                    let e = exact[i].get_or_insert((a, b));
                    *e = (e.0.min(a), e.1.max(b));
                }
            }
            for (lane, e) in lanes.iter_mut().zip(exact) {
                if let Some((a, b)) = e {
                    (lane.first, lane.last) = (a.max(from), b.min(to));
                }
            }
        }
        let mut by_lane: Vec<Vec<(i64, u32)>> = vec![Vec::new(); lanes.len()];
        for ((i, source), first) in firsts {
            by_lane[i].push((first, source));
        }
        for (lane, mut f) in lanes.iter_mut().zip(by_lane) {
            f.sort_unstable();
            lane.sources = f.into_iter().take(MAX_LANE_SOURCES).map(|(_, s)| s).collect();
        }
        // Marker clusters from the (few) marked events, as the event query does.
        let mut clusters: Vec<HashMap<(usize, usize), Cluster>> = (0..lanes.len()).map(|_| HashMap::new()).collect();
        let visible = ctx.visible;
        let any = visible | lifecycle;
        if any != 0 {
            let mut filter = vec!["e.ts IS NOT NULL".to_string(), format!("(e.markers & {any}::UBIGINT) <> 0")];
            filter.extend(
                cq.structural_exprs()
                    .iter()
                    .filter_map(|e| sql::expr_sql(e, &ctx.fields, &cq.group_by)),
            );
            let content = cq
                .content_exprs()
                .iter()
                .filter_map(|e| sql::expr_sql(e, &ctx.fields, &cq.group_by))
                .collect::<Vec<_>>();
            let content = if content.is_empty() {
                "TRUE".to_string()
            } else {
                content.join(" AND ")
            };
            let bucket = format!(
                "least(greatest(CAST(trunc(CAST(e.ts - {from} AS DOUBLE) / CAST('{bucket_ms:?}' AS DOUBLE)) AS BIGINT), 0), {})",
                buckets - 1
            );
            // The materialised lanes keep their marked events (with their keys) apart.
            let (lane_key, events) = match &materialised {
                Some(m) => {
                    filter.push(m.condition_of("e"));
                    ("e.k".to_string(), "lane_marks e")
                }
                None => (key.to_string(), FROM),
            };
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT k, mask, b, n, struct_extract(f, 'ts'), struct_extract(f, 'source'), struct_extract(f, 'line'), struct_extract(f, 'id'), msg FROM (
                       SELECT k, mask, b, count(*) AS n, min(o) AS f, arg_min(message, o) AS msg FROM (
                         SELECT {lane_key} AS k, {bucket} AS b, e.message AS message,
                                struct_pack(ts := e.ts, source := e.source, line := e.line, id := e.id) AS o,
                                CASE WHEN ({content}) THEN e.markers & {visible}::UBIGINT ELSE e.markers & {lifecycle}::UBIGINT END AS mask
                         FROM {events} WHERE {})
                       WHERE mask <> 0 GROUP BY k, mask, b)",
                    filter.join(" AND ")
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
        }
        let (lanes, truncated_lanes) = finish_lanes(lanes, clusters);
        Ok(Some(Timeline {
            from,
            to,
            bucket_ms,
            histogram,
            lanes,
            truncated_lanes,
        }))
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
        // No window functions and no wide columns: one pass for the bounds, one GROUP BY for lanes, density and
        // histogram, one for each lane's sources (bounds and first events), and one over the (few) marked events
        // for the clusters. Lanes × buckets and lanes × sources are grouped apart: their product (thousands of
        // sources × hundreds of buckets) exceeded the memory limit at 10M events.
        let filter = format!("e.ts IS NOT NULL AND {}", f.structural_sql());
        let (min, max) = conn
            .query_row(&format!("SELECT min(e.ts), max(e.ts) FROM {FROM} WHERE {filter}"), [], |r| {
                Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?))
            })
            .map_err(engine)?;
        let (from, to, bucket_ms) = Self::bounds(req, min, max, buckets);
        let bucket = format!(
            "least(greatest(CAST(trunc(CAST(e.ts - {from} AS DOUBLE) / CAST('{bucket_ms:?}' AS DOUBLE)) AS BIGINT), 0), {})",
            buckets - 1
        );
        let content = f.content_sql();
        let mut lanes: Vec<Lane> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut histogram = vec![[0u32; 6]; buckets];
        let mut lane_of = |k: String, lanes: &mut Vec<Lane>| -> usize {
            *index.entry(k).or_insert_with_key(|k| {
                lanes.push(Lane {
                    key: k.clone(),
                    first: i64::MAX,
                    last: i64::MIN,
                    total: 0,
                    matched: 0,
                    density: vec![0; buckets],
                    markers: Vec::new(),
                    sources: Vec::new(),
                });
                lanes.len() - 1
            })
        };
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {key} AS k, {bucket} AS b, e.level AS level, ({content}) AS m, count(*) AS n FROM {FROM} WHERE {filter} GROUP BY ALL"
            ))
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, u8>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            })
            .map_err(engine)?;
        for row in rows {
            let (k, b, level, m, n) = row.map_err(engine)?;
            let i = lane_of(k, &mut lanes);
            let lane = &mut lanes[i];
            let b = (b.max(0) as usize).min(buckets - 1);
            lane.total += n as usize;
            if m {
                lane.matched += n as usize;
                lane.density[b] += n as u32;
                histogram[b][(level as usize).min(5)] += n as u32;
            }
        }
        // The first event of each (lane, source), to order the lane's sources.
        let mut firsts: HashMap<(usize, u32), OrderKey> = HashMap::new();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT k, source, tmin, tmax, struct_extract(f, 'ts'), struct_extract(f, 'line'), struct_extract(f, 'id') FROM (
                   SELECT {key} AS k, e.source AS source, min(e.ts) AS tmin, max(e.ts) AS tmax,
                          min(struct_pack(ts := e.ts, line := e.line, id := e.id)) AS f
                   FROM {FROM} WHERE {filter} GROUP BY ALL)"
            ))
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    (r.get::<_, i64>(4)?, r.get::<_, u32>(5)?, r.get::<_, u64>(6)?),
                ))
            })
            .map_err(engine)?;
        for row in rows {
            let (k, source, tmin, tmax, (fts, fline, fid)) = row.map_err(engine)?;
            let i = lane_of(k, &mut lanes);
            let lane = &mut lanes[i];
            lane.first = lane.first.min(tmin);
            lane.last = lane.last.max(tmax);
            let key = (fts, source, fline, fid);
            firsts.entry((i, source)).and_modify(|k| *k = (*k).min(key)).or_insert(key);
        }
        let mut by_lane: Vec<Vec<(OrderKey, u32)>> = vec![Vec::new(); lanes.len()];
        for ((i, source), k) in firsts {
            by_lane[i].push((k, source));
        }
        for (lane, mut f) in lanes.iter_mut().zip(by_lane) {
            f.sort_unstable();
            lane.sources = f.into_iter().take(MAX_LANE_SOURCES).map(|(_, s)| s).collect();
        }
        let mut clusters: Vec<HashMap<(usize, usize), Cluster>> = (0..lanes.len()).map(|_| HashMap::new()).collect();
        let visible = ctx.visible;
        let any = visible | lifecycle;
        if any != 0 {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT k, mask, b, n, struct_extract(f, 'ts'), struct_extract(f, 'source'), struct_extract(f, 'line'), struct_extract(f, 'id'), msg FROM (
                       SELECT k, mask, b, count(*) AS n, min(o) AS f, arg_min(message, o) AS msg FROM (
                         SELECT {key} AS k, {bucket} AS b, e.message AS message,
                                struct_pack(ts := e.ts, source := e.source, line := e.line, id := e.id) AS o,
                                CASE WHEN ({content}) THEN e.markers & {visible}::UBIGINT ELSE e.markers & {lifecycle}::UBIGINT END AS mask
                         FROM {FROM} WHERE {filter} AND (e.markers & {any}::UBIGINT) <> 0)
                       WHERE mask <> 0 GROUP BY k, mask, b)"
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
            "SELECT {} FROM {FROM} WHERE e.ts IS NOT NULL AND {} {}",
            rows_sql(&self.schema),
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
        let mut page = self.inner.page(req)?;
        imt_core::ports::mark_search_matches(&mut page.rows, &req.query);
        Ok(page)
    }

    fn events(&self, req: &imt_core::ports::EventsRequest) -> StoreResult<Vec<EventView>> {
        self.inner.events(req)
    }

    fn timeline(&self, req: &TimelineRequest) -> StoreResult<Timeline> {
        // A live window bounds the timeline like a time range does.
        let query = self.inner.resolve_window(&req.query)?.into_owned();
        self.inner.timeline(&TimelineRequest { query, ..req.clone() })
    }

    fn facet(&self, req: &FacetRequest) -> StoreResult<Facet> {
        self.inner.facet(req)
    }

    fn scan(&self, filter: &Filter, visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
        self.inner.scan(filter, visit)
    }

    fn chart(&self, req: &ChartRequest) -> StoreResult<ChartData> {
        let query = self.inner.resolve_window(&req.query)?.into_owned();
        self.inner.chart(&ChartRequest { query, ..req.clone() })
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

    fn export(&self, req: &ExportRequest) -> StoreResult<ExportResult> {
        self.inner.export(req)
    }
}

/// The fields of a statement plus the word hits of its smart search (kept alive with the filter).
struct WithHits<'a> {
    fields: &'a sql::Fields,
    inner: &'a Inner,
    keep: std::cell::RefCell<Vec<Arc<crate::hits::HitsFile>>>,
}

impl sql::FieldSource for WithHits<'_> {
    fn events(&self) -> &imt_core::batch::EventSchema {
        self.fields.events()
    }
    fn promoted_text(&self, key: &str) -> bool {
        self.fields.promoted_text(key)
    }
    fn word_hits(&self, words: &imt_core::words::WordQuery) -> Option<String> {
        match self.inner.word_hits(words) {
            Ok(Some(hits)) => {
                let (sql, file) = hits.condition()?;
                self.keep.borrow_mut().extend(file);
                Some(sql)
            }
            Ok(None) => None,
            Err(e) => {
                log::warn!("word search falls back to substrings: {e:#}");
                None
            }
        }
    }
}

/// Events carrying a field, by (field, source position).
#[derive(Default)]
struct Presence(HashMap<(String, usize), usize>);

impl imt_core::instance::FieldPresence for Presence {
    fn count_in(&self, field: &str, source: usize) -> usize {
        self.0.get(&(field.to_string(), source)).copied().unwrap_or(0)
    }
}
