//! Reading a file from the middle, for live tail on a store: only the bytes appended since the last read
//! ([`read_appended`]), or nothing at all to start following a file at its end ([`begin_at_end`]).
//!
//! The file is planned from its start exactly as a full import plans it (parser, importer, encoding, text format
//! and day order), then read from a line start. Only whole lines are read: a last line without its newline waits
//! for the next read. Formats where an event spans lines hold their last event back too (continuation lines may
//! still follow), until `flush` (the file stopped growing). What cannot be read from the middle answers
//! [`Region::Whole`] with the reason, and the caller reads the file again in full: binary formats, archives,
//! importers that read a whole file, JSON arrays and documents, and text whose timestamps have no date (each
//! event's day depends on the events before it).

use super::batcher::{BatchedSource, RecordBatcher};
use super::chunk::{Encoding, LineReader, ReaderChunks};
use super::plan::{plan, score_text, FilePlan, JsonShape, PlanKind, HEAD_BYTES};
use super::sink::{BatchSink, EventSink, NewSource, ParsedEvent, SourceReport};
use super::source::ImportEnv;
use super::{json, text, ProblemKind};
use crate::api::FileKind;
use crate::batch::SourceId;
use crate::importers::ImportCtx;
use crate::parser::set::{Candidate, SCORE_CHARS};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Bytes read at a time when looking for newlines.
const SCAN_BLOCK: usize = 1 << 20;

/// Where a read starts: the start of a line, and that line's number (1-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailStart {
    pub byte: u64,
    pub line: u32,
}

/// What reading a file from the middle came to.
#[derive(Debug)]
pub enum Region {
    /// The bytes from the start to `next.byte` were read.
    Read(RegionRead),
    /// The file cannot be read from the middle; read it again in full. The reason is for the user.
    Whole(String),
}

/// A region that was read.
#[derive(Debug)]
pub struct RegionRead {
    /// What the events written came to (the pipeline ran over them as in a full import).
    pub batched: BatchedSource,
    /// Lines the importer skipped.
    pub skipped: usize,
    /// Where the next read starts.
    pub next: TailStart,
    /// The last event was held back (it may still get continuation lines); it starts at `next`.
    pub held: bool,
    /// The file's size when it was read.
    pub size: u64,
    pub importer: String,
}

/// A file planned for reading from the middle.
struct Planned {
    file: std::fs::File,
    size: u64,
    plan: FilePlan,
    profile_ctx: (std::sync::Arc<crate::profile::CompiledProfile>, crate::ports::ParserRef),
    file_date: Option<chrono::NaiveDate>,
}

/// Why `plan` cannot be read from the middle, if it cannot.
fn not_incremental(plan: &FilePlan) -> Option<&'static str> {
    match plan.kind {
        PlanKind::Text { time_only: true, .. } => Some("its timestamps have no date, so each event's day depends on the events before it"),
        PlanKind::Text { .. } | PlanKind::Json(JsonShape::Lines) => None,
        PlanKind::Json(_) => Some("it is one JSON document, not one event per line"),
        PlanKind::Binary => Some("it is a binary format"),
        PlanKind::Legacy => Some("its importer reads whole files only"),
    }
}

/// Opens and plans `path` as a full import does; `Err(reason)` when it cannot be read from the middle.
fn planned(env: &ImportEnv, path: &Path, src: &NewSource) -> Result<std::result::Result<Planned, String>> {
    let mut file = crate::workspace::open_with_retry(path)?;
    let meta = file.metadata()?;
    let size = meta.len();
    let file_date = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| crate::time::utc_date_of(d.as_millis() as i64));
    let mut head = Vec::with_capacity(HEAD_BYTES.min(size as usize));
    (&mut file).take(HEAD_BYTES as u64).read_to_end(&mut head)?;
    let whole = (head.len() as u64) >= size;
    if head.is_empty() {
        return Ok(Err("the file is empty".into()));
    }
    if crate::archive::is_zip(&head) || crate::archive::is_gzip(&head) {
        return Ok(Err("it is an archive".into()));
    }
    let text = score_text(&head, whole);
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
        Err(reason) => return Ok(Err(reason)),
    };
    let ctx = ImportCtx {
        profile: choice.profile,
        path,
        file_date,
    };
    let plan = match plan(env.importers, &ctx, &head, whole) {
        Ok(p) => p,
        Err(reason) => return Ok(Err(reason)),
    };
    if let Some(why) = not_incremental(&plan) {
        return Ok(Err(format!("read in full when it changes: {why}")));
    }
    Ok(Ok(Planned {
        file,
        size,
        plan,
        profile_ctx: (choice.profile.clone(), choice.reference.clone()),
        file_date,
    }))
}

