//! RFC 5424 and RFC 3164 (BSD) syslog text.

use super::{facility_name, severity_level, severity_name};
use anyhow::Result;
use chrono::{Datelike, NaiveDate, Utc};
use debrief_plugin_api::importer::{ImportCtx, Imported, TextImporter, MAX_LINE_LEN};
use debrief_plugin_api::model::{auto_value, Level, Record};
use serde_json::Value;

pub struct SyslogImporter;

/// How many non-empty lines `sniff` inspects, and the share that must parse.
const SNIFF_LINES: usize = 50;

impl TextImporter for SyslogImporter {
    fn id(&self) -> &'static str {
        "syslog"
    }

    /// Syslog header fields: host, application (tag) and process id.
    fn identity_fields(&self) -> Vec<String> {
        ["Host", "App", "ProcId"].map(String::from).to_vec()
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let (mut seen, mut hits) = (0usize, 0usize);
        for line in head
            .trim_start_matches('\u{feff}')
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
            .take(SNIFF_LINES)
        {
            // Indented lines continue the previous message (macOS writes multi-line messages so).
            if line.starts_with(char::is_whitespace) {
                continue;
            }
            seen += 1;
            hits += usize::from(parse_line(line).is_some());
        }
        if seen > 0 && hits * 2 >= seen {
            75
        } else {
            0
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let ts = &ctx.timestamps;
        let mut years = YearTracker::new(ctx.file_date);
        let mut out = Imported::default();
        for (i, line) in content.lines().enumerate() {
            let line = line.trim_start_matches('\u{feff}').trim_end();
            if line.trim().is_empty() {
                continue;
            }
            if line.len() > MAX_LINE_LEN {
                out.skipped += 1;
                continue;
            }
            let Some(p) = parse_line(line) else {
                match out.records.last_mut() {
                    Some(prev) => {
                        prev.message.push('\n');
                        prev.message.push_str(line);
                    }
                    None => out.skipped += 1,
                }
                continue;
            };
            let mut rec = Record::new(i as u32 + 1);
            rec.ts = match p.time {
                Time::Nil => None,
                Time::Iso(s) => ts.parse(s),
                Time::Bsd { month, day, hms } => {
                    let year = years.year_for(month);
                    ts.parse(&format!("{year:04}-{month:02}-{day:02} {}", hms))
                }
            };
            if let Some(pri) = p.pri {
                rec.level = severity_level(pri & 7);
                if let Some(f) = facility_name(pri >> 3) {
                    rec.fields.insert("Facility".into(), Value::from(f));
                }
                rec.fields.insert("Severity".into(), Value::from(severity_name(pri & 7)));
            } else {
                rec.level = Level::Information;
            }
            for (name, value) in [("Host", p.host), ("App", p.app), ("ProcId", p.procid), ("MsgId", p.msgid)] {
                if let Some(v) = value {
                    rec.fields
                        .insert(name.into(), if name == "ProcId" { auto_value(v) } else { Value::from(v) });
                }
            }
            for (id, param, value) in p.sd {
                rec.fields.insert(format!("sd.{id}.{param}"), Value::from(value));
            }
            rec.message = p.msg.to_string();
            out.records.push(rec);
        }
        Ok(out)
    }
}

/// BSD timestamps carry no year: take it from the file's date and move to the
/// next year when the month wraps from December to January.
struct YearTracker {
    file_date: Option<NaiveDate>,
    year: Option<i32>,
    last_month: u32,
}

impl YearTracker {
    fn new(file_date: Option<NaiveDate>) -> Self {
        YearTracker {
            file_date,
            year: None,
            last_month: 0,
        }
    }

    fn year_for(&mut self, month: u32) -> i32 {
        let year = match self.year {
            None => {
                let (base, file_month) = match self.file_date {
                    Some(d) => (d.year(), d.month()),
                    None => (Utc::now().year(), 12),
                };
                // The file was last written in January but starts in December.
                if month > file_month {
                    base - 1
                } else {
                    base
                }
            }
            Some(y) if self.last_month >= 10 && month <= 3 => y + 1,
            Some(y) => y,
        };
        self.year = Some(year);
        self.last_month = month;
        year
    }
}

#[derive(Debug, PartialEq)]
enum Time<'a> {
    Nil,
    Iso(&'a str),
    Bsd { month: u32, day: u32, hms: &'a str },
}

#[derive(Debug)]
struct Parsed<'a> {
    pri: Option<u8>,
    time: Time<'a>,
    host: Option<&'a str>,
    app: Option<&'a str>,
    procid: Option<&'a str>,
    msgid: Option<&'a str>,
    /// (SD-ID, PARAM-NAME, PARAM-VALUE)
    sd: Vec<(String, String, String)>,
    msg: &'a str,
}

