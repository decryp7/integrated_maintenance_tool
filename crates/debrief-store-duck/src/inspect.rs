//! The Store pane's reports (ADR 0030): the store folder, its segments, fields, full-text index and memory, read
//! from the catalog (no event is scanned), and the maintenance actions it offers. Reports are queries; only
//! [`Maintain::maintain`] changes anything.

use crate::promote::{PType, Promotion};
use crate::writer::meta_u64;
use crate::{engine, lock, read_lock, DuckStore, FORMAT_VERSION};
use debrief_core::batch::ColumnType;
use debrief_core::ports::{
    parse_size, FieldStorage, FieldType, ImportReport, Inspect, LastImport, Maintain, MaintenanceAction, MaintenanceReport, SegmentRow,
    SegmentsPage, SegmentsRequest, StoreActivity, StoreError, StoreField, StoreFields, StoreIndex, StoreMemory, StoreOverview, StoreResult,
    IMPORT_BUDGET_BYTES, MAX_FIELD_ROWS, QUERY_BUDGET_BYTES, SEGMENT_SOURCES, SQL_MEMORY_LIMIT,
};
use duckdb::{params, Connection};
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Most directory entries counted for a folder size (a folder beyond it is reported as at least that big).
const MAX_WALK: usize = 200_000;

/// The catalog key of the last import.
const LAST_IMPORT: &str = "last_import";

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Keeps the import's numbers in the catalog (the overview's "last import"). Not fatal: logged.
pub(crate) fn record_import(conn: &Connection, report: &ImportReport) {
    let last = LastImport::new(report.events as u64, report.bytes, report.elapsed_ms, now_ms());
    let saved = serde_json::to_string(&last).map_err(|e| e.to_string()).and_then(|json| {
        conn.execute("INSERT OR REPLACE INTO meta VALUES (?, ?)", params![LAST_IMPORT, json])
            .map_err(|e| e.to_string())
    });
    if let Err(e) = saved {
        log::warn!("the last import could not be recorded: {e}");
    }
}

fn last_import(conn: &Connection) -> Option<LastImport> {
    let text: String = conn
        .query_row("SELECT value FROM meta WHERE key = ?", params![LAST_IMPORT], |r| r.get(0))
        .ok()?;
    serde_json::from_str(&text).ok()
}

/// Bytes of the files under `dir` (at most [`MAX_WALK`] entries; unreadable entries count 0).
pub(crate) fn folder_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            seen += 1;
            if seen > MAX_WALK {
                return total;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total += e.metadata().map_or(0, |m| m.len()),
                _ => {}
            }
        }
    }
    total
}

fn pending_segments(conn: &Connection) -> StoreResult<u64> {
    conn.query_row("SELECT count(*) FROM segments WHERE NOT indexed", [], |r| r.get(0))
        .map_err(engine)
}