/// The offset just past the last newline in `[from, to)` of `file`, or `from` when there is none.
fn last_line_end(file: &mut std::fs::File, encoding: Encoding, from: u64, to: u64) -> Result<u64> {
    let unit = encoding.unit() as u64;
    let mut end = to;
    let mut buf = vec![0u8; SCAN_BLOCK];
    while end > from {
        // Blocks stay aligned to code units counted from `from` (a line start).
        let len = (end - from).min(SCAN_BLOCK as u64);
        let start = end - len;
        let start = start + (start - from) % unit;
        let n = (end - start) as usize;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buf[..n])?;
        let block = &buf[..n];
        let mut last = None;
        let mut pos = 0;
        while let Some(nl) = encoding.find_newline(&block[pos..]) {
            pos += nl + encoding.unit();
            last = Some(pos);
        }
        if let Some(p) = last {
            return Ok(start + p as u64);
        }
        if start == from {
            break;
        }
        end = start;
    }
    Ok(from)
}

/// Newlines in `[from, to)` of `file` (`from` a line start).
fn newlines(file: &mut std::fs::File, encoding: Encoding, from: u64, to: u64) -> Result<u64> {
    file.seek(SeekFrom::Start(from))?;
    let mut left = to.saturating_sub(from);
    let mut buf = vec![0u8; SCAN_BLOCK.min(left as usize).max(2)];
    let mut n = 0u64;
    while left > 0 {
        let take = (left as usize).min(buf.len());
        file.read_exact(&mut buf[..take])?;
        n += encoding.count_newlines(&buf[..take]) as u64;
        left -= take as u64;
    }
    Ok(n)
}

/// Passes events on, keeping the last one back.
struct HoldLast<'a> {
    inner: &'a mut dyn EventSink,
    held: Option<ParsedEvent>,
}

impl EventSink for HoldLast<'_> {
    fn event(&mut self, ev: ParsedEvent) -> Result<()> {
        if let Some(prev) = self.held.replace(ev) {
            self.inner.event(prev)?;
        }
        Ok(())
    }
    fn problem(&mut self, kind: ProblemKind, line: u32) {
        self.inner.problem(kind, line);
    }
}

/// The first read of a file that was imported in full and then followed: it starts at the last imported event
/// instead of after it, so continuation lines that arrived since can be seen for what they are.
#[derive(Debug, Clone, Copy)]
pub struct Resume {
    /// Where the last imported event starts (the read starts there).
    pub last: TailStart,
    /// Where it ended when the file was imported (after its last whole line).
    pub old_end: u64,
}

/// Drops the already imported last event when it is unchanged, and notes when it is not (lines continued it).
struct Confirm<'a> {
    inner: &'a mut dyn EventSink,
    resume: Resume,
    first: Option<ParsedEvent>,
    seen_first: bool,
    /// The imported event no longer matches the file: the source has to be read again in full.
    changed: bool,
}

impl Confirm<'_> {
    /// Called when the file's events ended at `end`: decides about a first event nothing followed.
    fn finish(&mut self, end: u64) {
        if self.first.take().is_some() && end != self.resume.old_end {
            self.changed = true;
        }
    }
}

impl EventSink for Confirm<'_> {
    fn event(&mut self, ev: ParsedEvent) -> Result<()> {
        if self.changed {
            return Ok(());
        }
        if !self.seen_first {
            self.seen_first = true;
            if ev.byte_offset != self.resume.last.byte {
                self.changed = true;
            } else {
                self.first = Some(ev);
            }
            return Ok(());
        }
        if self.first.take().is_some() && ev.byte_offset != self.resume.old_end {
            self.changed = true;
            return Ok(());
        }
        self.inner.event(ev)
    }
    fn problem(&mut self, kind: ProblemKind, line: u32) {
        self.inner.problem(kind, line);
    }
}

