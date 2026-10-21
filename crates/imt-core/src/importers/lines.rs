//! Per-line event parsing for plain-text logs, shared by the in-memory [`TextImporter`](super::TextImporter)
//! and the streaming text importer (`crate::ingest`), so both read a mixed file the same way (ADR 0020).
//!
//! Every line is classified on its own, with no state from the lines before it (so a file read in parallel
//! chunks gives the same events as read whole):
//!
//! 1. the **first text format in profile order** that matches starts an event; the file's dominant format
//!    (the one matching most of its first lines) is tried first as a shortcut, and only the formats listed
//!    before it are checked to keep the profile order;
//! 2. else a one-line **JSON object** with a timestamp key (`[parse.json] timestamp`) is a JSON event;
//! 3. else a line that does **not look like an event start** (indented, a stack frame, `Caused by`, a brace of
//!    a pretty-printed object, plain text...) continues the event above it;
//! 4. else the line looks like an event (it starts with a date, a time, a syslog priority or month) but no
//!    format reads it: it is **unmatched**. It still joins the event above (nothing is lost), and the importer
//!    reports it as an [`UnmatchedLine`](crate::ingest::ProblemKind::UnmatchedLine) problem.

use super::json::to_record as json_to_record;
use super::{first_lines, SNIFF_LINES};
use crate::model::{auto_value, Level, Record};
use crate::profile::CompiledProfile;
use crate::template::PROPERTIES_GROUP;
use crate::time::TimestampParser;
use chrono::NaiveDate;
use regex::Captures;
use serde_json::{Map, Value};

/// A format lines of a text file can have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineFormat {
    /// An output template or regex of the profile (index into `CompiledProfile::text_formats`).
    Text(usize),
    /// A JSON object on one line.
    Json,
}

/// The format matching the most of the first [`SNIFF_LINES`] lines of `text`; earlier text formats win
/// ties, and JSON wins only with strictly more lines than every text format.
pub fn best_format(profile: &CompiledProfile, text: &str) -> Option<LineFormat> {
    let lines: Vec<&str> = first_lines(text, SNIFF_LINES).collect();
    let mut best: Option<(LineFormat, usize)> = None;
    for (i, f) in profile.text_formats.iter().enumerate() {
        let hits = lines.iter().filter(|l| f.regex.is_match(l)).count();
        if hits > 0 && best.is_none_or(|(_, h)| hits > h) {
            best = Some((LineFormat::Text(i), hits));
        }
    }
    let json = lines.iter().filter(|l| json_event(profile, l).is_some()).count();
    if json > 0 && best.is_none_or(|(_, h)| json > h) {
        best = Some((LineFormat::Json, json));
    }
    best.map(|(f, _)| f)
}

/// Whether some of the first lines of `text` are text events of the profile's formats (not JSON objects).
pub fn has_text_events(profile: &CompiledProfile, text: &str) -> bool {
    first_lines(text, SNIFF_LINES).any(|l| !l.trim_start().starts_with('{') && profile.text_formats.iter().any(|f| f.regex.is_match(l)))
}

/// Whether `line` looks like the first line of an event: it starts (after an optional `[`, `(` or `<`) with
/// a date (`2026-10-14`, `2026/10/14`, `14.10.2026`, `10/14/2026`), a time (`10:41:22`), a syslog priority
/// (`<34>`), a month name and day (`Oct 14`) or an epoch number. Indented lines, stack frames and plain
/// text do not. Cheap: looks at the first few bytes only.
pub fn looks_like_event_start(line: &str) -> bool {
    let b = line.as_bytes();
    let b = match b.first() {
        Some(b'<') => return b.len() > 2 && b[1].is_ascii_digit() && b[1..].iter().take(5).any(|c| *c == b'>'),
        Some(b'[' | b'(') => &b[1..],
        _ => b,
    };
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    let sep = |i: usize| b.get(i).copied();
    match digits {
        // 2026-10-14, 2026/10/14, 2026.10.14
        4 => matches!(sep(4), Some(b'-' | b'/' | b'.')) && sep(5).is_some_and(|c| c.is_ascii_digit()),
        // 10:41:22, 14.10.2026, 10/14/2026, 14-10-2026
        1 | 2 => match sep(digits) {
            Some(b':') => sep(digits + 1).is_some_and(|c| c.is_ascii_digit()),
            Some(b'-' | b'/' | b'.') => sep(digits + 1).is_some_and(|c| c.is_ascii_digit()) && b.len() > digits + 4,
            _ => false,
        },
        // Epoch seconds or milliseconds.
        10..=13 => matches!(sep(digits), None | Some(b' ' | b'.' | b',' | b'\t')),
        0 => month_day(b),
        _ => false,
    }
}

