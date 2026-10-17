//! Delimited files (CSV, semicolon, tab, pipe) with or without a header row.

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
        let cfg = &ctx.profile.profile.parse.csv;
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
        let cfg = &ctx.profile.profile.parse.csv;
        let delim = delimiter(&cfg.delimiter, content).ok_or_else(|| anyhow!("could not detect the column delimiter"))?;
        let (headers, rows) = read_rows(delim, cfg.has_header, content)?;
        let layout = Layout::new(ctx, headers, &rows);
        Ok(layout.records(rows))
    }

    fn import_tail(&self, ctx: &ImportCtx, head: &str, tail: &str) -> Option<Result<Imported>> {
        let cfg = &ctx.profile.profile.parse.csv;
        let delim = delimiter(&cfg.delimiter, head)?;
        // The header (and the column roles) come from the file's start.
        let layout = read_rows(delim, cfg.has_header, head).map(|(headers, rows)| Layout::new(ctx, headers, &rows));
        Some(layout.and_then(|layout| read_rows(delim, false, tail).map(|(_, rows)| layout.records(rows))))
    }
}

type Rows = Vec<(u32, csv::StringRecord)>;

/// The rows of `content` and, with `has_header`, its header row.
fn read_rows(delim: u8, has_header: bool, content: &str) -> Result<(Option<Vec<String>>, Rows)> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(has_header)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(content.as_bytes());
    let rows: Rows = reader
        .records()
        .filter_map(|r| r.ok())
        .map(|r| (r.position().map(|p| p.line() as u32).unwrap_or(0), r))
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
    level_col: Option<usize>,
    message_col: Option<usize>,
    parser: crate::time::TimestampParser,
}

impl Layout {
    fn new(ctx: &ImportCtx, headers: Option<Vec<String>>, rows: &Rows) -> Layout {
        let cfg = &ctx.profile.profile.parse.csv;
        let headers: Vec<String> = headers.unwrap_or_else(|| {
            let width = rows.iter().map(|(_, r)| r.len()).max().unwrap_or(0);
            (1..=width).map(|i| format!("col{i}")).collect()
        });

        let find = |wanted: &str, names: &[&str]| -> Option<usize> {
            if wanted != "auto" {
                return headers.iter().position(|h| h.eq_ignore_ascii_case(wanted));
            }
            headers.iter().position(|h| names.contains(&h.to_ascii_lowercase().as_str()))
        };
        let parser = &ctx.profile.timestamps;
        let ts_col = find(&cfg.timestamp, TS_NAMES).or_else(|| {
            // No recognisable header: the first column whose values parse as timestamps.
            let first = rows.first()?;
            (0..first.1.len()).find(|&i| parser.parse(&first.1[i]).is_some())
        });
        let level_col = find(&cfg.level, LEVEL_NAMES);
        let message_col = find(&cfg.message, MESSAGE_NAMES);

        let parser = match (ts_col, parser.day_first()) {
            (Some(c), None) => parser.with_day_first(detect_day_first(rows.iter().take(2000).filter_map(|(_, r)| r.get(c)))),
            _ => parser.clone(),
        };
        Layout {
            headers,
            ts_col,
            level_col,
            message_col,
            parser,
        }
    }

    fn records(&self, rows: Rows) -> Imported {
        let Layout {
            headers,
            ts_col,
            level_col,
            message_col,
            parser,
        } = self;
        let (ts_col, level_col, message_col) = (*ts_col, *level_col, *message_col);
        let mut out = Imported::default();
        for (line, row) in rows {
            if row.iter().all(|c| c.is_empty()) {
                continue;
            }
            let mut rec = Record::new(line);
            for (i, value) in row.iter().enumerate() {
                if Some(i) == ts_col {
                    rec.ts = parser.parse(value);
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
                    .filter(|(i, v)| Some(*i) != ts_col && Some(*i) != level_col && !v.is_empty())
                    .map(|(_, v)| v)
                    .collect::<Vec<_>>()
                    .join(" · ");
            }
            out.records.push(rec);
        }
        out
    }
}

/// The configured delimiter, or the candidate that splits the first lines
/// into the same number (> 1) of columns most consistently.
fn delimiter(cfg: &str, text: &str) -> Option<u8> {
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

fn count_outside_quotes(line: &str, d: u8) -> usize {
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
        let p = CompiledProfile::compile(Profile::from_toml(profile).unwrap()).unwrap();
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
            "[parse.csv]\nhas_header = false",
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
            "[parse.csv]\ndelimiter = 'tab'\ntimestamp = 'When'\nlevel = 'Kind'\nmessage = 'What'",
            "When\tKind\tWhat\tExtra\n2026-10-14 10:00:00\tAudit\tlogin\t\n2026-10-14 10:00:01\tError\tboom\tx\n",
        )
        .unwrap();
        assert_eq!(out.records[0].level, Level::Information);
        assert_eq!(out.records[0].fields["Kind"], "Audit");
        assert!(!out.records[0].fields.contains_key("Extra"));
        assert_eq!(out.records[1].level, Level::Error);
        assert_eq!(out.records[1].message, "boom");
        let pipes = run("[parse.csv]\ndelimiter = '|'", "time|msg\n2026-10-14 10:00:00|a\n").unwrap();
        assert_eq!(pipes.records[0].message, "a");
        let tabs = run("[parse.csv]\ndelimiter = '\\t'", "time\tmsg\n2026-10-14 10:00:00\tb\n").unwrap();
        assert_eq!(tabs.records[0].message, "b");
    }

    #[test]
    fn undetectable_delimiter() {
        assert!(run("", "only one line, here\n").is_err());
        assert!(run("", "a,b\na;b;c\n").is_err());
        assert_eq!(count_outside_quotes("\"a,b\",c", b','), 1);
    }
}