/// Reads the whole lines of `path` from `start` on as events of the existing source `source` into `sink`. With
/// `flush`, everything up to the end of the file is read, a last line without its newline and a held-back event
/// included (the file stopped growing). A start before the data (a new file) reads from the first line.
pub fn read_appended(
    env: &ImportEnv,
    path: &Path,
    src: &NewSource,
    source: SourceId,
    start: TailStart,
    flush: bool,
    sink: &mut dyn BatchSink,
) -> Result<Region> {
    read_appended_from(env, path, src, source, start, flush, None, sink)
}

/// [`read_appended`], optionally [`Resume`]d: the read starts at the last imported event; that event is not
/// written again when the file still ends it where the import saw it, otherwise (lines continued it)
/// `Region::Whole` asks for the source to be read again in full.
#[allow(clippy::too_many_arguments)]
pub fn read_appended_from(
    env: &ImportEnv,
    path: &Path,
    src: &NewSource,
    source: SourceId,
    start: TailStart,
    flush: bool,
    resume: Option<Resume>,
    sink: &mut dyn BatchSink,
) -> Result<Region> {
    let mut p = match planned(env, path, src)? {
        Ok(p) => p,
        Err(why) => return Ok(Region::Whole(why)),
    };
    let encoding = p.plan.encoding;
    let start = if start.byte < p.plan.data_start {
        TailStart {
            byte: p.plan.data_start,
            line: 1,
        }
    } else {
        start
    };
    let end = if flush {
        p.size.max(start.byte)
    } else {
        last_line_end(&mut p.file, encoding, start.byte, p.size)?
    };
    let importer = p.plan.importer.clone();
    if end <= start.byte {
        return Ok(Region::Read(RegionRead {
            batched: BatchedSource::default(),
            skipped: 0,
            next: start,
            held: false,
            size: p.size,
            importer,
        }));
    }
    let lines_in = newlines(&mut p.file, encoding, start.byte, end)?;
    // A flushed last line without its newline still counts as a line.
    let partial = u64::from(flush && last_line_end(&mut p.file, encoding, start.byte, end)? < end);
    let after = TailStart {
        byte: end,
        line: u32::try_from(u64::from(start.line) + lines_in + partial).unwrap_or(u32::MAX),
    };
    let ctx = ImportCtx {
        profile: &p.profile_ctx.0,
        path,
        file_date: p.file_date,
    };
    p.file.seek(SeekFrom::Start(start.byte))?;
    let reader = (&mut p.file).take(end - start.byte);
    let mut chunks = ReaderChunks::new(reader, encoding, start.byte).starting_at_line(start.line);
    let mut lines = LineReader::new(&mut chunks);
    let mut batcher = RecordBatcher::new(ctx.profile, env.schema.clone(), source, &src.name, sink, env.cancel);
    let hold = p.plan.multiline && !flush;
    let mut held = None;
    let mut changed = false;
    let skipped = {
        let mut holder = HoldLast {
            inner: &mut batcher,
            held: None,
        };
        let plain: &mut dyn EventSink = if hold { &mut holder } else { holder.inner };
        let mut confirm: Option<Confirm> = None;
        let events: &mut dyn EventSink = match resume {
            Some(resume) => confirm.insert(Confirm {
                inner: plain,
                resume,
                first: None,
                seen_first: false,
                changed: false,
            }),
            None => plain,
        };
        let skipped = match p.plan.kind {
            PlanKind::Text { format, day_first, .. } => text::import_text(&ctx, format, day_first, &mut lines, events)?,
            PlanKind::Json(shape) => json::import_json(&ctx, shape, end - start.byte, &mut lines, events)?,
            PlanKind::Binary | PlanKind::Legacy => return Err(anyhow!("{importer} cannot read from the middle of a file")),
        };
        if let Some(c) = confirm.as_mut() {
            c.finish(end);
            changed = c.changed;
        }
        if hold {
            held = holder.held.take();
        }
        skipped
    };
    let batched = batcher.finish()?;
    if changed {
        return Ok(Region::Whole(
            "its last event continued on the lines written after it was imported".into(),
        ));
    }
    let (next, held) = match held {
        Some(ev) => (
            TailStart {
                byte: ev.byte_offset,
                line: ev.record.line,
            },
            true,
        ),
        None => (after, false),
    };
    Ok(Region::Read(RegionRead {
        batched,
        skipped,
        next,
        held,
        size: p.size,
        importer,
    }))
}

