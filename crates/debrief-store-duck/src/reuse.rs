//! Source ids that survive a reload (ADR 0050). Sources are never renumbered: notes, saved queries and links refer to
//! them. When the opened folders are read again, a file that was a source before keeps that source's id:
//!
//! 1. by path: a source at the same path (archive members by their `archive!entry` path) is the same source;
//! 2. else by content: a file that is new on disk and whose fingerprint (size, hash of the first and last 64 KiB)
//!    equals that of exactly one source whose file is gone is that source, renamed or moved. The source records the new
//!    path; its id, and so its notes, stay.
//!
//! A match is never guessed: when two vanished sources or two new files share a fingerprint (copies of one file), the
//! new files become new sources and the import reports it in its notices. Sources without a stored fingerprint
//! (imported by an older Debrief) and archive members only match by path.
//!
//! New sources get ids after the highest one ever used, in the sorted order of their paths, exactly as in a first import.

use crate::{engine, DuckStore, Inner};
use debrief_core::archive::ENTRY_SEPARATOR;
use debrief_core::ingest::{fingerprint, Fingerprint};
use debrief_core::ports::StoreResult;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The ids a reload keeps, by the path of the source in the new import, and what to tell the user.
#[derive(Debug, Default)]
pub(crate) struct Reuse {
    pub keep: HashMap<String, u32>,
    pub notices: Vec<String>,
}

/// One earlier source, read from the catalog kept aside.
struct Old {
    id: u32,
    path: String,
    fingerprint: Option<Fingerprint>,
}

/// The earlier sources (from the `aside_` tables a reload keeps while it runs).
fn old_sources(inner: &Inner) -> StoreResult<Vec<Old>> {
    let conn = inner.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.path, f.size, f.hash FROM aside_sources s LEFT JOIN aside_source_fingerprints f ON f.source = s.id ORDER BY s.id",
        )
        .map_err(engine)?;
    let rows = stmt
        .query_map([], |r| {
            let (size, hash): (Option<u64>, Option<u64>) = (r.get(2)?, r.get(3)?);
            Ok(Old {
                id: r.get(0)?,
                path: r.get(1)?,
                fingerprint: size.zip(hash).map(|(size, hash)| Fingerprint { size, hash }),
            })
        })
        .map_err(engine)?;
    rows.collect::<duckdb::Result<Vec<_>>>().map_err(engine)
}

/// Decides which earlier sources the files under `paths` continue. `files_now` lists the files the new import will
/// read (only called when a rename has to be looked for).
pub(crate) fn plan(inner: &Inner, files_now: impl FnOnce() -> Vec<PathBuf>) -> StoreResult<Reuse> {
    let old = old_sources(inner)?;
    let mut keep: HashMap<String, u32> = old.iter().map(|o| (o.path.clone(), o.id)).collect();
    let mut reuse = Reuse::default();
    let file_of = |p: &str| p.split(ENTRY_SEPARATOR).next().unwrap_or(p).to_string();
    // Sources whose file is gone, and that can be recognised by content (plain files with a fingerprint).
    let vanished: Vec<&Old> = old
        .iter()
        .filter(|o| o.fingerprint.is_some() && !o.path.contains(ENTRY_SEPARATOR) && !Path::new(&o.path).exists())
        .collect();
    if vanished.is_empty() {
        reuse.keep = keep;
        return Ok(reuse);
    }
    let known: HashSet<String> = old.iter().map(|o| file_of(&o.path)).collect();
    let candidates: Vec<PathBuf> = files_now()
        .into_iter()
        .filter(|f| !known.contains(f.to_string_lossy().as_ref()))
        .collect();
    let mut by_print: HashMap<Fingerprint, Vec<&Old>> = HashMap::new();
    for o in &vanished {
        if let Some(fp) = o.fingerprint {
            by_print.entry(fp).or_default().push(o);
        }
    }
    let prints: Vec<(PathBuf, Option<Fingerprint>)> = candidates
        .into_par_iter()
        .map(|p| {
            let fp = std::fs::metadata(&p)
                .ok()
                .filter(|m| by_print.keys().any(|k| k.size == m.len()))
                .and_then(|_| fingerprint(&p));
            (p, fp)
        })
        .collect();
    let mut new_by_print: HashMap<Fingerprint, Vec<PathBuf>> = HashMap::new();
    for (p, fp) in prints {
        if let Some(fp) = fp.filter(|fp| by_print.contains_key(fp)) {
            new_by_print.entry(fp).or_default().push(p);
        }
    }
    let mut sorted: Vec<_> = new_by_print.into_iter().collect();
    sorted.sort_by(|a, b| a.1.cmp(&b.1));
    for (fp, news) in sorted {
        let olds = &by_print[&fp];
        match (olds.as_slice(), news.as_slice()) {
            ([o], [n]) => {
                keep.insert(n.to_string_lossy().into_owned(), o.id);
                keep.remove(&o.path);
            }
            _ => reuse.notices.push(format!(
                "{} looks like a renamed or copied file but the match is ambiguous ({} earlier and {} new files have the same content): read as new {}",
                news[0].display(),
                olds.len(),
                news.len(),
                if news.len() == 1 { "source" } else { "sources" }
            )),
        }
    }
    reuse.keep = keep;
    Ok(reuse)
}

impl DuckStore {
    /// For tests: the id and path of every source, by id.
    #[doc(hidden)]
    pub fn source_ids(&self) -> StoreResult<Vec<(u32, String)>> {
        let conn = self.inner.conn()?;
        let mut stmt = conn.prepare("SELECT id, path FROM sources ORDER BY id").map_err(engine)?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(engine)?;
        rows.collect::<duckdb::Result<Vec<_>>>().map_err(engine)
    }
}
