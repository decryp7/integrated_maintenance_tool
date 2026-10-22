//! Delimited files (CSV, semicolon, tab, pipe) with or without a header row.

use super::text::{finish, MAX_EVENT_BYTES};
use super::{first_lines, ImportCtx, Imported, Importer};
use crate::model::{auto_value, Level, Record};
use crate::time::detect_day_first;
use anyhow::{anyhow, Result};

pub struct CsvImporter;

const DELIMITERS: [u8; 4] = *b",;\t|";
const TS_NAMES: &[&str] = &[
    "timestamp",
    "time",
    "date",
    "datetime",
    "date and time",
    "@t",
    "timegenerated",
    "time created",
    "timecreated",
    "logged",
    "created",
    "eventtime",
];
const LEVEL_NAMES: &[&str] = &[
    "level",
    "severity",
    "loglevel",
    "log level",
    "type",
    "entry type",
    "entrytype",
    "leveldisplayname",
];
const MESSAGE_NAMES: &[&str] = &[
    "message",
    "msg",
    "text",
    "description",
    "renderedmessage",
    "rendered message",
    "details",
];

impl Importer for CsvImporter {
    fn id(&self) -> &'static str {
        "csv"
    }

    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8 {
        let cfg = &ctx.profile.format.csv;
        let Some(delim) = delimiter(&cfg.delimiter, head) else { return 0 };
        let Some(header) = first_lines(head, 1).next() else { return 0 };
        let cols: Vec<String> = header
            .split(delim as char)
            .map(|c| c.trim().trim_matches('"').to_ascii_lowercase())
            .collect();
        if cfg.has_header && cols.iter().any(|c| TS_NAMES.contains(&c.as_str())) {
            80
        } else {
            50
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let cfg = &ctx.profile.format.csv;
        let delim = delimiter(&cfg.delimiter, content).ok_or_else(|| anyhow!("could not detect the column delimiter"))?;
        let has_header = cfg.has_header && !first_row_is_data(ctx, delim, content);
        let (headers, rows) = read_rows(delim, has_header, content)?;
        let layout = Layout::new(ctx, delim, headers, &rows);
        Ok(layout.records(content, rows))
    }

    fn import_tail(&self, ctx: &ImportCtx, head: &str, tail: &str) -> Option<Result<Imported>> {
        let cfg = &ctx.profile.format.csv;
        let delim = delimiter(&cfg.delimiter, head)?;
        // The header (and the column roles) come from the file's start.
        let has_header = cfg.has_header && !first_row_is_data(ctx, delim, head);
        let layout = read_rows(delim, has_header, head).map(|(headers, rows)| Layout::new(ctx, delim, headers, &rows));
        Some(layout.and_then(|layout| read_rows(delim, false, tail).map(|(_, rows)| layout.records(tail, rows))))
    }
}

/// Whether the first row is an event rather than a header: a header names its columns, it never holds a timestamp
/// or a date. Many tools write no header at all (`has_header = true` only says a header may be there).
fn first_row_is_data(ctx: &ImportCtx, delim: u8, content: &str) -> bool {
    let Ok((_, rows)) = read_rows(delim, false, first_lines(content, 1).next().unwrap_or("")) else {
        return false;
    };
    let Some(Row { record: first, .. }) = rows.first() else {
        return false;
    };
    let parser = &ctx.profile.timestamps;
    first.iter().any(|cell| parser.parse(cell).is_some() || is_date(cell))
}

/// A date on its own (`2026/10/14`, `2026-10-14`, `14.10.2026`): a timestamp whose time is in the next column.
fn is_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split(['/', '-', '.']).collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit()))
        && parts.iter().any(|p| p.len() == 4)
}

/// A time of day, optionally with a fraction and an offset (`00:04:37.759+01:00`, `10:00`, `10:00:00Z`).
fn is_time(s: &str) -> bool {
    let (clock, _) = s.split_once(['Z', '+']).unwrap_or((s, ""));
    // A minus sign after the seconds starts an offset; the clock itself has none.
    let clock = clock.rsplit_once('-').map_or(clock, |(c, _)| c);
    let (hms, fraction) = clock.split_once(['.', ',']).unwrap_or((clock, ""));
    let parts: Vec<&str> = hms.split(':').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| (1..=2).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
        && fraction.bytes().all(|b| b.is_ascii_digit())
}

/// Most rows looked at to find the level and message columns of a file without a header.
const SAMPLE_ROWS: usize = 50;