/// A file followed from its end: its source (with no events) and where its next read starts.
#[derive(Debug)]
pub struct AtEnd {
    pub report: SourceReport,
    pub next: TailStart,
}

/// Begins following `path` at its end: plans it as a full import would, begins and finishes its source in `sink`
/// with no events, and answers where the next read starts (after the last whole line). `Whole(reason)` when it
/// cannot be read from the middle (read it in full instead).
pub fn begin_at_end(env: &ImportEnv, path: &Path, src: &NewSource, sink: &mut dyn BatchSink) -> Result<std::result::Result<AtEnd, String>> {
    let mut p = match planned(env, path, src)? {
        Ok(p) => p,
        Err(why) => return Ok(Err(why)),
    };
    let encoding = p.plan.encoding;
    let data = p.plan.data_start;
    let end = last_line_end(&mut p.file, encoding, data, p.size)?;
    let lines = newlines(&mut p.file, encoding, data, end)?;
    let id = sink.begin_source(src)?;
    let report = SourceReport {
        id,
        path: src.path.clone(),
        name: src.name.clone(),
        importer: p.plan.importer.clone(),
        parser: p.profile_ctx.1.clone(),
        file_kind: FileKind::Text,
        events: 0,
        skipped: 0,
        dropped: 0,
        step_stats: Vec::new(),
        from: None,
        to: None,
        bytes: p.size,
        problems: Vec::new(),
    };
    sink.finish_source(&report)?;
    Ok(Ok(AtEnd {
        report,
        next: TailStart {
            byte: end,
            line: u32::try_from(lines + 1).unwrap_or(u32::MAX),
        },
    }))
}

