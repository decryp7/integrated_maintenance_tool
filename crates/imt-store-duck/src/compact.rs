//! Segment compaction: small segments (a slow import, many small files) are merged and segments holding
//! rows of removed or aborted sources are rewritten without them. The new segment is written to a
//! temporary file and swapped in for the old ones in one catalog transaction (the view changes with it), so
//! a reader sees either the old or the new files. Old files a reader still has open (Windows refuses to
//! delete them) go to `trash` and are deleted later.

use crate::schema::{canonical_columns, parquet_from, READY};
use crate::segments::{Segment, SegmentWriter, SEGMENTS_DIR, SEGMENT_ROWS};
use crate::writer::{delete_segment, insert_segment, meta_u64, refresh_view, segment_files, set_meta};
use anyhow::Result;
use duckdb::arrow::array::{RecordBatch, StructArray};
use duckdb::{params, Connection};
use imt_core::batch::{EventBatch, EventSchema, BATCH_ROWS};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Small segments (under half full) tolerated before they are merged.
pub const MAX_SMALL_SEGMENTS: usize = 32;

/// What a compaction did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Compaction {
    /// Segments replaced or dropped.
    pub replaced: usize,
    /// Segments written.
    pub written: usize,
}

#[derive(Debug)]
struct Listed {
    file: String,
    first_id: u64,
    end_id: u64,
    rows: u64,
    live: u64,
    /// In the full-text index (merged segments keep the flag; indexed and unindexed ones are never merged
    /// together, so a merged segment is never indexed twice).
    indexed: bool,
}

