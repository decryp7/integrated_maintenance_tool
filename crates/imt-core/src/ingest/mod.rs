//! Streaming import (ADR 0014 §2): files are read in bounded chunks into event batches for a store,
//! never as whole strings (except by importers that do not stream yet, through an adapter).
//!
//! ```text
//! file ──▶ FilePlan (sniff) ──▶ ChunkSource ──▶ importer ──▶ RecordBatcher ──▶ BatchSink (store writer)
//!          encoding, importer,   8 MiB chunks    ParsedEvent   profile.ingest     ids assigned
//!          format, day order     of whole lines                per ≤ 8,192 rows
//! ```
//!
//! [`import_files`] reads files one after another; [`import_files_parallel`] parses many files and the
//! chunks of large files on a bounded pool (ADR 0014 §2).

mod batcher;
mod chunk;
mod json;
mod legacy;
mod parallel;
mod plan;
mod problems;
mod region;
mod sink;
mod source;
mod text;

pub use batcher::{column_type, schema_for, BatchedSource, RecordBatcher};
pub use chunk::{Chunk, ChunkSource, Encoding, Line, LineReader, LineSpan, OwnedLine, ReaderChunks, TextChunks, CHUNK_BYTES};
pub use parallel::{default_threads, import_files_parallel, ParallelOptions, SharedRef, SharedSink, MAX_DEFAULT_THREADS};
pub use plan::{detect_encoding, plan, scan_chunk_starts, score_text, ChunkStart, FilePlan, JsonShape, PlanKind, CHUNK_TARGET, HEAD_BYTES};
pub use problems::{Problem, ProblemKind, Problems, MAX_SAMPLES};
pub use sink::{BatchSink, EventSink, ImportCancelled, NewSource, ParsedEvent, SourceReport};
pub use region::{begin_at_end, read_appended, resume_after, AtEnd, Region, RegionRead, TailStart};
pub use source::{import_path, ImportEnv, Outcome};

use crate::api::UnrecognizedFile;
use crate::profile::CompiledProfile;
use crate::workspace::{ImportProgress, NOT_IMPORTED};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Largest event (all its lines), 8 MiB: continuation lines past it are dropped and reported, so one
/// runaway multi-line event cannot exhaust memory.
pub const MAX_EVENT_BYTES: usize = 8 << 20;

/// Adds the files under `path` (itself if it is a file) that the profile's include/exclude globs
/// select (relative to `path`) and that were not removed.
pub fn collect_files(path: &Path, profile: &CompiledProfile, removed: &HashSet<PathBuf>, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        out.push(path.to_path_buf());
        return;
    }
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            let rel = p.strip_prefix(path).unwrap_or(&p);
            if profile.exclude.is_match(rel) || removed.contains(&p) {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(p),
                Ok(t) if t.is_file() && profile.include.is_match(rel) => out.push(p),
                _ => {}
            }
        }
    }
}

/// The files under `paths` (canonical) to read, sorted, each with its display name (relative to the
/// opened folder of `roots` it came from). Files in `known` (already read) are left out.
pub fn files_to_read(
    paths: &[PathBuf],
    roots: &[PathBuf],
    profile: &CompiledProfile,
    removed: &HashSet<PathBuf>,
    known: &HashSet<String>,
) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    for path in paths {
        collect_files(path, profile, removed, &mut files);
    }
    files.retain(|f| !known.contains(f.to_string_lossy().as_ref()));
    files.sort();
    files.dedup();
    files
        .into_iter()
        .map(|f| {
            let name = crate::workspace::display_name(&f, roots);
            (f, name)
        })
        .collect()
}

/// Reads `files` one after another into `sink`, reporting progress; after a cancel the remaining files
/// are listed as not imported.
pub fn import_files(
    env: &ImportEnv,
    files: &[(PathBuf, String)],
    removed: &HashSet<PathBuf>,
    progress: &ImportProgress,
    sink: &mut dyn BatchSink,
) -> Vec<Outcome> {
    let sizes: Vec<u64> = files.iter().map(|(p, _)| std::fs::metadata(p).map_or(0, |m| m.len())).collect();
    progress.start(files.len(), sizes.iter().sum());
    let mut out = Vec::new();
    for ((path, name), size) in files.iter().zip(sizes) {
        if progress.is_cancelled() {
            out.push(Outcome::Unrecognized(UnrecognizedFile {
                path: path.to_string_lossy().into_owned(),
                name: name.clone(),
                reason: NOT_IMPORTED.into(),
                crashed: false,
            }));
        } else {
            progress.reading(path);
            out.extend(import_path(env, path, name, removed, sink));
        }
        progress.file_done(size);
    }
    progress.finish();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;

    #[test]
    fn lists_files_with_names_relative_to_their_folder() {
        let dir = std::env::temp_dir().join(format!("imt-ingest-list-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.log"), "x").unwrap();
        std::fs::write(dir.join("sub/b.log"), "x").unwrap();
        std::fs::write(dir.join("sub/gone.log"), "x").unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let removed: HashSet<PathBuf> = [dir.join("sub").join("gone.log")].into();
        let known: HashSet<String> = [dir.join("a.log").to_string_lossy().into_owned()].into();
        let files = files_to_read(std::slice::from_ref(&dir), std::slice::from_ref(&dir), &p, &removed, &known);
        let names: Vec<&str> = files.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(names, ["sub/b.log"]);
        let all = files_to_read(
            &[dir.join("a.log")],
            std::slice::from_ref(&dir),
            &p,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(all[0].1, "a.log");
        assert_eq!(all.len(), 1);
        std::fs::remove_dir_all(dir).ok();
    }
}
