//! Reading one file into a [`BatchSink`]: archives are expanded (each zip member is a source), the
//! parser library picks the file's parser (ADR 0026), the file plan picks the importer, and the events
//! stream through the [`RecordBatcher`]. A failure in one file (an error or a crash) is reported as
//! unrecognised with a reason; its events are removed.

use super::batcher::{BatchedSource, RecordBatcher};
use super::chunk::{LineReader, ReaderChunks};
use super::encoding::EncodingPolicy;
use super::parallel::Split;
use super::plan::{plan_with, score_text_with, FilePlan, PlanKind, HEAD_BYTES};
use super::sink::{BatchSink, ImportCancelled, NewSource, SourceReport};
use super::{json, legacy, text};
use crate::api::{FileKind, UnrecognizedFile};
use crate::batch::{EventSchema, SourceId};
use crate::importers::{decode, ImportCtx, Importers};
use crate::parser::set::{Candidate, SCORE_CHARS};
use crate::parser::ParserSet;
use crate::workspace::NOT_IMPORTED;
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// What an import reads with.
pub struct ImportEnv<'a> {
    pub importers: &'a Importers,
    /// The parser library compiled with the profile: each file is read with the parser chosen for it.
    pub parsers: &'a ParserSet,
    pub schema: Arc<EventSchema>,
    /// How files are decoded: the profile's default and the user's per-source choices.
    pub encodings: &'a EncodingPolicy,
    /// Set to stop the import at the next batch; the file being read is then not imported.
    pub cancel: &'a AtomicBool,
    /// Only events inside this range are imported (ADR 0047); `None` imports everything.
    pub window: Option<super::ImportWindow>,
}

/// The result for one source.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // one per file, short-lived: boxing the report would only add an allocation
pub enum Outcome {
    Imported(SourceReport),
    Unrecognized(UnrecognizedFile),
}

fn unrecognized(path: &str, name: &str, reason: impl Into<String>) -> Outcome {
    Outcome::Unrecognized(UnrecognizedFile {
        path: path.to_string(),
        name: name.to_string(),
        reason: reason.into(),
        crashed: false,
    })
}

/// Up to `limit` bytes from the reader's position; the flag is true when that was everything.
fn read_head(reader: &mut impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut head = Vec::with_capacity(limit.min(1 << 16));
    let n = reader.by_ref().take(limit as u64).read_to_end(&mut head)?;
    if n < limit {
        return Ok((head, true));
    }
    // Exactly `limit` bytes: whole only if nothing follows (callers seek before reading on).
    let mut probe = [0u8; 1];
    Ok((head, reader.read(&mut probe)? == 0))
}

fn modified_ms(meta: Option<&std::fs::Metadata>) -> Option<i64> {
    let t = meta?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(t.as_millis()).ok()
}

/// Imports one file: each zip member is a source; a gzip file is one source. Members in `removed` are
/// skipped.
pub fn import_path(env: &ImportEnv, path: &Path, name: &str, removed: &HashSet<PathBuf>, sink: &mut dyn BatchSink) -> Vec<Outcome> {
    import_path_with(env, path, name, removed, sink, None)
}

