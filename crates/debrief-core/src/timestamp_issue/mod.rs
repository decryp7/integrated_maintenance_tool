//! Flags a source whose timestamps could not be read for many of its events, with examples and a
//! timestamp format the engine verified against them ([`suggest`]). The summary carries it as
//! [`TimestampIssue`] on the source; the Explorer and the notification centre show it.
//!
//! A source is flagged when more than [`MAX_UNPARSED_PERCENT`] % of its events have no timestamp, or
//! when none of its first [`HEAD_EVENTS`] events has one (a file whose start is unreadable, however
//! long it is). A source where no event has a timestamp is flagged only when its lines show
//! timestamp-like text (a JSON settings file or a data table without dates is not a log with unreadable
//! timestamps). Only importers that read timestamps with the profile's parser are checked
//! ([`CHECKED_IMPORTERS`]): a plugin's format knows whether it carries timestamps, and the profile could
//! not change that anyway.

pub mod suggest;

use crate::api::{TimestampIssue, TimestampSuggestion};
use crate::model::Record;
use crate::time::TimestampParser;

/// More events than this share (percent) without a timestamp flags the source.
pub const MAX_UNPARSED_PERCENT: usize = 5;
/// None of this many first events with a timestamp flags the source too.
pub const HEAD_EVENTS: usize = 20;
/// Example lines kept on the issue.
pub const MAX_EXAMPLES: usize = 3;
/// Characters kept of each example line.
pub const MAX_EXAMPLE_CHARS: usize = 120;
/// Lines without a timestamp the suggestion looks at.
const SUGGEST_SAMPLES: usize = 20;
/// Characters of a line the suggestion looks at.
const MAX_SAMPLE_CHARS: usize = 512;
/// The importers whose timestamps come from the parser (`[format]`).
pub const CHECKED_IMPORTERS: [&str; 3] = ["text", "csv", "json"];

/// The issue of a source read by `importer` from `content` into `records` (after the pipeline), or None
/// when its timestamps are fine. `parser` is the source parser's, which the suggestion must extend.
pub fn assess(importer: &str, records: &[Record], content: &str, parser: &TimestampParser) -> Option<TimestampIssue> {
    if !CHECKED_IMPORTERS.contains(&importer) {
        return None;
    }
    let total = records.len();
    let unparsed = records.iter().filter(|r| r.ts.is_none()).count();
    let head_unreadable = records.iter().take(HEAD_EVENTS).all(|r| r.ts.is_none());
    if unparsed == 0 || !(unparsed * 100 > total * MAX_UNPARSED_PERCENT || head_unreadable) {
        return None;
    }
    let mut wanted: Vec<u32> = records
        .iter()
        .filter(|r| r.ts.is_none())
        .map(|r| r.line)
        .take(SUGGEST_SAMPLES)
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    let lines = lines_at(content, &wanted);
    let samples: Vec<String> = lines.iter().map(|(_, l)| cut(l, MAX_SAMPLE_CHARS, false)).collect();
    let refs: Vec<&str> = samples.iter().map(String::as_str).collect();
    if unparsed == total && !suggest::looks_timestamped(&refs) {
        return None;
    }
    let suggestion = suggest::propose(&refs, parser).and_then(|p| {
        Some(TimestampSuggestion {
            parser_snippet: snippet(&p.format),
            parsed_samples: p.texts.len(),
            checked_samples: refs.len(),
            sample: p.texts.into_iter().next()?,
            format: p.format,
        })
    });
    Some(TimestampIssue {
        unparsed_lines: unparsed,
        total_lines: total,
        examples: lines
            .iter()
            .take(MAX_EXAMPLES)
            .map(|(_, l)| cut(l, MAX_EXAMPLE_CHARS, true))
            .collect(),
        example_lines: lines.iter().take(MAX_EXAMPLES).map(|(n, _)| *n).collect(),
        suggestion,
    })
}

/// The number and text of the given 1-based lines (sorted), skipping blank ones; one pass up to the last of them.
fn lines_at<'a>(content: &'a str, wanted: &[u32]) -> Vec<(u32, &'a str)> {
    let Some(&last) = wanted.last() else { return Vec::new() };
    let mut next = wanted.iter().peekable();
    let mut out = Vec::new();
    for (i, line) in content.lines().enumerate().take(last as usize) {
        let n = i as u32 + 1;
        while next.next_if(|&&w| w < n).is_some() {}
        if next.next_if_eq(&&n).is_some() {
            let line = line.trim_end_matches('\r');
            if !line.trim().is_empty() {
                out.push((n, line));
            }
        }
    }
    out
}

/// At most `max` characters of `s` (control characters as spaces), with "…" when cut and `ellipsis`.
fn cut(s: &str, max: usize, ellipsis: bool) -> String {
    let mut out: String = s.chars().take(max).map(|c| if c.is_control() { ' ' } else { c }).collect();
    if ellipsis && s.chars().nth(max).is_some() {
        out.push('…');
    }
    out
}

