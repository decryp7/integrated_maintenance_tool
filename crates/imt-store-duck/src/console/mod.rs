//! The read-only SQL console (UX audit C2, ADR 0030). Queries run in a DuckDB database of their own, in emory,
//! never in the store's: it can read the store's segment folder and nothing else (`allowed_directories`, then
//! `enable_external_access = false`), loads and installs no extension, has a 128 MB memory limit and two threads,
//! and its configuration is locked (`lock_configuration`), so no statement can undo any of it. It holds the
//! documented views ([`views`]); the small tables behind `sources`, `fields` and `notes` are copied in from the
//! catalog when the data version (or the notes) changed. Every text is checked in Rust first ([`guard`]): one
//! SELECT, allowed table functions, no file names. A run keeps at most 10,000 rows (paged 500 at a time), stops
//! after 30 s, and stops at once when its control is cancelled.

pub mod guard;
pub mod views;

use crate::{engine, lock, DuckStore};
use duckdb::types::Value as Duck;
use duckdb::{params, Connection};
use imt_core::ports::{
    Console, Inspect, SqlColumn, SqlNote, SqlPage, SqlPageRequest, SqlRequest, SqlResult, SqlSchema, StoreError, StoreResult,
    MAX_CELL_CHARS, MAX_SQL_LEN, SQL_MAX_NOTES, SQL_MAX_RESULT_BYTES, SQL_MAX_ROWS, SQL_MEMORY_LIMIT, SQL_NOTE_CHARS, SQL_PAGE_ROWS,
    SQL_THREADS, SQL_TIME_LIMIT, SQL_VIEWS_VERSION,
};
use imt_core::ql::{QlError, QueryControl};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Results kept for paging (the newest).
const KEPT_RESULTS: usize = 4;

/// Types whose values reach the UI as they are; every other column is cast to text in SQL.
const SCALARS: [&str; 12] = [
    "BOOLEAN",
    "TINYINT",
    "SMALLINT",
    "INTEGER",
    "BIGINT",
    "UTINYINT",
    "USMALLINT",
    "UINTEGER",
    "UBIGINT",
    "FLOAT",
    "DOUBLE",
    "VARCHAR",
];
const INTEGERS: [&str; 8] = [
    "TINYINT",
    "SMALLINT",
    "INTEGER",
    "BIGINT",
    "UTINYINT",
    "USMALLINT",
    "UINTEGER",
    "UBIGINT",
];

/// The console's database and what it was last filled with.
struct Sandbox {
    conn: Connection,
    /// (store generation, notes hash) of the copied tables.
    filled: Option<(u64, u64)>,
}

/// A run's rows, kept for paging.
struct Kept {
    id: u64,
    rows: Vec<Vec<Value>>,
}

/// The console of one store.
#[derive(Default)]
pub(crate) struct ConsoleState {
    sandbox: Mutex<Option<Sandbox>>,
    results: Mutex<VecDeque<Arc<Kept>>>,
    next: AtomicU64,
    /// The time limit in ms when a test shortened it (0: [`SQL_TIME_LIMIT`]).
    time_limit_ms: AtomicU64,
}

impl ConsoleState {
    fn time_limit(&self) -> Duration {
        match self.time_limit_ms.load(Ordering::Relaxed) {
            0 => SQL_TIME_LIMIT,
            ms => Duration::from_millis(ms),
        }
    }
}

impl DuckStore {
    /// Shortens the SQL console's time limit (tests of the limit; the app keeps [`SQL_TIME_LIMIT`]).
    pub fn set_sql_time_limit(&self, limit: Duration) {
        self.inner
            .console
            .time_limit_ms
            .store(limit.as_millis().clamp(1, u128::from(u64::MAX)) as u64, Ordering::Relaxed);
    }
}

fn sql_text(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\'', "''")
}

