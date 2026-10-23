//! The persistent catalogue of names ([`debrief_core::catalogue`]): field values and the entities found in message
//! and exception text, kept in the store's database so a lookup is a query over a few tables, not a scan of events.
//!
//! - Tables: `catalogue` (one row per source, scope and name: events, first and last seen), `catalogue_trigrams`
//!   (the three-letter groups of each name, for typo-tolerant lookups) and `catalogue_sources` (the sources
//!   catalogued, with the layout version they were built with).
//! - Built per source in the background (`debrief-catalogue` thread, one source per turn, its events read once through
//!   the `events` view): text scanned by [`debrief_core::entities::extract`], field values from the `extra` map. The
//!   rows of a source are written in one transaction under the store's write lock, and only while the source is still
//!   ready, so a lookup sees a source's names exactly when it sees its events.
//! - Lifecycle: rows are joined with the ready sources, so a re-import's old sources (still ready) keep answering until
//!   the new ones are ready; rows of sources that no longer exist are deleted on the next turn, and eagerly on clear,
//!   remove and refresh. A source built with another [`VERSION`] is built again.
//! - Bounds: see [`debrief_core::catalogue`]; one thread, one source in memory at a time.

use crate::{engine, lock};
use anyhow::Result;
use debrief_core::catalogue::*;
use debrief_core::entities::extract;
use debrief_core::ports::StoreResult;
use duckdb::{params, params_from_iter, Connection};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Layout and extraction version; sources built with another are built again.
pub const VERSION: u32 = 1;

/// Rows of the catalogue tables of `source` are removed with this.
pub(crate) const DELETE_SQL: [&str; 3] = [
    "DELETE FROM catalogue WHERE source = ?",
    "DELETE FROM catalogue_trigrams WHERE source = ?",
    "DELETE FROM catalogue_sources WHERE source = ?",
];

/// Removes the rows of the sources in `list` (comma-separated ids) or all rows with `None`.
pub(crate) fn forget(conn: &Connection, list: Option<&str>) -> duckdb::Result<()> {
    for t in ["catalogue", "catalogue_trigrams", "catalogue_sources"] {
        match list {
            Some(l) => conn.execute_batch(&format!("DELETE FROM {t} WHERE source IN ({l})"))?,
            None => conn.execute_batch(&format!("DELETE FROM {t}"))?,
        }
    }
    Ok(())
}

/// Sources that are ready and not catalogued with the current version.
fn pending(conn: &Connection) -> Result<Vec<u32>> {
    let mut stmt = conn
        .prepare("SELECT id FROM sources WHERE ready AND id NOT IN (SELECT source FROM catalogue_sources WHERE version = ?) ORDER BY id")?;
    let v = stmt
        .query_map(params![VERSION], |r| r.get::<_, u32>(0))?
        .collect::<duckdb::Result<Vec<_>>>()?;
    Ok(v)
}

/// Reads one source into a tally: entities of the text of its events, then the values of its low-cardinality fields.
fn build(conn: &Connection, source: u32) -> Result<(Vec<Entry>, u64, u64)> {
    let mut tally = Tally::default();
    {
        let mut stmt = conn.prepare("SELECT id, ts, message, exception FROM events WHERE source = ? LIMIT ?")?;
        let mut rows = stmt.query(params![source, MAX_EVENTS_SCANNED as u64])?;
        while let Some(r) = rows.next()? {
            let (id, ts): (u64, Option<i64>) = (r.get(0)?, r.get(1)?);
            let message: String = r.get(2)?;
            let exception: Option<String> = r.get(3)?;
            let mut texts = vec![message.as_str()];
            if let Some(e) = exception.as_deref() {
                texts.push(e);
            }
            let found = extract(&texts);
            tally.add_event(&found, id, ts);
        }
    }
    let sql = format!(
        "WITH kv AS (SELECT unnest(map_keys(extra)) AS k, unnest(map_values(extra)) AS v FROM events WHERE source = ?),
g AS (SELECT k, v, count(*) AS n FROM kv WHERE v IS NOT NULL AND length(v) BETWEEN 1 AND {} AND TRY_CAST(v AS DOUBLE) IS NULL GROUP BY k, v),
d AS (SELECT k FROM g GROUP BY k HAVING count(*) <= {MAX_FIELD_DISTINCT}),
r AS (SELECT g.k, g.v, g.n, row_number() OVER (PARTITION BY g.k ORDER BY g.n DESC, g.v) AS rk FROM g JOIN d USING (k))
SELECT k, v, n FROM r WHERE rk <= {MAX_VALUES_PER_FIELD} ORDER BY k, rk",
        debrief_core::entities::MAX_NAME_CHARS
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![source])?;
    while let Some(r) = rows.next()? {
        let (k, v, n): (String, String, u64) = (r.get(0)?, r.get(1)?, r.get(2)?);
        tally.add(Scope::Field(k), &v, 0, None, n);
    }
    let (dropped, guids) = (tally.dropped, tally.guids);
    Ok((tally.finish(), dropped, guids))
}

