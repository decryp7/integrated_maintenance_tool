//! The text importer, streaming: the same parsing as [`crate::importers::TextImporter`] (Serilog output
//! templates and regexes tried per line, JSON lines, continuation lines, midnight rollover of time-only
//! formats; [`crate::importers::lines`]), fed line by line from a [`LineReader`] with the dominant format
//! and day order the [`FilePlan`](super::FilePlan) fixed.
//!
//! Differences from the in-memory importer (ADR 0014 §5): text before the first event is one untimed
//! event instead of skipped lines, and an event stops growing at [`MAX_EVENT_BYTES`] (the rest of its
//! continuation lines are dropped and reported).

use super::chunk::LineReader;
use super::problems::ProblemKind;
use super::sink::{EventSink, ParsedEvent};
use super::MAX_EVENT_BYTES;
use crate::importers::lines::{Line, LineFormat, LineParser};
use crate::importers::{text_finish, ImportCtx};
use crate::model::Record;
use anyhow::Result;

/// An event being collected: its record, original text and continuation lines.
struct Open {
    rec: Record,
    raw: String,
    byte_offset: u64,
    extra: String,
    /// The event reached [`MAX_EVENT_BYTES`]; further lines are dropped.
    full: bool,
}

impl Open {
    fn new(rec: Record, first: &str, byte_offset: u64) -> Open {
        Open {
            rec,
            raw: first.to_string(),
            byte_offset,
            extra: String::new(),
            full: false,
        }
    }

    /// Adds a continuation line; true when it is the first line dropped because the event is full.
    fn push(&mut self, line: &str, to_extra: bool) -> bool {
        if self.full {
            return false;
        }
        if self.raw.len() + line.len() + 1 > MAX_EVENT_BYTES {
            self.full = true;
            return true;
        }
        self.raw.push('\n');
        self.raw.push_str(line);
        if to_extra {
            if !self.extra.is_empty() {
                self.extra.push('\n');
            }
            self.extra.push_str(line);
        }
        false
    }
}

/// Reads text events trying format `fmt` first, with the plan's day order; returns the number of
/// skipped lines.
pub fn import_text(
    ctx: &ImportCtx,
    fmt: LineFormat,
    day_first: Option<bool>,
    lines: &mut LineReader,
    sink: &mut dyn EventSink,
) -> Result<usize> {
    let mut parser = LineParser::new(ctx.profile, fmt, day_first, ctx.file_date);
    let mut skipped = 0;
    let mut current: Option<Open> = None;
    // Text before the first event, kept as one untimed event.
    let mut orphan: Option<Open> = None;

    while let Some(l) = lines.next_line()? {
        if l.too_long {
            skipped += 1;
            sink.problem(ProblemKind::LineTooLong, l.number);
            continue;
        }
        let line = l.text.trim_end_matches('\r');
        let rec = match parser.line(line, l.number) {
            Line::Event(rec) => rec,
            other => {
                if matches!(other, Line::Unmatched) {
                    sink.problem(ProblemKind::UnmatchedLine, l.number);
                }
                let to_extra = current.is_some();
                if let Some(open) = current.as_mut().or(orphan.as_mut()) {
                    if open.push(line, to_extra) {
                        sink.problem(ProblemKind::EventTooLarge, l.number);
                    }
                } else if !line.trim().is_empty() {
                    let mut rec = Record::new(l.number);
                    rec.message = line.to_string();
                    orphan = Some(Open::new(rec, line, l.byte_offset));
                }
                continue;
            }
        };
        if let Some(o) = orphan.take() {
            emit_orphan(o, sink)?;
        }
        if let Some(open) = current.take() {
            emit(open, sink)?;
        }
        current = Some(Open::new(rec, line, l.byte_offset));
    }
    if let Some(o) = orphan.take() {
        emit_orphan(o, sink)?;
    }
    if let Some(open) = current.take() {
        emit(open, sink)?;
    }
    Ok(skipped)
}

fn emit(open: Open, sink: &mut dyn EventSink) -> Result<()> {
    let mut raw = open.raw;
    raw.truncate(raw.trim_end().len());
    sink.event(ParsedEvent {
        record: text_finish(open.rec, open.extra),
        raw,
        byte_offset: open.byte_offset,
    })
}

