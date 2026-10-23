//! Sysinternals Process Monitor CSV exports ("File > Save... > Comma-Separated Values").
//!
//! The header row names the columns shown in ProcMon when the file was saved. The
//! default set is `Time of Day, Process Name, PID, Operation, Path, Result, Detail`;
//! any of the optional columns (`Sequence`, `Date & Time`, `TID`, `Duration`,
//! `Event Class`, `Image Path`, `Command Line`, `User`, `Session`, `Architecture`,
//! `Category`, `Parent PID`, `Company`, `Description`, `Version`, `Integrity`, ...)
//! may be present too, in any order.
//!
//! One record per row. Known columns get stable field names (`ProcessName`,
//! `ProcessId`, `ThreadId`, `Operation`, `Path`, `Result`, `Detail`, `Duration`, ...)
//! with numbers typed; unknown columns keep their header with non-alphanumerics
//! removed. The level follows `Result` ([`result_level`]).
//!
//! Timestamps are local time of the capture, interpreted in the profile's zone:
//! `Date & Time` gives the date, `Time of Day` the (sub-second) time. Without a
//! `Date & Time` column the file date is used, rolling over to the next day when the
//! time of day jumps back by more than 12 hours.

use anyhow::{bail, Result};
use debrief_plugin_api::importer::{ImportCtx, Imported, TextImporter, MAX_LINE_LEN};
use debrief_plugin_api::model::{Level, Record};
use debrief_plugin_api::time::{detect_day_first, utc_date_of, TimestampParser};
use serde_json::Value;

/// Columns that identify a ProcMon export; all must be in the header.
const REQUIRED: [&str; 4] = ["Process Name", "Operation", "Path", "Result"];
const TIME_OF_DAY: &str = "Time of Day";
const DATE_AND_TIME: &str = "Date & Time";
/// Rows sampled to decide whether `Date & Time` is day-first.
const DAY_ORDER_SAMPLES: usize = 1000;
const HALF_DAY_MS: i64 = 12 * 3_600_000;
const DAY_MS: i64 = 24 * 3_600_000;

pub struct ProcmonCsvImporter;

impl TextImporter for ProcmonCsvImporter {
    fn id(&self) -> &'static str {
        "procmon-csv"
    }

    /// 95 when the first line is a ProcMon header: above the generic CSV importer (≤ 80).
    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let first = head.trim_start_matches('\u{feff}').lines().next().unwrap_or("");
        if is_procmon_header(&split_header(first)) {
            95
        } else {
            0
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        parse_procmon(content, ctx.timestamps, ctx.file_date)
    }

    /// Rows never span lines in practice, but `Command Line` and `Detail` may be quoted
    /// with embedded newlines, so live tail re-reads from the last record.
    fn multiline(&self) -> bool {
        true
    }
}

fn split_header(line: &str) -> Vec<String> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(line.as_bytes());
    match reader.records().next() {
        Some(Ok(r)) => r.iter().map(|c| c.trim().to_string()).collect(),
        _ => Vec::new(),
    }
}

fn is_procmon_header(cols: &[String]) -> bool {
    let has = |name: &str| cols.iter().any(|c| c.eq_ignore_ascii_case(name));
    REQUIRED.iter().all(|c| has(c)) && (has(TIME_OF_DAY) || has(DATE_AND_TIME))
}

/// How a column is stored.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Integer,
    Number,
    /// Used for the timestamp; also kept as text.
    TimeOfDay,
    DateTime,
}

