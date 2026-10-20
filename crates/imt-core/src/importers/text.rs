//! Plain-text logs described by Serilog output templates or regexes. Each line is read with the first
//! format that matches it (a file may mix formats, and JSON lines; see [`super::lines`]); lines that do not
//! start a new event (stack traces, multi-line messages) are attached to the previous event.

use super::lines::{self, Line, LineFormat, LineParser};
use super::{ImportCtx, Imported, Importer, MAX_LINE_LEN};
use crate::ingest::ProblemKind;
use crate::model::Record;
use crate::time::detect_day_first;
use anyhow::{anyhow, Result};
use serde_json::{Map, Value};

pub struct TextImporter;

pub(crate) const SNIFF_LINES: usize = 200;
/// Most text one event collects from continuation lines; the next line starts a new event.
pub const MAX_EVENT_BYTES: usize = 1 << 20;
/// Most continuation lines one event collects; the next line starts a new event.
pub const MAX_EVENT_LINES: usize = 10_000;
/// Field of an event that continues an event cut at [`MAX_EVENT_BYTES`] or [`MAX_EVENT_LINES`]: the line
/// the original event starts on. The workspace counts these per source (`SourceInfo::split_events`).
pub const CONTINUED_FIELD: &str = "ContinuedFromLine";

/// How many of `records` continue an event that was cut (see [`CONTINUED_FIELD`]).
pub fn split_events(records: &[Record]) -> usize {
    records.iter().filter(|r| r.fields.contains_key(CONTINUED_FIELD)).count()
}

impl Importer for TextImporter {
    fn id(&self) -> &'static str {
        "text"
    }

    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8 {
        if lines::best_format(ctx.profile, head).is_some() {
            70
        } else {
            0
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let fmt = lines::best_format(ctx.profile, content)
            .ok_or_else(|| anyhow!("no output template or regex in the profile matches this file"))?;
        Ok(parse(ctx, fmt, content, content))
    }

    fn import_tail(&self, ctx: &ImportCtx, head: &str, tail: &str) -> Option<Result<Imported>> {
        let fmt = lines::best_format(ctx.profile, head)?;
        Some(Ok(parse(ctx, fmt, head, tail)))
    }
}

/// The day order of a file: the profile's, else detected from the timestamps `fmt` reads in `sample`
/// (the file's start).
pub(crate) fn day_order(ctx: &ImportCtx, fmt: LineFormat, sample: &str, max_lines: usize) -> Option<bool> {
    ctx.profile.timestamps.day_first().or_else(|| match fmt {
        LineFormat::Text(i) => {
            let regex = &ctx.profile.text_formats[i].regex;
            detect_day_first(
                sample
                    .lines()
                    .take(max_lines)
                    .filter_map(|l| regex.captures(l).and_then(|c| c.name("timestamp")).map(|m| m.as_str())),
            )
        }
        LineFormat::Json => None,
    })
}

/// Parses `content` trying `fmt` first; `sample` (the file's start) decides an ambiguous day/month order.
fn parse(ctx: &ImportCtx, fmt: LineFormat, sample: &str, content: &str) -> Imported {
    let mut parser = LineParser::new(ctx.profile, fmt, day_order(ctx, fmt, sample, 2000), ctx.file_date);
    let mut out = Imported::default();
    let mut current: Option<Record> = None;
    let mut extra = String::new();
    // Continuation lines in `extra`, and the line the current event (or the one it continues) starts on.
    let mut extra_lines = 0usize;
    let mut event_start = 0u32;
    // Text a continuation event already holds in its message.
    let mut held = 0usize;

    for (i, raw) in content.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        let number = i as u32 + 1;
        if line.len() > MAX_LINE_LEN {
            out.skipped += 1;
            continue;
        }
        let rec = match parser.line(line, number) {
            Line::Event(rec) => rec,
            other => {
                if matches!(other, Line::Unmatched) {
                    out.problems.add(ProblemKind::UnmatchedLine, number);
                }
                let size = held + extra.len();
                let full = size > 0 && (size + 1 + line.len() > MAX_EVENT_BYTES || extra_lines >= MAX_EVENT_LINES);
                if let Some(rec) = current.take_if(|_| full) {
                    // Too big for one event: the rest continues in a new one (same time and level).
                    let mut next = Record::new(number);
                    (next.ts, next.level) = (rec.ts, rec.level);
                    next.fields.insert(CONTINUED_FIELD.into(), Value::from(event_start));
                    next.message = line.to_string();
                    out.records.push(finish(rec, std::mem::take(&mut extra)));
                    (extra_lines, held) = (0, line.len());
                    current = Some(next);
                } else if current.is_some() {
                    if !extra.is_empty() {
                        extra.push('\n');
                    }
                    extra.push_str(line);
                    extra_lines += 1;
                } else if !line.trim().is_empty() {
                    out.skipped += 1;
                }
                continue;
            }
        };
        if let Some(prev) = current.take() {
            out.records.push(finish(prev, std::mem::take(&mut extra)));
        }
        (extra_lines, held) = (0, 0);
        event_start = number;
        current = Some(rec);
    }
    if let Some(rec) = current.take() {
        out.records.push(finish(rec, extra));
    }
    out
}