/// Text before the first event: one untimed event whose message is that text.
fn emit_orphan(open: Open, sink: &mut dyn EventSink) -> Result<()> {
    let mut raw = open.raw;
    raw.truncate(raw.trim_end().len());
    let mut rec = open.rec;
    rec.message = raw.clone();
    sink.event(ParsedEvent {
        record: rec,
        raw,
        byte_offset: open.byte_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importers::{Importer, TextImporter};
    use crate::ingest::chunk::{Encoding, ReaderChunks};
    use crate::ingest::problems::Problems;
    use crate::profile::{CompiledProfile, Profile};
    use chrono::NaiveDate;
    use std::path::Path;

    #[derive(Default)]
    struct Events {
        events: Vec<ParsedEvent>,
        problems: Problems,
    }

    impl EventSink for Events {
        fn event(&mut self, ev: ParsedEvent) -> Result<()> {
            self.events.push(ev);
            Ok(())
        }
        fn problem(&mut self, kind: ProblemKind, line: u32) {
            self.problems.add(kind, line);
        }
    }

    fn stream(profile: &CompiledProfile, content: &str, block: usize) -> (Events, usize) {
        let ctx = ImportCtx {
            profile,
            path: Path::new("x.log"),
            file_date: NaiveDate::from_ymd_opt(2026, 10, 14),
        };
        let fmt = crate::importers::lines::best_format(profile, content).unwrap();
        let mut src = ReaderChunks::with_limits(content.as_bytes(), Encoding::Utf8, 0, block, crate::importers::MAX_LINE_LEN);
        let mut lines = LineReader::new(&mut src);
        let mut out = Events::default();
        let skipped = import_text(&ctx, fmt, profile.timestamps.day_first(), &mut lines, &mut out).unwrap();
        (out, skipped)
    }

    const LOG: &str = "2026-10-14 10:41:22.318 +00:00 [ERR] Failed to capture payment {\"OrderId\": 88213}\n\
        System.TimeoutException: The operation has timed out.\n   at Foo.Bar()\n\
        2026-10-14 10:41:24.050 +00:00 [FTL] Host terminated unexpectedly\n\
        2026/10/14 10:00:01.000 WARN Disk low\nplain continuation\n\n";

    #[test]
    fn matches_the_in_memory_importer_at_any_chunk_size() {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.log"),
            file_date: NaiveDate::from_ymd_opt(2026, 10, 14),
        };
        let old = TextImporter.import(&ctx, LOG).unwrap();
        for block in [7, 64, 1 << 20] {
            let (new, skipped) = stream(&p, LOG, block);
            assert_eq!(skipped, old.skipped);
            assert_eq!(new.events.len(), old.records.len(), "block {block}");
            for (a, b) in new.events.iter().zip(&old.records) {
                let r = &a.record;
                assert_eq!(
                    (r.ts, r.level, &r.message, &r.exception, r.line),
                    (b.ts, b.level, &b.message, &b.exception, b.line)
                );
                assert_eq!(r.fields, b.fields);
            }
        }
        let (new, _) = stream(&p, LOG, 64);
        assert!(new.events[0].raw.starts_with("2026-10-14 10:41:22.318") && new.events[0].raw.ends_with("at Foo.Bar()"));
        assert_eq!(new.events[0].byte_offset, 0);
        assert_eq!(new.events[1].byte_offset, LOG.find("2026-10-14 10:41:24").unwrap() as u64);
        assert_eq!(new.events[2].raw, "2026/10/14 10:00:01.000 WARN Disk low\nplain continuation");
    }

    #[test]
    fn a_file_without_events_is_one_untimed_event_and_sinks_may_drop_aggregated_problems() {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.log"),
            file_date: None,
        };
        let content = "just text
Oct 14 10:00:00 host app: no format reads this
";
        let mut src = ReaderChunks::with_limits(content.as_bytes(), Encoding::Utf8, 0, 64, crate::importers::MAX_LINE_LEN);
        let mut lines = LineReader::new(&mut src);
        let mut out = Events::default();
        import_text(&ctx, LineFormat::Text(0), None, &mut lines, &mut out).unwrap();
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].record.ts, None);
        assert_eq!(out.problems.count(ProblemKind::UnmatchedLine), 1);
        // The default `problems` of a sink drops what a non-streaming importer aggregated.
        let mut more = Problems::default();
        more.add(ProblemKind::RouteLimit, 1);
        out.problems(more);
        assert_eq!(out.problems.count(ProblemKind::RouteLimit), 0);
    }

    #[test]
    fn leading_text_becomes_one_untimed_event() {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let (out, skipped) = stream(&p, "banner line\n\nsecond banner\n2026-10-14 10:00:00 INFO a\n", 1 << 20);
        assert_eq!(skipped, 0);
        assert_eq!(out.events.len(), 2);
        let orphan = &out.events[0];
        assert_eq!(orphan.record.ts, None);
        assert_eq!(orphan.record.line, 1);
        assert_eq!(orphan.record.message, "banner line\n\nsecond banner");
        assert_eq!(orphan.raw, orphan.record.message);
        assert_eq!(out.events[1].record.message, "a");
    }

    #[test]
    fn huge_events_are_capped_and_long_lines_skipped() {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let chunk = "y".repeat(1 << 20);
        let mut content = String::from("2026-10-14 10:00:00 ERROR a\n");
        for _ in 0..(MAX_EVENT_BYTES >> 20) + 3 {
            content.push_str(&chunk);
            content.push('\n');
        }
        content.push_str(&"z".repeat(crate::importers::MAX_LINE_LEN + 1));
        content.push_str("\n2026-10-14 10:00:01 INFO b\n");
        let (out, skipped) = stream(&p, &content, 1 << 20);
        assert_eq!(out.events.len(), 2);
        assert!(out.events[0].raw.len() <= MAX_EVENT_BYTES);
        assert_eq!(out.problems.count(ProblemKind::EventTooLarge), 1);
        assert_eq!(out.problems.count(ProblemKind::LineTooLong), 1);
        assert_eq!(skipped, 1);
        assert_eq!(out.events[1].record.message, "b");
    }

    #[test]
    fn time_only_formats_roll_over_midnight() {
        let p = crate::parser::testing::compiled(
            "[format]\noutput_templates = [\"[{Timestamp:HH:mm:ss} {Level:u3}] {Message:lj}{NewLine}{Exception}\"]\nregexes = []",
        );
        let (out, _) = stream(&p, "[23:59:59 INF] late\n[00:00:01 WRN] early\n", 8);
        let (a, b) = (out.events[0].record.ts.unwrap(), out.events[1].record.ts.unwrap());
        assert_eq!(b - a, 2000);
    }
}