/// The field name and kind for a header, by the column names ProcMon writes.
fn column(header: &str) -> (String, Kind) {
    let known = match header.to_ascii_lowercase().as_str() {
        "time of day" => Some(("TimeOfDay", Kind::TimeOfDay)),
        "date & time" => Some(("DateTime", Kind::DateTime)),
        "process name" => Some(("ProcessName", Kind::Text)),
        "pid" => Some(("ProcessId", Kind::Integer)),
        "tid" => Some(("ThreadId", Kind::Integer)),
        "parent pid" => Some(("ParentProcessId", Kind::Integer)),
        "session" => Some(("Session", Kind::Integer)),
        "sequence" => Some(("Sequence", Kind::Integer)),
        "duration" => Some(("Duration", Kind::Number)),
        "operation" => Some(("Operation", Kind::Text)),
        "path" => Some(("Path", Kind::Text)),
        "result" => Some(("Result", Kind::Text)),
        "detail" => Some(("Detail", Kind::Text)),
        "event class" => Some(("EventClass", Kind::Text)),
        "image path" => Some(("ImagePath", Kind::Text)),
        "command line" => Some(("CommandLine", Kind::Text)),
        "user" => Some(("User", Kind::Text)),
        _ => None,
    };
    match known {
        Some((name, kind)) => (name.to_string(), kind),
        None => {
            let name: String = header.chars().filter(|c| c.is_alphanumeric()).collect();
            (if name.is_empty() { "Column".to_string() } else { name }, Kind::Text)
        }
    }
}

/// Results that are part of normal operation (probing for files, enumerating until
/// the end, buffer size negotiation), not failures.
const BENIGN: &[&str] = &[
    "NAME NOT FOUND",
    "PATH NOT FOUND",
    "NO MORE ENTRIES",
    "NO MORE FILES",
    "NO SUCH FILE",
    "BUFFER OVERFLOW",
    "BUFFER TOO SMALL",
    "END OF FILE",
    "REPARSE",
    "NOT REPARSE POINT",
    "FAST IO DISALLOWED",
    "OPLOCK NOT GRANTED",
    "FILE LOCKED WITH ONLY READERS",
    "FILE LOCKED WITH WRITERS",
    "IS DIRECTORY",
    "NOT A DIRECTORY",
    "NAME COLLISION",
    "INVALID DEVICE REQUEST",
    "NOT IMPLEMENTED",
];

/// Results that usually explain a malfunction.
const FAILURES: &[&str] = &[
    "ACCESS DENIED",
    "SHARING VIOLATION",
    "PRIVILEGE NOT HELD",
    "DISK FULL",
    "INSUFFICIENT RESOURCES",
    "IO DEVICE ERROR",
    "BAD NETWORK PATH",
    "BAD NETWORK NAME",
    "LOGON FAILURE",
    "CANNOT DELETE",
    "DELETE PENDING",
    "FILE CORRUPT",
    "OBJECT PATH SYNTAX BAD",
    "BAD IMPERSONATION LEVEL",
    "NETWORK ACCESS DENIED",
    "CONNECTION REFUSED",
    "CONNECTION RESET",
    "TIMEOUT",
];

/// The level of an event by its `Result`: success and empty results are Information,
/// expected misses ([`BENIGN`]) Debug, known failures ([`FAILURES`]) Error, anything
/// else (an unusual status, a hex code) Warning.
pub fn result_level(result: &str) -> Level {
    let r = result.trim();
    if r.is_empty() || r.eq_ignore_ascii_case("SUCCESS") {
        Level::Information
    } else if BENIGN.iter().any(|b| r.eq_ignore_ascii_case(b)) {
        Level::Debug
    } else if FAILURES.iter().any(|f| r.eq_ignore_ascii_case(f)) {
        Level::Error
    } else {
        Level::Warning
    }
}

fn integer(cell: &str) -> Option<Value> {
    let c = cell.trim().replace([',', '\u{a0}'], "");
    c.parse::<i64>().ok().map(Value::from)
}

fn number(cell: &str) -> Option<Value> {
    let v: f64 = cell.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    serde_json::Number::from_f64(v).map(Value::Number)
}

/// The date part (first word) of a `Date & Time` cell.
fn date_part(cell: &str) -> &str {
    cell.split_whitespace().next().unwrap_or("")
}

