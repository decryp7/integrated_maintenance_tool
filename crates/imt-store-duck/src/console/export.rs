//! CSV export of a console query (ADR 0030 follow-up): the checked query runs again in the sandbox and its rows
//! stream to a file next to the chosen one, renamed into place when complete, so a failed, cancelled or stopped
//! export leaves nothing behind. Bounded by rows, bytes and time ([`Limits`]); cells are written whole (not cut
//! as the console's pages cut them), and text cells that a spreadsheet would run as a formula are quoted.

use super::{Stopper, SCALARS};
use duckdb::types::Value as Duck;
use duckdb::Connection;
use imt_core::ports::{
    SqlExportRequest, SqlExportResult, StoreError, StoreResult, SQL_EXPORT_MAX_BYTES, SQL_EXPORT_MAX_ROWS, SQL_EXPORT_TIME_LIMIT,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// An export's bounds (the port's constants; tests lower them).
#[derive(Debug, Clone, Copy)]
pub(super) struct Limits {
    pub rows: u64,
    pub bytes: u64,
    pub time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            rows: SQL_EXPORT_MAX_ROWS,
            bytes: SQL_EXPORT_MAX_BYTES,
            time: SQL_EXPORT_TIME_LIMIT,
        }
    }
}

/// The file being written; removed unless it was kept.
struct Partial {
    path: PathBuf,
    kept: bool,
}

impl Drop for Partial {
    fn drop(&mut self) {
        if !self.kept {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Where an export is written until it is complete: a hidden sibling of the target.
fn partial_path(target: &Path) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    target.with_file_name(format!(".{name}.{}.partial", std::process::id()))
}

/// A cell as CSV text. Text that a spreadsheet would evaluate (`=`, `+`, `-`, `@`, tab, CR first) gets a leading
/// `'`, so opening the export never runs a formula that came from a log line.
fn csv_cell(v: Duck) -> String {
    match v {
        Duck::Null => String::new(),
        Duck::Boolean(b) => b.to_string(),
        Duck::TinyInt(n) => n.to_string(),
        Duck::SmallInt(n) => n.to_string(),
        Duck::Int(n) => n.to_string(),
        Duck::BigInt(n) => n.to_string(),
        Duck::UTinyInt(n) => n.to_string(),
        Duck::USmallInt(n) => n.to_string(),
        Duck::UInt(n) => n.to_string(),
        Duck::UBigInt(n) => n.to_string(),
        Duck::Float(f) => f.to_string(),
        Duck::Double(f) => f.to_string(),
        Duck::Text(s) => defuse(s),
        other => defuse(format!("{other:?}")),
    }
}

fn defuse(s: String) -> String {
    if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s
    }
}

fn io(path: &Path, e: impl std::fmt::Display) -> StoreError {
    StoreError::Io(format!("cannot write {}: {e}", path.display()))
}

/// Runs `sql` (already checked) on the sandbox and writes its rows as CSV to the request's path.
pub(super) fn write_csv(conn: &Connection, sql: &str, req: &SqlExportRequest, limits: Limits) -> StoreResult<SqlExportResult> {
    let target = Path::new(&req.path);
    if req.path.trim().is_empty() {
        return Err(StoreError::Invalid("choose a file to export to".into()));
    }
    imt_core::overwrite::check_overwrite(target, req.overwrite).map_err(|e| StoreError::Invalid(e.to_string()))?;
    let stopper = Stopper::start(conn, limits.time)?;
    let described: Vec<(String, String)> = (|| -> duckdb::Result<Vec<(String, String)>> {
        let mut stmt = conn.prepare(&format!("DESCRIBE SELECT * FROM (\n{sql}\n) AS imt_result"))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect()
    })()
    .map_err(|e| stopper.error(e, limits.time))?;
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
        "SELECT {} FROM (\n{sql}\n) AS imt_result({}) LIMIT {}",
        select.join(", "),
        aliases.join(", "),
        // DuckDB's LIMIT is a signed 64-bit value; "no row limit" (u64::MAX) must not overflow it.
        limits.rows.min(i64::MAX as u64 - 1) + 1
    );
    let mut partial = Partial {
        path: partial_path(target),
        kept: false,
    };
    let file = std::fs::File::create(&partial.path).map_err(|e| io(target, e))?;
    let mut out = csv::Writer::from_writer(std::io::BufWriter::new(file));
    out.write_record(described.iter().map(|(n, _)| defuse(n.clone())))
        .map_err(|e| io(target, e))?;
    let (mut rows, mut bytes, mut truncated) = (0u64, 0u64, false);
    let mut record: Vec<String> = Vec::with_capacity(described.len());
    let mut stmt = conn.prepare(&wrapped).map_err(|e| stopper.error(e, limits.time))?;
    let mut result = stmt.query([]).map_err(|e| stopper.error(e, limits.time))?;
    while let Some(r) = result.next().map_err(|e| stopper.error(e, limits.time))? {
        record.clear();
        for i in 0..described.len() {
            record.push(csv_cell(r.get::<_, Duck>(i).map_err(|e| stopper.error(e, limits.time))?));
        }
        // Separators, quotes and the line end: an upper bound of what the row adds.
        let row_bytes: u64 = record.iter().map(|c| c.len() as u64 * 2 + 3).sum::<u64>() + 2;
        if rows >= limits.rows || bytes + row_bytes > limits.bytes {
            truncated = true;
            break;
        }
        out.write_record(&record).map_err(|e| io(target, e))?;
        rows += 1;
        bytes += row_bytes;
    }
    drop(result);
    drop(stmt);
    drop(stopper);
    let file = out
        .into_inner()
        .map_err(|e| io(target, e.error()))?
        .into_inner()
        .map_err(|e| io(target, e.error()))?;
    file.sync_all().map_err(|e| io(target, e))?;
    let written = file.metadata().map_err(|e| io(target, e))?.len();
    drop(file);
    std::fs::rename(&partial.path, target).map_err(|e| io(target, e))?;
    partial.kept = true;
    Ok(SqlExportResult {
        path: req.path.clone(),
        rows,
        bytes: written,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_whole_and_formulas_are_defused() {
        assert_eq!(csv_cell(Duck::Null), "");
        assert_eq!(csv_cell(Duck::BigInt(-5)), "-5");
        assert_eq!(csv_cell(Duck::Text("=HYPERLINK(\"x\")".into())), "'=HYPERLINK(\"x\")");
        assert_eq!(csv_cell(Duck::Text("@SUM(A1)".into())), "'@SUM(A1)");
        assert_eq!(csv_cell(Duck::Text("plain, \"quoted\"".into())), "plain, \"quoted\"");
        let long = "x".repeat(10_000);
        assert_eq!(csv_cell(Duck::Text(long.clone())), long, "not cut like a page's cells");
        assert_eq!(csv_cell(Duck::Boolean(true)), "true");
    }

    #[test]
    fn the_partial_file_is_a_hidden_sibling() {
        let p = partial_path(Path::new("/x/out.csv"));
        assert_eq!(p.parent(), Some(Path::new("/x")));
        assert!(p.file_name().unwrap().to_string_lossy().starts_with(".out.csv."));
    }
}