fn parse_line(line: &str) -> Option<Parsed<'_>> {
    let (pri, rest) = split_pri(line);
    if pri.is_some() {
        if let Some(p) = parse_5424(pri, rest) {
            return Some(p);
        }
    }
    parse_3164(pri, rest)
}

fn split_pri(line: &str) -> (Option<u8>, &str) {
    let parsed = line.strip_prefix('<').and_then(|r| r.split_once('>')).and_then(|(n, rest)| {
        let valid = (1..=3).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit());
        let value: u8 = n.parse().ok().filter(|v| valid && *v <= 191)?;
        Some((value, rest))
    });
    match parsed {
        Some((v, rest)) => (Some(v), rest),
        None => (None, line),
    }
}

fn nil(s: &str) -> Option<&str> {
    Some(s).filter(|s| *s != "-" && !s.is_empty())
}

fn looks_iso(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 19
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && b[10] == b'T'
}

fn parse_5424(pri: Option<u8>, rest: &str) -> Option<Parsed<'_>> {
    let (version, rest) = rest.split_once(' ')?;
    if !(1..=2).contains(&version.len()) || !version.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut it = rest.splitn(6, ' ');
    let (ts, host, app, procid, msgid, tail) = (it.next()?, it.next()?, it.next()?, it.next()?, it.next()?, it.next()?);
    let time = if ts == "-" {
        Time::Nil
    } else if looks_iso(ts) {
        Time::Iso(ts)
    } else {
        return None;
    };
    let (sd, msg) = if let Some(msg) = tail.strip_prefix('-') {
        (Vec::new(), msg)
    } else {
        parse_sd(tail)?
    };
    let msg = msg.strip_prefix(' ').unwrap_or(msg);
    Some(Parsed {
        pri,
        time,
        host: nil(host),
        app: nil(app),
        procid: nil(procid),
        msgid: nil(msgid),
        sd,
        msg: msg.strip_prefix('\u{feff}').unwrap_or(msg),
    })
}

type SdParam = (String, String, String);

/// Parses `[id a="b" c="d"][id2 ...]` and returns the params and what follows.
fn parse_sd(mut s: &str) -> Option<(Vec<SdParam>, &str)> {
    let mut params = Vec::new();
    if !s.starts_with('[') {
        return None;
    }
    while let Some(body) = s.strip_prefix('[') {
        let id_end = body.find([' ', ']'])?;
        let id = &body[..id_end];
        let mut cur = &body[id_end..];
        loop {
            if let Some(after) = cur.strip_prefix(']') {
                s = after;
                break;
            }
            let named = cur.strip_prefix(' ')?;
            let (name, value) = named.split_once("=\"")?;
            let mut text = String::new();
            let mut chars = value.char_indices();
            let end = loop {
                match chars.next()? {
                    (i, '"') => break i + 1,
                    (_, '\\') => match chars.next()? {
                        (_, c @ ('"' | '\\' | ']')) => text.push(c),
                        (_, c) => {
                            text.push('\\');
                            text.push(c);
                        }
                    },
                    (_, c) => text.push(c),
                }
            };
            params.push((id.to_string(), name.to_string(), text));
            cur = &value[end..];
        }
    }
    Some((params, s))
}

fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    MONTHS.iter().position(|m| m.eq_ignore_ascii_case(name)).map(|i| i as u32 + 1)
}

/// `Oct 14 10:00:00` / `Oct  4 10:00:00` at the start of `s`.
fn bsd_time(s: &str) -> Option<(Time<'_>, &str)> {
    let b = s.as_bytes();
    if b.len() < 15 || b[3] != b' ' || b[6] != b' ' || b[9] != b':' || b[12] != b':' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    if !(digits(7..9) && digits(10..12) && digits(13..15) && (digits(4..6) || (b[4] == b' ' && digits(5..6)))) {
        return None;
    }
    let month = month_number(s.get(..3)?)?;
    let day: u32 = s[4..6].trim().parse().ok().filter(|d| (1..=31).contains(d))?;
    let rest = &s[15..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((
        Time::Bsd {
            month,
            day,
            hms: &s[7..15],
        },
        rest,
    ))
}

fn is_tag(word: &str) -> bool {
    word.len() > 1 && word.ends_with(':')
}

fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.split_once(' ') {
        Some((w, rest)) => (w, rest.trim_start()),
        None => (s, ""),
    }
}