pub(crate) fn finish(mut rec: Record, extra: String) -> Record {
    if extra.trim().is_empty() {
        return rec;
    }
    let looks_like_exception =
        extra.contains("Exception") || extra.contains("\n   at ") || extra.trim_start().starts_with("at ") || extra.contains("Traceback");
    if looks_like_exception && rec.exception.is_none() {
        rec.exception = Some(extra.trim_end().to_string());
    } else {
        rec.message.push('\n');
        rec.message.push_str(extra.trim_end());
    }
    rec
}

/// `{Message:lj} {Properties:j}` renders a JSON object after the message;
/// lift it into fields.
pub(crate) fn split_trailing_properties(message: &mut String, fields: &mut Map<String, Value>) {
    if !message.ends_with('}') {
        return;
    }
    for (pos, _) in message.match_indices("{\"").take(8) {
        if let Ok(Value::Object(props)) = serde_json::from_str::<Value>(&message[pos..]) {
            fields.extend(props);
            message.truncate(pos);
            let trimmed = message.trim_end().len();
            message.truncate(trimmed);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level;
    use crate::profile::{CompiledProfile, Profile};
    use chrono::NaiveDate;
    use std::path::Path;

    fn import(profile: &str, content: &str) -> Imported {
        let p = CompiledProfile::compile(Profile::from_toml(profile).unwrap()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.log"),
            file_date: NaiveDate::from_ymd_opt(2026, 10, 14),
        };
        assert!(TextImporter.sniff(&ctx, content) > 0);
        TextImporter.import(&ctx, content).unwrap()
    }

    #[test]
    fn huge_multi_line_events_are_split() {
        let profile = "";
        let mut content = String::from("2026-10-14 08:00:00.000 +00:00 [INF] start\n");
        for _ in 0..MAX_EVENT_LINES + 5 {
            content.push_str("  more\n");
        }
        content.push_str("2026-10-14 08:00:01.000 +00:00 [INF] next\n");
        let out = import(profile, &content);
        assert_eq!(out.records.len(), 3, "the event, its continuation and the next event");
        let (first, rest) = (&out.records[0], &out.records[1]);
        assert!(!first.fields.contains_key(CONTINUED_FIELD));
        assert_eq!(rest.fields.get(CONTINUED_FIELD), Some(&Value::from(1)));
        assert_eq!((rest.ts, rest.level), (first.ts, first.level));
        assert_eq!(rest.line as usize, MAX_EVENT_LINES + 2);
        assert!(!out.records[2].fields.contains_key(CONTINUED_FIELD));
        // By size: continuation lines of 600 KB, so each one after the first starts a new event.
        let big = "x".repeat(600 << 10);
        let content = format!("2026-10-14 08:00:00.000 +00:00 [INF] start\n{big}\n{big}\n{big}\n");
        let out = import(profile, &content);
        assert_eq!(out.records.len(), 3);
        assert!(out.records.iter().all(|r| r.message.len() <= MAX_EVENT_BYTES + 64));
        assert_eq!(
            out.records[2].fields.get(CONTINUED_FIELD),
            Some(&Value::from(1)),
            "still the original start"
        );
        assert_eq!(split_events(&out.records), 2);
    }

    #[test]
    fn serilog_file_sink_with_exception() {
        let out = import(
            "",
            "2026-10-14 10:41:22.318 +00:00 [ERR] Failed to capture payment {\"OrderId\": 88213, \"MachineName\": \"WEB02\"}\n\
             System.TimeoutException: The operation has timed out.\n   at Foo.Bar()\n\
             2026-10-14 10:41:24.050 +00:00 [FTL] Host terminated unexpectedly\n",
        );
        assert_eq!(out.records.len(), 2);
        let r = &out.records[0];
        assert_eq!(r.level, Level::Error);
        assert_eq!(r.message, "Failed to capture payment");
        assert_eq!(r.fields["OrderId"], 88213);
        assert!(r.exception.as_deref().unwrap().starts_with("System.TimeoutException"));
        assert_eq!(out.records[1].level, Level::Fatal);
    }

    #[test]
    fn legacy_slash_dates_via_generic_regex() {
        let out = import(
            "",
            "2026/10/14 10:00:00.123 INFO Service started\n2026/10/14 10:00:01.000 WARN Disk low\nplain continuation\n",
        );
        assert_eq!(out.records.len(), 2);
        assert_eq!(out.records[1].level, Level::Warning);
        assert_eq!(out.records[1].message, "Disk low\nplain continuation");
    }

    #[test]
    fn common_layouts_via_default_regexes() {
        // (line, level, message) for layouts seen in real logs (synthetic lines).
        let cases = [
            // ZooKeeper / log4j "%d - %-5p [%t] - %m"
            (
                "2026-10-14 10:00:00,747 - WARN  [main:QuorumPeer@1] - Interrupted",
                Level::Warning,
                "[main:QuorumPeer@1] - Interrupted",
            ),
            // Kafka "[%d] %p %m (%c)"
            (
                "[2026-10-14 10:00:00,163] ERROR [Controller id=1] failed (kafka.Controller)",
                Level::Error,
                "[Controller id=1] failed (kafka.Controller)",
            ),
            // Elasticsearch / Logstash "[%d][%-5p][%c]"
            (
                "[2026-10-14T10:00:00,177][INFO ][o.e.n.Node] [n1] started",
                Level::Information,
                "[o.e.n.Node] [n1] started",
            ),
            // Windows CBS.log / DISM.log
            (
                "2026-10-14 10:00:00, Info                  CBS    Loaded Servicing Stack",
                Level::Information,
                "CBS    Loaded Servicing Stack",
            ),
            (
                "2026-10-14 10:00:01, Error                 CSI    Failed",
                Level::Error,
                "CSI    Failed",
            ),
            // MySQL: thread id, then the level in brackets
            (
                "2026-10-14T10:00:00.335060Z 0 [Warning] TIMESTAMP is deprecated",
                Level::Warning,
                "TIMESTAMP is deprecated",
            ),
            ("2026-10-14T10:00:00.335060Z 3 [Note] Ready", Level::Information, "Ready"),
            // OpenStack / oslo.log: process id, then the level
            (
                "2026-10-14 10:00:00.008 25746 ERROR nova.compute.manager [-] boom",
                Level::Error,
                "nova.compute.manager [-] boom",
            ),
            // nginx error log
            (
                "2026/10/14 10:00:00 [error] 1176#1176: *1 connect() failed",
                Level::Error,
                "1176#1176: *1 connect() failed",
            ),
            // Python logging default
            (
                "2026-10-14 10:00:00,123 - app.db - CRITICAL - lost connection",
                Level::Fatal,
                "lost connection",
            ),
            // Slashes and a comma between date and time, with an offset.
            (
                "2025/11/08,00:04:37.759+01:00 WARN  Cache warm-up slow",
                Level::Warning,
                "Cache warm-up slow",
            ),
            ("[2025-11-08,00:04:37.759Z] INFO ready", Level::Information, "ready"),
            // A number that is not followed by a level stays in the message.
            ("2026-10-14 10:00:00 42 items left", Level::Information, "42 items left"),
            ("2026-10-14 10:00:00 errors: 3", Level::Information, "errors: 3"),
        ];
        for (line, level, message) in cases {
            let out = import("", &format!("{line}\n"));
            assert_eq!(out.records.len(), 1, "{line}");
            let r = &out.records[0];
            assert!(r.ts.is_some(), "{line}");
            assert_eq!((r.level, r.message.as_str()), (level, message), "{line}");
        }
        let out = import("", "2026-10-14 10:00:00,123 - app.db - ERROR - x\n");
        assert_eq!(out.records[0].fields["Logger"], "app.db");
        // A Java stack trace stays with its event.
        let out = import(
            "",
            "[2026-10-14 10:00:00,163] ERROR failed (kafka.Controller)\njava.io.IOException: closed\n\tat kafka.Foo.bar(Foo.scala:1)\n[2026-10-14 10:00:01,000] INFO next\n",
        );
        assert_eq!(out.records.len(), 2);
        assert!(out.records[0]
            .exception
            .as_deref()
            .is_some_and(|x| x.starts_with("java.io.IOException")));
    }

    #[test]
    fn custom_template_and_console_rollover() {
        let out = import(
            "[parse]\noutput_templates = [\"[{Timestamp:HH:mm:ss} {Level:u3}] [{MachineName}] {Message:lj}{NewLine}{Exception}\"]\nregexes = []",
            "[23:59:59 INF] [WEB01] late\n[00:00:01 WRN] [WEB01] early\n",
        );
        assert_eq!(out.records[0].fields["MachineName"], "WEB01");
        let (a, b) = (out.records[0].ts.unwrap(), out.records[1].ts.unwrap());
        assert_eq!(b - a, 2000);
    }

    #[test]
    fn day_first_detected_per_file() {
        let out = import(
            "[parse]\nregexes = ['^(?P<timestamp>\\d\\d/\\d\\d/\\d{4} \\d\\d:\\d\\d:\\d\\d) (?P<message>.*)$']\noutput_templates = []",
            "03/04/2026 10:00:00 a\n25/04/2026 10:00:00 b\n",
        );
        let first = chrono::DateTime::from_timestamp_millis(out.records[0].ts.unwrap()).unwrap();
        assert_eq!(first.date_naive(), NaiveDate::from_ymd_opt(2026, 4, 3).unwrap());
    }

    #[test]
    fn profile_day_order_wins_over_detection() {
        // The second line would detect day-first; the profile says month-first.
        let out = import(
            "[parse]\nday_first = false\nregexes = ['^(?P<timestamp>\\d\\d/\\d\\d/\\d{4} \\d\\d:\\d\\d:\\d\\d) (?P<message>.*)$']\noutput_templates = []",
            "03/04/2026 10:00:00 a\n25/04/2026 10:00:00 b\n",
        );
        let first = chrono::DateTime::from_timestamp_millis(out.records[0].ts.unwrap()).unwrap();
        assert_eq!(first.date_naive(), NaiveDate::from_ymd_opt(2026, 3, 4).unwrap());
        assert_eq!(out.records[1].ts, None, "25 is not a month");
    }

    #[test]
    fn unmatched_file_and_leading_garbage() {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.log"),
            file_date: None,
        };
        assert_eq!(TextImporter.sniff(&ctx, "no timestamps here"), 0);
        assert!(TextImporter.import(&ctx, "no timestamps here").is_err());
        let out = TextImporter.import(&ctx, "banner line\n\n2026-10-14 10:00:00 INFO a\n").unwrap();
        assert_eq!((out.records.len(), out.skipped), (1, 1));
    }

    #[test]
    fn explicit_exception_and_properties_groups() {
        let out = import(
            "[parse]\noutput_templates = []\nregexes = ['^(?P<timestamp>\\S+ \\S+) (?P<level>\\w+) (?P<message>[^|]*)\\|(?P<exception>[^|]*)\\|(?P<__props>.*)$']",
            "2026-10-14 10:00:00 ERR boom|System.Exception: x|{\"A\": 1}\n2026-10-14 10:00:01 INF fine| |not json\nnot an event\n",
        );
        assert_eq!(out.records[0].exception.as_deref(), Some("System.Exception: x"));
        assert_eq!(out.records[0].fields["A"], 1);
        assert_eq!(out.records[1].exception, None);
        assert!(out.records[1].fields.is_empty());
        assert_eq!(out.records[1].message, "fine\nnot an event");
    }

    #[test]
    fn trailing_properties_edge_cases() {
        let mut m = String::from("Value is {\"not\": closed} {\"A\": 2}");
        let mut f = Map::new();
        split_trailing_properties(&mut m, &mut f);
        assert_eq!(m, "Value is {\"not\": closed}");
        assert_eq!(f["A"], 2);
        let mut m = String::from("braces {x}");
        split_trailing_properties(&mut m, &mut f);
        assert_eq!(m, "braces {x}");
        // Exception text after an event that already has one goes to the message.
        let mut r = Record::new(1);
        r.exception = Some("first".into());
        let r = finish(r, "   at Foo()".into());
        assert_eq!(r.message, "\n   at Foo()");
    }

    #[test]
    fn skips_oversized_lines() {
        let huge = "x".repeat(MAX_LINE_LEN + 1);
        let out = import("", &format!("2026-10-14 10:00:00 INFO a\n2026-10-14 10:00:01 INFO {huge}\n"));
        assert_eq!((out.records.len(), out.skipped), (1, 1));
    }
}