fn totals(conn: &Connection) -> StoreResult<(u64, u64)> {
    conn.query_row(
        "SELECT coalesce(sum(events), 0)::UBIGINT, count(*) FROM sources WHERE ready",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(engine)
}

fn ptype(t: PType, numeric: bool) -> FieldType {
    match t {
        PType::Int => FieldType::Integer,
        PType::Bool => FieldType::Boolean,
        PType::Text if numeric => FieldType::Number,
        PType::Text => FieldType::Text,
    }
}

fn declared_type(t: ColumnType) -> FieldType {
    match t {
        ColumnType::Utf8 => FieldType::Text,
        ColumnType::Int64 => FieldType::Integer,
        ColumnType::Float64 => FieldType::Number,
        ColumnType::Boolean => FieldType::Boolean,
    }
}

/// `'a', 'b'` for an `IN` list (file names are the store's own, quoted anyway).
fn quoted(files: &[String]) -> String {
    files
        .iter()
        .map(|f| format!("'{}'", f.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ")
}

impl DuckStore {
    /// Compression codec per segment file name (from the Parquet footers; `?` when a footer cannot be read).
    fn codecs(&self, conn: &Connection, files: &[String]) -> HashMap<String, String> {
        let seg_dir = self.inner.dir.join(crate::segments::SEGMENTS_DIR);
        let paths: Vec<String> = files.iter().map(|f| seg_dir.join(f).to_string_lossy().into_owned()).collect();
        let sql = format!(
            "SELECT file_name, string_agg(DISTINCT compression, ', ' ORDER BY compression) FROM parquet_metadata([{}]) GROUP BY file_name",
            quoted(&paths)
        );
        let read = || -> duckdb::Result<HashMap<String, String>> {
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?;
            let mut out = HashMap::new();
            for row in rows {
                let (path, codec) = row?;
                let name = Path::new(&path)
                    .file_name()
                    .map_or(path.clone(), |n| n.to_string_lossy().into_owned());
                out.insert(name, codec.unwrap_or_default());
            }
            Ok(out)
        };
        match read() {
            Ok(m) => m,
            Err(e) => {
                log::warn!("segment footers could not be read: {e}");
                HashMap::new()
            }
        }
    }
}

impl Inspect for DuckStore {
    fn store_overview(&self) -> StoreResult<StoreOverview> {
        let conn = self.inner.conn()?;
        let (events, sources) = totals(&conn)?;
        let segments: u64 = conn.query_row("SELECT count(*) FROM segments", [], |r| r.get(0)).map_err(engine)?;
        let pending_compaction = crate::compact::pending(&conn).map_err(|e| engine(format!("{e:#}")))?.len() as u64;
        let activity = if self.inner.is_importing() {
            StoreActivity::Importing
        } else if self.inner.fts.is_some() && pending_segments(&conn)? > 0 {
            StoreActivity::Indexing
        } else {
            StoreActivity::Ready
        };
        let format_version = u32::try_from(meta_u64(&conn, "format_version")).unwrap_or(FORMAT_VERSION);
        Ok(StoreOverview {
            dir: self.inner.dir.display().to_string(),
            size_bytes: folder_size(&self.inner.dir),
            events,
            sources,
            segments,
            data_version: meta_u64(&conn, "data_version"),
            format_version,
            activity,
            last_import: last_import(&conn),
            pending_compaction,
        })
    }

    fn store_segments(&self, req: &SegmentsRequest) -> StoreResult<SegmentsPage> {
        let conn = self.inner.conn()?;
        let total: u64 = conn.query_row("SELECT count(*) FROM segments", [], |r| r.get(0)).map_err(engine)?;
        let mut stmt = conn
            .prepare("SELECT file, first_id, end_id, rows, live, indexed, promoted FROM segments ORDER BY first_id, file LIMIT ? OFFSET ?")
            .map_err(engine)?;
        type Listed = (String, u64, u64, u64, u64, bool, String);
        let listed: Vec<Listed> = stmt
            .query_map(params![req.rows() as u64, req.offset as u64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
            })
            .map_err(engine)?
            .collect::<duckdb::Result<_>>()
            .map_err(engine)?;
        if listed.is_empty() {
            return Ok(SegmentsPage {
                total,
                offset: req.offset,
                rows: Vec::new(),
            });
        }
        let files: Vec<String> = listed.iter().map(|l| l.0.clone()).collect();
        // Time range (to the minute) and sources, from the histogram the writer kept.
        let sql = format!(
            "SELECT file, min(minute), max(minute), count(DISTINCT source), list(DISTINCT source ORDER BY source)[1:{SEGMENT_SOURCES}] \
             FROM histogram WHERE file IN ({}) GROUP BY file",
            quoted(&files)
        );
        let mut stmt = conn.prepare(&sql).map_err(engine)?;
        type Hist = (Option<i64>, Option<i64>, u64, Vec<u32>);
        let mut hist: HashMap<String, Hist> = HashMap::new();
        let rows = stmt
            .query_map([], |r| {
                let sources: duckdb::types::Value = r.get(4)?;
                let ids = match sources {
                    duckdb::types::Value::List(v) => v
                        .into_iter()
                        .filter_map(|x| match x {
                            duckdb::types::Value::UInt(n) => Some(n),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, ids)))
            })
            .map_err(engine)?;
        for row in rows {
            let (file, h) = row.map_err(engine)?;
            hist.insert(file, h);
        }
        let codecs = self.codecs(&conn, &files);
        let pending = crate::compact::pending(&conn).map_err(|e| engine(format!("{e:#}")))?;
        let seg_dir = self.inner.dir.join(crate::segments::SEGMENTS_DIR);
        let rows = listed
            .into_iter()
            .map(|(file, first_id, end_id, rows, live, indexed, promoted)| {
                let (from, to, source_count, sources) = hist.remove(&file).unwrap_or((None, None, 0, Vec::new()));
                SegmentRow {
                    size_bytes: std::fs::metadata(seg_dir.join(&file)).map_or(0, |m| m.len()),
                    compression: codecs.get(&file).cloned().unwrap_or_else(|| "?".into()),
                    pending_compaction: pending.contains(&file),
                    promoted: Promotion::decode(&promoted).keys.into_iter().map(|(k, _)| k).collect(),
                    from: from.map(|m| m.saturating_mul(60_000)),
                    to: to.map(|m| m.saturating_mul(60_000)),
                    file,
                    first_id,
                    end_id,
                    rows,
                    live,
                    sources,
                    source_count,
                    indexed,
                }
            })
            .collect();
        Ok(SegmentsPage {
            total,
            offset: req.offset,
            rows,
        })
    }

    fn store_fields(&self) -> StoreResult<StoreFields> {
        let conn = self.inner.conn()?;
        let (events, _) = totals(&conn)?;
        // Promoted columns per key: in how many segments, and with which type.
        let mut promoted: HashMap<String, (u64, PType)> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT promoted FROM segments").map_err(engine)?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(engine)?;
            for row in rows {
                for (k, t) in Promotion::decode(&row.map_err(engine)?).keys {
                    let e = promoted.entry(k).or_insert((0, t));
                    e.0 += 1;
                    if e.1 != t {
                        e.1 = PType::Text;
                    }
                }
            }
        }
        let declared: HashMap<String, ColumnType> = self.inner.schema.declared().iter().map(|c| (c.name.clone(), c.ty)).collect();
        let mut stmt = conn
            .prepare(
                "SELECT key, sum(rows)::UBIGINT AS n, sum(numeric_rows)::UBIGINT, max(distinct_est), count(*) FROM segment_fields \
                 GROUP BY key ORDER BY n DESC, key LIMIT ?",
            )
            .map_err(engine)?;
        let rows = stmt
            .query_map(params![MAX_FIELD_ROWS as u64 + 1], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, u64>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            })
            .map_err(engine)?
            .collect::<duckdb::Result<Vec<_>>>()
            .map_err(engine)?;
        let truncated = rows.len() > MAX_FIELD_ROWS;
        let fields = rows
            .into_iter()
            .take(MAX_FIELD_ROWS)
            .map(|(key, n, numeric, distinct, with_key)| {
                let all_numeric = n > 0 && numeric == n;
                let (ty, storage, promoted_segments) = match (declared.get(&key), promoted.get(&key)) {
                    (Some(t), _) => (declared_type(*t), FieldStorage::Declared, with_key),
                    (None, Some((p, t))) => (
                        ptype(*t, all_numeric),
                        if *p >= with_key {
                            FieldStorage::Promoted
                        } else {
                            FieldStorage::Partly
                        },
                        *p,
                    ),
                    (None, None) => (
                        if all_numeric { FieldType::Number } else { FieldType::Text },
                        FieldStorage::Overflow,
                        0,
                    ),
                };
                StoreField {
                    fill_rate: if events == 0 { 0.0 } else { (n as f64 / events as f64).min(1.0) },
                    approx_distinct: distinct,
                    events: n,
                    segments: with_key,
                    promoted_segments,
                    key,
                    ty,
                    storage,
                }
            })
            .collect();
        Ok(StoreFields { events, fields, truncated })
    }

    fn store_index(&self) -> StoreResult<StoreIndex> {
        let conn = self.inner.conn()?;
        let (events, _) = totals(&conn)?;
        let pending = pending_segments(&conn)?;
        let dir = self.inner.dir.join(crate::fts::INDEX_DIR);
        let last_commit_ms = std::fs::metadata(dir.join("meta.json"))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        let Some(f) = &self.inner.fts else {
            return Ok(StoreIndex {
                docs: 0,
                events,
                size_bytes: folder_size(&dir),
                index_segments: 0,
                pending_segments: pending,
                catching_up: false,
                last_commit_ms,
                writer_heap_bytes: 0,
                writer_threads: 0,
                error: self.inner.fts_error.clone(),
            });
        };
        let index = &f.shared.fts;
        let budget = index.budget();
        Ok(StoreIndex {
            docs: index.docs(),
            events,
            size_bytes: folder_size(&dir),
            index_segments: index.segments() as u64,
            pending_segments: pending,
            catching_up: pending > 0,
            last_commit_ms,
            writer_heap_bytes: budget.heap as u64,
            writer_threads: budget.threads as u32,
            error: lock(&f.shared.failure).clone(),
        })
    }

    fn store_memory(&self) -> StoreResult<StoreMemory> {
        let conn = self.inner.conn()?;
        let (used, spilled): (u64, u64) = conn
            .query_row(
                "SELECT coalesce(sum(memory_usage_bytes), 0)::UBIGINT, coalesce(sum(temporary_storage_bytes), 0)::UBIGINT FROM duckdb_memory()",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(engine)?;
        let version = meta_u64(&conn, "data_version");
        let (cached, cached_bytes): (u64, u64) = conn
            .query_row(
                "SELECT count(*), coalesce(sum(strlen(value)), 0)::UBIGINT FROM analysis_cache WHERE version = ?",
                params![version],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(engine)?;
        let opts = read_lock(&self.inner.options).clone();
        Ok(StoreMemory {
            worker: None,
            import_budget_bytes: IMPORT_BUDGET_BYTES,
            query_budget_bytes: QUERY_BUDGET_BYTES,
            duck_limit_bytes: parse_size(&opts.memory_limit).unwrap_or(0),
            duck_limit: opts.memory_limit,
            duck_used_bytes: used,
            duck_spilled_bytes: spilled,
            duck_threads: opts.duck_threads as u32,
            console_limit_bytes: parse_size(SQL_MEMORY_LIMIT).unwrap_or(0),
            importing: self.inner.is_importing(),
            running_jobs: Vec::new(),
            cached_analyses: cached,
            cached_bytes,
        })
    }
}

impl Maintain for DuckStore {
    fn maintain(&self, action: MaintenanceAction) -> StoreResult<MaintenanceReport> {
        self.idle()?;
        let message = match action {
            MaintenanceAction::Compact => {
                let c = {
                    let _writing = lock(&self.inner.write_lock);
                    let conn = self.inner.conn()?;
                    let c = crate::compact::compact_with(&conn, &self.inner.dir, &self.inner.schema, true).map_err(|e| {
                        if e.is::<crate::compact::CompactCancelled>() {
                            StoreError::Cancelled
                        } else {
                            engine(format!("{e:#}"))
                        }
                    })?;
                    if c.replaced > 0 {
                        crate::pos::rebuild(&conn).map_err(engine)?;
                        conn.execute_batch("CHECKPOINT").map_err(engine)?;
                    }
                    c
                };
                if c.replaced == 0 {
                    "Nothing to compact: the segments are full and hold no removed rows.".to_string()
                } else {
                    self.changed();
                    if let Some(f) = &self.inner.fts {
                        f.wake();
                    }
                    format!("Compacted {} segments into {}.", c.replaced, c.written)
                }
            }
            MaintenanceAction::RebuildIndex => {
                let f = self
                    .inner
                    .fts
                    .as_ref()
                    .ok_or_else(|| StoreError::Engine(self.inner.fts_error.clone().unwrap_or_else(|| "no full-text index".into())))?;
                let n = {
                    let _writing = lock(&self.inner.write_lock);
                    let conn = self.inner.conn()?;
                    f.rebuild(&conn).map_err(|e| engine(format!("{e:#}")))?
                };
                // Search results come from the index: cached word hits are stale now.
                self.inner.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                format!("Indexing {n} segments again; search covers more events as it goes.")
            }
            MaintenanceAction::ClearCachedAnalyses => {
                let conn = self.inner.conn()?;
                let n = conn.execute("DELETE FROM analysis_cache", []).map_err(engine)?;
                conn.execute_batch(&crate::lanes::prune_sql(u64::MAX)).map_err(engine)?;
                *lock(&self.inner.derived) = None;
                format!("Cleared {n} cached analyses; they are computed again when next used.")
            }
        };
        Ok(MaintenanceReport { action, message })
    }
}