/// `app[pid]:` -> (app, pid)
fn split_tag(tag: &str) -> (&str, Option<&str>) {
    let tag = tag.trim_end_matches(':');
    match tag.strip_suffix(']').and_then(|t| t.split_once('[')) {
        Some((app, pid)) => (app, nil(pid)),
        None => (tag, None),
    }
}

fn parse_3164(pri: Option<u8>, rest: &str) -> Option<Parsed<'_>> {
    let (time, after, iso) = if let Some((t, a)) = bsd_time(rest) {
        (t, a, false)
    } else {
        let (word, a) = split_word(rest);
        if !looks_iso(word) {
            return None;
        }
        (Time::Iso(word), a, true)
    };
    let (w1, r1) = split_word(after);
    let (host, tag, msg) = if is_tag(w1) {
        (None, Some(w1), r1)
    } else {
        let (w2, r2) = split_word(r1);
        if is_tag(w2) {
            (nil(w1), Some(w2), r2)
        } else {
            (nil(w1), None, r1)
        }
    };
    // The ISO form is only trusted with a tag, to avoid claiming ordinary app logs.
    if iso && tag.is_none() {
        return None;
    }
    let (app, procid) = tag.map_or((None, None), |t| {
        let (a, p) = split_tag(t);
        (nil(a), p)
    });
    Some(Parsed {
        pri,
        time,
        host,
        app,
        procid,
        msgid: None,
        sd: Vec::new(),
        msg,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_plugin_api::time::TimestampParser;
    use std::path::Path;

    const RFC5424: &str = include_str!("../tests/fixtures/rfc5424.log");
    const BSD: &str = include_str!("../tests/fixtures/auth.log");

    fn with_ctx<T>(date: Option<NaiveDate>, f: impl FnOnce(&ImportCtx) -> T) -> T {
        let ts = TimestampParser::default();
        let mut ctx = ImportCtx::new(Path::new("syslog"), &ts);
        ctx.file_date = date;
        f(&ctx)
    }

    fn ymd(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(y, m, d)
    }

    fn import(content: &str, date: Option<NaiveDate>) -> Imported {
        with_ctx(date, |ctx| SyslogImporter.import(ctx, content).unwrap())
    }

    fn sniff(content: &str) -> u8 {
        with_ctx(None, |ctx| SyslogImporter.sniff(ctx, content))
    }

    fn utc(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis()
    }

    #[test]
    fn imports_rfc5424_with_structured_data() {
        let out = import(RFC5424, None);
        assert_eq!(out.skipped, 0);
        assert_eq!(out.records.len(), 6);
        let r = &out.records[0];
        assert_eq!(r.ts, Some(utc("2026-10-14T10:00:00.123Z")));
        assert_eq!(r.level, Level::Error);
        assert_eq!(r.fields["Facility"], "auth");
        assert_eq!(r.fields["Severity"], "err");
        assert_eq!(r.fields["Host"], "web01.example.com");
        assert_eq!(r.fields["App"], "sshd");
        assert_eq!(r.fields["ProcId"], 4242);
        assert_eq!(r.fields["MsgId"], "ID47");
        assert_eq!(r.fields["sd.exampleSDID@32473.iut"], "3");
        assert_eq!(r.fields["sd.exampleSDID@32473.eventSource"], "Application");
        assert_eq!(r.fields["sd.exampleSDID@32473.note"], r#"a "quoted" \ ] \x"#);
        assert_eq!(r.fields["sd.origin@32473.ip"], "10.0.0.7");
        assert_eq!(r.message, "Failed password for root from 10.0.0.7");
        // NILVALUEs, offset timestamp, BOM
        let nilled = &out.records[1];
        assert_eq!(nilled.ts, Some(utc("2026-10-14T12:00:01+02:00")));
        assert!(!nilled.fields.contains_key("Host"));
        assert!(!nilled.fields.contains_key("App"));
        assert!(!nilled.fields.contains_key("ProcId"));
        assert!(!nilled.fields.contains_key("MsgId"));
        assert_eq!(nilled.level, Level::Information);
        assert_eq!(nilled.fields["Facility"], "local4");
        assert_eq!(nilled.message, "BOM message");
        assert_eq!(out.records[2].ts, None);
        assert_eq!(out.records[2].level, Level::Fatal);
        assert_eq!(out.records[2].message, "");
        assert_eq!(out.records[3].level, Level::Debug);
        // continuation line is appended
        assert_eq!(out.records[4].message, "multi line\n    continued");
        assert_eq!(out.records[5].fields["Facility"], "kern");
    }

    #[test]
    fn imports_bsd_lines_with_year_from_file_date() {
        let out = import(BSD, ymd(2026, 12, 31));
        assert_eq!(out.skipped, 0);
        assert_eq!(out.records.len(), 7);
        let r = &out.records[0];
        assert_eq!(r.ts, Some(utc("2026-10-14T10:00:00Z")));
        assert_eq!(r.level, Level::Warning);
        assert_eq!(r.fields["Facility"], "auth");
        assert_eq!(r.fields["Host"], "web01");
        assert_eq!(r.fields["App"], "sshd");
        assert_eq!(r.fields["ProcId"], 2210);
        assert_eq!(r.message, "Invalid user bob from 10.0.0.9");
        // no PRI, padded day, host without tag brackets
        let p = &out.records[1];
        assert_eq!(p.ts, Some(utc("2026-10-04T08:15:30Z")));
        assert_eq!(p.level, Level::Information);
        assert!(!p.fields.contains_key("Facility"));
        assert_eq!(p.fields["App"], "CRON");
        // no host: tag first
        let nohost = &out.records[2];
        assert!(!nohost.fields.contains_key("Host"));
        assert_eq!(nohost.fields["App"], "kernel");
        assert_eq!(nohost.message, "[    0.000000] Linux version 6.8");
        // no tag at all
        let notag = &out.records[3];
        assert_eq!(notag.fields["Host"], "web01");
        assert!(!notag.fields.contains_key("App"));
        assert_eq!(notag.message, "last message repeated 3 times");
        // Dec -> Jan rollover
        assert_eq!(out.records[4].ts, Some(utc("2026-12-31T23:59:59Z")));
        assert_eq!(out.records[5].ts, Some(utc("2027-01-01T00:00:01Z")));
        // rsyslog ISO format with tag
        assert_eq!(out.records[6].ts, Some(utc("2026-10-14T10:00:05.250+00:00")));
        assert_eq!(out.records[6].fields["App"], "rsyslogd");
    }

    #[test]
    fn year_tracking() {
        let file = "Dec 31 23:00:00 h a: x\nJan  1 00:00:00 h a: y\nFeb  1 00:00:00 h a: z\n";
        let ts: Vec<_> = import(file, ymd(2027, 1, 2)).records.iter().map(|r| r.ts.unwrap()).collect();
        assert_eq!(
            ts,
            [
                utc("2026-12-31T23:00:00Z"),
                utc("2027-01-01T00:00:00Z"),
                utc("2027-02-01T00:00:00Z")
            ]
        );
        // without a file date the current year is used
        let year = Utc::now().year();
        let out = import("Jan  1 00:00:00 h a: x\n", None);
        assert_eq!(out.records[0].ts, Some(utc(&format!("{year}-01-01T00:00:00Z"))));
        // Feb 30 is not a date
        assert_eq!(import("Feb 30 00:00:00 h a: x\n", ymd(2026, 3, 1)).records[0].ts, None);
    }

    #[test]
    fn parses_individual_lines() {
        assert!(parse_line("<34>Oct 11 22:14:15 mymachine su: 'su root' failed").is_some());
        assert!(parse_line("<34>1 2003-10-11T22:14:15.003Z mymachine.example.com su - ID47 - BOM'su root' failed").is_some());
        // invalid / non-syslog
        for line in [
            "",
            "plain text",
            "<999>Oct 11 22:14:15 h a: x",
            "<34>1 bad-time h a p m - x",
            "<34>x 2003-10-11T22:14:15Z h a p m - x",
            "<34>1 2003-10-11T22:14:15Z h a p",
            "<34>1 2003-10-11T22:14:15Z h a p m [broken",
            "<34>1 2003-10-11T22:14:15Z h a p m garbage",
            "<34>1 2003-10-11T22:14:15Z h a p m [id a=\"unterminated]",
            "<34>1 2003-10-11T22:14:15Z h a p m [id a=\"v\"",
            "<34>1 2003-10-11T22:14:15Z h a p m [id a]",
            "<34>1 2003-10-11T22:14:15Z h a p m [id a=\"x\\",
            "Foo 11 22:14:15 h a: x",
            "Oct 99 22:14:15 h a: x",
            "Oct 1x 22:14:15 h a: x",
            "Oct 11 22:14:15x h a: x",
            "Oct 11 22-14-15 h a: x",
            "2026-10-14T09:00:00Z INFO no tag here",
            "2026-10-14 09:00:00,000 [1] INFO  Billing.Service - starting",
        ] {
            assert!(parse_line(line).is_none(), "{line:?}");
        }
        let p = parse_line("<13>Oct 11 22:14:15").unwrap();
        assert_eq!((p.host, p.app, p.msg), (None, None, ""));
        let p = parse_line("<13>1 - h a p m - ").unwrap();
        assert_eq!(p.time, Time::Nil);
        assert_eq!(p.msg, "");
        let p = parse_line("<13>Oct 11 22:14:15 h app[]: m").unwrap();
        assert_eq!((p.app, p.procid), (Some("app"), None));
        assert_eq!(split_tag("a[1]:"), ("a", Some("1")));
        assert_eq!(split_tag("a:"), ("a", None));
    }

    #[test]
    fn sniff_threshold_and_negatives() {
        assert_eq!(sniff(RFC5424), 75);
        assert_eq!(sniff(BSD), 75);
        assert_eq!(sniff("\u{feff}Oct 14 10:00:00 h a: x\nnot syslog\n"), 75);
        assert_eq!(sniff("Oct 14 10:00:00 h a: x\nnot syslog\nnot syslog\n"), 0);
        // Indented continuation lines do not count against the file.
        let continued =
            "Oct 14 10:00:00 mac app[1]: Checking <Store:0x1\n\t\tpath=\"/a\"\n\t\tlocked=NO\n\t>\nOct 14 10:00:01 mac app[1]: done\n";
        assert_eq!(sniff(continued), 75);
        assert_eq!(
            import(continued, None).records[0].message,
            "Checking <Store:0x1\n\t\tpath=\"/a\"\n\t\tlocked=NO\n\t>"
        );
        assert_eq!(sniff("\tonly indented\n"), 0);
        assert_eq!(sniff(""), 0);
        assert_eq!(sniff("   \n\n"), 0);
        assert_eq!(sniff("{\"__REALTIME_TIMESTAMP\":\"1\",\"MESSAGE\":\"x\"}"), 0);
        assert_eq!(sniff("2026-10-14 09:00:00,000 [1] INFO  Billing.Service - starting"), 0);
        // only the first 50 non-empty lines count
        let mut long = "Oct 14 10:00:00 h a: x\n".repeat(50);
        long.push_str(&"garbage\n".repeat(100));
        assert_eq!(sniff(&long), 75);
    }

    #[test]
    fn does_not_steal_sample_files() {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples");
        // Syslog and journald samples of its own live in samples/linux; every other sample belongs to another importer.
        let own = samples.join("linux");
        let mut checked = 0;
        let mut stack = vec![samples];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path == own {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Some(text) = std::fs::read(&path).ok().and_then(|b| debrief_plugin_api::text::decode(&b)) else {
                    continue;
                };
                let head: String = text.chars().take(8192).collect();
                assert_eq!(sniff(&head), 0, "{}", path.display());
                assert_eq!(with_ctx(None, |c| crate::JournaldImporter.sniff(c, &head)), 0, "{}", path.display());
                checked += 1;
            }
        }
        assert!(checked >= 6);
    }

    #[test]
    fn malformed_input_never_panics() {
        let out = import("garbage first\n\n   \nOct 14 10:00:00 h a: ok\n", ymd(2026, 10, 15));
        assert_eq!(out.skipped, 1);
        assert_eq!(out.records.len(), 1);
        let big = format!("Oct 14 10:00:00 h a: {}\n", "x".repeat(MAX_LINE_LEN));
        assert_eq!(import(&big, None).skipped, 1);
        let garbage = [
            "<",
            "<>",
            "<1",
            "<1>",
            "<191>",
            "<192>1 x",
            "<13>1 ",
            "<13>1 - - - - - [",
            "<13>1 - - - - - [a",
            "<13>1 - - - - - [a b=\"\\",
            "Oct",
            "Oct 1",
            "Oct 14 10:00:0",
            "\u{0}\u{1}",
            "<13>Oct  \u{e9}\u{e9} 10:00:00 h",
            "é\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}",
            "2026-10-14T",
            "2026-10-14T09:00:00Z",
            "<13>1 2026-10-14T09:00:00Z",
        ];
        for g in garbage {
            let _ = import(g, ymd(2026, 1, 1));
            let _ = import(&format!("{g}\n{g}"), None);
            let _ = sniff(g);
        }
        for text in [RFC5424, BSD] {
            for i in (0..text.len()).step_by(11) {
                if text.is_char_boundary(i) {
                    let _ = import(&text[..i], ymd(2026, 1, 1));
                    let _ = sniff(&text[..i]);
                }
            }
        }
    }
}
