//! How source files are decoded: the `source_encodings` catalog table (what each source was read as, and the user's
//! per-source choices), the policy an import reads with, and "Encoding…" (read one source again as a chosen encoding).

use crate::{engine, write_lock, DuckStore, Inner};
use duckdb::{params, Connection};
use imt_core::batch::SourceId;
use imt_core::ingest::{EncodingChoice, EncodingInfo, EncodingOrigin, EncodingPolicy};
use imt_core::ports::{ImportSession, Ingest, StoreError, StoreResult};
use std::path::PathBuf;

fn origin_text(o: EncodingOrigin) -> &'static str {
    match o {
        EncodingOrigin::Bom => "bom",
        EncodingOrigin::Detected => "detected",
        EncodingOrigin::Profile => "profile",
        EncodingOrigin::Manual => "manual",
    }
}

fn origin_of(text: &str) -> EncodingOrigin {
    match text {
        "bom" => EncodingOrigin::Bom,
        "profile" => EncodingOrigin::Profile,
        "manual" => EncodingOrigin::Manual,
        _ => EncodingOrigin::Detected,
    }
}

/// Records how `path` was decoded (replacing an earlier row).
pub(crate) fn record(conn: &Connection, path: &str, e: &EncodingInfo) -> duckdb::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO source_encodings VALUES (?, ?, ?, ?)",
        params![path, e.label, e.confidence, origin_text(e.origin)],
    )?;
    Ok(())
}

/// Records how each of `rows` (path, encoding) was decoded, replacing earlier rows, a statement per few hundred
/// rows (a statement per source is parsed, bound and optimised each time). A path listed twice keeps its last row.
pub(crate) fn record_many(conn: &Connection, rows: &[(&str, &EncodingInfo)]) -> duckdb::Result<()> {
    const PER_STATEMENT: usize = 200;
    let mut last: std::collections::HashMap<&str, &EncodingInfo> = std::collections::HashMap::new();
    for (path, e) in rows {
        last.insert(path, e);
    }
    let mut unique: Vec<(&str, &EncodingInfo)> = last.into_iter().collect();
    unique.sort_by(|a, b| a.0.cmp(b.0));
    for chunk in unique.chunks(PER_STATEMENT) {
        let values = vec!["(?, ?, ?, ?)"; chunk.len()].join(", ");
        let texts: Vec<(&str, &str, u8, &str)> = chunk
            .iter()
            .map(|(path, e)| (*path, e.label.as_str(), e.confidence, origin_text(e.origin)))
            .collect();
        let args: Vec<&dyn duckdb::ToSql> = texts
            .iter()
            .flat_map(|(path, label, confidence, origin)| [path as &dyn duckdb::ToSql, label, confidence, origin])
            .collect();
        conn.execute(&format!("INSERT OR REPLACE INTO source_encodings VALUES {values}"), args.as_slice())?;
    }
    Ok(())
}

/// A summary row's encoding columns as the wire type (`None` when nothing was recorded).
pub(crate) fn info(label: Option<String>, confidence: Option<u8>, origin: Option<String>) -> Option<EncodingInfo> {
    Some(EncodingInfo {
        label: label?,
        confidence: confidence.unwrap_or(0),
        origin: origin_of(&origin.unwrap_or_default()),
    })
}

/// The user's choices by source path (rows that cannot be read are ignored: that file is detected again).
fn manual(conn: &Connection) -> Vec<(String, String)> {
    let Ok(mut stmt) = conn.prepare("SELECT path, label FROM source_encodings WHERE origin = 'manual'") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

impl Inner {
    /// What an import reads with: the profile's `[files] encoding` and the user's per-source choices.
    pub(crate) fn encoding_policy(&self, conn: &Connection) -> EncodingPolicy {
        let mut policy = EncodingPolicy {
            default: self.profile().profile.files.encoding_choice(),
            ..EncodingPolicy::default()
        };
        for (path, label) in manual(conn) {
            if let Some(e) = imt_core::ingest::Encoding::from_label(&label) {
                policy.overrides.insert(path, e);
            }
        }
        policy
    }
}

impl DuckStore {
    /// "Encoding…": reads one source again as `encoding` (a label; `None` or `auto`: detected again).
    pub(crate) fn assign_encoding(&self, source: SourceId, encoding: Option<&str>) -> StoreResult<ImportSession> {
        let choice = match encoding {
            Some(label) => EncodingChoice::parse(label).map_err(StoreError::Invalid)?,
            None => EncodingChoice::Auto,
        };
        let conn = self.inner.conn()?;
        let path: String = conn
            .query_row("SELECT path FROM sources WHERE id = ?", params![source.0], |r| r.get(0))
            .map_err(|_| StoreError::NotFound(format!("source {source}")))?;
        match choice {
            EncodingChoice::Fixed(e) => record(
                &conn,
                &path,
                &EncodingInfo {
                    label: e.label().to_string(),
                    confidence: 100,
                    origin: EncodingOrigin::Manual,
                },
            ),
            EncodingChoice::Auto => conn
                .execute("DELETE FROM source_encodings WHERE path = ?", params![path])
                .map(|_| ()),
        }
        .map_err(engine)?;
        drop(conn);
        let file = PathBuf::from(path.split(imt_core::archive::ENTRY_SEPARATOR).next().unwrap_or(&path));
        {
            let mut s = write_lock(&self.inner.state);
            if !s.roots.iter().any(|r| file.starts_with(r)) {
                s.roots.push(file.clone());
            }
        }
        Ingest::refresh(self, &[file])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(label: &str, confidence: u8, origin: EncodingOrigin) -> EncodingInfo {
        EncodingInfo {
            label: label.into(),
            confidence,
            origin,
        }
    }

    #[test]
    fn many_rows_are_recorded_in_bulk_replacing_earlier_ones_and_keeping_the_last_of_a_repeated_path() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(crate::schema::ADDED_DDL).unwrap();
        // An earlier row is replaced; more rows than one statement takes are all written.
        record(&conn, "/old.log", &info("windows-1252", 40, EncodingOrigin::Detected)).unwrap();
        let utf8 = info("UTF-8", 100, EncodingOrigin::Bom);
        let sjis = info("Shift_JIS", 70, EncodingOrigin::Detected);
        let mut paths: Vec<String> = (0..450).map(|i| format!("/logs/{i}.log")).collect();
        paths.push("/old.log".into());
        paths.push("/logs/7.log".into());
        let rows: Vec<(&str, &EncodingInfo)> = paths
            .iter()
            .enumerate()
            .map(|(i, p)| (p.as_str(), if i == 450 || i == 451 { &sjis } else { &utf8 }))
            .collect();
        record_many(&conn, &rows).unwrap();
        record_many(&conn, &[]).unwrap();
        let count: u64 = conn.query_row("SELECT count(*) FROM source_encodings", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 451);
        let of = |p: &str| -> (String, u8, String) {
            conn.query_row(
                "SELECT label, confidence, origin FROM source_encodings WHERE path = ?",
                params![p],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        assert_eq!(of("/old.log"), ("Shift_JIS".into(), 70, "detected".into()));
        assert_eq!(of("/logs/449.log"), ("UTF-8".into(), 100, "bom".into()));
        // Listed twice: the later row wins.
        assert_eq!(of("/logs/7.log").0, "Shift_JIS");
    }
}
