//! Time-order positions (ADR 0014 R2b): `ord(pos, id)` numbers every visible event in time order
//! (`coalesce(ts, i64::MAX), source, line, id`, untimed last), stored sorted by `pos` so DuckDB's zone maps
//! turn a position range into a short scan. It is rebuilt at the end of every import and after a source is
//! removed; events imported since have no position until then ([`Positions::positioned`] tells).

use crate::segments::SEGMENTS_DIR;
use crate::{engine, lock, DuckStore};
use duckdb::arrow::array::{RecordBatch, StructArray};
use duckdb::{params, Connection};
use imt_core::batch::{EventBatch, EventId, BATCH_ROWS};
use imt_core::ports::{Positions, StoreResult, MAX_PAGE_ROWS};

/// Renumbers the visible events.
pub(crate) fn rebuild(conn: &Connection) -> duckdb::Result<()> {
    conn.execute_batch(
        "BEGIN TRANSACTION;
DELETE FROM ord;
INSERT INTO ord SELECT (row_number() OVER (ORDER BY coalesce(ts, 9223372036854775807), source, line, id) - 1)::UBIGINT AS pos, id FROM events ORDER BY pos;
COMMIT;",
    )
    .inspect_err(|_| {
        let _ = conn.execute_batch("ROLLBACK");
    })
}

impl Positions for DuckStore {
    fn ids_at(&self, from: u64, len: usize) -> StoreResult<Vec<EventId>> {
        let conn = self.inner.conn()?;
        let len = len.min(MAX_PAGE_ROWS) as u64;
        let mut stmt = conn
            .prepare("SELECT id FROM ord WHERE pos >= ? AND pos < ? ORDER BY pos")
            .map_err(engine)?;
        let rows = stmt
            .query_map(params![from, from.saturating_add(len)], |r| r.get::<_, u64>(0))
            .map_err(engine)?;
        rows.map(|r| r.map(EventId).map_err(engine)).collect()
    }

    fn position_of(&self, id: EventId) -> StoreResult<Option<u64>> {
        let conn = self.inner.conn()?;
        match conn.query_row("SELECT pos FROM ord WHERE id = ?", params![id.0], |r| r.get::<_, u64>(0)) {
            Ok(p) => Ok(Some(p)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(engine(e)),
        }
    }

    fn positioned(&self) -> StoreResult<u64> {
        let conn = self.inner.conn()?;
        conn.query_row("SELECT count(*) FROM ord", [], |r| r.get::<_, u64>(0))
            .map_err(engine)
    }
}

impl DuckStore {
    /// The visible events with these ids, in id order, as batches (the second phase of a page: positions
    /// or search give the ids, this fetches the rows). Only the segments whose id blocks hold the ids are
    /// read; at most [`MAX_PAGE_ROWS`] ids.
    pub fn rows(&self, ids: &[EventId]) -> StoreResult<Vec<EventBatch>> {
        let mut ids: Vec<u64> = ids.iter().map(|i| i.0).take(MAX_PAGE_ROWS).collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.inner.conn()?;
        let seg_dir = self.inner.dir.join(SEGMENTS_DIR);
        let mut files = Vec::new();
        {
            let mut stmt = conn
                .prepare("SELECT file, first_id, end_id FROM segments ORDER BY first_id")
                .map_err(engine)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?, r.get::<_, u64>(2)?)))
                .map_err(engine)?;
            for row in rows {
                let (file, first, end) = row.map_err(engine)?;
                let at = ids.partition_point(|i| *i < first);
                if ids.get(at).is_some_and(|i| *i < end) {
                    files.push(format!("'{}'", seg_dir.join(file).to_string_lossy().replace('\'', "''")));
                }
            }
        }
        if files.is_empty() {
            return Ok(Vec::new());
        }
        let list: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
        let sql = format!(
            "SELECT * FROM read_parquet([{}]) WHERE id IN ({}) AND source IN (SELECT id FROM sources WHERE ready) ORDER BY id",
            files.join(", "),
            list.join(", ")
        );
        let mut stmt = conn.prepare(&sql).map_err(engine)?;
        let _ = stmt.stream_arrow([]).map_err(engine)?;
        let mut out = Vec::new();
        while let Some(chunk) = stmt.step().map_err(engine)? {
            let rb = RecordBatch::from(&chunk as &StructArray);
            let mut offset = 0;
            while offset < rb.num_rows() {
                let len = (rb.num_rows() - offset).min(BATCH_ROWS);
                out.push(EventBatch::try_new(self.inner.schema.clone(), rb.slice(offset, len)).map_err(engine)?);
                offset += len;
            }
        }
        Ok(out)
    }

    /// Runs a read-only SQL statement and counts its rows (benchmarks of queries not yet behind a port).
    #[doc(hidden)]
    pub fn bench_sql(&self, sql: &str) -> StoreResult<u64> {
        let conn = self.inner.conn()?;
        let mut stmt = conn.prepare(sql).map_err(engine)?;
        let mut rows = stmt.query([]).map_err(engine)?;
        let mut n = 0;
        while rows.next().map_err(engine)?.is_some() {
            n += 1;
        }
        Ok(n)
    }

    /// Renumbers the events now (normally done by imports and removals).
    pub fn rebuild_positions(&self) -> StoreResult<()> {
        let _writing = lock(&self.inner.write_lock);
        let conn = self.inner.conn()?;
        rebuild(&conn).map_err(engine)
    }
}