/// One row of the file: its line number, the byte offset its text starts at and its cells.
struct Row {
    line: u32,
    start: usize,
    record: csv::StringRecord,
}

type Rows = Vec<Row>;

/// The rows of `content` and, with `has_header`, its header row.
fn read_rows(delim: u8, has_header: bool, content: &str) -> Result<(Option<Vec<String>>, Rows)> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(has_header)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(content.as_bytes());
    // The reader's positions count a CRLF as two line ends and can point at the `\n` of the previous line: the line
    // number and the start of a row's text come from the content itself (rows ascend, so one pass counts the lines).
    let (mut counted_to, mut line) = (0usize, 1u32);
    let rows: Rows = reader
        .records()
        .filter_map(|r| r.ok())
        .map(|record| {
            let at = record.position().map_or(0, |p| p.byte() as usize).min(content.len());
            let start = at
                + content
                    .get(at..)
                    .map_or(0, |rest| rest.len() - rest.trim_start_matches(['\r', '\n']).len());
            if start >= counted_to {
                line += content
                    .get(counted_to..start)
                    .map_or(0, |s| s.bytes().filter(|&b| b == b'\n').count()) as u32;
                counted_to = start;
            }
            Row { line, start, record }
        })
        .collect();
    let headers = if has_header {
        Some(reader.headers()?.iter().map(|h| h.to_string()).collect())
    } else {
        None
    };
    Ok((headers, rows))
}

/// Column names and the columns holding the timestamp, level and message.
struct Layout {
    headers: Vec<String>,
    ts_col: Option<usize>,
    /// The time of a timestamp split over two columns (`2026/10/14,10:00:00` with `,` as the delimiter).
    time_col: Option<usize>,
    level_col: Option<usize>,
    message_col: Option<usize>,
    /// How many columns a row has when the message is the last of them: more cells mean the message held the
    /// delimiter without quotes (`…,Unhandled exception, the service stops`), and the rest of the line is the message.
    message_last: Option<usize>,
    delim: u8,
    parser: crate::time::TimestampParser,
}