/// `Oct 14` / `Oct  4` (BSD syslog).
fn month_day(b: &[u8]) -> bool {
    const MONTHS: [&[u8; 3]; 12] = [
        b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec",
    ];
    b.len() >= 6
        && MONTHS.iter().any(|m| b[..3] == m[..])
        && b[3] == b' '
        && (b[4].is_ascii_digit() || (b[4] == b' ' && b[5].is_ascii_digit()))
}

/// A JSON object on one line that has a timestamp key of `[parse.json] timestamp`: a JSON event.
pub fn json_event(profile: &CompiledProfile, line: &str) -> Option<Map<String, Value>> {
    let t = line.trim();
    if !(t.starts_with('{') && t.ends_with('}')) {
        return None;
    }
    let obj: Map<String, Value> = serde_json::from_str(t).ok()?;
    let keys = &profile.profile.parse.json.timestamp;
    keys.iter()
        .any(|k| obj.get(k).is_some_and(|v| !v.is_null()) || k.split_once('.').is_some_and(|(head, _)| obj.contains_key(head)))
        .then_some(obj)
}

/// The first text format in profile order that matches `line`, with its captures; `dominant` is tried
/// first.
fn text_match<'l>(profile: &CompiledProfile, dominant: LineFormat, line: &'l str) -> Option<(usize, Captures<'l>)> {
    let formats = &profile.text_formats;
    if let LineFormat::Text(d) = dominant {
        if let Some(caps) = formats.get(d).and_then(|f| f.regex.captures(line)) {
            return match formats[..d].iter().position(|f| f.regex.is_match(line)) {
                Some(earlier) => formats[earlier].regex.captures(line).map(|c| (earlier, c)),
                None => Some((d, caps)),
            };
        }
    }
    // Lines that do not start like an event (stack frames, wrapped text) skip the other formats.
    if !looks_like_event_start(line) {
        return None;
    }
    let i = match &profile.text_format_set {
        Some(set) => set.matches(line).iter().next(),
        None => formats.iter().position(|f| f.regex.is_match(line)),
    }?;
    formats[i].regex.captures(line).map(|c| (i, c))
}

/// Whether `line` begins an event (as [`LineParser::line`] would read it), for moving a parallel chunk's
/// start to an event boundary.
pub fn begins_event(profile: &CompiledProfile, dominant: LineFormat, line: &str) -> bool {
    let formats = &profile.text_formats;
    let text = match dominant {
        LineFormat::Text(d) if formats.get(d).is_some_and(|f| f.regex.is_match(line)) => true,
        _ => {
            looks_like_event_start(line)
                && match &profile.text_format_set {
                    Some(set) => set.is_match(line),
                    None => formats.iter().any(|f| f.regex.is_match(line)),
                }
        }
    };
    text || (line.trim_start().starts_with('{') && json_event(profile, line).is_some())
}

/// What one line is.
#[derive(Debug)]
pub enum Line {
    /// The first line of an event.
    Event(Record),
    /// More text of the event above.
    Continuation,
    /// Looks like an event but no format reads it (joins the event above; reported).
    Unmatched,
}

/// Parses the lines of one text file (or one chunk of it) into events.
pub struct LineParser<'p> {
    profile: &'p CompiledProfile,
    dominant: LineFormat,
    parser: TimestampParser,
    /// Capture group names of every text format.
    names: Vec<Vec<&'p str>>,
    default_level: Level,
    clock: Clock,
}

/// The day time-only timestamps fall on, rolled forward at midnight, and the last timestamp read.
struct Clock {
    date: Option<NaiveDate>,
    last_ts: Option<i64>,
}

impl Clock {
    /// A timestamp; for time-only formats on the current date, rolled past midnight.
    fn read(&mut self, parser: &TimestampParser, v: &str, time_only: bool) -> Option<i64> {
        let ts = parser.parse(v).or_else(|| {
            let ts = self.date.filter(|_| time_only).and_then(|d| parser.parse_time_on(v, d))?;
            if self.last_ts.is_some_and(|prev| ts + 12 * 3_600_000 < prev) {
                self.date = self.date.and_then(|d| d.succ_opt());
                return Some(ts + 86_400_000);
            }
            Some(ts)
        });
        self.saw(ts);
        ts
    }

    fn saw(&mut self, ts: Option<i64>) {
        if ts.is_some() {
            self.last_ts = ts;
        }
    }
}