/// Writes the rows of a source (replacing older ones) if it is still ready; false when it is not.
fn commit(conn: &Connection, source: u32, entries: &[Entry], dropped: u64, guids: u64) -> Result<bool> {
    let ready: bool = conn.query_row("SELECT count(*) > 0 FROM sources WHERE id = ? AND ready", params![source], |r| {
        r.get(0)
    })?;
    if !ready {
        return Ok(false);
    }
    conn.execute_batch("BEGIN TRANSACTION")?;
    let written = (|| -> Result<()> {
        for sql in DELETE_SQL {
            conn.execute(sql, params![source])?;
        }
        let mut app = conn.appender("catalogue")?;
        for e in entries {
            app.append_row(params![
                source,
                e.scope.encode(),
                e.name,
                squash(&e.name),
                e.count,
                e.first_ts,
                e.last_ts,
                e.first_id,
                e.last_id
            ])?;
        }
        app.flush()?;
        let mut app = conn.appender("catalogue_trigrams")?;
        for e in entries {
            for t in trigram_keys(&squash(&e.name)) {
                app.append_row(params![source, t, e.scope.encode(), e.name])?;
            }
        }
        app.flush()?;
        conn.execute(
            "INSERT INTO catalogue_sources VALUES (?, ?, ?, ?, ?)",
            params![source, entries.len() as u64, dropped, guids, VERSION],
        )?;
        Ok(())
    })();
    match written {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(true)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

struct Shared {
    stop: AtomicBool,
    wake: (Mutex<bool>, Condvar),
    /// Sources not catalogued yet, as last seen.
    pending: AtomicU64,
    /// Sources whose build failed (not retried until the store is reopened).
    failed: Mutex<HashSet<u32>>,
}

/// The catalogue thread of a store; stopped and joined on drop.
pub(crate) struct CatalogueService {
    shared: Arc<Shared>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl CatalogueService {
    pub(crate) fn start(conn: Connection, write_lock: Arc<Mutex<()>>) -> Result<CatalogueService> {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            wake: (Mutex::new(true), Condvar::new()),
            pending: AtomicU64::new(0),
            failed: Mutex::new(HashSet::new()),
        });
        let s = shared.clone();
        let handle = std::thread::Builder::new()
            .name("debrief-catalogue".into())
            .spawn(move || run(&s, &conn, &write_lock))?;
        Ok(CatalogueService {
            shared,
            handle: Some(handle),
        })
    }

    pub(crate) fn wake(&self) {
        *lock(&self.shared.wake.0) = true;
        self.shared.wake.1.notify_all();
    }
}

impl Drop for CatalogueService {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.wake();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// One turn: tidy, then build the next source. True when a source was built (go on at once).
fn step(s: &Shared, conn: &Connection, write_lock: &Mutex<()>) -> Result<bool> {
    let todo = {
        let Ok(_writing) = write_lock.try_lock() else { return Ok(false) };
        conn.execute_batch(
            "DELETE FROM catalogue WHERE source NOT IN (SELECT id FROM sources);
             DELETE FROM catalogue_trigrams WHERE source NOT IN (SELECT id FROM sources);
             DELETE FROM catalogue_sources WHERE source NOT IN (SELECT id FROM sources);",
        )?;
        pending(conn)?
    };
    let failed = lock(&s.failed).clone();
    let todo: Vec<u32> = todo.into_iter().filter(|id| !failed.contains(id)).collect();
    s.pending.store(todo.len() as u64, Ordering::SeqCst);
    let Some(&source) = todo.first() else { return Ok(false) };
    let built = match build(conn, source) {
        Ok(b) => b,
        Err(e) => {
            // A source whose events cannot be read now (compaction swapped its files) is tried again later.
            lock(&s.failed).insert(source);
            return Err(e.context(format!("cataloguing source {source}")));
        }
    };
    let (entries, dropped, guids) = built;
    // The write lock is taken without waiting for an import: the next turn tries again.
    let Ok(_writing) = write_lock.try_lock() else { return Ok(false) };
    commit(conn, source, &entries, dropped, guids)?;
    s.pending.store(todo.len() as u64 - 1, Ordering::SeqCst);
    Ok(true)
}

fn run(s: &Shared, conn: &Connection, write_lock: &Mutex<()>) {
    while !s.stop.load(Ordering::SeqCst) {
        match step(s, conn, write_lock) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(e) => log::warn!("catalogue: {e:#}"),
        }
        let mut woken = lock(&s.wake.0);
        if !*woken {
            woken = s
                .wake
                .1
                .wait_timeout(woken, Duration::from_millis(500))
                .map(|r| r.0)
                .unwrap_or_else(|p| p.into_inner().0);
        }
        *woken = false;
    }
}

impl crate::Inner {
    /// Names that look like `req.text`, from the catalogue.
    pub(crate) fn catalogue(&self, req: &CatalogueRequest) -> StoreResult<CatalogueReply> {
        let conn = self.conn()?;
        let pending = pending(&conn).map_err(|e| engine(format!("{e:#}")))?.len() as u64;
        let text: String = req.text.trim().chars().take(debrief_core::find::MAX_TEXT_CHARS).collect();
        let keys = search_keys(&text);
        let squashed = squash(&text);
        let mut reply = CatalogueReply {
            built: pending == 0,
            pending,
            candidates: Vec::new(),
        };
        if squashed.is_empty() {
            return Ok(reply);
        }
        let (sql, args): (String, Vec<String>) = if keys.is_empty() {
            (
                format!(
                    "SELECT e.scope, e.name, sum(e.n), min(e.first_ts), max(e.last_ts), list(DISTINCT s.name)
                     FROM catalogue e JOIN sources s ON s.id = e.source AND s.ready
                     WHERE e.norm LIKE ? ESCAPE '\\' GROUP BY e.scope, e.name ORDER BY sum(e.n) DESC, e.name LIMIT {MAX_CANDIDATES}"
                ),
                vec![format!(
                    "{}%",
                    squashed.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
                )],
            )
        } else {
            let marks = vec!["?"; keys.len()].join(", ");
            (
                format!(
                    "WITH c AS (SELECT scope, name, count(DISTINCT tri) AS ov FROM catalogue_trigrams
                         WHERE source IN (SELECT id FROM sources WHERE ready) AND tri IN ({marks})
                         GROUP BY scope, name ORDER BY ov DESC, name LIMIT {MAX_CANDIDATES})
                     SELECT e.scope, e.name, sum(e.n), min(e.first_ts), max(e.last_ts), list(DISTINCT s.name)
                     FROM c JOIN catalogue e ON e.scope = c.scope AND e.name = c.name
                     JOIN sources s ON s.id = e.source AND s.ready
                     GROUP BY e.scope, e.name, c.ov ORDER BY c.ov DESC, sum(e.n) DESC, e.name"
                ),
                keys,
            )
        };
        let mut stmt = conn.prepare(&sql).map_err(engine)?;
        let mut rows = stmt.query(params_from_iter(args.iter())).map_err(engine)?;
        while let Some(r) = rows.next().map_err(engine)? {
            let scope: String = r.get(0).map_err(engine)?;
            let Some(scope) = Scope::decode(&scope) else { continue };
            let seen: duckdb::types::Value = r.get(5).map_err(engine)?;
            let mut seen_in = match seen {
                duckdb::types::Value::List(l) => l
                    .into_iter()
                    .filter_map(|v| if let duckdb::types::Value::Text(t) = v { Some(t) } else { None })
                    .collect(),
                _ => Vec::new(),
            };
            seen_in.sort();
            seen_in.truncate(MAX_SEEN_IN);
            reply.candidates.push(Candidate {
                scope,
                name: r.get(1).map_err(engine)?,
                count: r.get::<_, f64>(2).map_err(engine)? as u64,
                first_ts: r.get(3).map_err(engine)?,
                last_ts: r.get(4).map_err(engine)?,
                seen_in,
            });
        }
        Ok(reply)
    }
}

impl crate::DuckStore {
    /// Waits until every ready source is catalogued (tests and benchmarks), at most `timeout`; true when it is.
    pub fn wait_for_catalogue(&self, timeout: Duration) -> StoreResult<bool> {
        let started = std::time::Instant::now();
        loop {
            let reply = self.inner.catalogue(&CatalogueRequest { text: String::new() })?;
            if reply.built {
                return Ok(true);
            }
            if started.elapsed() > timeout {
                return Ok(false);
            }
            if let Some(c) = &self.inner.catalogue {
                c.wake();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