impl Layout {
    fn new(ctx: &ImportCtx, delim: u8, headers: Option<Vec<String>>, rows: &Rows) -> Layout {
        let cfg = &ctx.profile.format.csv;
        let has_header = headers.is_some();
        let headers: Vec<String> = headers.unwrap_or_else(|| {
            let width = rows.iter().map(|r| r.record.len()).max().unwrap_or(0);
            // Names from the profile for a file without a header, then col<N> for the rest.
            (1..=width)
                .map(|i| {
                    cfg.columns
                        .get(i - 1)
                        .filter(|n| !n.trim().is_empty())
                        .cloned()
                        .unwrap_or_else(|| format!("col{i}"))
                })
                .collect()
        });

        let named = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name.trim()));
        let find = |wanted: &str, names: &[&str]| -> Option<usize> {
            if wanted != "auto" {
                return named(wanted);
            }
            headers.iter().position(|h| names.contains(&h.to_ascii_lowercase().as_str()))
        };
        let parser = &ctx.profile.timestamps;
        let sample: Vec<&csv::StringRecord> = rows.iter().take(SAMPLE_ROWS).map(|r| &r.record).collect();
        let joined = |r: &csv::StringRecord, d: usize, t: usize| format!("{},{}", r.get(d).unwrap_or(""), r.get(t).unwrap_or(""));

        // `timestamp = "Date+Time"` joins two named columns.
        let (ts_col, time_col) = match cfg.timestamp.split_once('+') {
            Some((d, t)) if cfg.timestamp != "auto" => (named(d), named(t)),
            _ => {
                let single = find(&cfg.timestamp, TS_NAMES);
                match single {
                    Some(c) => (Some(c), None),
                    // No recognisable header: a date column followed by a time column, else the first column whose
                    // values parse as timestamps.
                    None => sample
                        .first()
                        .and_then(|first| {
                            (0..first.len().saturating_sub(1))
                                .find(|&i| is_date(&first[i]) && is_time(&first[i + 1]) && parser.parse(&joined(first, i, i + 1)).is_some())
                                .map(|i| (Some(i), Some(i + 1)))
                                .or_else(|| {
                                    (0..first.len())
                                        .find(|&i| parser.parse(&first[i]).is_some())
                                        .map(|i| (Some(i), None))
                                })
                        })
                        .unwrap_or((None, None)),
                }
            }
        };
        let taken = |i: usize| Some(i) == ts_col || Some(i) == time_col;
        let width = headers.len();
        let level_col = find(&cfg.level, LEVEL_NAMES).or_else(|| {
            // Without a header: the column whose values are all level names (INFO, Warning, ERR…).
            (!has_header && cfg.level == "auto")
                .then(|| {
                    (0..width).find(|&i| {
                        !taken(i)
                            && !sample.is_empty()
                            && sample.iter().all(|r| {
                                r.get(i)
                                    .is_some_and(|v| Level::parse(v).is_some() && v.bytes().any(|b| b.is_ascii_alphabetic()))
                            })
                    })
                })
                .flatten()
        });
        let message_col = find(&cfg.message, MESSAGE_NAMES).or_else(|| {
            // Without a header: the column with the longest text (a message reads as words, not codes or numbers).
            (!has_header && cfg.message == "auto")
                .then(|| {
                    let avg = |i: usize| sample.iter().map(|r| r.get(i).map_or(0, str::len)).sum::<usize>() / sample.len().max(1);
                    (0..width)
                        .filter(|&i| !taken(i) && Some(i) != level_col)
                        .max_by_key(|&i| (avg(i), i))
                        .filter(|&i| avg(i) >= 12)
                })
                .flatten()
        });

        let parser = match (ts_col, parser.day_first()) {
            (Some(c), None) => parser.with_day_first(detect_day_first(rows.iter().take(2000).filter_map(|r| r.record.get(c)))),
            _ => parser.clone(),
        };
        // The named columns: the header's, else the profile's `columns`.
        let named_width = if has_header { width } else { cfg.columns.len() };
        let message_last = message_col.filter(|&m| m + 1 == named_width).map(|_| named_width);
        Layout {
            headers,
            ts_col,
            time_col,
            level_col,
            message_col,
            message_last,
            delim,
            parser,
        }
    }

    /// The events of `rows` (read from `content`).
    ///
    /// With a timestamp column, a row whose timestamp does not parse and that has fewer cells than the layout is a
    /// continuation line of the event before it (a multi-line message or exception written without quotes): its
    /// text, as written, joins that event.
    fn records(&self, content: &str, rows: Rows) -> Imported {
        let Layout {
            headers,
            ts_col,
            time_col,
            level_col,
            message_col,
            message_last,
            delim,
            parser,
        } = self;
        let (ts_col, time_col, level_col, message_col) = (*ts_col, *time_col, *level_col, *message_col);
        let width = message_last.unwrap_or(headers.len());
        let mut out = Imported::default();
        let mut current: Option<Record> = None;
        let mut extra = String::new();
        for Row { line, start, record: row } in rows {
            if row.iter().all(|c| c.is_empty()) {
                continue;
            }
            let ts = ts_col
                .and_then(|c| row.get(c))
                .and_then(|value| match time_col.and_then(|t| row.get(t)) {
                    Some(time) => parser.parse(&format!("{value},{time}")),
                    None => parser.parse(value),
                });
            if ts.is_none() && ts_col.is_some() && current.is_some() && row.len() < width {
                if extra.len() < MAX_EVENT_BYTES {
                    if !extra.is_empty() {
                        extra.push('\n');
                    }
                    extra.push_str(raw_line(content, start));
                }
                continue;
            }
            if let Some(prev) = current.take() {
                out.records.push(finish(prev, std::mem::take(&mut extra)));
            }
            let mut rec = Record::new(line);
            let overflow = message_last.is_some_and(|n| row.len() > n);
            for (i, value) in row.iter().enumerate() {
                if overflow && message_col.is_some_and(|m| i > m) {
                    // Part of the message.
                } else if overflow && Some(i) == message_col {
                    rec.message = rest_after(raw_line(content, start), *delim, i).unwrap_or(value).trim().to_string();
                } else if Some(i) == ts_col {
                    rec.ts = ts;
                } else if Some(i) == time_col {
                    // Part of the timestamp.
                } else if Some(i) == level_col {
                    rec.level = Level::parse(value).unwrap_or(Level::Information);
                    if Level::parse(value).is_none() && !value.is_empty() {
                        rec.fields.insert(headers[i].clone(), auto_value(value));
                    }
                } else if Some(i) == message_col {
                    rec.message = value.to_string();
                } else if !value.is_empty() {
                    let name = headers.get(i).cloned().unwrap_or_else(|| format!("col{}", i + 1));
                    rec.fields.insert(name, auto_value(value));
                }
            }
            if rec.message.is_empty() {
                // Show the row itself so the grid is never empty.
                rec.message = row
                    .iter()
                    .enumerate()
                    .filter(|(i, v)| Some(*i) != ts_col && Some(*i) != time_col && Some(*i) != level_col && !v.is_empty())
                    .map(|(_, v)| v)
                    .collect::<Vec<_>>()
                    .join(" · ");
            }
            current = Some(rec);
        }
        if let Some(rec) = current.take() {
            out.records.push(finish(rec, extra));
        }
        out
    }
}