/// Opens the sandboxed database: only `segments` (the store's segment folder) is readable, then external access,
/// extension loading and installing are switched off, the limits set and the configuration locked.
fn open_sandbox(segments: &std::path::Path) -> StoreResult<Connection> {
    let conn = Connection::open_in_memory().map_err(engine)?;
    let mut dir = sql_text(segments);
    if !dir.ends_with(['/', '\\']) {
        dir.push(std::path::MAIN_SEPARATOR);
    }
    conn.execute_batch(&format!(
        "SET allowed_directories = ['{dir}'];
         SET enable_external_access = false;
         SET autoload_known_extensions = false;
         SET autoinstall_known_extensions = false;
         SET memory_limit = '{SQL_MEMORY_LIMIT}';
         SET threads = {SQL_THREADS};
         SET preserve_insertion_order = true;
         CREATE SCHEMA imt;
         {}
         SET search_path = 'imt,main';
         SET lock_configuration = true;",
        views::tables_ddl()
    ))
    .map_err(engine)?;
    Ok(conn)
}

/// FNV-1a of the notes as JSON (whether `imt.notes` must be filled again).
fn notes_hash(notes: &[SqlNote]) -> u64 {
    let text = serde_json::to_string(notes).unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The first `max` characters of `s` (cut ones end with `…`).
fn cut(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_string(),
    }
}

/// A DuckDB value as a JSON cell (non-scalars were cast to text in SQL).
fn cell(v: Duck) -> Value {
    match v {
        Duck::Null => Value::Null,
        Duck::Boolean(b) => Value::Bool(b),
        Duck::TinyInt(n) => n.into(),
        Duck::SmallInt(n) => n.into(),
        Duck::Int(n) => n.into(),
        Duck::BigInt(n) => n.into(),
        Duck::UTinyInt(n) => n.into(),
        Duck::USmallInt(n) => n.into(),
        Duck::UInt(n) => n.into(),
        Duck::UBigInt(n) => n.into(),
        Duck::Float(f) => serde_json::Number::from_f64(f64::from(f)).map_or_else(|| Value::String(f.to_string()), Value::Number),
        Duck::Double(f) => serde_json::Number::from_f64(f).map_or_else(|| Value::String(f.to_string()), Value::Number),
        Duck::Text(s) => Value::String(cut(&s, MAX_CELL_CHARS)),
        other => Value::String(cut(&format!("{other:?}"), MAX_CELL_CHARS)),
    }
}

/// Bytes a cell holds (the result's memory cap).
fn cell_bytes(v: &Value) -> usize {
    match v {
        Value::String(s) => s.len() + 8,
        _ => 8,
    }
}

/// Interrupts the console's statement when the run's control is cancelled or its time is up.
struct Stopper {
    timed_out: Arc<AtomicBool>,
    done: Option<mpsc::Sender<()>>,
    _cancel: Option<imt_core::ql::HookGuard>,
}

impl Stopper {
    fn start(conn: &Connection, limit: Duration) -> StoreResult<Stopper> {
        let handle = conn.interrupt_handle();
        let control = QueryControl::current();
        if control.as_ref().is_some_and(QueryControl::is_cancelled) {
            return Err(StoreError::Cancelled);
        }
        let on_cancel = control.map(|c| {
            let h = handle.clone();
            c.on_cancel(Box::new(move || h.interrupt()))
        });
        let timed_out = Arc::new(AtomicBool::new(false));
        let (done, wait) = mpsc::channel::<()>();
        let flag = timed_out.clone();
        let spawned = std::thread::Builder::new().name("imt-sql-timer".into()).spawn(move || {
            if let Err(mpsc::RecvTimeoutError::Timeout) = wait.recv_timeout(limit) {
                flag.store(true, Ordering::SeqCst);
                handle.interrupt();
            }
        });
        if let Err(e) = spawned {
            log::warn!("the SQL console's time limit is unavailable: {e}");
        }
        Ok(Stopper {
            timed_out,
            done: Some(done),
            _cancel: on_cancel,
        })
    }

    /// The error a failed statement stands for: a stop by the user or the clock, the memory limit, or what
    /// DuckDB said about the query.
    fn error(&self, e: duckdb::Error, limit: Duration) -> StoreError {
        if QueryControl::current().is_some_and(|c| c.is_cancelled()) {
            return StoreError::Cancelled;
        }
        if self.timed_out.load(Ordering::SeqCst) {
            let after = match limit.as_secs() {
                0 => format!("{} ms", limit.as_millis()),
                s => format!("{s} s"),
            };
            return StoreError::Invalid(format!("Stopped after {after}. Narrow the time range, add LIMIT or aggregate."));
        }
        query_error(&e.to_string())
    }
}

