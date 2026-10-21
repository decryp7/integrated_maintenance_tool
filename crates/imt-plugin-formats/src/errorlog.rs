//! SQL Server `ERRORLOG` (UTF-16 LE with BOM on Windows, UTF-8 on Linux; the
//! workspace decodes both): `2026-10-17 12:00:00.12 spid52      message`.
//!
//! * Lines that do not start with a timestamp continue the record before them.
//! * `Error: 18456, Severity: 14, State: 8.` folds into the next record of the
//!   same process (the message SQL Server writes right after it) as the
//!   numeric fields `ErrorNumber`, `Severity` and `State`; alone it is a record.
//! * Level from the severity: 0-10 Information, 11-16 (user-correctable)
//!   Warning, 17-19 (resource and software errors) Error, 20-25 (fatal) Fatal.
//! * Times are local to the server. The log's `UTC adjustment: -4:00` line
//!   gives the offset; without it the profile's assumed time zone applies.

use crate::first_line;
use anyhow::Result;
use chrono::{FixedOffset, NaiveDateTime, TimeZone};
use imt_plugin_api::importer::{ImportCtx, Imported, TextImporter, MAX_LINE_LEN};
use imt_plugin_api::model::{Level, Record};
use regex::{Captures, Regex};
use serde_json::Value;
use std::sync::OnceLock;

pub struct SqlErrorLogImporter;

fn line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{2}) (\S+)(?: +(.*))?$").expect("static regex"))
}

fn error_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^Error: (\d{1,10}), Severity: (\d{1,3}), State: (\d{1,5})\.?\s*$").expect("static regex"))
}

fn spid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^spid(\d{1,9})[A-Za-z]?$").expect("static regex"))
}

/// The process column of a line SQL Server itself writes.
fn known_source(s: &str) -> bool {
    matches!(s, "Server" | "Logon" | "Backup") || spid_re().is_match(s)
}

impl TextImporter for SqlErrorLogImporter {
    fn id(&self) -> &'static str {
        "sql-errorlog"
    }

    /// Above the generic text importer (70): the first line has the ERRORLOG
    /// shape (centisecond timestamp, process column) and a known process.
    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let Some(c) = first_line(head).and_then(|l| line_re().captures(l)) else {
            return 0;
        };
        let known = known_source(&c[2])
            || head
                .lines()
                .take(20)
                .filter_map(|l| line_re().captures(l.trim_end()))
                .any(|c| known_source(&c[2]));
        if known {
            95
        } else {
            0
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        Ok(import(ctx, content, content))
    }

    fn import_tail(&self, ctx: &ImportCtx, head: &str, tail: &str) -> Option<Result<Imported>> {
        Some(Ok(import(ctx, head, tail)))
    }
}

/// The server's offset from UTC, from `UTC adjustment: 2:00` near the start.
fn utc_adjustment(head: &str) -> Option<FixedOffset> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"UTC adjustment: (-?)(\d{1,2}):(\d{2})").expect("static regex"));
    let head = &head[..head.char_indices().nth(64 * 1024).map_or(head.len(), |(i, _)| i)];
    let c = re.captures(head)?;
    let secs = c[2].parse::<i32>().ok()? * 3600 + c[3].parse::<i32>().ok()? * 60;
    FixedOffset::east_opt(if &c[1] == "-" { -secs } else { secs })
}

/// Level for a SQL Server error severity.
pub(crate) fn level_for_severity(severity: u64) -> Level {
    match severity {
        0..=10 => Level::Information,
        11..=16 => Level::Warning,
        17..=19 => Level::Error,
        _ => Level::Fatal,
    }
}

/// `Error: n, Severity: s, State: t.` already parsed.
struct ErrorLine {
    record: Record,
    number: u64,
    severity: u64,
    state: u64,
}

impl ErrorLine {
    fn apply(&self, r: &mut Record) {
        r.fields.insert("ErrorNumber".into(), Value::from(self.number));
        r.fields.insert("Severity".into(), Value::from(self.severity));
        r.fields.insert("State".into(), Value::from(self.state));
        r.level = level_for_severity(self.severity);
    }

    fn standalone(mut self) -> Record {
        let mut r = std::mem::replace(&mut self.record, Record::new(0));
        self.apply(&mut r);
        r
    }
}

struct Builder<'a> {
    ctx: &'a ImportCtx<'a>,
    offset: Option<FixedOffset>,
    out: Imported,
    current: Option<Record>,
    error: Option<ErrorLine>,
}

