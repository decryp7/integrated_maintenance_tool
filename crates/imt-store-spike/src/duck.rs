//! DuckDB adapter: one `events` table with typed columns (fixed, promoted, residual JSON), filled
//! with the appender, queried with parameterised SQL. The full-text index lives next to it.

use crate::corpus::Chunk;
use crate::fts::{Finished, Fts, FtsDoc, Indexer};
use crate::port::{Column, ColumnKind, FacetOut, Filter, PageOut, SearchOut, SpikeStore, StatsOut, TimelineOut, HISTOGRAM_BINS};
use crate::rows::{promoted, rest_json, Cell};
use anyhow::{Context, Result};
use duckdb::types::Value;
use duckdb::{params, params_from_iter, AccessMode, Config, Connection};
use std::path::Path;
use std::time::Duration;

use crate::port::{MEMORY_LIMIT, THREADS};

const SCHEMA: &str = "
CREATE TABLE sources (id UINTEGER PRIMARY KEY, path VARCHAR NOT NULL, name VARCHAR NOT NULL);
CREATE TABLE events (
  id UBIGINT NOT NULL, ts BIGINT, level UTINYINT NOT NULL, source UINTEGER NOT NULL, line UINTEGER NOT NULL, markers UBIGINT NOT NULL,
  machine_name VARCHAR, process_id BIGINT, source_context VARCHAR, status_code BIGINT, elapsed DOUBLE,
  message VARCHAR NOT NULL, template VARCHAR, exception VARCHAR, fields VARCHAR
);";

const ROW_COLUMNS: &str = "id, ts, level, message, template, exception, machine_name, process_id, source_context, fields";

fn db_path(dir: &Path) -> std::path::PathBuf {
    dir.join("events.duckdb")
}

fn settings(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "SET threads = {THREADS}; SET memory_limit = '{MEMORY_LIMIT}'; SET preserve_insertion_order = true;"
    ))?;
    Ok(())
}

fn text(c: &Cell<'_>) -> Option<String> {
    match c {
        Cell::Text(s) => Some((*s).to_string()),
        _ => None,
    }
}
fn int(c: &Cell<'_>) -> Option<i64> {
    match c {
        Cell::Integer(i) => Some(*i),
        _ => None,
    }
}
fn float(c: &Cell<'_>) -> Option<f64> {
    match c {
        Cell::Float(f) => Some(*f),
        _ => None,
    }
}

/// Writes a new store: the table, then each chunk through the appender, while the full-text index
/// is built on its own thread.
pub struct DuckIngest {
    conn: Connection,
    fts: Indexer,
}

impl DuckIngest {
    pub fn create(dir: &Path) -> Result<DuckIngest> {
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        std::fs::create_dir_all(dir)?;
        let conn = Connection::open(db_path(dir))?;
        settings(&conn)?;
        conn.execute_batch(SCHEMA)?;
        Ok(DuckIngest {
            conn,
            fts: Indexer::start(&dir.join("fts"))?,
        })
    }

    pub fn append(&mut self, chunk: Chunk<'_>) -> Result<()> {
        {
            let mut app = self.conn.appender("sources")?;
            for s in &chunk.sources {
                app.append_row(params![s.id, s.path, s.name])?;
            }
        }
        let mut docs = Vec::with_capacity(chunk.records.len());
        {
            let mut app = self.conn.appender("events")?;
            for r in chunk.records {
                let id = chunk.id_base + r.id as u64;
                let p = promoted(r);
                let rest = rest_json(r);
                app.append_row(params![
                    id,
                    r.ts,
                    r.level.index() as u8,
                    chunk.source_base + r.source,
                    r.line,
                    r.markers,
                    text(&p[0]),
                    int(&p[1]),
                    text(&p[2]),
                    int(&p[3]),
                    float(&p[4]),
                    r.message,
                    r.template,
                    r.exception,
                    (!rest.is_empty()).then_some(rest),
                ])?;
                docs.push(FtsDoc {
                    id,
                    ts: r.ts.unwrap_or(i64::MAX),
                    message: r.message.clone(),
                    exception: r.exception.clone(),
                });
            }
            app.flush()?;
        }
        self.fts.send(docs)
    }