impl<'p> LineParser<'p> {
    /// A parser trying `dominant` first, reading timestamps with the profile's parser and `day_first`.
    pub fn new(profile: &'p CompiledProfile, dominant: LineFormat, day_first: Option<bool>, file_date: Option<NaiveDate>) -> Self {
        LineParser {
            profile,
            dominant,
            parser: profile.timestamps.with_day_first(day_first),
            names: profile
                .text_formats
                .iter()
                .map(|f| f.regex.capture_names().flatten().collect())
                .collect(),
            default_level: Level::parse(&profile.profile.parse.json.default_level).unwrap_or(Level::Information),
            clock: Clock {
                date: file_date,
                last_ts: None,
            },
        }
    }

    /// Classifies `line` (without its line break), numbered `number` (1-based).
    pub fn line(&mut self, line: &str, number: u32) -> Line {
        if let Some((format, caps)) = text_match(self.profile, self.dominant, line) {
            return Line::Event(self.text_event(format, &caps, number));
        }
        if line.trim_start().starts_with('{') {
            if let Some(obj) = json_event(self.profile, line) {
                let cfg = &self.profile.profile.parse.json;
                let rec = json_to_record(obj, cfg, &self.parser, self.default_level, number);
                self.clock.saw(rec.ts);
                return Line::Event(rec);
            }
        }
        if looks_like_event_start(line) {
            Line::Unmatched
        } else {
            Line::Continuation
        }
    }