/// [`import_path`] that may read a large uncompressed text file in parallel chunks (`split`).
pub(super) fn import_path_with(
    env: &ImportEnv,
    path: &Path,
    name: &str,
    removed: &HashSet<PathBuf>,
    sink: &mut dyn BatchSink,
    split: Option<Split<'_>>,
) -> Vec<Outcome> {
    let path_s = path.to_string_lossy().into_owned();
    let mut file = match crate::workspace::open_with_retry(path) {
        Ok(f) => f,
        Err(e) if crate::workspace::is_locked(&e) => {
            return vec![unrecognized(&path_s, name, format!("the file is locked by another process ({e})"))];
        }
        Err(e) => return vec![unrecognized(&path_s, name, e.to_string())],
    };
    let meta = file.metadata().ok();
    let size = meta.as_ref().map_or(0, |m| m.len());
    let file_date = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| crate::time::utc_date_of(d.as_millis() as i64));
    // A file that stopped growing before the window began cannot hold an event of it (ADR 0047).
    if let (Some(w), Some(ms)) = (env.window, modified_ms(meta.as_ref())) {
        if w.rules_out_file(ms) {
            return Vec::new();
        }
    }
    let (head, whole) = match read_head(&mut file, HEAD_BYTES) {
        Ok(h) => h,
        Err(e) => return vec![unrecognized(&path_s, name, e.to_string())],
    };
    let compressed = crate::archive::is_zip(&head);
    if !compressed && !crate::archive::is_gzip(&head) {
        let src = NewSource {
            path: path_s,
            name: name.to_string(),
        };
        return vec![import_source(
            env,
            file,
            head,
            whole,
            size,
            src,
            FileKind::Text,
            path,
            file_date,
            sink,
            split,
        )];
    }
    // Archives are expanded in memory, as before, within the archive limits.
    let mut bytes = head;
    if let Err(e) = file
        .seek(SeekFrom::Start(bytes.len() as u64))
        .and_then(|_| file.read_to_end(&mut bytes))
    {
        return vec![unrecognized(&path_s, name, e.to_string())];
    }
    let file_name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let entries = match crate::archive::expand(&file_name, bytes) {
        Ok(e) => e,
        Err(e) => return vec![unrecognized(&path_s, name, format!("{e:#}"))],
    };
    let kind = if compressed {
        FileKind::ArchiveMember
    } else {
        FileKind::Compressed
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let (source_path, source_name) = match (&entry.name, compressed) {
                (Some(inner), true) => (
                    format!("{path_s}{}{inner}", crate::archive::ENTRY_SEPARATOR),
                    format!("{name}/{inner}"),
                ),
                _ => (path_s.clone(), name.to_string()),
            };
            if removed.contains(Path::new(&source_path)) {
                return None;
            }
            let len = entry.bytes.len();
            let head = entry.bytes[..len.min(HEAD_BYTES)].to_vec();
            let src = NewSource {
                path: source_path,
                name: source_name,
            };
            Some(import_source(
                env,
                Cursor::new(entry.bytes),
                head,
                len <= HEAD_BYTES,
                len as u64,
                src,
                kind,
                path,
                file_date,
                sink,
                None,
            ))
        })
        .collect()
}

/// Imports one source from a seekable reader whose first bytes are `head`.
#[allow(clippy::too_many_arguments)]
fn import_source<R: Read + Seek>(
    env: &ImportEnv,
    mut reader: R,
    head: Vec<u8>,
    whole: bool,
    size: u64,
    src: NewSource,
    file_kind: FileKind,
    path: &Path,
    file_date: Option<chrono::NaiveDate>,
    sink: &mut dyn BatchSink,
    split: Option<Split<'_>>,
) -> Outcome {
    if head.is_empty() {
        return unrecognized(&src.path, &src.name, "empty file");
    }
    let request = env.encodings.request_for(&src.path);
    let text = score_text_with(&head, whole, request);
    let candidate = Candidate {
        source: &src.path,
        path,
        rel: &src.name,
        file_date,
        bytes: &head[..head.len().min(SCORE_CHARS)],
        text: text.as_deref(),
    };
    let choice = match env.parsers.choose(&candidate, env.importers) {
        Ok(c) => c,
        Err(_) if text.is_none() => return unrecognized(&src.path, &src.name, "binary file"),
        Err(reason) => return unrecognized(&src.path, &src.name, reason),
    };
    drop(text);
    let ctx = ImportCtx {
        profile: choice.profile,
        path,
        file_date,
    };
    let plan = match plan_with(env.importers, &ctx, &head, whole, request) {
        Ok(p) => p,
        Err(reason) => return unrecognized(&src.path, &src.name, reason),
    };
    drop(head);
    let id = match sink.begin_source(&src) {
        Ok(id) => id,
        Err(e) => return unrecognized(&src.path, &src.name, format!("{e:#}")),
    };
    // A crash anywhere while reading one file (even in built-in code) fails only that file.
    let caught = crate::guard::catch("IMT", || {
        let mut read = || -> Result<(BatchedSource, usize)> {
            if let Some(split) = split.filter(|s| s.applies(&plan, size)) {
                if let Some(r) = super::parallel::read_split(env, &ctx, &plan, path, id, &src.name, split)? {
                    return Ok(r);
                }
            }
            read_events(env, &ctx, &plan, &mut reader, size, id, &src.name, sink)
        };
        Ok(read())
    });
    let mut crashed = false;
    let result = match caught {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(e)) if e.is::<ImportCancelled>() => Err(NOT_IMPORTED.to_string()),
        Ok(Err(e)) => {
            crashed = crate::guard::Crashed::is_in(&e);
            Err(format!("{e:#}"))
        }
        Err(panic) => {
            crashed = true;
            Err(format!("{panic:#} (please report this file)"))
        }
    };
    let reason = match result {
        Ok((batched, skipped)) if batched.parsed > 0 => {
            let dropped = batched
                .step_stats
                .first()
                .map_or(0, |s| s.input)
                .saturating_sub(batched.step_stats.last().map_or(0, |s| s.output));
            let report = SourceReport {
                id,
                path: src.path.clone(),
                name: src.name.clone(),
                importer: plan.importer.to_string(),
                parser: choice.reference.clone(),
                file_kind: if plan.kind == PlanKind::Binary {
                    FileKind::Binary
                } else {
                    file_kind
                },
                events: batched.events,
                skipped,
                dropped,
                step_stats: batched.step_stats,
                from: batched.from,
                to: batched.to,
                bytes: size,
                problems: batched.problems.into_vec(),
                encoding: plan.encoding_info.clone(),
            };
            match sink.finish_source(&report) {
                Ok(()) => return Outcome::Imported(report),
                Err(e) => format!("{e:#}"),
            }
        }
        Ok(_) => format!("{} importer found no events", plan.importer),
        Err(reason) => reason,
    };
    // The reason is what the user sees; a failure to remove the partial source is logged.
    if let Err(e) = sink.abort_source(id) {
        log::warn!(source = src.path.as_str(); "could not remove a partly imported source: {e:#}");
    }
    Outcome::Unrecognized(UnrecognizedFile {
        path: src.path.clone(),
        name: src.name.clone(),
        reason,
        crashed,
    })
}