/// Turns the time columns of a row into epoch milliseconds.
struct Clock {
    parser: TimestampParser,
    /// Parses the bare date (midnight UTC) so its calendar date can be read back.
    date_parser: TimestampParser,
    fallback_date: Option<chrono::NaiveDate>,
    /// Day offset added after a time-of-day rollover (only without `Date & Time`).
    days: i64,
    last: Option<i64>,
}

impl Clock {
    fn at(&mut self, time_of_day: Option<&str>, date_time: Option<&str>) -> Option<i64> {
        match (
            time_of_day.filter(|t| !t.trim().is_empty()),
            date_time.filter(|d| !d.trim().is_empty()),
        ) {
            (Some(t), Some(d)) => {
                let date = self.date_parser.parse(date_part(d)).and_then(utc_date_of);
                date.and_then(|date| self.parser.parse_time_on(t, date))
                    .or_else(|| self.parser.parse(d))
            }
            (None, Some(d)) => self.parser.parse(d),
            (Some(t), None) => {
                let ms = self.parser.parse_time_on(t, self.fallback_date?)?;
                let shifted = |days: i64| days.checked_mul(DAY_MS).and_then(|d| ms.checked_add(d));
                if self
                    .last
                    .zip(shifted(self.days))
                    .is_some_and(|(last, now)| now < last.saturating_sub(HALF_DAY_MS))
                {
                    self.days = self.days.saturating_add(1);
                }
                let ms = shifted(self.days)?;
                self.last = Some(ms);
                Some(ms)
            }
            (None, None) => None,
        }
    }
}