impl Builder<'_> {
    fn header(&self, line: u32, c: &Captures) -> Record {
        let mut r = Record::new(line);
        r.ts = match self.offset {
            Some(off) => NaiveDateTime::parse_from_str(&c[1], "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .and_then(|n| off.from_local_datetime(&n).single())
                .map(|d| d.timestamp_millis()),
            None => self.ctx.timestamps.parse(&c[1]),
        };
        let source = &c[2];
        r.fields.insert("Source".into(), Value::from(source));
        if let Some(spid) = spid_re().captures(source).and_then(|s| s[1].parse::<u64>().ok()) {
            r.fields.insert("Spid".into(), Value::from(spid));
        }
        r.message = c.get(3).map_or("", |m| m.as_str()).to_string();
        r
    }

    /// A record is complete: fold an error line into it or keep it for the next one.
    fn finish_current(&mut self) {
        let Some(mut r) = self.current.take() else { return };
        if let Some(c) = error_re().captures(&r.message) {
            let parse = |i: usize| c[i].parse::<u64>().unwrap_or(0);
            let next = ErrorLine {
                number: parse(1),
                severity: parse(2),
                state: parse(3),
                record: r,
            };
            if let Some(prev) = self.error.replace(next) {
                self.out.records.push(prev.standalone());
            }
            return;
        }
        if let Some(e) = self.error.take() {
            // SQL Server writes the message right after the error line, from the same process.
            if e.record.fields.get("Source") == r.fields.get("Source") {
                e.apply(&mut r);
                r.line = e.record.line;
                if r.ts.is_none() {
                    r.ts = e.record.ts;
                }
            } else {
                self.out.records.push(e.standalone());
            }
        }
        self.out.records.push(r);
    }

    fn finish(mut self) -> Imported {
        self.finish_current();
        if let Some(e) = self.error.take() {
            self.out.records.push(e.standalone());
        }
        self.out
    }
}

fn import(ctx: &ImportCtx, head: &str, content: &str) -> Imported {
    let mut b = Builder {
        ctx,
        offset: utc_adjustment(head),
        out: Imported::default(),
        current: None,
        error: None,
    };
    for (i, line) in content.trim_start_matches('\u{feff}').lines().enumerate() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_LINE_LEN {
            b.out.skipped += 1;
            continue;
        }
        if let Some(c) = line_re().captures(line) {
            b.finish_current();
            b.current = Some(b.header(i as u32 + 1, &c));
            continue;
        }
        match &mut b.current {
            Some(r) if r.message.len() + line.len() < MAX_LINE_LEN => {
                r.message.push('\n');
                r.message.push_str(line);
            }
            _ => b.out.skipped += 1,
        }
    }
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{garbage, ms, with_ctx};

    const LOG: &str = "\u{feff}2026-10-17 12:00:00.12 Server      Microsoft SQL Server 2022 (RTM-CU14) - 16.0.4135.4 (X64) \r
