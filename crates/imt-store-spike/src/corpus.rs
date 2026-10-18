//! The benchmark corpus: the files in `/samples`, parsed once by the real engine, written back
//! as CLEF and repeated with shifted timestamps and instance names until the requested number of
//! events is reached. Ingest then reads the corpus through the real importers and profile, one
//! chunk of copies at a time, so a persistent store never holds more than one chunk in memory.

use anyhow::{bail, Context, Result};
use imt_core::model::Record;
use imt_core::{Profile, Workspace};
use serde_json::{Map, Value};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Copies get `-NN` appended to `MachineName`, cycling through this many instance names, so the
/// corpus has this many times the sample's instances.
pub const INSTANCE_GROUPS: usize = 20;

/// Sample folders that are not logs (profiler captures) or not readable by the engine.
const NOT_LOGS: &[&str] = &["profiling", "profiles", "unsupported", "generate.py"];

/// The sample profile (instance key `MachineName`, `ProcessId`, extractors, charts).
pub fn sample_profile(samples: &Path) -> Result<Profile> {
    let path = samples.join("profiles/orderservice.imtprofile.toml");
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Profile::from_toml(&text)?)
}

/// The sample log folders, sorted.
pub fn sample_paths(samples: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for e in std::fs::read_dir(samples).with_context(|| format!("reading {}", samples.display()))? {
        let p = e?.path();
        if !p.file_name().and_then(|n| n.to_str()).is_some_and(|n| NOT_LOGS.contains(&n)) {
            paths.push(p);
        }
    }
    paths.sort();
    Ok(paths)
}

/// What [`generate`] wrote.
#[derive(Debug)]
pub struct GenReport {
    pub events: u64,
    pub copies: usize,
    pub files: usize,
    pub bytes: u64,
    pub per_copy: usize,
    pub period_ms: i64,
}

/// Writes `events` events under `out/copy-NNNN/`, copy `k` shifted by `k` periods (the sample's
/// span rounded up to a whole minute, plus one) so copies follow each other in time.
pub fn generate(samples: &Path, out: &Path, events: u64) -> Result<GenReport> {
    let mut ws = Workspace::new(sample_profile(samples)?)?;
    ws.open(&sample_paths(samples)?)?;
    let summary = ws.summary();
    // Records without a timestamp cannot be shifted; the samples have none worth keeping.
    let timed: Vec<&Record> = ws.records().iter().filter(|r| r.ts.is_some()).collect();
    if timed.is_empty() {
        bail!("the samples contain no timestamped events; run `python3 samples/generate.py`");
    }
    let from = timed.iter().filter_map(|r| r.ts).min().unwrap_or(0);
    let to = timed.iter().filter_map(|r| r.ts).max().unwrap_or(0);
    let period_ms = ((to - from) / 60_000 + 2) * 60_000;
    let names: Vec<String> = {
        let mut n = vec![String::new(); summary.sources.iter().map(|s| s.id as usize + 1).max().unwrap_or(0)];
        for s in &summary.sources {
            n[s.id as usize] = s.name.replace(['/', '\\', ':'], "_");
        }
        n
    };

    std::fs::create_dir_all(out)?;
    let (mut written, mut copies, mut files, mut bytes) = (0u64, 0usize, 0usize, 0u64);
    'copies: for k in 0.. {
        let dir = out.join(format!("copy-{k:04}"));
        std::fs::create_dir_all(&dir)?;
        copies += 1;
        for (sid, name) in names.iter().enumerate() {
            let mine: Vec<&&Record> = timed.iter().filter(|r| r.source as usize == sid).collect();
            if mine.is_empty() {
                continue;
            }
            let path = dir.join(format!("{name}.clef"));
            let mut w = BufWriter::new(std::fs::File::create(&path)?);
            for r in mine {
                if written >= events {
                    w.flush()?;
                    files += 1;
                    bytes += std::fs::metadata(&path)?.len();
                    break 'copies;
                }
                let line = clef_line(r, k as i64 * period_ms, k % INSTANCE_GROUPS);
                w.write_all(line.as_bytes())?;
                w.write_all(b"\n")?;
                written += 1;
            }
            w.flush()?;
            files += 1;
            bytes += std::fs::metadata(&path)?.len();
        }
        if written >= events {
            break;
        }
    }
    Ok(GenReport {
        events: written,
        copies,
        files,
        bytes,
        per_copy: timed.len(),
        period_ms,
    })
}

