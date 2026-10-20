//! The text importer, streaming: the same parsing as [`crate::importers::TextImporter`] (Serilog output
//! templates and regexes, continuation lines, midnight rollover of time-only formats), fed line by line
//! from a [`LineReader`] with the format and day order the [`FilePlan`](super::FilePlan) fixed.
//!
//! Differences from the in-memory importer (ADR 0014 §5): text before the first event is one untimed
//! event instead of skipped lines, and an event stops growing at [`MAX_EVENT_BYTES`] (the rest of its
//! continuation lines are dropped and reported).

use super::chunk::LineReader;
use super::problems::ProblemKind;
use super::sink::{EventSink, ParsedEvent};
use super::MAX_EVENT_BYTES;
use crate::importers::{split_trailing_properties, text_finish, ImportCtx};
use crate::model::{auto_value, Level, Record};
use crate::profile::TextFormat;
use crate::template::PROPERTIES_GROUP;
use anyhow::Result;
use serde_json::Value;

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

/// Reads text events with format `fmt` and the plan's day order; returns the number of skipped lines.
pub fn import_text(
    ctx: &ImportCtx,
    fmt: &TextFormat,
    day_first: Option<bool>,
    lines: &mut LineReader,
    sink: &mut dyn EventSink,
) -> Result<usize> {
    let parser = ctx.profile.timestamps.with_day_first(day_first);
    let names: Vec<&str> = fmt.regex.capture_names().flatten().collect();
    let mut skipped = 0;
    let mut current: Option<Open> = None;
    // Text before the first event, kept as one untimed event.
    let mut orphan: Option<Open> = None;
    let mut date = ctx.file_date;
    let mut last_ts: Option<i64> = None;

    while let Some(l) = lines.next_line()? {
        if l.too_long {
            skipped += 1;
            sink.problem(ProblemKind::LineTooLong, l.number);
            continue;
        }
        let line = l.text.trim_end_matches('\r');
        let Some(caps) = fmt.regex.captures(line) else {
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
        };
        if let Some(o) = orphan.take() {
            emit_orphan(o, sink)?;
        }
        if let Some(open) = current.take() {
            emit(open, sink)?;
        }

        let mut rec = Record::new(l.number);
        for name in &names {
            let Some(m) = caps.name(name) else { continue };
            let v = m.as_str();
            match *name {
                "timestamp" => {
                    rec.ts = if fmt.time_only {
                        parser.parse(v).or_else(|| {
                            date.and_then(|d| parser.parse_time_on(v, d)).map(|mut ts| {
                                // Rolled past midnight.
                                if last_ts.is_some_and(|prev| ts + 12 * 3_600_000 < prev) {
                                    ts += 86_400_000;
                                    date = date.and_then(|d| d.succ_opt());
                                }
                                ts
                            })
                        })
                    } else {
                        parser.parse(v)
                    };
                    if rec.ts.is_some() {
                        last_ts = rec.ts;
                    }
                }
                "level" => rec.level = Level::parse(v).unwrap_or(Level::Information),
                "message" => rec.message = v.to_string(),
                "exception" => {
                    if !v.trim().is_empty() {
                        rec.exception = Some(v.to_string());
                    }
                }
                PROPERTIES_GROUP => {
                    if let Ok(Value::Object(props)) = serde_json::from_str::<Value>(v) {
                        rec.fields.extend(props);
                    }
                }
                other => {
                    rec.fields.insert(other.to_string(), auto_value(v));
                }
            }
        }
        if !names.contains(&PROPERTIES_GROUP) {
            split_trailing_properties(&mut rec.message, &mut rec.fields);
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
        let fmt = crate::importers::text_best_format(&profile.text_formats, content).unwrap();
        let mut src = ReaderChunks::with_limits(content.as_bytes(), Encoding::Utf8, 0, block, crate::importers::MAX_LINE_LEN);
        let mut lines = LineReader::new(&mut src);
        let mut out = Events::default();
        let skipped = import_text(
            &ctx,
            &profile.text_formats[fmt],
            profile.timestamps.day_first(),
            &mut lines,
            &mut out,
        )
        .unwrap();
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
        let p = CompiledProfile::compile(
            Profile::from_toml(
                "[parse]\noutput_templates = [\"[{Timestamp:HH:mm:ss} {Level:u3}] {Message:lj}{NewLine}{Exception}\"]\nregexes = []",
            )
            .unwrap(),
        )
        .unwrap();
        let (out, _) = stream(&p, "[23:59:59 INF] late\n[00:00:01 WRN] early\n", 8);
        let (a, b) = (out.events[0].record.ts.unwrap(), out.events[1].record.ts.unwrap());
        assert_eq!(b - a, 2000);
    }
}