/// The line of `content` starting at byte `start`, without its line break.
fn raw_line(content: &str, start: usize) -> &str {
    let rest = content.get(start..).unwrap_or("");
    rest[..rest.find('\n').unwrap_or(rest.len())].trim_end_matches('\r')
}

/// The text of `line` after its `n`-th delimiter outside quotes.
fn rest_after(line: &str, delim: u8, n: usize) -> Option<&str> {
    if n == 0 {
        return Some(line);
    }
    let mut quoted = false;
    let mut seen = 0;
    for (i, b) in line.bytes().enumerate() {
        if b == b'"' {
            quoted = !quoted;
        } else if b == delim && !quoted {
            seen += 1;
            if seen == n {
                return line.get(i + 1..);
            }
        }
    }
    None
}

/// The configured delimiter, or the candidate that splits the first lines
/// into the same number (> 1) of columns most consistently.
pub(crate) fn delimiter(cfg: &str, text: &str) -> Option<u8> {
    match cfg {
        "auto" | "" => {}
        "\\t" | "tab" => return Some(b'\t'),
        other => return other.bytes().next(),
    }
    let lines: Vec<&str> = first_lines(text, 20).collect();
    if lines.len() < 2 {
        return None;
    }
    DELIMITERS
        .iter()
        .filter_map(|&d| {
            let counts: Vec<usize> = lines.iter().map(|l| count_outside_quotes(l, d)).collect();
            let first = counts[0];
            (first > 0 && counts.iter().all(|&c| c == first)).then_some((d, first))
        })
        .max_by_key(|&(_, n)| n)
        .map(|(d, _)| d)
}

