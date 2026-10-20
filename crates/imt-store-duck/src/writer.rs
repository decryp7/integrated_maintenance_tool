//! The single writer: assigns event and source ids and appends batches with the Arrow appender.
//!
//! Commits are expensive on a file database (45–580 ms each on the test VM, measured; the Arrow append
//! itself runs at about 240k events/s in one transaction), so writes are grouped: one transaction is
//! committed after [`COMMIT_BATCHES`] batches or [`COMMIT_EVERY`], whichever comes first, across source
//! boundaries. Readers therefore see an import's progress about once a second. An aborted source's rows
//! are deleted (inside or after the transaction). Ids are never reused: the next ids are kept in `meta`.

use anyhow::{Context, Result};
use duckdb::{params, Connection};
use imt_core::batch::{EventId, PendingBatch, SourceId};
use imt_core::ingest::{BatchSink, NewSource, SourceReport};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Batches appended per transaction at most (about 260k events).
pub const COMMIT_BATCHES: usize = 32;

/// Longest a transaction stays open while an import runs.
pub const COMMIT_EVERY: Duration = Duration::from_secs(1);

/// An open transaction.
struct Open {
    since: Instant,
    batches: usize,
}

/// Writes one import into the database. Only one exists at a time per store; call
/// [`DuckWriter::finish`] at the end (dropping it also commits what was finished).
pub struct DuckWriter<'a> {
    conn: &'a Connection,
    next_id: u64,
    next_source: u32,
    open: Option<Open>,
    /// The source being written, until it is finished or aborted.
    current: Option<SourceId>,
    /// Bumped whenever the set of events changes (read caches are keyed by it).
    generation: &'a AtomicU64,
}