    /// Checkpoints the table and waits for the full-text index.
    pub fn finish(self, started: std::time::Instant) -> Result<Finished> {
        self.conn.execute_batch("CHECKPOINT;")?;
        let table_ms = started.elapsed().as_millis();
        let indexed_at_table = self.fts.progress();
        drop(self.conn);
        let report = self.fts.finish()?;
        Ok(Finished {
            table_ms,
            indexed_at_table,
            total_ms: started.elapsed().as_millis(),
            indexed: report.docs,
            cancelled: report.cancelled,
        })
    }
}

/// An opened (read-only) store.
pub struct DuckStore {
    conn: Connection,
    fts: Fts,
    pub total: u64,
    pub from: i64,
    pub to: i64,
}

impl DuckStore {
    /// The full-text index (for its progress after reopening).
    pub fn fts(&self) -> &Fts {
        &self.fts
    }

    pub fn open(dir: &Path) -> Result<DuckStore> {
        let config = Config::default().access_mode(AccessMode::ReadOnly)?.enable_external_access(false)?;
        let conn = Connection::open_with_flags(db_path(dir), config).with_context(|| format!("opening {}", dir.display()))?;
        settings(&conn)?;
        conn.execute_batch("SET lock_configuration = true;")?;
        let (total, from, to) = conn.query_row("SELECT count(*), coalesce(min(ts), 0), coalesce(max(ts), 0) FROM events", [], |r| {
            Ok((r.get::<_, u64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?;
        Ok(DuckStore {
            conn,
            fts: Fts::open(&dir.join("fts"))?,
            total,
            from,
            to,
        })
    }

    fn count(&self, w: &Where) -> Result<u64> {
        Ok(self.conn.query_row(
            &format!("SELECT count(*) FROM events {}", w.sql),
            params_from_iter(w.args.iter()),
            |r| r.get(0),
        )?)
    }

    fn rows(&self, sql: &str, args: &[Value]) -> Result<PageOut> {
        let mut stmt = self.conn.prepare(sql)?;
        let mut ids = Vec::new();
        let mut bytes = 0usize;
        let mut rows = stmt.query(params_from_iter(args.iter()))?;
        while let Some(row) = rows.next()? {
            ids.push(row.get::<_, u64>(0)?);
            let message: String = row.get(3)?;
            let _fields: Option<String> = row.get(9)?;
            bytes += message.len();
        }
        Ok(PageOut {
            total: 0,
            ids,
            message_bytes: bytes,
        })
    }

    /// A query from the SQL console: read-only (the connection is), no file or network access, at
    /// most `max_rows` rows, interrupted after `timeout`.
    pub fn console(&self, sql: &str, timeout: Duration, max_rows: usize) -> Result<Vec<Vec<String>>> {
        let handle = self.conn.interrupt_handle();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let d = done.clone();
        let timer = std::thread::spawn(move || {
            let until = std::time::Instant::now() + timeout;
            while std::time::Instant::now() < until {
                if d.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            handle.interrupt();
        });
        let run = || -> Result<Vec<Vec<String>>> {
            let mut stmt = self.conn.prepare(sql)?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                if out.len() == max_rows {
                    break;
                }
                let n = row.as_ref().column_count();
                let mut cells = Vec::with_capacity(n);
                for i in 0..n {
                    let v: Value = row.get(i)?;
                    cells.push(format!("{v:?}"));
                }
                out.push(cells);
            }
            Ok(out)
        };
        let out = run();
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = timer.join();
        out
    }
}

/// A WHERE clause with its parameters.
pub struct Where {
    pub sql: String,
    pub args: Vec<Value>,
}

/// Compiles a filter to a parameterised WHERE clause (empty for no criteria). User text is only
/// ever a parameter, never spliced into the SQL.
pub fn where_clause(f: &Filter) -> Where {
    let mut parts = Vec::new();
    let mut args = Vec::new();
    if !f.levels.is_empty() {
        parts.push(format!("level IN ({})", vec!["?"; f.levels.len()].join(", ")));
        args.extend(f.levels.iter().map(|l| Value::UTinyInt(l.index() as u8)));
    }
    if let Some(t) = &f.text {
        parts.push("(message ILIKE ? ESCAPE '\\' OR exception ILIKE ? ESCAPE '\\' OR template ILIKE ? ESCAPE '\\')".into());
        let p = Filter::like_pattern(t.trim());
        args.extend([Value::Text(p.clone()), Value::Text(p.clone()), Value::Text(p)]);
    }
    if let Some((c, v)) = &f.field {
        let value = match c.kind() {
            ColumnKind::Text => Some(Value::Text(v.clone())),
            ColumnKind::Integer => v.trim().parse().ok().map(Value::BigInt),
            ColumnKind::Float => v.trim().parse().ok().map(Value::Double),
        };
        match value {
            Some(value) => {
                parts.push(format!("{} = ?", c.column()));
                args.push(value);
            }
            // A value that cannot be of the column's type matches nothing.
            None => parts.push("FALSE".into()),
        }
    }
    let sql = if parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", parts.join(" AND "))
    };
    Where { sql, args }
}

/// `w` plus one more condition.
fn and(w: &Where, cond: &str) -> String {
    if w.sql.is_empty() {
        format!("WHERE {cond}")
    } else {
        format!("{} AND {cond}", w.sql)
    }
}

impl SpikeStore for DuckStore {
    fn page(&self, filter: &Filter, offset: u64, limit: usize) -> Result<PageOut> {
        if filter.is_empty() {
            // Ids are time-ordered ordinals: an unfiltered page is an id range (zone-map pruned).
            let mut p = self.rows(
                &format!("SELECT {ROW_COLUMNS} FROM events WHERE id >= ? AND id < ? ORDER BY id"),
                &[Value::UBigInt(offset), Value::UBigInt(offset + limit as u64)],
            )?;
            p.total = self.total;
            return Ok(p);
        }
        let w = where_clause(filter);
        let total = self.count(&w)?;
        let mut args = w.args.clone();
        args.extend([Value::BigInt(limit as i64), Value::BigInt(offset as i64)]);
        let mut p = self.rows(
            &format!("SELECT {ROW_COLUMNS} FROM events {} ORDER BY id LIMIT ? OFFSET ?", w.sql),
            &args,
        )?;
        p.total = total;
        Ok(p)
    }

    fn facet(&self, column: Column, filter: &Filter, limit: usize) -> Result<FacetOut> {
        let w = where_clause(filter);
        let c = column.column();
        let (total, distinct) = self.conn.query_row(
            &format!("SELECT count({c}), count(DISTINCT {c}) FROM events {}", w.sql),
            params_from_iter(w.args.iter()),
            |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?)),
        )?;
        let mut args = w.args.clone();
        args.push(Value::BigInt(limit as i64));
        let sql = format!(
            "SELECT CAST({c} AS VARCHAR), count(*) AS n FROM events {} GROUP BY {c} ORDER BY n DESC, 1 LIMIT ?",
            and(&w, &format!("{c} IS NOT NULL"))
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let top = stmt
            .query_map(params_from_iter(args.iter()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(FacetOut { total, distinct, top })
    }

    fn timeline(&self, filter: &Filter, buckets: usize) -> Result<TimelineOut> {
        let w = where_clause(filter);
        let (from, to) = (self.from, self.to);
        let span = (to - from).max(1);
        let mut args = vec![Value::BigInt(from), Value::BigInt(buckets as i64), Value::BigInt(span)];
        args.extend(w.args.iter().cloned());
        let sql = format!(
            "SELECT (ts - ?) * ? // ? AS b, level, count(*) FROM events {} GROUP BY ALL",
            and(&w, "ts IS NOT NULL")
        );
        let mut histogram = vec![[0u64; 6]; buckets];
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params_from_iter(args.iter()))?;
        while let Some(r) = rows.next()? {
            let b: i64 = r.get(0)?;
            let level: u8 = r.get(1)?;
            let n: u64 = r.get(2)?;
            histogram[(b.max(0) as usize).min(buckets - 1)][level as usize % 6] += n;
        }
        let lanes: u64 = self.conn.query_row(
            "SELECT count(*) FROM (SELECT DISTINCT machine_name, process_id, \
             CASE WHEN machine_name IS NULL AND process_id IS NULL THEN source END FROM events)",
            [],
            |r| r.get(0),
        )?;
        Ok(TimelineOut {
            from,
            to,
            histogram,
            lanes: lanes as usize,
        })
    }

    fn field_stats(&self, column: Column, filter: &Filter) -> Result<StatsOut> {
        let w = where_clause(filter);
        let c = column.column();
        let sql = format!(
            "SELECT count({c}), count(*) - count({c}), count(DISTINCT {c}), min({c})::DOUBLE, max({c})::DOUBLE, avg({c})::DOUBLE, \
             approx_quantile({c}, 0.5)::DOUBLE, approx_quantile({c}, 0.95)::DOUBLE, approx_quantile({c}, 0.99)::DOUBLE FROM events {}",
            w.sql
        );
        let mut s = self.conn.query_row(&sql, params_from_iter(w.args.iter()), |r| {
            Ok(StatsOut {
                count: r.get(0)?,
                missing: r.get(1)?,
                distinct: r.get(2)?,
                min: r.get::<_, Option<f64>>(3)?.unwrap_or(f64::NAN),
                max: r.get::<_, Option<f64>>(4)?.unwrap_or(f64::NAN),
                mean: r.get::<_, Option<f64>>(5)?.unwrap_or(f64::NAN),
                p50: r.get::<_, Option<f64>>(6)?.unwrap_or(f64::NAN),
                p95: r.get::<_, Option<f64>>(7)?.unwrap_or(f64::NAN),
                p99: r.get::<_, Option<f64>>(8)?.unwrap_or(f64::NAN),
                histogram: Vec::new(),
            })
        })?;
        if s.count > 0 && column.kind() != ColumnKind::Text {
            let width = ((s.max - s.min) / HISTOGRAM_BINS as f64).max(f64::MIN_POSITIVE);
            let mut args = vec![Value::Double(s.min), Value::Double(width)];
            args.extend(w.args.iter().cloned());
            let sql = format!(
                "SELECT CAST(floor(({c} - ?) / ?) AS BIGINT) AS b, count(*) FROM events {} GROUP BY b",
                and(&w, &format!("{c} IS NOT NULL"))
            );
            let mut hist = vec![0u64; HISTOGRAM_BINS];
            let mut stmt = self.conn.prepare(&sql)?;
            let mut rows = stmt.query(params_from_iter(args.iter()))?;
            while let Some(r) = rows.next()? {
                let b: i64 = r.get(0)?;
                hist[(b.max(0) as usize).min(HISTOGRAM_BINS - 1)] += r.get::<_, u64>(1)?;
            }
            s.histogram = hist;
        }
        // The top values, as the field-stats panel shows them.
        let top = self.facet(column, filter, 20)?;
        let _ = top;
        Ok(s)
    }

    fn search(&self, text: &str, limit: usize) -> Result<SearchOut> {
        let (hits, ids) = self.fts.search(text, limit)?;
        if ids.is_empty() {
            return Ok(SearchOut { hits, ids, fetched: 0 });
        }
        let list = ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        let p = self.rows(&format!("SELECT {ROW_COLUMNS} FROM events WHERE id IN ({list}) ORDER BY id"), &[])?;
        Ok(SearchOut {
            hits,
            fetched: p.ids.len(),
            ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::model::Level;

    #[test]
    fn filters_compile_to_parameterised_sql() {
        let w = where_clause(&Filter::default());
        assert!(w.sql.is_empty() && w.args.is_empty());
        let f = Filter {
            levels: vec![Level::Warning, Level::Error],
            text: Some("50%_off'; DROP TABLE events; --".into()),
            field: Some((Column::ProcessId, "3312".into())),
        };
        let w = where_clause(&f);
        assert_eq!(
            w.sql,
            "WHERE level IN (?, ?) AND (message ILIKE ? ESCAPE '\\' OR exception ILIKE ? ESCAPE '\\' OR template ILIKE ? ESCAPE '\\') AND process_id = ?"
        );
        assert_eq!(w.args.len(), 6);
        assert_eq!(w.args[2], Value::Text("%50\\%\\_off'; DROP TABLE events; --%".into()));
        assert_eq!(w.args[5], Value::BigInt(3312));
        let w = where_clause(&Filter {
            field: Some((Column::ProcessId, "abc".into())),
            ..Filter::default()
        });
        assert_eq!(w.sql, "WHERE FALSE");
    }
}