    fn text_event(&mut self, format: usize, caps: &Captures, number: u32) -> Record {
        let time_only = self.profile.text_formats[format].time_only;
        let mut rec = Record::new(number);
        for name in &self.names[format] {
            let Some(m) = caps.name(name) else { continue };
            let v = m.as_str();
            match *name {
                "timestamp" => rec.ts = self.clock.read(&self.parser, v, time_only),
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
        if !self.names[format].contains(&PROPERTIES_GROUP) {
            super::text::split_trailing_properties(&mut rec.message, &mut rec.fields);
        }
        rec
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Profile;

    fn profile(toml: &str) -> CompiledProfile {
        CompiledProfile::compile(Profile::from_toml(toml).unwrap()).unwrap()
    }

    #[test]
    fn event_starts() {
        for yes in [
            "2026-10-14 10:00:00 x",
            "2026/10/14 10:00",
            "[2026-10-14T10:00:00Z] INFO",
            "14.10.2026 10:00:00",
            "10/14/2026 10:00",
            "4-10-2026 x",
            "10:41:22 INF x",
            "[10:41:22 INF] x",
            "(10:41:22) x",
            "<34>1 2026-10-14T10:00:00Z host",
            "Oct 14 10:00:00 host app: x",
            "Oct  4 10:00:00 host app: x",
            "1791968400 x",
            "1791968400123",
            "1791968400.5 x",
        ] {
            assert!(looks_like_event_start(yes), "{yes}");
        }
        for no in [
            "",
            "   at Foo.Bar()",
            "\tat kafka.Foo",
            "Caused by: java.io.IOException",
            "--- End of inner exception stack trace ---",
            "{",
            "}",
            "  \"a\": 1",
            "plain continuation",
            "System.TimeoutException: timed out",
            "42 items left",
            "123456 rows",
            "2026",
            "2026-",
            "1:",
            "12/x",
            "1/2",
            "<",
            "<x>",
            "Oct",
            "Octx14",
            "Oct 1",
            "Foo 14 10:00",
            "Traceback (most recent call last):",
            "12345678901234 x",
        ] {
            assert!(!looks_like_event_start(no), "{no}");
        }
    }

    #[test]
    fn json_events_need_a_timestamp_key() {
        let p = profile("");
        assert!(json_event(&p, r#"{"@t":"2026-10-14T10:00:00Z","@m":"x"}"#).is_some());
        assert!(json_event(&p, r#"  {"time":"2026-10-14T10:00:00Z"}  "#).is_some());
        assert!(json_event(&p, r#"{"@timestamp":null}"#).is_none());
        assert!(json_event(&p, r#"{"order":1}"#).is_none(), "a payload, not an event");
        assert!(json_event(&p, r#"{"broken": "#).is_none());
        assert!(json_event(&p, "[1,2]").is_none());
        let nested = profile("[parse.json]\ntimestamp = ['event.created']");
        assert!(json_event(&nested, r#"{"event":{"created":"2026-10-14T10:00:00Z"}}"#).is_some());
    }

    #[test]
    fn best_format_counts_json_lines() {
        let p = profile("");
        let json = "{\"@t\":\"2026-10-14T10:00:00Z\"}\n";
        let text = "2026-10-14 10:00:00.000 +00:00 [INF] a\n";
        assert_eq!(best_format(&p, &format!("banner\n{json}{json}{text}")), Some(LineFormat::Json));
        assert_eq!(
            best_format(&p, &format!("{json}{text}")),
            Some(LineFormat::Text(0)),
            "ties go to text"
        );
        assert_eq!(best_format(&p, "nothing\n"), None);
        assert!(has_text_events(&p, &format!("{json}{text}")));
        assert!(!has_text_events(&p, &format!("{json}{json}")));
    }

    fn kinds(p: &CompiledProfile, dominant: LineFormat, lines: &[&str]) -> Vec<String> {
        let mut parser = LineParser::new(p, dominant, None, NaiveDate::from_ymd_opt(2026, 10, 14));
        lines
            .iter()
            .enumerate()
            .map(|(i, l)| match parser.line(l, i as u32 + 1) {
                Line::Event(r) => format!("event {:?} {}", r.level, r.message),
                Line::Continuation => "continuation".into(),
                Line::Unmatched => "unmatched".into(),
            })
            .collect()
    }

    #[test]
    fn each_line_takes_the_first_matching_format_in_profile_order() {
        let p = profile("");
        let serilog = "2026-10-14 10:00:00.000 +00:00 [WRN] from A";
        let log4net = "2026-10-14 10:00:01,000 [worker-3] ERROR Inventory.Sync - from B";
        let bsd = "Oct 14 10:00:02 host app[1]: unknown";
        let lines = [
            serilog,
            log4net,
            "   at Foo()",
            bsd,
            r#"{"@t":"2026-10-14T10:00:03Z","@m":"json","@l":"Error"}"#,
        ];
        // Whatever format dominates, every line gets the same reading.
        for dominant in [LineFormat::Text(0), LineFormat::Text(2), LineFormat::Text(4), LineFormat::Json] {
            assert_eq!(
                kinds(&p, dominant, &lines),
                [
                    "event Warning from A",
                    "event Error from B",
                    "continuation",
                    "unmatched",
                    "event Error json",
                ],
                "{dominant:?}"
            );
        }
    }

    #[test]
    fn falls_back_to_trying_formats_one_by_one() {
        let mut p = profile("");
        let line = "2026-10-14 10:00:01,000 [t] INFO L - b";
        assert!(begins_event(&p, LineFormat::Json, line));
        p.text_format_set = None;
        assert_eq!(kinds(&p, LineFormat::Json, &[line]), ["event Information b"]);
        assert!(begins_event(&p, LineFormat::Json, line));
        assert!(!begins_event(&p, LineFormat::Json, "Oct 14 10:00:00 h a: x"));
    }

    #[test]
    fn event_boundaries_agree_with_the_parser() {
        let p = profile("");
        let lines = [
            "2026-10-14 10:00:00.000 +00:00 [WRN] a",
            "2026-10-14 10:00:01,000 [t] ERROR L - b",
            "   at Foo()",
            "Oct 14 10:00:02 host app[1]: c",
            r#"{"@t":"2026-10-14T10:00:03Z"}"#,
            r#"{"x":1}"#,
            "plain",
        ];
        for dominant in [LineFormat::Text(0), LineFormat::Text(2), LineFormat::Json] {
            let mut parser = LineParser::new(&p, dominant, None, None);
            for l in lines {
                assert_eq!(
                    begins_event(&p, dominant, l),
                    matches!(parser.line(l, 1), Line::Event(_)),
                    "{dominant:?} {l}"
                );
            }
        }
    }

    #[test]
    fn time_only_formats_roll_over_midnight() {
        let p =
            profile("[parse]\noutput_templates = [\"[{Timestamp:HH:mm:ss} {Level:u3}] {Message:lj}{NewLine}{Exception}\"]\nregexes = []");
        let mut parser = LineParser::new(&p, LineFormat::Text(0), None, NaiveDate::from_ymd_opt(2026, 10, 14));
        let ts = |l: Line| match l {
            Line::Event(r) => r.ts.unwrap(),
            other => panic!("{other:?}"),
        };
        let a = ts(parser.line("[23:59:59 INF] late", 1));
        let b = ts(parser.line("[00:00:01 INF] early", 2));
        let c = ts(parser.line("[00:00:02 INF] later", 3));
        assert_eq!((b - a, c - b), (2000, 1000));
        // Without a file date a time-only timestamp is unknown.
        let mut parser = LineParser::new(&p, LineFormat::Text(0), None, None);
        assert!(matches!(parser.line("[10:00:00 INF] x", 1), Line::Event(r) if r.ts.is_none()));
    }
}