fn meta_u64(conn: &Connection, key: &str) -> u64 {
    conn.query_row("SELECT value FROM meta WHERE key = ?", params![key], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

impl<'a> DuckWriter<'a> {
    /// A writer continuing after the ids already used in the database.
    pub fn new(conn: &'a Connection, generation: &'a AtomicU64) -> Result<DuckWriter<'a>> {
        let (max_id, max_source) = conn.query_row(
            "SELECT (SELECT coalesce(max(id) + 1, 0) FROM events), (SELECT coalesce(max(id) + 1, 0) FROM sources)",
            [],
            |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u32>(1)?)),
        )?;
        let next_source = u32::try_from(meta_u64(conn, "next_source")).unwrap_or(u32::MAX).max(max_source);
        Ok(DuckWriter {
            conn,
            next_id: meta_u64(conn, "next_id").max(max_id),
            next_source,
            open: None,
            current: None,
            generation,
        })
    }

    fn begin(&mut self) -> Result<&mut Open> {
        if self.open.is_none() {
            self.conn.execute_batch("BEGIN TRANSACTION")?;
        }
        Ok(self.open.get_or_insert_with(|| Open {
            since: Instant::now(),
            batches: 0,
        }))
    }

    fn commit(&mut self) -> Result<()> {
        if self.open.is_some() {
            for (k, v) in [("next_id", self.next_id), ("next_source", self.next_source as u64)] {
                self.conn
                    .execute("INSERT OR REPLACE INTO meta VALUES (?, ?)", params![k, v.to_string()])?;
            }
            self.conn.execute_batch("COMMIT")?;
            self.open = None;
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    fn maybe_commit(&mut self) -> Result<()> {
        if self
            .open
            .as_ref()
            .is_some_and(|o| o.batches >= COMMIT_BATCHES || o.since.elapsed() >= COMMIT_EVERY)
        {
            self.commit()?;
        }
        Ok(())
    }

    fn delete_source(&self, id: SourceId) -> Result<()> {
        self.conn.execute("DELETE FROM events WHERE source = ?", params![id.0])?;
        self.conn.execute("DELETE FROM diagnostics WHERE source = ?", params![id.0])?;
        self.conn.execute("DELETE FROM sources WHERE id = ?", params![id.0])?;
        Ok(())
    }

    /// Commits everything written.
    pub fn finish(mut self) -> Result<()> {
        if let Some(id) = self.current.take() {
            self.delete_source(id)?;
        }
        self.commit()
    }
}

impl BatchSink for DuckWriter<'_> {
    fn begin_source(&mut self, source: &NewSource) -> Result<SourceId> {
        self.begin()?;
        let id = SourceId(self.next_source);
        self.conn.execute(
            "INSERT INTO sources (id, path, name) VALUES (?, ?, ?)",
            params![id.0, source.path, source.name],
        )?;
        self.next_source += 1;
        self.current = Some(id);
        Ok(id)
    }

    fn write(&mut self, batch: PendingBatch) -> Result<()> {
        let n = batch.len() as u64;
        let batch = batch.assign_ids(EventId(self.next_id))?;
        self.begin()?;
        {
            let mut app = self.conn.appender("events")?;
            app.append_record_batch(batch.into_record_batch()).context("appending events")?;
            app.flush()?;
        }
        self.next_id += n;
        self.begin()?.batches += 1;
        self.maybe_commit()
    }

    fn finish_source(&mut self, r: &SourceReport) -> Result<()> {
        self.begin()?;
        self.conn.execute(
            "UPDATE sources SET importer = ?, file_kind = ?, events = ?, skipped = ?, dropped = ?, ts_from = ?, ts_to = ?, bytes = ?, ready = TRUE WHERE id = ?",
            params![
                r.importer,
                format!("{:?}", r.file_kind),
                r.events as u64,
                r.skipped as u64,
                r.dropped as u64,
                r.from,
                r.to,
                r.bytes,
                r.id.0
            ],
        )?;
        {
            let mut app = self.conn.appender("diagnostics")?;
            for p in &r.problems {
                let samples = serde_json::to_string(&p.sample_lines)?;
                app.append_row(params![r.id.0, p.kind.id(), format!("{:?}", p.kind.severity()), p.count, samples])?;
            }
            app.flush()?;
        }
        self.current = None;
        self.maybe_commit()
    }

    fn abort_source(&mut self, id: SourceId) -> Result<()> {
        self.delete_source(id)?;
        if self.current == Some(id) {
            self.current = None;
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl Drop for DuckWriter<'_> {
    fn drop(&mut self) {
        // A writer dropped without `finish` (an error or a crash in the import): keep what was finished,
        // remove the source in progress, leave no transaction open on the pooled connection.
        if let Some(id) = self.current.take() {
            if let Err(e) = self.delete_source(id) {
                log::warn!("could not remove a partly written source: {e:#}");
            }
        }
        if let Err(e) = self.commit() {
            log::warn!("could not commit the import: {e:#}");
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::batch::{EventBatchBuilder, EventSchema, EventTime, LevelCode, NewEvent};
    use std::sync::Arc;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(&crate::schema::ddl(&EventSchema::plain())).unwrap();
        c
    }

    fn batch(source: SourceId, n: usize) -> PendingBatch {
        let mut b = EventBatchBuilder::new(Arc::new(EventSchema::plain()));
        for i in 0..n {
            let ev = NewEvent {
                time: EventTime::Parsed(i as i64),
                level: LevelCode::new(2).unwrap(),
                source,
                line: i as u32 + 1,
                byte_offset: 0,
                message: "m",
                template: None,
                exception: None,
                markers: 0,
                raw: "m",
            };
            b.push(&ev, [("K", imt_core::batch::FieldValue::Text("v"))]).unwrap();
        }
        b.finish()
    }

    fn report(id: SourceId, events: usize) -> SourceReport {
        SourceReport {
            id,
            path: "p".into(),
            name: "n".into(),
            importer: "text".into(),
            file_kind: imt_core::api::FileKind::Text,
            events,
            skipped: 0,
            dropped: 0,
            step_stats: Vec::new(),
            from: Some(0),
            to: Some(1),
            bytes: 10,
            problems: vec![imt_core::ingest::Problem {
                kind: imt_core::ingest::ProblemKind::LineTooLong,
                count: 1,
                sample_lines: vec![3],
            }],
        }
    }

    fn count(c: &Connection, sql: &str) -> u64 {
        c.query_row(sql, [], |r| r.get::<_, u64>(0)).unwrap()
    }

    #[test]
    fn writes_commits_aborts_and_never_reuses_ids() {
        let c = conn();
        let generation = AtomicU64::new(0);
        let src = NewSource {
            path: "a".into(),
            name: "a".into(),
        };
        let mut w = DuckWriter::new(&c, &generation).unwrap();
        let a = w.begin_source(&src).unwrap();
        for _ in 0..COMMIT_BATCHES + 1 {
            w.write(batch(a, 3)).unwrap();
        }
        assert!(generation.load(Ordering::SeqCst) >= 1, "committed after {COMMIT_BATCHES} batches");
        w.finish_source(&report(a, 3 * (COMMIT_BATCHES + 1))).unwrap();
        // Aborted after a commit in its middle: nothing of it stays.
        let b = w.begin_source(&src).unwrap();
        for _ in 0..COMMIT_BATCHES + 1 {
            w.write(batch(b, 2)).unwrap();
        }
        w.abort_source(b).unwrap();
        w.finish().unwrap();
        assert_eq!(count(&c, "SELECT count(*) FROM events"), 3 * (COMMIT_BATCHES as u64 + 1));
        assert_eq!(count(&c, "SELECT count(*) FROM diagnostics"), 1);
        assert_eq!(count(&c, "SELECT count(*) FROM sources"), 1);
        // Ids continue after the aborted source's.
        let mut w = DuckWriter::new(&c, &generation).unwrap();
        let next = w.begin_source(&src).unwrap();
        assert!(next.0 > b.0, "{next:?} after {b:?}");
        w.write(batch(next, 1)).unwrap();
        w.finish_source(&report(next, 1)).unwrap();
        w.finish().unwrap();
        let id = count(&c, &format!("SELECT id FROM events WHERE source = {}", next.0));
        assert!(id >= 3 * (COMMIT_BATCHES as u64 + 1) + 2 * COMMIT_BATCHES as u64);
        // A writer dropped mid-source keeps what was finished and drops the rest.
        let mut w = DuckWriter::new(&c, &generation).unwrap();
        let e = w.begin_source(&src).unwrap();
        w.write(batch(e, 1)).unwrap();
        w.finish_source(&report(e, 1)).unwrap();
        let d = w.begin_source(&src).unwrap();
        w.write(batch(d, 1)).unwrap();
        drop(w);
        assert_eq!(count(&c, &format!("SELECT count(*) FROM events WHERE source = {}", d.0)), 0);
        assert_eq!(count(&c, &format!("SELECT count(*) FROM events WHERE source = {}", e.0)), 1);
        // The connection has no transaction left open.
        c.execute_batch("BEGIN TRANSACTION; COMMIT").unwrap();
    }
}