pub(crate) fn count_outside_quotes(line: &str, d: u8) -> usize {
    let mut quoted = false;
    line.bytes()
        .filter(|&b| {
            if b == b'"' {
                quoted = !quoted;
            }
            b == d && !quoted
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{CompiledProfile, Profile};
    use std::path::Path;

    #[test]
    fn csv_with_header() {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.csv"),
            file_date: None,
        };
        let content = "Timestamp;Level;Machine;Cpu;Message\n14/10/2026 10:00:00;Warning;WEB01;71.5;\"High CPU; throttling\"\n15/10/2026 10:00:00;Information;WEB02;12;ok\n";
        assert_eq!(CsvImporter.sniff(&ctx, content), 80);
        let out = CsvImporter.import(&ctx, content).unwrap();
        assert_eq!(out.records.len(), 2);
        let r = &out.records[0];
        assert_eq!(r.level, Level::Warning);
        assert_eq!(r.message, "High CPU; throttling");
        assert_eq!(r.fields["Cpu"], 71.5);
        // A blank message column falls back to the row's other values.
        let out = CsvImporter
            .import(&ctx, "Timestamp,Machine,Message\n2026-10-14 10:00:00,WEB01,\n")
            .unwrap();
        assert_eq!(out.records[0].message, "WEB01");
        assert!(r.ts.is_some());
    }

    #[test]
    fn log_lines_are_not_csv() {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.log"),
            file_date: None,
        };
        let content = "2026-10-14 10:00:00,123 INFO a, b\n2026-10-14 10:00:01,000 INFO c\n";
        assert_eq!(CsvImporter.sniff(&ctx, content), 0);
    }

    fn run(profile: &str, content: &str) -> anyhow::Result<Imported> {
        let p = crate::parser::testing::compiled(profile);
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.csv"),
            file_date: None,
        };
        CsvImporter.import(&ctx, content)
    }

    #[test]
    fn headerless_detects_timestamp_column() {
        let out = run(
            "[format.csv]\nhas_header = false",
            "WEB01,2026-10-14 10:00:00,5\nWEB02,2026-10-14 10:01:00,7\n,,\n",
        )
        .unwrap();
        assert_eq!(out.records.len(), 2);
        assert!(out.records[0].ts.is_some());
        assert_eq!(out.records[0].fields["col1"], "WEB01");
        assert_eq!(out.records[1].message, "WEB02 · 7");
    }

    #[test]
    fn explicit_columns_and_delimiter() {
        let out = run(
            "[format.csv]\ndelimiter = 'tab'\ntimestamp = 'When'\nlevel = 'Kind'\nmessage = 'What'",
            "When\tKind\tWhat\tExtra\n2026-10-14 10:00:00\tAudit\tlogin\t\n2026-10-14 10:00:01\tError\tboom\tx\n",
        )
        .unwrap();
        assert_eq!(out.records[0].level, Level::Information);
        assert_eq!(out.records[0].fields["Kind"], "Audit");
        assert!(!out.records[0].fields.contains_key("Extra"));
        assert_eq!(out.records[1].level, Level::Error);
        assert_eq!(out.records[1].message, "boom");
        let pipes = run("[format.csv]\ndelimiter = '|'", "time|msg\n2026-10-14 10:00:00|a\n").unwrap();
        assert_eq!(pipes.records[0].message, "a");
        let tabs = run("[format.csv]\ndelimiter = '\\t'", "time\tmsg\n2026-10-14 10:00:00\tb\n").unwrap();
        assert_eq!(tabs.records[0].message, "b");
    }

    fn utc_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, milli: u32) -> i64 {
        chrono::NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_milli_opt(h, mi, s, milli)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    }

    /// A log4net-style CSV without a header whose date and time are split by the delimiter (invented values).
    const SPLIT_TS: &str = "2024/03/02,09:15:00.013+01:00,INFO,0x4000000000000001,alpha.exe,1200,6,(3300),,unit:10,\"<ALPHA>:\"\"Started job 41\"\"\"\n\
        2024/03/02,09:15:02.500+01:00,ERROR,0xc000000000000002,beta.exe,2400,6,(4400),,worker:22,\"<BETA> Connection lost:0x10:retry:3:\"\"Timeout\"\"\"\n";

    // Regression: the first event was read as a header and only the date column became the timestamp.
    #[test]
    fn a_headerless_file_with_date_and_time_in_two_columns() {
        let out = run("", SPLIT_TS).unwrap();
        assert_eq!(out.records.len(), 2, "the first row is an event, not a header");
        let first = &out.records[0];
        // 09:15:00.013 at +01:00 is 08:15:00.013 UTC.
        assert_eq!(first.ts, Some(utc_ms(2024, 3, 2, 8, 15, 0, 13)));
        assert_eq!(first.level, Level::Information);
        assert_eq!(first.message, "<ALPHA>:\"Started job 41\"");
        assert_eq!(first.fields["col5"], "alpha.exe");
        assert!(!first.fields.contains_key("col2"), "the time is part of the timestamp");
        let second = &out.records[1];
        assert_eq!(second.level, Level::Error);
        assert_eq!(second.message, "<BETA> Connection lost:0x10:retry:3:\"Timeout\"");
    }

    #[test]
    fn headerless_columns_are_named_by_the_profile_and_two_columns_can_be_joined() {
        let profile =
            "[format.csv]\ncolumns = ['Date', 'Time', 'Severity', 'Code', 'Process', 'Pid']\ntimestamp = 'Date+Time'\nlevel = 'Severity'";
        let out = run(profile, SPLIT_TS).unwrap();
        let first = &out.records[0];
        assert_eq!(first.ts, Some(utc_ms(2024, 3, 2, 8, 15, 0, 13)));
        assert_eq!(first.level, Level::Information);
        assert_eq!(first.fields["Process"], "alpha.exe");
        assert_eq!(first.fields["Pid"], 1200);
        assert_eq!(first.fields["col10"], "unit:10", "columns past the named ones keep col<N>");
        // A real header still wins: a header never holds a timestamp.
        let out = run("", "Time,Level,Message\n2024-03-02 09:15:00,Warning,disk almost full\n").unwrap();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0].message, "disk almost full");
    }

    /// Messages with backslash paths, doubled quotes and dates of their own, a summer-time offset (invented values).
    #[test]
    fn headerless_messages_with_paths_quotes_and_dates_inside() {
        let content = "2024/06/01,08:30:15.250+02:00,INFO,0x4000000000000007,gamma.exe,3100,2,(700),,copier:42,\":\"\"MSG:[Changed]\\srv01\\data\\unit7\\a_b.cfg    Src:Sat Jun  1 08:29:59 2024    Dest:Fri May 31 08:29:58 2024\"\"\"\n\
            2024/06/01,08:30:15.300+02:00,INFO,0x4000000000000007,gamma.exe,3100,2,(700),,copier:42,\":\"\"MSG:[Changed]\\srv01\\data\\unit7\\c_d.cfg    Src:Sat Jun  1 08:30:01 2024    Dest:Fri May 31 08:30:00 2024\"\"\"\n";
        let out = run("", content).unwrap();
        assert_eq!(out.records.len(), 2);
        let r = &out.records[0];
        assert_eq!(r.ts, Some(utc_ms(2024, 6, 1, 6, 30, 15, 250)), "+02:00 is two hours ahead of UTC");
        assert_eq!(r.level, Level::Information);
        assert_eq!(
            r.message,
            ":\"MSG:[Changed]\\srv01\\data\\unit7\\a_b.cfg    Src:Sat Jun  1 08:29:59 2024    Dest:Fri May 31 08:29:58 2024\""
        );
        assert_eq!(r.fields["col10"], "copier:42");
    }

    /// The log4net CSV layout writes the message last and unquoted, and an exception on the lines below it (invented values).
    const CSV_EXCEPTION: &str = "2024/05/03,11:00:00.000+02:00,INFO,7,svc,90,Started\n\
        2024/05/03,11:00:01.000+02:00,ERROR,8,svc,90,Copy failed, retrying\n\
        System.IO.IOException: The disk is full (unit 3, slot 1)\n\
        \x20  at Copier.Run() in C:\\src\\Copier.cs:line 12\n\
        2024/05/03,11:00:02.000+02:00,WARN,9,svc,90,\"Quoted, with a comma\"\n\
        2024/05/03,11:00:03.000+02:00,INFO,9,svc,90,Line one\n\
        line two of the same message\n";

    // Regression: a message with commas became an extra column, and the exception lines empty events without a time.
    #[test]
    fn headerless_unquoted_last_message_and_continuation_lines() {
        let profile = "[format.csv]\ndelimiter = ','\nhas_header = false\ncolumns = ['Date', 'Time', 'Level', 'Code', 'Process', 'Pid', 'Message']\ntimestamp = 'Date+Time'\nlevel = 'Level'\nmessage = 'Message'";
        let out = run(profile, CSV_EXCEPTION).unwrap();
        let msgs: Vec<&str> = out.records.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(
            msgs,
            [
                "Started",
                "Copy failed, retrying",
                "Quoted, with a comma",
                "Line one\nline two of the same message"
            ]
        );
        assert!(out.records.iter().all(|r| r.ts.is_some()));
        let failed = &out.records[1];
        assert_eq!(
            failed.exception.as_deref(),
            Some("System.IO.IOException: The disk is full (unit 3, slot 1)\n   at Copier.Run() in C:\\src\\Copier.cs:line 12")
        );
        assert!(!failed.fields.contains_key("col8"), "the rest of the line is the message");
        assert_eq!(failed.fields["Pid"], 90);
        assert_eq!(out.records[3].line, 6);
        let crlf = run(profile, &CSV_EXCEPTION.replace('\n', "\r\n")).unwrap();
        assert_eq!(crlf.records.len(), 4);
        assert_eq!(
            crlf.records[1].exception, failed.exception,
            "the line breaks are not part of the text"
        );
        assert_eq!(crlf.records[1].message, "Copy failed, retrying");
    }

    #[test]
    fn rest_after_the_nth_delimiter_skips_quoted_ones() {
        assert_eq!(rest_after("a,\"b,c\",d, e", b',', 2), Some("d, e"));
        assert_eq!(rest_after("a,b", b',', 0), Some("a,b"));
        assert_eq!(rest_after("a,b", b',', 3), None);
        assert_eq!(raw_line("x\r\ny", 0), "x");
        assert_eq!(raw_line("x", 9), "");
    }

    #[test]
    fn dates_and_times_on_their_own() {
        assert!(is_date("2024/03/02") && is_date("2024-03-02") && is_date("02.03.2024"));
        assert!(!is_date("0x4000") && !is_date("12/34") && !is_date("1/2/3") && !is_date("2024/03/02 10:00"));
        assert!(is_time("09:15:00.013+01:00") && is_time("09:15") && is_time("09:15:00Z") && is_time("09:15:00,5-05:00"));
        assert!(!is_time("unit:10") && !is_time("(3300)") && !is_time("9:15:00:00") && !is_time(""));
    }

    #[test]
    fn undetectable_delimiter() {
        assert!(run("", "only one line, here\n").is_err());
        assert!(run("", "a,b\na;b;c\n").is_err());
        assert_eq!(count_outside_quotes("\"a,b\",c", b','), 1);
    }
}