/// Rows of each segment that belong to ready sources.
fn live_rows(conn: &Connection, dir: &Path) -> Result<HashMap<String, u64>> {
    let files = segment_files(conn, dir)?;
    let mut live = HashMap::new();
    if files.is_empty() {
        return Ok(live);
    }
    let list = files
        .iter()
        .map(|f| format!("'{}'", f.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ");
    let mut stmt = conn.prepare(&format!(
        "SELECT filename, count(*) FROM read_parquet([{list}], filename = true) WHERE source IN (SELECT id FROM sources WHERE ready) GROUP BY filename"
    ))?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?;
    for row in rows {
        let (path, n) = row?;
        let name = Path::new(&path)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or(path);
        live.insert(name, n);
    }
    Ok(live)
}

/// Deletes the files in `trash` that can be deleted now.
pub(crate) fn empty_trash(conn: &Connection) -> Result<()> {
    let paths: Vec<String> = {
        let mut stmt = conn.prepare("SELECT path FROM trash")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<duckdb::Result<_>>()?
    };
    for p in paths {
        let gone = match std::fs::remove_file(&p) {
            Ok(()) => true,
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        };
        if gone {
            conn.execute("DELETE FROM trash WHERE path = ?", params![p])?;
        }
    }
    Ok(())
}

fn delete_or_trash(conn: &Connection, path: &Path) -> Result<()> {
    if std::fs::remove_file(path).is_err() && path.exists() {
        conn.execute("INSERT OR IGNORE INTO trash VALUES (?)", params![path.to_string_lossy()])?;
    }
    Ok(())
}

/// Writes the live rows of `group` (in id order) into one new segment, through the segment writer, so its
/// promoted fields, field catalog and histogram are computed as for an import.
fn rewrite(conn: &Connection, seg_dir: &Path, schema: &Arc<EventSchema>, group: &[&Listed], generation: u64) -> Result<Segment> {
    let first_id = group.iter().map(|l| l.first_id).min().unwrap_or(0);
    let end_id = group.iter().map(|l| l.end_id).max().unwrap_or(0);
    let files: Vec<String> = group.iter().map(|l| seg_dir.join(&l.file).to_string_lossy().into_owned()).collect();
    let sql = format!(
        "SELECT {} FROM {} WHERE {READY} ORDER BY id",
        canonical_columns(schema),
        parquet_from(&files)
    );
    let mut w = SegmentWriter::create(seg_dir, schema, first_id, end_id, generation)?;
    let written = (|| -> Result<()> {
        let mut stmt = conn.prepare(&sql)?;
        let _ = stmt.stream_arrow([])?;
        while let Some(chunk) = stmt.step()? {
            let rb = RecordBatch::from(&chunk as &StructArray);
            let mut offset = 0;
            while offset < rb.num_rows() {
                let len = (rb.num_rows() - offset).min(BATCH_ROWS);
                w.write_existing(&EventBatch::try_new(schema.clone(), rb.slice(offset, len))?)?;
                offset += len;
            }
        }
        Ok(())
    })();
    match written {
        Ok(()) => w.close(),
        Err(e) => {
            w.discard();
            Err(e.context("rewriting segments"))
        }
    }
}

/// Compacts the segments of the store in `dir`. Run while nothing writes (the store's write lock).
pub(crate) fn compact(conn: &Connection, dir: &Path, schema: &Arc<EventSchema>) -> Result<Compaction> {
    empty_trash(conn)?;
    let live = live_rows(conn, dir)?;
    let listed: Vec<Listed> = {
        let mut stmt = conn.prepare("SELECT file, first_id, end_id, rows, indexed FROM segments ORDER BY indexed, first_id")?;
        let rows = stmt.query_map([], |r| {
            Ok(Listed {
                file: r.get(0)?,
                first_id: r.get(1)?,
                end_id: r.get(2)?,
                rows: r.get(3)?,
                live: 0,
                indexed: r.get(4)?,
            })
        })?;
        rows.map(|r| {
            r.map(|mut l| {
                l.live = live.get(&l.file).copied().unwrap_or(0);
                l
            })
        })
        .collect::<duckdb::Result<_>>()?
    };
    for l in &listed {
        conn.execute("UPDATE segments SET live = ? WHERE file = ?", params![l.live, l.file])?;
    }
    // Rows of removed or aborted sources: rewritten, so the catalog and histograms stay exact.
    let garbage = |l: &Listed| l.live < l.rows;
    let small = |l: &Listed| l.rows < SEGMENT_ROWS / 2;
    // Every import leaves up to one partial segment per parse thread: small segments are merged only once
    // there are many of them.
    let smalls = listed.iter().filter(|l| small(l)).count();
    let merge_small = smalls > MAX_SMALL_SEGMENTS;
    let candidates: Vec<&Listed> = listed.iter().filter(|l| garbage(l) || (merge_small && small(l))).collect();
    if candidates.is_empty() {
        return Ok(Compaction::default());
    }
    let mut groups: Vec<Vec<&Listed>> = vec![Vec::new()];
    let mut acc = 0;
    for c in candidates {
        let other_kind = groups.last().and_then(|g| g.last()).is_some_and(|l| l.indexed != c.indexed);
        if acc >= SEGMENT_ROWS || other_kind {
            groups.push(Vec::new());
            acc = 0;
        }
        acc += c.live;
        if let Some(g) = groups.last_mut() {
            g.push(c);
        }
    }
    let seg_dir = dir.join(SEGMENTS_DIR);
    let mut done = Compaction::default();
    for group in groups.into_iter().filter(|g| !g.is_empty()) {
        let live: u64 = group.iter().map(|l| l.live).sum();
        let generation = meta_u64(conn, "file_generation") + 1;
        set_meta(conn, "file_generation", generation)?;
        let new = if live > 0 {
            Some(rewrite(conn, &seg_dir, schema, &group, generation)?)
        } else {
            None
        };
        conn.execute_batch("BEGIN TRANSACTION")?;
        let swapped = (|| -> Result<()> {
            for l in &group {
                delete_segment(conn, &l.file)?;
            }
            if let Some(seg) = &new {
                insert_segment(conn, seg, seg.rows, group[0].indexed)?;
            }
            refresh_view(conn, dir, schema)
        })();
        match swapped {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                if let Some(seg) = &new {
                    let _ = std::fs::remove_file(seg_dir.join(&seg.file));
                }
                return Err(e);
            }
        }
        for l in &group {
            delete_or_trash(conn, &seg_dir.join(&l.file))?;
        }
        done.replaced += group.len();
        done.written += usize::from(live > 0);
    }
    Ok(done)
}