/// Streams the events of one planned source into the sink; returns them and the importer's skipped count.
#[allow(clippy::too_many_arguments)]
fn read_events<R: Read + Seek>(
    env: &ImportEnv,
    ctx: &ImportCtx,
    plan: &FilePlan,
    reader: &mut R,
    size: u64,
    id: SourceId,
    name: &str,
    sink: &mut dyn BatchSink,
) -> Result<(BatchedSource, usize)> {
    let mut batcher = RecordBatcher::new(ctx.profile, env.schema.clone(), id, name, sink, env.cancel).with_window(env.window);
    let skipped = match plan.kind {
        PlanKind::Binary => {
            reader.seek(SeekFrom::Start(0))?;
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes)?;
            let importer = env
                .importers
                .binary_importers()
                .find(|i| i.id() == plan.importer)
                .ok_or_else(|| anyhow!("importer {} is not installed", plan.importer))?;
            legacy::import_binary(importer, ctx, &bytes, &mut batcher)?
        }
        PlanKind::Text { format, day_first, .. } => {
            reader.seek(SeekFrom::Start(plan.data_start))?;
            let mut chunks = ReaderChunks::new(reader, plan.encoding, plan.data_start);
            let mut lines = LineReader::new(&mut chunks);
            text::import_text(ctx, format, day_first, &mut lines, &mut batcher)?
        }
        PlanKind::Json(shape) => {
            reader.seek(SeekFrom::Start(plan.data_start))?;
            let mut chunks = ReaderChunks::new(reader, plan.encoding, plan.data_start);
            let mut lines = LineReader::new(&mut chunks);
            json::import_json(ctx, shape, size.saturating_sub(plan.data_start), &mut lines, &mut batcher)?
        }
        PlanKind::Legacy => {
            reader.seek(SeekFrom::Start(0))?;
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes)?;
            let content = decode(&bytes).ok_or_else(|| anyhow!("binary file"))?;
            drop(bytes);
            let importer = env
                .importers
                .find(&plan.importer)
                .ok_or_else(|| anyhow!("importer {} is not installed", plan.importer))?;
            legacy::import_text_whole(importer, ctx, &content, plan.data_start, &mut batcher)?
        }
    };
    Ok((batcher.finish()?, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{EventBatch, EventId, PendingBatch};
    use crate::profile::{CompiledProfile, Profile};

    /// Keeps batches in memory and records what happened to sources.
    #[derive(Default)]
    struct Memory {
        batches: Vec<EventBatch>,
        next_id: u64,
        next_source: u32,
        finished: Vec<SourceReport>,
        aborted: Vec<SourceId>,
        fail_finish: bool,
        fail_begin: bool,
    }

    impl BatchSink for Memory {
        fn begin_source(&mut self, _source: &NewSource) -> Result<SourceId> {
            if self.fail_begin {
                anyhow::bail!("no space left");
            }
            self.next_source += 1;
            Ok(SourceId(self.next_source - 1))
        }
        fn write(&mut self, batch: PendingBatch) -> Result<()> {
            let n = batch.len() as u64;
            self.batches.push(batch.assign_ids(EventId(self.next_id))?);
            self.next_id += n;
            Ok(())
        }
        fn finish_source(&mut self, report: &SourceReport) -> Result<()> {
            if self.fail_finish {
                anyhow::bail!("disk full");
            }
            self.finished.push(report.clone());
            Ok(())
        }
        fn abort_source(&mut self, id: SourceId) -> Result<()> {
            self.aborted.push(id);
            self.batches.retain(|b| b.rows().all(|r| r.source() != id));
            Ok(())
        }
    }

    fn env_parts() -> (Importers, ParserSet) {
        (
            Importers::default(),
            ParserSet::of_compiled(CompiledProfile::compile(Profile::default_profile()).unwrap()),
        )
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("imt-ingest-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rand_suffix() -> u64 {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        u64::from_le_bytes(b)
    }

    #[test]
    fn imports_text_and_json_files_and_reports_unknown_ones() {
        let dir = temp();
        let (importers, profile) = env_parts();
        let cancel = AtomicBool::new(false);
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &profile,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
            window: None,
        };
        let mut sink = Memory::default();
        let none = HashSet::new();
        let text = write(
            &dir,
            "a.log",
            b"\xEF\xBB\xBFbanner\n2026-10-14 10:00:00 INFO a\n2026-10-14 10:00:01 ERROR b\n",
        );
        let out = import_path(&env, &text, "a.log", &none, &mut sink);
        let Outcome::Imported(r) = &out[0] else { panic!("{out:?}") };
        assert_eq!((r.events, r.importer.as_str(), r.file_kind), (3, "text", FileKind::Text));
        assert_eq!(sink.batches[0].row(1).byte_offset(), 10, "after the BOM and the banner");
        let json = write(&dir, "b.json", b"{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"x\"}\n");
        let r = &imported(import_path(&env, &json, "b.json", &none, &mut sink));
        assert_eq!((r.events, r.id), (1, SourceId(1)));
        let csv = write(&dir, "c.csv", b"Time,Level,Message\n2026-10-14 10:00:00,Info,first\n");
        let r = &imported(import_path(&env, &csv, "c.csv", &none, &mut sink));
        assert_eq!(r.importer, "csv");
        for (name, bytes, reason) in [
            ("empty.log", &b""[..], "empty file"),
            (
                "words.txt",
                b"just words",
                "no enabled parser reads this file (see Settings ▸ Parsers)",
            ),
            ("bin.dat", &[0u8, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 1, 2, 3][..], "binary file"),
        ] {
            let p = write(&dir, name, bytes);
            let u = &unrecognised(import_path(&env, &p, name, &none, &mut sink));
            assert_eq!(u.reason, reason);
        }
        let u = &unrecognised(import_path(&env, &dir.join("missing.log"), "missing.log", &none, &mut sink));
        assert!(!u.reason.is_empty());
        // A file the importer finds nothing in is removed again.
        let empty_json = write(&dir, "e.json", b"{\"@t\":\"2026-10-14T10:00:00Z\"}\n");
        let p = CompiledProfile::compile(Profile::from_toml("[[pipeline]]\nstep = 'drop'").unwrap()).unwrap();
        let p = ParserSet::of_compiled(p);
        let env2 = ImportEnv { parsers: &p, ..env };
        let r = &imported(import_path(&env2, &empty_json, "e.json", &none, &mut sink));
        assert_eq!((r.events, r.dropped), (0, 1));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn archives_expand_into_sources_and_failures_are_contained() {
        let dir = temp();
        let (importers, profile) = env_parts();
        let cancel = AtomicBool::new(false);
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &profile,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
            window: None,
        };
        let mut gz = Vec::new();
        {
            use std::io::Write;
            let mut w = flate2::write::GzEncoder::new(&mut gz, flate2::Compression::fast());
            w.write_all(b"2026-10-14 10:00:00 INFO zipped\n").unwrap();
            w.finish().unwrap();
        }
        let p = write(&dir, "app.log.gz", &gz);
        let mut sink = Memory::default();
        let out = import_path(&env, &p, "app.log.gz", &HashSet::new(), &mut sink);
        let Outcome::Imported(r) = &out[0] else { panic!("{out:?}") };
        assert_eq!((r.events, r.file_kind), (1, FileKind::Compressed));
        // A sink that fails to finish: the source is aborted and reported.
        let mut failing = Memory {
            fail_finish: true,
            ..Memory::default()
        };
        let text = write(&dir, "x.log", b"2026-10-14 10:00:00 INFO a\n");
        let u = &unrecognised(import_path(&env, &text, "x.log", &HashSet::new(), &mut failing));
        assert_eq!(u.reason, "disk full");
        assert_eq!(failing.aborted, [SourceId(0)]);
        assert!(failing.batches.is_empty());
        // Cancelled: not imported, removed.
        let cancelled = AtomicBool::new(true);
        let env3 = ImportEnv { cancel: &cancelled, ..env };
        let u = &unrecognised(import_path(&env3, &text, "x.log", &HashSet::new(), &mut sink));
        assert_eq!(u.reason, NOT_IMPORTED);
        std::fs::remove_dir_all(dir).ok();
    }

    /// The report of the only outcome of an import, which must have been imported.
    #[rustfmt::skip]
    fn imported(mut outcomes: Vec<Outcome>) -> SourceReport {
        let Outcome::Imported(r) = outcomes.remove(0) else { panic!("not imported") };
        r
    }

    /// The only outcome of an import, which must have been refused.
    #[rustfmt::skip]
    fn unrecognised(mut outcomes: Vec<Outcome>) -> UnrecognizedFile {
        let Outcome::Unrecognized(u) = outcomes.remove(0) else { panic!("not unrecognised") };
        u
    }

    /// A binary importer for files starting with `BINX`: `BINX<n>` yields n events; `BINXfail` fails and
    /// `BINXpanic` crashes.
    struct Bin;

    impl crate::importers::BinaryImporter for Bin {
        fn id(&self) -> &str {
            "test-bin"
        }
        fn sniff(&self, _ctx: &ImportCtx, head: &[u8]) -> u8 {
            if head.starts_with(b"BINX") {
                90
            } else {
                0
            }
        }
        fn import(&self, _ctx: &ImportCtx, bytes: &[u8]) -> Result<crate::importers::Imported> {
            let rest = String::from_utf8_lossy(&bytes[4..]).into_owned();
            match rest.as_str() {
                "fail" => anyhow::bail!("the file is damaged"),
                "panic" => panic!("importer bug"),
                n => Ok(crate::importers::Imported {
                    records: (0..n.parse::<u32>().unwrap_or(0))
                        .map(|i| {
                            let mut r = crate::model::Record::new(i + 1);
                            r.ts = Some(1_000 + i64::from(i));
                            r.message = format!("event {i}");
                            r
                        })
                        .collect(),
                    skipped: 0,
                    problems: Default::default(),
                }),
            }
        }
    }

    #[test]
    fn binary_importers_read_whole_files_and_their_failures_are_contained() {
        let dir = temp();
        let (mut importers, profile) = env_parts();
        importers.register_binary(Box::new(Bin));
        let cancel = AtomicBool::new(false);
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &profile,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
            window: None,
        };
        let none = HashSet::new();
        let mut sink = Memory::default();
        let ok = write(&dir, "a.bin", b"BINX3");
        let r = imported(import_path(&env, &ok, "a.bin", &none, &mut sink));
        assert_eq!(
            (r.events, r.importer.as_str(), r.file_kind, r.bytes),
            (3, "test-bin", FileKind::Binary, 5)
        );
        // Other files are still sniffed by it and read by their own importer.
        let text = write(&dir, "t.log", b"2026-10-14 10:00:00 INFO plain\n");
        assert_eq!(imported(import_path(&env, &text, "t.log", &none, &mut sink)).importer, "text");
        // An importer that finds nothing, one that fails and one that crashes: each only fails its own file.
        let empty = write(&dir, "empty.bin", b"BINX0");
        let u = unrecognised(import_path(&env, &empty, "empty.bin", &none, &mut sink));
        assert_eq!((u.reason.as_str(), u.crashed), ("test-bin importer found no events", false));
        let bad = write(&dir, "bad.bin", b"BINXfail");
        let u = unrecognised(import_path(&env, &bad, "bad.bin", &none, &mut sink));
        assert!(u.reason.contains("the file is damaged") && !u.crashed, "{}", u.reason);
        let boom = write(&dir, "boom.bin", b"BINXpanic");
        let u = unrecognised(import_path(&env, &boom, "boom.bin", &none, &mut sink));
        assert!(u.crashed && u.reason.contains("please report this file"), "{}", u.reason);
        // Everything that failed was removed again; the good sources are the only ones left.
        assert_eq!(sink.finished.len(), 2, "the binary file and the text file");
        assert_eq!(sink.aborted.len(), 3);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_sink_that_cannot_begin_a_source_and_unreadable_files_are_reported_not_raised() {
        let dir = temp();
        let (importers, profile) = env_parts();
        let cancel = AtomicBool::new(false);
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &profile,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
            window: None,
        };
        let mut no_space = Memory {
            fail_begin: true,
            ..Memory::default()
        };
        let log = write(&dir, "a.log", b"2026-10-14 10:00:00 INFO a\n");
        let u = unrecognised(import_path(&env, &log, "a.log", &HashSet::new(), &mut no_space));
        assert_eq!(u.reason, "no space left");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = write(&dir, "locked.log", b"2026-10-14 10:00:00 INFO a\n");
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
            if std::fs::File::open(&locked).is_ok() {
                // Running as root (a CI container): permissions are not enforced, so the file is readable.
                std::fs::remove_dir_all(dir).ok();
                return;
            }
            let mut sink = Memory::default();
            let out = import_path(&env, &locked, "locked.log", &HashSet::new(), &mut sink);
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
            let u = &unrecognised(out);
            assert!(u.reason.to_lowercase().contains("permission"), "{}", u.reason);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn heads_know_whether_they_are_the_whole_file() {
        let (h, whole) = read_head(&mut &b"abc"[..], 3).unwrap();
        assert_eq!((h.as_slice(), whole), (&b"abc"[..], true));
        let (h, whole) = read_head(&mut &b"abcd"[..], 3).unwrap();
        assert_eq!((h.as_slice(), whole), (&b"abc"[..], false));
        let (_, whole) = read_head(&mut &b"ab"[..], 3).unwrap();
        assert!(whole);
    }

    #[test]
    fn a_directory_given_as_a_file_is_reported_not_read() {
        let dir = temp();
        let (importers, profile) = env_parts();
        let cancel = AtomicBool::new(false);
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &profile,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
            window: None,
        };
        let mut sink = Memory::default();
        let u = unrecognised(import_path(&env, &dir, "dir", &HashSet::new(), &mut sink));
        assert!(!u.reason.is_empty());
        assert!(sink.batches.is_empty() && sink.finished.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_file_that_stopped_growing_before_the_window_is_skipped_without_a_source() {
        let dir = temp();
        let (importers, profile) = env_parts();
        let cancel = AtomicBool::new(false);
        let far_future = crate::ingest::ImportWindow::new(Some(i64::MAX / 2), None).unwrap();
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &profile,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
            window: far_future,
        };
        let mut sink = Memory::default();
        let f = write(&dir, "old.log", b"2026-10-14 10:00:00 INFO a\n");
        assert!(import_path(&env, &f, "old.log", &HashSet::new(), &mut sink).is_empty());
        assert!(sink.batches.is_empty() && sink.finished.is_empty() && sink.aborted.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }
}