impl Drop for Stopper {
    fn drop(&mut self) {
        self.done.take();
    }
}

/// DuckDB's message about a query as the error the editor underlines (its `LINE n:` caret gives no byte offset,
/// so the span is the start), or the memory limit's message.
fn query_error(message: &str) -> StoreError {
    let lower = message.to_lowercase();
    if lower.contains("out of memory") {
        return StoreError::Invalid(format!(
            "The query needed more than the SQL console's {SQL_MEMORY_LIMIT} of memory. Add filters or LIMIT, or select fewer columns."
        ));
    }
    if lower.contains("permission error") || lower.contains("disabled by configuration") {
        return StoreError::Query(QlError::new(
            "File, network and extension access is turned off in the SQL console; query the views events, sources, fields and notes.",
            0,
            0,
        ));
    }
    let first = message.split("\n\n").next().unwrap_or(message).trim();
    StoreError::Query(QlError::new(first.to_string(), 0, 0))
}

impl DuckStore {
    /// Copies the catalog tables into the sandbox and points `imt.events` at the current segments, when the store
    /// changed since (or the notes did).
    fn fill(&self, sandbox: &mut Sandbox, notes: &[SqlNote]) -> StoreResult<()> {
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let hash = notes_hash(notes);
        let (store_changed, notes_changed) = match sandbox.filled {
            Some((g, h)) => (g != generation, h != hash),
            None => (true, true),
        };
        let c = &sandbox.conn;
        if store_changed {
            let main = self.inner.conn()?;
            let files = crate::writer::segment_files(&main, &self.inner.dir).map_err(|e| engine(format!("{e:#}")))?;
            let mut stmt = main
                .prepare(
                    "SELECT id, name, path, importer, parser_id, events, skipped, ts_from, ts_to, bytes FROM sources WHERE ready ORDER BY id",
                )
                .map_err(engine)?;
            type Row = (
                u32,
                String,
                String,
                Option<String>,
                Option<String>,
                u64,
                u64,
                Option<i64>,
                Option<i64>,
                u64,
            );
            let sources: Vec<Row> = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                    ))
                })
                .map_err(engine)?
                .collect::<duckdb::Result<_>>()
                .map_err(engine)?;
            drop(stmt);
            drop(main);
            let fields = self.store_fields()?;
            c.execute_batch("DELETE FROM imt.sources; DELETE FROM imt.fields;")
                .map_err(engine)?;
            {
                let mut insert = c
                    .prepare("INSERT INTO imt.sources VALUES (?, ?, ?, ?, ?, ?, ?, epoch_ms(?::BIGINT), epoch_ms(?::BIGINT), ?)")
                    .map_err(engine)?;
                for s in &sources {
                    insert
                        .execute(params![s.0, s.1, s.2, s.3, s.4, s.5, s.6, s.7, s.8, s.9])
                        .map_err(engine)?;
                }
                let mut insert = c.prepare("INSERT INTO imt.fields VALUES (?, ?, ?, ?, ?, ?)").map_err(engine)?;
                for f in &fields.fields {
                    insert
                        .execute(params![
                            f.key,
                            wire_name(&f.ty),
                            wire_name(&f.storage),
                            f.events,
                            f.fill_rate,
                            f.approx_distinct
                        ])
                        .map_err(engine)?;
                }
            }
            c.execute_batch(&views::events_view(&self.inner.schema, &files)).map_err(engine)?;
        }
        if notes_changed || store_changed {
            c.execute_batch("DELETE FROM imt.notes;").map_err(engine)?;
            let mut insert = c
                .prepare(
                    "INSERT INTO imt.notes VALUES (?, ?, ?, (SELECT id FROM imt.sources WHERE path = ?), epoch_ms(?::BIGINT), epoch_ms(?::BIGINT), ?, ?::VARCHAR[], ?, ?, epoch_ms(?::BIGINT), epoch_ms(?::BIGINT))",
                )
                .map_err(engine)?;
            for n in notes.iter().take(SQL_MAX_NOTES) {
                let tags = serde_json::to_string(&n.tags.iter().take(8).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into());
                insert
                    .execute(params![
                        cut(&n.id, 64),
                        cut(&n.anchor, 32),
                        n.event_id,
                        n.source_path,
                        n.from,
                        n.to,
                        cut(&n.severity, 16),
                        tags,
                        cut(&n.text, SQL_NOTE_CHARS),
                        cut(&n.author, 128),
                        n.created,
                        n.updated
                    ])
                    .map_err(engine)?;
            }
        }
        sandbox.filled = Some((generation, hash));
        Ok(())
    }

    /// The console's checks on `sql` (lexical, then DuckDB's parser in the sandbox).
    fn check_sql(conn: &Connection, sql: &str) -> StoreResult<guard::Checked> {
        if sql.len() > MAX_SQL_LEN {
            return Err(StoreError::Query(QlError::new(
                format!("The SQL text is longer than {} KB.", MAX_SQL_LEN >> 10),
                MAX_SQL_LEN,
                0,
            )));
        }
        let checked = guard::lexical(sql).map_err(StoreError::Query)?;
        let tree: String = conn
            .query_row("SELECT json_serialize_sql(?::VARCHAR)", [&checked.sql], |r| r.get(0))
            .map_err(engine)?;
        let tree: Value = serde_json::from_str(&tree).map_err(|e| StoreError::Engine(format!("DuckDB's parser answered: {e}")))?;
        // Offsets in the tree are into the checked text; shift them back into what the user wrote.
        let shift = sql.find(checked.sql.as_str()).unwrap_or(0);
        guard::tree(&checked.sql, &tree).map_err(|mut e| {
            e.offset += shift;
            StoreError::Query(e)
        })?;
        Ok(checked)
    }

    fn keep(&self, rows: Vec<Vec<Value>>) -> u64 {
        let console = &self.inner.console;
        let id = console.next.fetch_add(1, Ordering::SeqCst) + 1;
        let mut kept = lock(&console.results);
        if kept.len() >= KEPT_RESULTS {
            kept.pop_front();
        }
        kept.push_back(Arc::new(Kept { id, rows }));
        id
    }

    fn run_sql(&self, conn: &Connection, checked: &guard::Checked, started: Instant) -> StoreResult<SqlResult> {
        let limit = self.inner.console.time_limit();
        let stopper = Stopper::start(conn, limit)?;
        if checked.explain {
            let plan = (|| -> duckdb::Result<String> {
                let mut stmt = conn.prepare(&format!("EXPLAIN\n{}\n", checked.sql))?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
                Ok(rows.collect::<duckdb::Result<Vec<_>>>()?.join("\n"))
            })()
            .map_err(|e| stopper.error(e, limit))?;
            return Ok(SqlResult {
                result: 0,
                columns: Vec::new(),
                rows: Vec::new(),
                total: 0,
                truncated: false,
                event_ids: false,
                plan: Some(plan),
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }
        let described: Vec<(String, String)> = (|| -> duckdb::Result<Vec<(String, String)>> {
            let mut stmt = conn.prepare(&format!("DESCRIBE SELECT * FROM (\n{}\n) AS imt_result", checked.sql))?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.collect()
        })()
        .map_err(|e| stopper.error(e, limit))?;
        let aliases: Vec<String> = (0..described.len()).map(|i| format!("c{i}")).collect();
        let select: Vec<String> = described
            .iter()
            .zip(&aliases)
            .map(|((_, ty), a)| {
                if SCALARS.contains(&ty.as_str()) {
                    a.clone()
                } else {
                    format!("CAST({a} AS VARCHAR)")
                }
            })
            .collect();
        let wrapped = format!(
            "SELECT {} FROM (\n{}\n) AS imt_result({}) LIMIT {}",
            select.join(", "),
            checked.sql,
            aliases.join(", "),
            SQL_MAX_ROWS + 1
        );
        let mut rows: Vec<Vec<Value>> = Vec::new();
        let mut truncated = false;
        (|| -> duckdb::Result<()> {
            let mut stmt = conn.prepare(&wrapped)?;
            let mut result = stmt.query([])?;
            let mut bytes = 0usize;
            while let Some(r) = result.next()? {
                if rows.len() >= SQL_MAX_ROWS || bytes > SQL_MAX_RESULT_BYTES {
                    truncated = true;
                    break;
                }
                let mut row = Vec::with_capacity(described.len());
                for i in 0..described.len() {
                    let v = cell(r.get::<_, Duck>(i)?);
                    bytes += cell_bytes(&v);
                    row.push(v);
                }
                rows.push(row);
            }
            Ok(())
        })()
        .map_err(|e| stopper.error(e, limit))?;
        drop(stopper);
        let event_ids = described
            .iter()
            .any(|(n, t)| n.eq_ignore_ascii_case("id") && INTEGERS.contains(&t.as_str()));
        let columns = described.into_iter().map(|(name, ty)| SqlColumn { name, ty }).collect();
        let total = rows.len() as u64;
        let first: Vec<Vec<Value>> = rows.iter().take(SQL_PAGE_ROWS).cloned().collect();
        let result = self.keep(rows);
        Ok(SqlResult {
            result,
            columns,
            rows: first,
            total,
            truncated,
            event_ids,
            plan: None,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

/// The wire name of an enum (the `fields` table's text columns).
fn wire_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl Console for DuckStore {
    fn sql_schema(&self) -> StoreResult<SqlSchema> {
        let mut guard = self.sandbox()?;
        let sandbox = guard
            .as_mut()
            .ok_or_else(|| StoreError::Engine("the SQL console is unavailable".into()))?;
        let mut stmt = sandbox
            .conn
            .prepare(
                "SELECT DISTINCT function_name FROM duckdb_functions() WHERE function_type IN ('scalar', 'aggregate', 'macro') ORDER BY 1",
            )
            .map_err(engine)?;
        let functions = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(engine)?
            .collect::<duckdb::Result<Vec<_>>>()
            .map_err(engine)?
            .into_iter()
            .filter(|f| f.chars().next().is_some_and(|c| c.is_ascii_alphabetic()))
            .collect();
        Ok(SqlSchema {
            version: SQL_VIEWS_VERSION,
            views: views::schema(),
            functions,
            table_functions: guard::TABLE_FUNCTIONS.iter().map(|f| f.to_string()).collect(),
            max_rows: SQL_MAX_ROWS,
            page_rows: SQL_PAGE_ROWS,
            time_limit_ms: SQL_TIME_LIMIT.as_millis() as u64,
        })
    }

    fn sql_run(&self, req: &SqlRequest) -> StoreResult<SqlResult> {
        let started = Instant::now();
        let mut guard = self.sandbox()?;
        let sandbox = guard
            .as_mut()
            .ok_or_else(|| StoreError::Engine("the SQL console is unavailable".into()))?;
        let mut checked = Self::check_sql(&sandbox.conn, &req.sql)?;
        checked.explain |= req.explain;
        self.fill(sandbox, &req.notes)?;
        self.run_sql(&sandbox.conn, &checked, started)
    }

    fn sql_page(&self, req: &SqlPageRequest) -> StoreResult<SqlPage> {
        let kept = lock(&self.inner.console.results).iter().find(|k| k.id == req.result).cloned();
        let kept =
            kept.ok_or_else(|| StoreError::NotFound("this result is gone (newer queries replaced it); run the query again".into()))?;
        Ok(SqlPage {
            offset: req.offset,
            rows: kept.rows.iter().skip(req.offset).take(SQL_PAGE_ROWS).cloned().collect(),
        })
    }
}

impl DuckStore {
    /// The sandbox, opened on first use; busy while another run holds it.
    fn sandbox(&self) -> StoreResult<std::sync::MutexGuard<'_, Option<Sandbox>>> {
        let mut guard = match self.inner.console.sandbox.try_lock() {
            Ok(g) => g,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(StoreError::Busy("a SQL query is running; stop it or wait for it".into()));
            }
        };
        if guard.is_none() {
            let segments = self.inner.dir.join(crate::segments::SEGMENTS_DIR);
            std::fs::create_dir_all(&segments)?;
            *guard = Some(Sandbox {
                conn: open_sandbox(&segments)?,
                filled: None,
            });
        }
        Ok(guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_json_scalars_cut_to_their_limit() {
        assert_eq!(cell(Duck::Null), Value::Null);
        assert_eq!(cell(Duck::Boolean(true)), Value::Bool(true));
        assert_eq!(cell(Duck::TinyInt(-1)), -1);
        assert_eq!(cell(Duck::SmallInt(2)), 2);
        assert_eq!(cell(Duck::Int(3)), 3);
        assert_eq!(cell(Duck::BigInt(4)), 4);
        assert_eq!(cell(Duck::UTinyInt(5)), 5);
        assert_eq!(cell(Duck::USmallInt(6)), 6);
        assert_eq!(cell(Duck::UInt(7)), 7);
        assert_eq!(cell(Duck::UBigInt(8)), 8);
        assert_eq!(cell(Duck::Float(0.5)), 0.5);
        assert_eq!(cell(Duck::Double(1.5)), 1.5);
        assert_eq!(cell(Duck::Double(f64::NAN)), "NaN");
        assert_eq!(cell(Duck::Float(f32::INFINITY)), "inf");
        assert_eq!(cell(Duck::Text("x".into())), "x");
        let long = cell(Duck::Text("é".repeat(MAX_CELL_CHARS + 5)));
        assert_eq!(long.as_str().unwrap().chars().count(), MAX_CELL_CHARS + 1);
        assert!(long.as_str().unwrap().ends_with('…'));
        assert!(cell(Duck::Blob(vec![1])).is_string());
        assert_eq!(cell_bytes(&Value::from("abc")), 11);
        assert_eq!(cell_bytes(&Value::Null), 8);
        assert_ne!(notes_hash(&[]), notes_hash(&[SqlNote::default()]));
    }

    /// Behind the checks: even a statement that got past them could not read or write files outside the segment
    /// folder, load or install an extension, or undo the sandbox.
    #[test]
    fn the_sandbox_holds_without_the_checks() {
        let dir = std::env::temp_dir().join(format!("imt-sql-sandbox-{}", std::process::id()));
        let seg = dir.join("segments");
        std::fs::create_dir_all(&seg).unwrap();
        let outside = dir.join("secret.csv");
        std::fs::write(&outside, "a\n1\n").unwrap();
        let conn = open_sandbox(&seg).unwrap();
        let fails = |sql: &str| {
            let e = conn.execute_batch(sql).expect_err(sql).to_string();
            assert!(!e.is_empty(), "{sql}");
            e
        };
        let p = sql_text(&outside);
        assert!(fails(&format!("SELECT * FROM read_csv('{p}')")).contains("disabled by configuration"));
        fails(&format!("COPY (SELECT 1) TO '{}'", sql_text(&dir.join("out.csv"))));
        fails(&format!("ATTACH '{}' AS x", sql_text(&dir.join("x.duckdb"))));
        fails("INSTALL httpfs");
        fails("LOAD httpfs");
        fails("SELECT * FROM read_csv('https://example.com/x.csv')");
        for setting in [
            "SET enable_external_access = true",
            "SET allowed_directories = ['/']",
            "SET memory_limit = '4GB'",
            "SET autoload_known_extensions = true",
            "SET lock_configuration = false",
            "RESET memory_limit",
        ] {
            assert!(fails(setting).contains("locked"), "{setting}");
        }
        assert!(!dir.join("out.csv").exists() && !dir.join("x.duckdb").exists());
        let setting = |name: &str| -> String {
            conn.query_row(&format!("SELECT current_setting('{name}')::VARCHAR"), [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(setting("enable_external_access"), "false");
        assert_eq!(setting("threads"), SQL_THREADS.to_string());
        assert_eq!(setting("lock_configuration"), "true");
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duckdb_messages_become_the_editors_errors() {
        assert!(matches!(query_error("Out of Memory Error: failed to allocate"), StoreError::Invalid(m) if m.contains("128MB")));
        assert!(
            matches!(query_error("Permission Error: Cannot access file \"x\" - file system operations are disabled by configuration"), StoreError::Query(q) if q.message.contains("turned off"))
        );
        assert!(
            matches!(query_error("Catalog Error: Table with name nope does not exist!\n\nLINE 1: select"), StoreError::Query(q) if q.message == "Catalog Error: Table with name nope does not exist!")
        );
    }
}