/// Parses a ProcMon CSV export. `zone` supplies the time zone (and custom formats) of
/// the profile; `file_date` dates files that only have `Time of Day`.
pub fn parse_procmon(content: &str, zone: &TimestampParser, file_date: Option<chrono::NaiveDate>) -> Result<Imported> {
    let content = content.trim_start_matches('\u{feff}');
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes());
    let headers: Vec<String> = reader.headers()?.iter().map(|h| h.trim().to_string()).collect();
    if !is_procmon_header(&headers) {
        bail!(
            "not a Process Monitor CSV export: the header must contain \"Process Name\", \"Operation\", \"Path\", \"Result\" \
             and \"Time of Day\" or \"Date & Time\" (save from ProcMon as CSV)"
        );
    }
    let columns: Vec<(String, Kind)> = headers.iter().map(|h| column(h)).collect();
    let find = |kind: Kind| columns.iter().position(|(_, k)| *k == kind);
    let (tod_col, dt_col) = (find(Kind::TimeOfDay), find(Kind::DateTime));
    let named = |name: &str| columns.iter().position(|(n, _)| n == name);
    let (op_col, path_col, result_col, proc_col, pid_col) = (
        named("Operation"),
        named("Path"),
        named("Result"),
        named("ProcessName"),
        named("ProcessId"),
    );

    let mut rows = Vec::new();
    let mut skipped = 0;
    for row in reader.records() {
        match row {
            Ok(r) if r.as_slice().len() <= MAX_LINE_LEN => rows.push(r),
            _ => skipped += 1,
        }
    }
    let day_first = match (zone.day_first(), dt_col) {
        (Some(d), _) => Some(d),
        (None, Some(c)) => detect_day_first(rows.iter().take(DAY_ORDER_SAMPLES).filter_map(|r| r.get(c)).map(date_part)),
        (None, None) => None,
    };
    let mut clock = Clock {
        parser: zone.with_day_first(day_first),
        date_parser: TimestampParser::default().with_day_first(day_first),
        fallback_date: file_date,
        days: 0,
        last: None,
    };

    let mut out = Imported {
        records: Vec::with_capacity(rows.len()),
        skipped,
        ..Default::default()
    };
    // 1-based line of each row's start, counted from byte offsets (rows may hold quoted newlines).
    let (mut at, mut line_no) = (0usize, 1usize);
    for row in rows {
        let start = row
            .position()
            .map_or(at, |p| usize::try_from(p.byte()).unwrap_or(usize::MAX))
            .min(content.len());
        // With CRLF, csv reports a row as starting at the LF of the previous terminator.
        let start = start
            + content.as_bytes()[start..]
                .iter()
                .take_while(|&&b| b == b'\r' || b == b'\n')
                .count();
        if start > at {
            line_no += content.as_bytes()[at..start].iter().filter(|&&b| b == b'\n').count();
            at = start;
        }
        let line = u32::try_from(line_no).unwrap_or(u32::MAX);
        let cell = |c: Option<usize>| c.and_then(|c| row.get(c)).map(str::trim).unwrap_or("");
        let mut rec = Record::new(line);
        rec.ts = clock.at(tod_col.and_then(|c| row.get(c)), dt_col.and_then(|c| row.get(c)));
        for ((name, kind), value) in columns.iter().zip(row.iter()) {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let v = match kind {
                Kind::Integer => integer(value).unwrap_or_else(|| value.into()),
                Kind::Number => number(value).unwrap_or_else(|| value.into()),
                Kind::Text | Kind::TimeOfDay | Kind::DateTime => value.into(),
            };
            rec.fields.insert(name.clone(), v);
        }
        let result = cell(result_col);
        rec.level = result_level(result);
        let process = match (cell(proc_col), cell(pid_col)) {
            (p, "") => p.to_string(),
            (p, pid) => format!("{p} ({pid})"),
        };
        rec.message = format!("{process} {} {}", cell(op_col), cell(path_col)).trim().to_string();
        if !result.is_empty() {
            rec.message = format!("{}: {result}", rec.message);
        }
        out.records.push(rec);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use debrief_plugin_api::time::TimestampParser;
    use std::path::Path;

    const DEFAULT_EXPORT: &str = "\u{feff}\"Time of Day\",\"Process Name\",\"PID\",\"Operation\",\"Path\",\"Result\",\"Detail\"\r\n\
\"11:59:58.1234567 PM\",\"OrderService.exe\",\"4120\",\"CreateFile\",\"C:\\app\\orders.db\",\"ACCESS DENIED\",\"Desired Access: Generic Write\"\r\n\
\"11:59:59.5000000 PM\",\"OrderService.exe\",\"4120\",\"RegOpenKey\",\"HKLM\\Software\\Orders\",\"NAME NOT FOUND\",\"Desired Access: Read\"\r\n\
\"12:00:01.0000000 AM\",\"svchost.exe\",\"1,024\",\"Process Exit\",\"\",\"SUCCESS\",\"Exit Status: 0\"\r\n";

    /// The timestamp settings of a profile with the given `[parse]` zone and day order.
    fn profile(zone: &str, day_first: Option<bool>) -> TimestampParser {
        TimestampParser::new(&[], zone, day_first).unwrap()
    }

    fn ctx<'a>(p: &'a TimestampParser, date: Option<NaiveDate>) -> ImportCtx<'a> {
        let mut c = ImportCtx::new(Path::new("Logfile.CSV"), p);
        c.file_date = date;
        c
    }

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, ms: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_milli_opt(h, mi, s, ms)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    }

    #[test]
    fn sniffs_the_header_above_generic_csv() {
        let p = profile("UTC", None);
        let c = ctx(&p, None);
        assert_eq!(ProcmonCsvImporter.sniff(&c, DEFAULT_EXPORT), 95);
        assert_eq!(
            ProcmonCsvImporter.sniff(&c, "Sequence,Date & Time,Process Name,Operation,Path,Result\n1,x,a,b,c,d"),
            95
        );
        // Missing the time columns or one of the required ones.
        assert_eq!(ProcmonCsvImporter.sniff(&c, "Process Name,Operation,Path,Result\n"), 0);
        assert_eq!(ProcmonCsvImporter.sniff(&c, "Time of Day,Process Name,Operation,Path\n"), 0);
        assert_eq!(ProcmonCsvImporter.sniff(&c, ""), 0);
        assert_eq!(ProcmonCsvImporter.sniff(&c, "\"unterminated"), 0);
        assert_eq!(ProcmonCsvImporter.id(), "procmon-csv");
        assert!(ProcmonCsvImporter.multiline());
    }

    #[test]
    fn default_columns_with_the_file_date_and_midnight_rollover() {
        let p = profile("UTC", None);
        let out = ProcmonCsvImporter
            .import(&ctx(&p, NaiveDate::from_ymd_opt(2026, 10, 14)), DEFAULT_EXPORT)
            .unwrap();
        assert_eq!((out.records.len(), out.skipped), (3, 0));
        let r = &out.records[0];
        assert_eq!(r.ts, Some(utc(2026, 10, 14, 23, 59, 58, 123)));
        assert_eq!(r.level, Level::Error);
        assert_eq!(r.message, "OrderService.exe (4120) CreateFile C:\\app\\orders.db: ACCESS DENIED");
        assert_eq!(r.fields["ProcessName"], "OrderService.exe");
        assert_eq!(r.fields["ProcessId"], 4120);
        assert_eq!(r.fields["Operation"], "CreateFile");
        assert_eq!(r.fields["Path"], "C:\\app\\orders.db");
        assert_eq!(r.fields["Result"], "ACCESS DENIED");
        assert_eq!(r.fields["Detail"], "Desired Access: Generic Write");
        assert_eq!(r.fields["TimeOfDay"], "11:59:58.1234567 PM");
        assert_eq!(r.line, 2);
        assert_eq!(out.records[1].level, Level::Debug);
        // After midnight the date moves on; "1,024" is still a number; empty Path is omitted.
        let exit = &out.records[2];
        assert_eq!(exit.ts, Some(utc(2026, 10, 15, 0, 0, 1, 0)));
        assert_eq!(exit.fields["ProcessId"], 1024);
        assert!(!exit.fields.contains_key("Path"));
        assert_eq!(exit.message, "svchost.exe (1,024) Process Exit: SUCCESS");
        assert_eq!(exit.level, Level::Information);
    }

    #[test]
    fn without_a_file_date_time_of_day_has_no_timestamp() {
        let p = profile("UTC", None);
        let out = ProcmonCsvImporter.import(&ctx(&p, None), DEFAULT_EXPORT).unwrap();
        assert!(out.records.iter().all(|r| r.ts.is_none()));
    }

    #[test]
    fn date_and_time_gives_the_date_and_time_of_day_the_precision() {
        let p = profile("+02:00", None);
        let csv = "Sequence,Date & Time,Time of Day,Process Name,PID,TID,Operation,Path,Result,Duration,Parent PID,Integrity,Event Class\n\
7,14/10/2026 9:00:00 AM,9:00:00.2500000 AM,a.exe,10,11,ReadFile,C:\\x,SUCCESS,0.0000123,4,High,File System\n\
8,14/10/2026 9:00:01 AM,,a.exe,10,11,WriteFile,C:\\x,DISK FULL,n/a,4,High,File System\n\
9,,,a.exe,x,11,TCP Send,host:443,0xC000020C,,,,Network\n";
        let out = ProcmonCsvImporter.import(&ctx(&p, None), csv).unwrap();
        assert_eq!(out.records.len(), 3);
        let r = &out.records[0];
        // Day-first detected from 14/10; +02:00 => UTC 07:00.
        assert_eq!(r.ts, Some(utc(2026, 10, 14, 7, 0, 0, 250)));
        assert_eq!(r.fields["Sequence"], 7);
        assert_eq!(r.fields["ThreadId"], 11);
        assert_eq!(r.fields["ParentProcessId"], 4);
        assert_eq!(r.fields["Duration"].as_f64(), Some(0.0000123));
        assert_eq!(r.fields["Integrity"], "High");
        assert_eq!(r.fields["EventClass"], "File System");
        assert_eq!(r.fields["DateTime"], "14/10/2026 9:00:00 AM");
        // Only Date & Time.
        let w = &out.records[1];
        assert_eq!(w.ts, Some(utc(2026, 10, 14, 7, 0, 1, 0)));
        assert_eq!((w.level, &w.fields["Duration"]), (Level::Error, &Value::from("n/a")));
        // No time at all; an unparsable PID stays text; an NTSTATUS is a warning.
        let n = &out.records[2];
        assert_eq!((n.ts, n.level), (None, Level::Warning));
        assert_eq!(n.fields["ProcessId"], "x");
    }

    #[test]
    fn unknown_columns_bad_rows_and_errors() {
        let p = profile("UTC", Some(false));
        let csv = "Time of Day,Process Name,Operation,Path,Result,Date & Time,Weird (col),***\n\
1:02:03 PM,a,Load Image,C:\\a.dll,SUCCESS,10/11/2026 1:02:03 PM,v,w\n\
\"bad\"row\",a\n";
        let out = ProcmonCsvImporter.import(&ctx(&p, None), csv).unwrap();
        let r = &out.records[0];
        // Month-first forced by the profile.
        assert_eq!(r.ts, Some(utc(2026, 10, 11, 13, 2, 3, 0)));
        assert_eq!((r.fields["Weirdcol"].as_str(), r.line), (Some("v"), 2));
        assert_eq!(r.fields["Column"], "w");
        assert_eq!(r.message, "a Load Image C:\\a.dll: SUCCESS");
        // The broken quoted row is still read leniently by csv or skipped; never a panic.
        assert_eq!(out.records.len() + out.skipped, 2);
        let err = ProcmonCsvImporter.import(&ctx(&p, None), "a,b\n1,2").err().unwrap().to_string();
        assert!(err.contains("Process Monitor"), "{err}");
        // A header only file has no records; a row without result has no ": " suffix.
        let only = "Time of Day,Process Name,Operation,Path,Result\n";
        assert!(ProcmonCsvImporter.import(&ctx(&p, None), only).unwrap().records.is_empty());
        let short = "Time of Day,Process Name,Operation,Path,Result\n1:00:00 AM,p,Thread Create\n";
        let r = &ProcmonCsvImporter.import(&ctx(&p, None), short).unwrap().records[0];
        assert_eq!((r.message.as_str(), r.level), ("p Thread Create", Level::Information));
    }

    #[test]
    fn levels_by_result() {
        assert_eq!(result_level("SUCCESS"), Level::Information);
        assert_eq!(result_level(""), Level::Information);
        assert_eq!(result_level("name not found"), Level::Debug);
        assert_eq!(result_level("BUFFER OVERFLOW"), Level::Debug);
        assert_eq!(result_level("SHARING VIOLATION"), Level::Error);
        assert_eq!(result_level("0xC0000001"), Level::Warning);
    }

    #[test]
    fn clock_overflow_and_garbage_are_none() {
        let mut clock = Clock {
            parser: TimestampParser::default(),
            date_parser: TimestampParser::default(),
            fallback_date: NaiveDate::from_ymd_opt(2026, 10, 14),
            days: i64::MAX,
            last: None,
        };
        assert_eq!(clock.at(Some("1:00:00 AM"), None), None);
        assert_eq!(clock.at(Some("x"), Some("y")), None);
        assert_eq!(clock.at(None, None), None);
        // A bad time of day falls back to the full Date & Time.
        clock.days = 0;
        assert_eq!(
            clock.at(Some("x"), Some("2026-10-14 01:00:00")),
            Some(utc(2026, 10, 14, 1, 0, 0, 0))
        );
    }
}