\tJul 10 2026 16:09:21 \r
\tCopyright (C) 2022 Microsoft Corporation\r
\r
2026-10-17 12:00:00.12 Server      UTC adjustment: 2:00\r
2026-10-17 12:00:00.45 spid10s     Starting up database 'master'.\r
2026-10-17 12:00:05.30 Logon       Error: 18456, Severity: 14, State: 8.\r
2026-10-17 12:00:05.30 Logon       Login failed for user 'sa'. Reason: Password did not match that for the login provided. [CLIENT: 10.0.0.5]\r
2026-10-17 12:00:06.00 spid52      Error: 1205, Severity: 13, State: 51.\r
2026-10-17 12:00:06.01 spid53      Error: 9002, Severity: 17, State: 2.\r
2026-10-17 12:00:06.01 spid53      The transaction log for database 'Orders' is full due to 'LOG_BACKUP'.\r
2026-10-17 12:00:07.00 spid60      Error: 823, Severity: 24, State: 2.\r
";

    #[test]
    fn parses_continuations_folds_errors_and_converts_time() {
        let out = with_ctx(|c| {
            assert_eq!(SqlErrorLogImporter.sniff(c, LOG), 95);
            SqlErrorLogImporter.import(c, LOG).unwrap()
        });
        assert_eq!(SqlErrorLogImporter.id(), "sql-errorlog");
        assert_eq!(out.skipped, 0);
        let r = &out.records;
        assert_eq!(r.len(), 7, "{:#?}", r.iter().map(|r| &r.message).collect::<Vec<_>>());
        // Banner with its continuation lines; local time is UTC+2.
        assert!(r[0].message.starts_with("Microsoft SQL Server 2022"));
        assert!(r[0].message.ends_with("Copyright (C) 2022 Microsoft Corporation"));
        assert_eq!(r[0].message.lines().count(), 3);
        assert_eq!(r[0].ts, Some(ms("2026-10-17T10:00:00.12Z")));
        assert_eq!((r[0].fields["Source"].as_str(), r[0].level), (Some("Server"), Level::Information));
        assert_eq!(r[2].fields["Spid"], 10);
        // Folded login failure.
        let login = &r[3];
        assert!(login.message.starts_with("Login failed for user 'sa'"));
        assert_eq!(
            (
                login.fields["ErrorNumber"].as_u64(),
                login.fields["Severity"].as_u64(),
                login.fields["State"].as_u64()
            ),
            (Some(18456), Some(14), Some(8))
        );
        assert_eq!((login.level, login.line), (Level::Warning, 7));
        // An error line followed by another process stays alone.
        assert_eq!(
            (r[4].message.as_str(), r[4].fields["ErrorNumber"].as_u64()),
            ("Error: 1205, Severity: 13, State: 51.", Some(1205))
        );
        assert_eq!(r[5].level, Level::Error);
        assert!(r[5].message.starts_with("The transaction log"));
        // Error line at the end of the file.
        assert_eq!((r[6].level, r[6].fields["Severity"].as_u64()), (Level::Fatal, Some(24)));
    }

    #[test]
    fn without_utc_adjustment_the_profile_zone_applies() {
        let log = "2026-10-17 12:00:00.12 spid5s      Recovery is complete.\nstray continuation\n";
        let out = with_ctx(|c| SqlErrorLogImporter.import(c, log).unwrap());
        assert_eq!(out.records[0].ts, Some(ms("2026-10-17T12:00:00.12Z")));
        assert!(out.records[0].message.ends_with("\nstray continuation"));
        // Continuations before any record are skipped; tail uses the head's offset.
        let out = with_ctx(|c| {
            SqlErrorLogImporter
                .import_tail(c, "x UTC adjustment: -4:30\n", "orphan\n2026-10-17 12:00:00.00 Server      x\n")
                .unwrap()
                .unwrap()
        });
        assert_eq!((out.records.len(), out.skipped), (1, 1));
        assert_eq!(out.records[0].ts, Some(ms("2026-10-17T16:30:00Z")));
        assert_eq!(utc_adjustment("UTC adjustment: 99:00"), None);
    }

    #[test]
    fn sniff_and_severity_levels() {
        with_ctx(|c| {
            for head in [
                "",
                "2026-10-14 09:00:00.000 +00:00 [INF] hello",
                "2026-10-14 09:00:00,000 [1] INFO Billing - x",
                "2026-10-17 12:00:00.12 Custom      x",
                "garbage\n2026-10-17 12:00:00.12 Server      x",
            ] {
                assert_eq!(SqlErrorLogImporter.sniff(c, head), 0, "{head}");
            }
            assert_eq!(SqlErrorLogImporter.sniff(c, "2026-10-17 12:00:00.12 Backup      x"), 95);
            assert_eq!(
                SqlErrorLogImporter.sniff(c, "2026-10-17 12:00:00.12 Other x\n2026-10-17 12:00:00.12 spid7s x"),
                95
            );
            let long = format!(
                "2026-10-17 12:00:00.12 Server      {}\n{}\n",
                "x".repeat(MAX_LINE_LEN - 100),
                "y".repeat(200)
            );
            assert_eq!(SqlErrorLogImporter.import(c, &long).unwrap().skipped, 1);
            let huge = format!("{}\n", "z".repeat(MAX_LINE_LEN + 1));
            assert_eq!(SqlErrorLogImporter.import(c, &huge).unwrap().skipped, 1);
        });
        assert_eq!(level_for_severity(10), Level::Information);
        assert_eq!(level_for_severity(16), Level::Warning);
        assert_eq!(level_for_severity(19), Level::Error);
        assert_eq!(level_for_severity(25), Level::Fatal);
    }

    #[test]
    fn garbage_never_panics() {
        with_ctx(|c| {
            for g in garbage(
                &[
                    "2026-10-17 12:00:00.12 ",
                    "spid52",
                    "Server",
                    "      ",
                    "Error: 18456, Severity: 14, State: 8.",
                    "UTC adjustment: -4:00",
                    "\n",
                    "\r\n",
                    "\t",
                    "x",
                    "é",
                ],
                300,
            ) {
                let _ = SqlErrorLogImporter.sniff(c, &g);
                let _ = SqlErrorLogImporter.import(c, &g);
            }
        });
    }
}