/// The parser lines that add `format` (TOML-quoted).
fn snippet(format: &str) -> String {
    let quoted = toml::Value::String(format.to_string()).to_string();
    format!("[format]\ntimestamp_formats = [{quoted}]\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(ts: &[Option<i64>]) -> Vec<Record> {
        ts.iter()
            .enumerate()
            .map(|(i, t)| {
                let mut r = Record::new(i as u32 + 1);
                r.ts = *t;
                r
            })
            .collect()
    }

    fn content(n: usize) -> String {
        (0..n).map(|i| format!("Sat Nov 08 00:04:{:02} 2025 line {i}\n", i % 60)).collect()
    }

    #[test]
    fn thresholds() {
        let p = TimestampParser::default();
        let c = content(200);
        // All readable: nothing.
        assert_eq!(assess("text", &records(&[Some(1); 100]), &c, &p), None);
        // 5 of 100 unreadable (not more than 5 %) after a readable start: nothing.
        let mut ts = vec![Some(1); 100];
        ts[50..55].fill(None);
        assert_eq!(assess("text", &records(&ts), &c, &p), None);
        // 6 of 100: flagged.
        ts[55] = None;
        let issue = assess("text", &records(&ts), &c, &p).unwrap();
        assert_eq!((issue.unparsed_lines, issue.total_lines), (6, 100));
        assert_eq!(issue.examples.len(), MAX_EXAMPLES);
        assert!(issue.examples[0].starts_with("Sat Nov 08 00:04:50 2025 line 50"));
        assert_eq!(issue.example_lines, [51, 52, 53]);
        // The first 20 unreadable, the rest fine (1 % overall): flagged too.
        let mut ts = vec![Some(1); 2000];
        ts[..HEAD_EVENTS].fill(None);
        assert!(assess("csv", &records(&ts), &content(2000), &p).is_some());
        // 19 of the first 20: not flagged by the start rule.
        ts[HEAD_EVENTS - 1] = Some(1);
        assert_eq!(assess("csv", &records(&ts), &content(2000), &p), None);
        // Plugin importers and empty sources are not checked.
        assert_eq!(assess("syslog", &records(&[None; 10]), &c, &p), None);
        assert_eq!(assess("text", &[], "", &p), None);
    }

    #[test]
    fn suggestion_is_verified_and_snippet_is_toml() {
        let p = TimestampParser::default();
        let issue = assess("json", &records(&[None; 30]), &content(30), &p).unwrap();
        let s = issue.suggestion.unwrap();
        assert_eq!(s.format, "%a %b %d %H:%M:%S %Y");
        assert_eq!(s.sample, "Sat Nov 08 00:04:00 2025");
        assert_eq!((s.parsed_samples, s.checked_samples), (20, 20));
        assert_eq!(s.parser_snippet, "[format]\ntimestamp_formats = [\"%a %b %d %H:%M:%S %Y\"]\n");
        let parsed: toml::Value = toml::from_str(&s.parser_snippet).unwrap();
        assert_eq!(parsed["format"]["timestamp_formats"][0].as_str(), Some(s.format.as_str()));
        // Lines without anything timestamp-like next to readable ones: examples but no suggestion.
        let ts = [None, None, None, Some(1)];
        let issue = assess("text", &records(&ts), "alpha\nbeta\ngamma\n2025-11-08 x\n", &p).unwrap();
        assert_eq!(issue.examples, ["alpha", "beta", "gamma"]);
        assert_eq!(issue.example_lines, [1, 2, 3]);
        assert_eq!(issue.suggestion, None);
        // No timestamp anywhere and nothing timestamp-like (a settings file): not a log with unreadable timestamps.
        assert_eq!(assess("json", &records(&[None; 2]), "{\n}\n", &p), None);
    }

    #[test]
    fn examples_are_bounded_and_clean() {
        let p = TimestampParser::default();
        let long = format!("2025-11-08_x {}\u{7}tail\n", "é".repeat(300));
        let issue = assess("text", &records(&[None]), &long, &p).unwrap();
        assert_eq!(issue.examples[0].chars().count(), MAX_EXAMPLE_CHARS + 1);
        assert!(issue.examples[0].ends_with('…'));
        assert_eq!(cut("a\u{7}b", 10, true), "a b");
        assert_eq!(cut("abc", 3, true), "abc");
    }

    #[test]
    fn lines_at_skips_blank_and_missing_lines() {
        let text = "one\r\n\ntwo\nthree\n";
        assert_eq!(lines_at(text, &[1, 2, 3, 9]), [(1, "one"), (3, "two")]);
        assert_eq!(lines_at(text, &[4]), [(4, "three")]);
        assert_eq!(lines_at(text, &[]), Vec::<(u32, &str)>::new());
        // Line numbers the content does not have (e.g. 0) are passed over.
        assert_eq!(lines_at(text, &[0, 3]), [(3, "two")]);
    }
}