/// One record as a CLEF line, `shift_ms` later and with its instance renamed.
fn clef_line(r: &Record, shift_ms: i64, instance: usize) -> String {
    let mut o = Map::new();
    let ts = r.ts.unwrap_or(0) + shift_ms;
    let t = chrono::DateTime::from_timestamp_millis(ts).unwrap_or_default();
    o.insert("@t".into(), Value::String(t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()));
    o.insert("@l".into(), Value::String(r.level.as_str().into()));
    o.insert("@m".into(), Value::String(r.message.clone()));
    if let Some(t) = &r.template {
        o.insert("@mt".into(), Value::String(t.clone()));
    }
    if let Some(x) = &r.exception {
        o.insert("@x".into(), Value::String(x.clone()));
    }
    for (k, v) in &r.fields {
        let v = match (k.as_str(), v) {
            ("MachineName", Value::String(m)) => Value::String(format!("{m}-{instance:02}")),
            _ => v.clone(),
        };
        o.insert(k.clone(), v);
    }
    Value::Object(o).to_string()
}

/// The copy folders of a corpus, in time order, at most `limit` of them.
pub fn copy_dirs(corpus: &Path, limit: Option<usize>) -> Result<Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(corpus)
        .with_context(|| format!("reading corpus {}", corpus.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("copy-")))
        .collect();
    dirs.sort();
    if let Some(n) = limit {
        dirs.truncate(n);
    }
    if dirs.is_empty() {
        bail!("{} holds no copy-NNNN folders; run `imt-store-spike gen` first", corpus.display());
    }
    Ok(dirs)
}

/// A source (file) of the corpus with its store-wide id.
#[derive(Debug, Clone)]
pub struct SourceRow {
    pub id: u32,
    pub path: String,
    pub name: String,
}

/// One parsed chunk handed to a store's ingest.
pub struct Chunk<'a> {
    /// Parsed, profile-processed, time-sorted records of the chunk (ids local to the chunk).
    pub records: &'a [Record],
    /// Added to a record's local id for its store-wide id.
    pub id_base: u64,
    /// Added to a record's local source id for its store-wide source id.
    pub source_base: u32,
    /// The chunk's sources with store-wide ids.
    pub sources: Vec<SourceRow>,
}

/// Parses the corpus `copies_per_chunk` copies at a time with the real importers and profile and
/// hands each chunk to `sink`. Copies do not overlap in time, so chunk order is time order and
/// local ids plus `id_base` are store-wide time-ordered ids, as in the in-memory workspace.
pub fn for_each_chunk(
    dirs: &[PathBuf],
    profile: &Profile,
    copies_per_chunk: usize,
    mut sink: impl FnMut(Chunk<'_>) -> Result<()>,
) -> Result<(u64, u32)> {
    let (mut id_base, mut source_base) = (0u64, 0u32);
    for group in dirs.chunks(copies_per_chunk.max(1)) {
        let mut ws = Workspace::new(profile.clone())?;
        ws.open(group)?;
        let summary = ws.summary();
        let n_sources = summary.sources.iter().map(|s| s.id + 1).max().unwrap_or(0);
        let sources = summary
            .sources
            .iter()
            .map(|s| SourceRow {
                id: source_base + s.id,
                path: s.path.clone(),
                name: s.name.clone(),
            })
            .collect();
        let records = ws.records();
        sink(Chunk {
            records,
            id_base,
            source_base,
            sources,
        })?;
        id_base += records.len() as u64;
        source_base += n_sources;
    }
    Ok((id_base, source_base))
}