/// Where the next read of a fully imported file starts: after its last whole line, given the start (byte and
/// line) of its last event. `None` when the file cannot be read (gone).
pub fn resume_after(path: &Path, last_event: TailStart, size: u64) -> Option<TailStart> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 4];
    let n = file.read(&mut head).ok()?;
    let (encoding, data) = super::plan::detect_encoding(&head[..n]);
    let from = last_event.byte.max(data).min(size);
    let end = last_line_end(&mut file, encoding, from, size).ok()?;
    let lines = newlines(&mut file, encoding, from, end).ok()?;
    Some(TailStart {
        byte: end,
        line: u32::try_from(u64::from(last_event.line.max(1)) + lines).unwrap_or(u32::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{EventBatch, EventId, EventSchema, PendingBatch};
    use crate::importers::Importers;
    use crate::parser::ParserSet;
    use crate::profile::{CompiledProfile, Profile};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[derive(Default)]
    struct Memory {
        batches: Vec<EventBatch>,
        next_id: u64,
        begun: u32,
        finished: Vec<SourceReport>,
    }

    impl BatchSink for Memory {
        fn begin_source(&mut self, _source: &NewSource) -> Result<SourceId> {
            self.begun += 1;
            Ok(SourceId(self.begun - 1))
        }
        fn write(&mut self, batch: PendingBatch) -> Result<()> {
            let n = batch.len() as u64;
            self.batches.push(batch.assign_ids(EventId(self.next_id))?);
            self.next_id += n;
            Ok(())
        }
        fn finish_source(&mut self, report: &SourceReport) -> Result<()> {
            self.finished.push(report.clone());
            Ok(())
        }
        fn abort_source(&mut self, _id: SourceId) -> Result<()> {
            Ok(())
        }
    }

    impl Memory {
        fn messages(&self) -> Vec<(u32, String)> {
            self.batches
                .iter()
                .flat_map(|b| b.rows().map(|r| (r.line(), r.message().to_string())).collect::<Vec<_>>())
                .collect()
        }
    }

    fn temp(name: &str, bytes: &[u8]) -> (std::path::PathBuf, NewSource) {
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).unwrap();
        let dir = std::env::temp_dir().join(format!("imt-region-{}-{:x}", std::process::id(), u64::from_le_bytes(b)));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        let src = NewSource {
            path: p.to_string_lossy().into_owned(),
            name: name.into(),
        };
        (p, src)
    }

    fn append(p: &Path, text: &[u8]) {
        let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
        std::io::Write::write_all(&mut f, text).unwrap();
    }

    fn with_env<T>(f: impl FnOnce(&ImportEnv) -> T) -> T {
        let importers = Importers::default();
        let parsers = ParserSet::of_compiled(CompiledProfile::compile(Profile::default_profile()).unwrap());
        let cancel = AtomicBool::new(false);
        let env = ImportEnv {
            importers: &importers,
            parsers: &parsers,
            schema: Arc::new(EventSchema::plain()),
            cancel: &cancel,
        };
        f(&env)
    }

    fn read(env: &ImportEnv, p: &Path, src: &NewSource, start: TailStart, flush: bool, sink: &mut Memory) -> RegionRead {
        match read_appended(env, p, src, SourceId(7), start, flush, sink).unwrap() {
            Region::Read(r) => r,
            Region::Whole(why) => panic!("whole: {why}"),
        }
    }

    #[test]
    fn reads_only_whole_appended_lines_and_numbers_them() {
        let (p, src) = temp("a.log", b"2026-10-14 10:00:00 INFO one\n");
        with_env(|env| {
            let mut sink = Memory::default();
            let start = resume_after(&p, TailStart { byte: 0, line: 1 }, 29).unwrap();
            assert_eq!(start, TailStart { byte: 29, line: 2 });
            // A last line without its newline waits; the event read last is held (plain text events may continue
            // on following lines), so nothing is written yet and the next read starts at it.
            append(&p, b"2026-10-14 10:00:01 INFO two\n2026-10-14 10:00:02 INFO thr");
            let r = read(env, &p, &src, start, false, &mut sink);
            assert_eq!((sink.messages(), r.batched.events, r.held, r.next), (vec![], 0, true, start));
            // The line completes: the first event is released by the second, which is held in turn.
            append(&p, b"ee\n");
            let r = read(env, &p, &src, r.next, false, &mut sink);
            assert_eq!(sink.messages(), [(2, "two".to_string())]);
            assert_eq!((r.held, r.next), (true, TailStart { byte: 58, line: 3 }));
            // Nothing new: the held event is read and held again, nothing is written twice.
            let again = read(env, &p, &src, r.next, false, &mut sink);
            assert_eq!((again.batched.events, again.next, again.held), (0, r.next, true));
            assert_eq!(sink.messages().len(), 1);
            // Flushed (the file stopped growing): the held event and an unterminated last line are read.
            append(&p, b"2026-10-14 10:00:03 INFO four");
            let f = read(env, &p, &src, r.next, true, &mut sink);
            let all = sink.messages();
            assert_eq!(&all[1..], [(3, "three".to_string()), (4, "four".to_string())]);
            assert_eq!((f.next.line, f.held), (5, false));
            assert_eq!(f.size, std::fs::metadata(&p).unwrap().len());
        });
    }

    #[test]
    fn events_of_one_line_are_not_held_back() {
        // CLEF: one JSON object per line, so a complete line is a complete event.
        let line = |n: u32| format!("{{\"@t\":\"2026-10-14T10:00:0{n}Z\",\"@m\":\"m{n}\"}}\n");
        let (p, src) = temp("j.clef", format!("{}{}", line(0), line(1)).as_bytes());
        with_env(|env| {
            let mut sink = Memory::default();
            let start = resume_after(&p, TailStart { byte: 0, line: 1 }, std::fs::metadata(&p).unwrap().len()).unwrap();
            assert_eq!(start.line, 3);
            append(&p, line(2).as_bytes());
            let r = read(env, &p, &src, start, false, &mut sink);
            assert_eq!((sink.messages(), r.held), (vec![(3, "m2".to_string())], false));
        });
    }

    #[test]
    fn multi_line_events_hold_their_last_event_back_until_flushed() {
        let (p, src) = temp("m.log", b"2026-10-14 10:00:00 INFO one\n");
        with_env(|env| {
            let mut sink = Memory::default();
            let start = TailStart { byte: 29, line: 2 };
            append(&p, b"2026-10-14 10:00:01 ERROR boom\n");
            let r = read(env, &p, &src, start, false, &mut sink);
            assert!(r.held, "{r:?}");
            assert_eq!((r.batched.events, r.next), (0, start), "the event may still get continuation lines");
            append(&p, b"   at Foo.Bar()\n2026-10-14 10:00:02 INFO after\n");
            let r = read(env, &p, &src, r.next, false, &mut sink);
            assert_eq!(sink.messages(), [(2, "boom".to_string())]);
            let raw: Vec<String> = sink
                .batches
                .iter()
                .flat_map(|b| b.rows().map(|r| r.raw().to_string()).collect::<Vec<_>>())
                .collect();
            assert!(raw[0].contains("at Foo.Bar()"), "{raw:?}");
            assert_eq!(r.next.line, 4);
            let f = read(env, &p, &src, r.next, true, &mut sink);
            assert!(!f.held);
            assert_eq!(sink.messages().last().unwrap(), &(4, "after".to_string()));
        });
    }

    #[test]
    fn files_that_cannot_be_read_from_the_middle_are_read_whole() {
        with_env(|env| {
            let mut sink = Memory::default();
            let (p, src) = temp("a.json", b"[{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"x\"}]");
            let r = read_appended(env, &p, &src, SourceId(0), TailStart { byte: 0, line: 1 }, false, &mut sink).unwrap();
            assert!(matches!(&r, Region::Whole(why) if why.contains("JSON document")), "{r:?}");
            let (p, src) = temp("a.zip", b"PK\x03\x04rest");
            let r = read_appended(env, &p, &src, SourceId(0), TailStart { byte: 0, line: 1 }, false, &mut sink).unwrap();
            assert!(matches!(&r, Region::Whole(why) if why.contains("archive")), "{r:?}");
            assert!(begin_at_end(env, &p, &src, &mut sink).unwrap().is_err());
            let (p, src) = temp("empty.log", b"");
            assert!(matches!(
                read_appended(env, &p, &src, SourceId(0), TailStart { byte: 0, line: 1 }, false, &mut sink).unwrap(),
                Region::Whole(_)
            ));
            assert!(read_appended(
                env,
                &p.with_extension("gone"),
                &src,
                SourceId(0),
                TailStart { byte: 0, line: 1 },
                false,
                &mut sink
            )
            .is_err());
            assert!(resume_after(&p.with_extension("gone"), TailStart { byte: 0, line: 1 }, 0).is_none());
        });
    }

    #[test]
    fn a_file_begun_at_its_end_has_no_events_and_continues_after_its_lines() {
        let (p, src) = temp("e.log", b"2026-10-14 10:00:00 INFO old\n2026-10-14 10:00:01 INFO old2\npartial");
        with_env(|env| {
            let mut sink = Memory::default();
            let at = begin_at_end(env, &p, &src, &mut sink).unwrap().unwrap();
            assert_eq!(at.report.events, 0);
            assert_eq!(sink.finished.len(), 1);
            assert_eq!(at.next, TailStart { byte: 59, line: 3 });
            append(&p, b" line\n");
            let held = read(env, &p, &src, at.next, false, &mut sink);
            assert!(
                held.held && sink.messages().is_empty(),
                "the completed line waits for a possible continuation"
            );
            read(env, &p, &src, at.next, true, &mut sink);
            assert_eq!(sink.messages().len(), 1, "the completed partial line is the first new event");
            assert_eq!(sink.messages()[0].0, 3);
        });
    }

    #[test]
    fn utf16_offsets_and_lines_stay_exact() {
        let enc = |s: &str| -> Vec<u8> {
            let mut b = vec![0xFF, 0xFE];
            b.extend(s.encode_utf16().flat_map(|u| u.to_le_bytes()));
            b
        };
        let first = enc("2026-10-14 10:00:00 INFO one\n");
        let (p, src) = temp("u.log", &first);
        with_env(|env| {
            let mut sink = Memory::default();
            let start = resume_after(&p, TailStart { byte: 2, line: 1 }, first.len() as u64).unwrap();
            assert_eq!(
                start,
                TailStart {
                    byte: first.len() as u64,
                    line: 2
                }
            );
            let more: Vec<u8> = "2026-10-14 10:00:01 INFO two\n"
                .encode_utf16()
                .flat_map(|u| u.to_le_bytes())
                .collect();
            append(&p, &more);
            let r = read(env, &p, &src, start, false, &mut sink);
            assert!(r.held && sink.messages().is_empty());
            assert_eq!(r.next, start, "the held event is read again from its own start");
            let r = read(env, &p, &src, start, true, &mut sink);
            assert_eq!(sink.messages(), [(2, "two".to_string())]);
            assert_eq!(r.next.byte, (first.len() + more.len()) as u64);
        });
    }
}
