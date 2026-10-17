//! Plain-text logs described by Serilog output templates or regexes. Lines
//! that do not start a new event (stack traces, multi-line messages) are
//! attached to the previous event.

use super::{first_lines, ImportCtx, Imported, Importer, MAX_LINE_LEN};
use crate::model::{auto_value, Level, Record};
use crate::profile::TextFormat;
use crate::template::PROPERTIES_GROUP;
use crate::time::detect_day_first;
use anyhow::{anyhow, Result};
use serde_json::{Map, Value};

pub struct TextImporter;

const SNIFF_LINES: usize = 200;

impl Importer for TextImporter {
    fn id(&self) -> &'static str {
        "text"
    }

    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8 {
        if best_format(&ctx.profile.text_formats, head).is_some() {
            70
        } else {
            0
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let formats = &ctx.profile.text_formats;
        let fmt = best_format(formats, content).ok_or_else(|| anyhow!("no output template or regex in the profile matches this file"))?;
        Ok(parse(ctx, &formats[fmt], content, content))
    }

    fn import_tail(&self, ctx: &ImportCtx, head: &str, tail: &str) -> Option<Result<Imported>> {
        let formats = &ctx.profile.text_formats;
        let fmt = best_format(formats, head)?;
        Some(Ok(parse(ctx, &formats[fmt], head, tail)))
    }
}

/// Parses `content` with one format; `sample` (the file's start) decides an
/// ambiguous day/month order.
fn parse(ctx: &ImportCtx, fmt: &TextFormat, sample: &str, content: &str) -> Imported {
    let mut parser = ctx.profile.timestamps.clone();
    if parser.day_first().is_none() {
        let samples: Vec<&str> = sample
            .lines()
            .take(2000)
            .filter_map(|l| fmt.regex.captures(l).and_then(|c| c.name("timestamp")).map(|m| m.as_str()))
            .collect();
        parser = parser.with_day_first(detect_day_first(samples));
    }

    let names: Vec<&str> = fmt.regex.capture_names().flatten().collect();
    let mut out = Imported::default();
    let mut current: Option<Record> = None;
    let mut extra = String::new();
    let mut date = ctx.file_date;
    let mut last_ts: Option<i64> = None;

    for (i, raw) in content.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.len() > MAX_LINE_LEN {
            out.skipped += 1;
            continue;
        }
        let Some(caps) = fmt.regex.captures(line) else {
            if current.is_some() {
                if !extra.is_empty() {
                    extra.push('\n');
                }
                extra.push_str(line);
            } else if !line.trim().is_empty() {
                out.skipped += 1;
            }
            continue;
        };
        if let Some(rec) = current.take() {
            out.records.push(finish(rec, std::mem::take(&mut extra)));
        }

        let mut rec = Record::new(i as u32 + 1);
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
        current = Some(rec);
    }
    if let Some(rec) = current.take() {
        out.records.push(finish(rec, extra));
    }
    out
}

/// Index of the format matching the most of the first lines, earlier formats
/// winning ties.
fn best_format(formats: &[TextFormat], text: &str) -> Option<usize> {
    let lines: Vec<&str> = first_lines(text, SNIFF_LINES).collect();
    let mut best: Option<(usize, usize)> = None;
    for (i, f) in formats.iter().enumerate() {
        let hits = lines.iter().filter(|l| f.regex.is_match(l)).count();
        if hits > 0 && best.is_none_or(|(_, h)| hits > h) {
            best = Some((i, hits));
        }
    }
    best.map(|(i, _)| i)
}

fn finish(mut rec: Record, extra: String) -> Record {
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
fn split_trailing_properties(message: &mut String, fields: &mut Map<String, Value>) {
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
