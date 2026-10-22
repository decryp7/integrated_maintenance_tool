//! Engine verification of a candidate parser (ADR 0032): what share of a file's lines it reads with a timestamp and
//! a level, the exact lines it fails on and why, whether its timestamp rule is acceptable, the time zone of the
//! file, and whether its identity fields and metrics hold up. A model may propose a parser; only this decides
//! whether it works.
//!
//! [`verify`] is pure: it judges the events [`crate::workspace::preview_parse`] read from a [`VerifySample`];
//! [`verify_text`] runs that preview and the verdict in one go.

pub mod fields;
pub mod sample;
pub mod timestamps;
pub mod zone;

use crate::api::{ParsePreview, ParsedPreview};
use crate::parser::Parser;
use crate::user_regex;
use serde::{Deserialize, Serialize};

pub use fields::{FieldCheck, MetricCheck, MetricRule, MetricUnit};
pub use sample::{model_sample, verification_sample, VerifySample, MAX_MODEL_CHARS, MAX_VERIFY_LINES};
pub use timestamps::TimestampCheck;
pub use zone::{ZoneEvidence, ZoneGuess, ZoneSource};

/// The share of lines a parser must read (with a timestamp and a level) to be done.
pub const TARGET_SCORE: f64 = 0.99;
/// Most failures reported (a few per reason).
pub const MAX_FAILURES: usize = 8;
/// Most failures reported per reason.
const PER_REASON: usize = 2;
/// Longest text fragment quoted in a failure.
const FRAGMENT_CHARS: usize = 120;

/// Which rule a line failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FailedRule {
    /// No line pattern reads it (or the parser does not compile, or reads nothing).
    Format,
    /// It looks like the start of an event, but the pattern read it as a continuation of the one before.
    MissedEntry,
    /// Its event has no readable timestamp.
    Timestamp,
    /// Its event has no level.
    Level,
}

/// One line a parser fails on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineFailure {
    /// 1-based line number in the file.
    pub line: u32,
    /// The text that failed (the timestamp text, or the start of the line).
    pub fragment: String,
    pub rule: FailedRule,
    pub reason: String,
}

/// Facts about the file that are not in its sample.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileFacts {
    /// Modification time, ms since the epoch.
    pub modified_ms: Option<i64>,
    /// Offsets (minutes east of UTC) that other files of the same machine carry.
    pub machine_offsets: Vec<i32>,
}

/// The engine's verdict on a parser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserVerdict {
    /// Readable lines / non-empty lines (0..1).
    pub score: f64,
    pub lines: usize,
    pub readable: usize,
    pub events: usize,
    pub with_timestamp: usize,
    pub with_level: usize,
    /// Whether the parser reads levels at all (else levels are not required of a line).
    pub reads_levels: bool,
    /// Lines that fail, a few per reason, at most [`MAX_FAILURES`].
    pub failures: Vec<LineFailure>,
    pub timestamps: TimestampCheck,
    pub zone: ZoneGuess,
    pub identity: Vec<FieldCheck>,
    pub metrics: Vec<MetricCheck>,
    /// The parser cannot be used at all (does not compile, no importer reads the sample).
    pub error: Option<String>,
}

impl ParserVerdict {
    /// Done: the target share of lines, and an accepted timestamp rule.
    pub fn done(&self) -> bool {
        self.error.is_none() && self.score >= TARGET_SCORE && self.timestamps.accepted
    }

    /// The failures as a short list for a model: line, rule, fragment and reason.
    pub fn feedback(&self) -> String {
        let mut out = format!(
            "The engine read {} of {} lines ({:.1}%); {} events, {} with a timestamp, {} with a level.\n",
            self.readable,
            self.lines,
            self.score * 100.0,
            self.events,
            self.with_timestamp,
            self.with_level
        );
        if let Some(e) = &self.error {
            out.push_str(&format!("The parser cannot be used: {e}\n"));
        }
        if let Some(r) = &self.timestamps.reason {
            out.push_str(&format!("Timestamp rule rejected: {r}\n"));
        }
        if let Some(f) = &self.timestamps.suggested_format {
            out.push_str(&format!("The engine reads these timestamps with the format \"{f}\".\n"));
        }
        for f in &self.failures {
            out.push_str(&format!("- line {} [{:?}] `{}`: {}\n", f.line, f.rule, f.fragment, f.reason));
        }
        for c in self.identity.iter().filter(|c| !c.accepted) {
            out.push_str(&format!("- identity field {} rejected: {}\n", c.field, c.reason));
        }
        for c in self.metrics.iter().filter(|c| !c.accepted) {
            out.push_str(&format!("- metric {} rejected: {}\n", c.name, c.reason));
        }
        out
    }

    fn failed(error: String, zone: ZoneGuess) -> ParserVerdict {
        ParserVerdict {
            score: 0.0,
            lines: 0,
            readable: 0,
            events: 0,
            with_timestamp: 0,
            with_level: 0,
            reads_levels: false,
            failures: vec![LineFailure {
                line: 1,
                fragment: String::new(),
                rule: FailedRule::Format,
                reason: error.clone(),
            }],
            timestamps: TimestampCheck::default(),
            zone,
            identity: Vec::new(),
            metrics: Vec::new(),
            error: Some(error),
        }
    }
}

fn cut(s: &str) -> String {
    s.chars().take(FRAGMENT_CHARS).collect()
}

/// Collects failures: a few per (rule, reason), at most [`MAX_FAILURES`].
#[derive(Default)]
struct Failures(Vec<LineFailure>, usize);

impl Failures {
    fn add(&mut self, f: LineFailure) {
        self.1 += 1;
        let same = self.0.iter().filter(|g| g.rule == f.rule && g.reason == f.reason).count();
        if same < PER_REASON && self.0.len() < MAX_FAILURES {
            self.0.push(f);
        }
    }
}

/// What the parser's line patterns capture of an event line: (timestamp text, whether a level was captured).
struct Captures {
    regexes: Vec<regex::Regex>,
    has_level_group: bool,
}

impl Captures {
    fn of(parser: &Parser) -> Captures {
        let regexes: Vec<regex::Regex> = parser
            .format
            .regexes
            .iter()
            .filter_map(|r| user_regex::compile(r, false).ok())
            .collect();
        let has_level_group = regexes.iter().any(|r| r.capture_names().flatten().any(|n| n == "level"));
        Captures { regexes, has_level_group }
    }

    fn read(&self, line: &str) -> Option<(Option<String>, bool)> {
        self.regexes.iter().find_map(|re| {
            re.captures(line).map(|c| {
                (
                    c.name("timestamp").map(|m| m.as_str().to_string()),
                    c.name("level").is_some_and(|m| !m.as_str().trim().is_empty()),
                )
            })
        })
    }
}

/// Judges `preview` (what `parser` read of `sample`, all of it: `limit` at least the sample's lines).
pub fn verify(parser: &Parser, sample: &VerifySample, preview: &ParsePreview, facts: &FileFacts, metrics: &[MetricRule]) -> ParserVerdict {
    let text_lines: Vec<&str> = sample.text.lines().collect();
    if let Some(e) = &preview.error {
        return ParserVerdict::failed(e.clone(), zone::detect(&ZoneEvidence::default()));
    }
    let records: &[ParsedPreview] = &preview.records;
    let text_kind = parser.format.kind == "text" || !parser.format.regexes.is_empty();
    let caps = Captures::of(parser);
    // Per event: its timestamp text and whether a level was read.
    let read: Vec<(Option<String>, bool)> = records
        .iter()
        .map(|r| {
            let line = r
                .line
                .checked_sub(1)
                .and_then(|i| text_lines.get(i as usize))
                .copied()
                .unwrap_or("");
            match (text_kind, caps.read(line)) {
                (true, Some((ts, level))) => (ts, level),
                // JSON, CSV and templates: the level is read when the event's level is not the default.
                _ => (None, !caps.has_level_group && r.level != crate::model::Level::Information),
            }
        })
        .collect();
    let reads_levels = if text_kind && !caps.regexes.is_empty() {
        caps.has_level_group && read.iter().any(|(_, l)| *l)
    } else {
        read.iter().any(|(_, l)| *l)
    };
    let entry_signature = {
        let mut counts = std::collections::HashMap::<String, usize>::new();
        for r in records {
            if let Some(l) = text_lines.get(r.line.saturating_sub(1) as usize) {
                *counts.entry(sample::signature(l)).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .filter(|(_, n)| *n * 2 >= records.len())
            .map(|(s, _)| s)
    };

    let mut failures = Failures::default();
    let (mut lines, mut readable) = (0usize, 0usize);
    let mut next = 0usize;
    let mut owner: Option<usize> = None;
    for (i, line) in text_lines.iter().enumerate() {
        let n = i as u32 + 1;
        while next < records.len() && records[next].line <= n {
            owner = Some(next);
            next += 1;
        }
        if line.trim().is_empty() {
            continue;
        }
        lines += 1;
        let file_line = sample.file_line(n);
        let Some(e) = owner else {
            failures.add(LineFailure {
                line: file_line,
                fragment: cut(line),
                rule: FailedRule::Format,
                reason: "no line pattern reads this line".into(),
            });
            continue;
        };
        let starts = records[e].line == n;
        if !starts && !line.starts_with(char::is_whitespace) && entry_signature.as_deref() == Some(sample::signature(line).as_str()) {
            failures.add(LineFailure {
                line: file_line,
                fragment: cut(line),
                rule: FailedRule::MissedEntry,
                reason: "this looks like the start of an event, but no line pattern reads it (it was joined to the event above)".into(),
            });
            continue;
        }
        if records[e].ts.is_none() {
            if starts {
                let fragment = read[e].0.clone().unwrap_or_else(|| line.to_string());
                failures.add(LineFailure {
                    line: file_line,
                    fragment: cut(&fragment),
                    rule: FailedRule::Timestamp,
                    reason: "the timestamp text is not understood; fix the timestamp group or timestamp_formats".into(),
                });
            }
            continue;
        }
        if reads_levels && !read[e].1 {
            if starts {
                failures.add(LineFailure {
                    line: file_line,
                    fragment: cut(line),
                    rule: FailedRule::Level,
                    reason: "no level is read from this event".into(),
                });
            }
            continue;
        }
        readable += 1;
    }
    if records.is_empty() && lines > 0 && failures.0.is_empty() {
        failures.add(LineFailure {
            line: sample.file_line(1),
            fragment: cut(text_lines.first().copied().unwrap_or("")),
            rule: FailedRule::Format,
            reason: "the parser reads no event".into(),
        });
    }

    let times: Vec<(u32, Option<i64>)> = records.iter().map(|r| (sample.file_line(r.line), r.ts)).collect();
    let ts_texts: Vec<String> = read.iter().filter_map(|(t, _)| t.clone()).take(200).collect();
    let mut timestamps = timestamps::check(&times, facts.modified_ms);
    if !timestamps.accepted {
        timestamps.suggested_format = timestamps::suggest_format(parser, &ts_texts, facts.modified_ms);
    }
    // The last event's wall clock as if UTC: undo the assumed zone.
    let assumed = zone::offset_minutes(&parser.format.assume_timezone);
    let last_wall_ms = match (records.iter().rev().find_map(|r| r.ts), assumed) {
        (Some(ts), Some(off)) => Some(ts + off as i64 * 60_000),
        _ => None,
    };
    let zone = zone::detect(&ZoneEvidence {
        timestamp_texts: &ts_texts,
        machine_offsets: &facts.machine_offsets,
        modified_ms: facts.modified_ms,
        last_wall_ms,
        text: &sample.text[..floor(&sample.text, 256 * 1024)],
    });
    ParserVerdict {
        score: if lines == 0 { 0.0 } else { readable as f64 / lines as f64 },
        lines,
        readable,
        events: records.len(),
        with_timestamp: records.iter().filter(|r| r.ts.is_some()).count(),
        with_level: read.iter().filter(|(_, l)| *l).count(),
        reads_levels,
        failures: failures.0,
        timestamps,
        zone,
        identity: fields::check_identity(&parser.format.identity, records),
        metrics: fields::check_metrics(metrics, records),
        error: None,
    }
}

/// Most events a parse preview returns (enough for a whole [`VerifySample`]).
pub const MAX_PREVIEW_RECORDS: usize = 5000;

/// Previews `toml` on `sample` with `preview` (a store's parse preview) and judges the result. A parser that does not
/// load is a failed verdict, not an error.
pub fn verify_text(
    toml: &str,
    sample: &VerifySample,
    file_name: &str,
    facts: &FileFacts,
    metrics: &[MetricRule],
    preview: impl FnOnce(&crate::api::ParsePreviewRequest) -> ParsePreview,
) -> ParserVerdict {
    let parser = match Parser::from_toml(toml) {
        Ok(p) => p,
        Err(e) => return ParserVerdict::failed(format!("the parser does not load: {e}"), zone::detect(&ZoneEvidence::default())),
    };
    let pv = preview(&crate::api::ParsePreviewRequest {
        toml: toml.to_string(),
        profile: String::new(),
        sample: sample.text.clone(),
        file_name: file_name.to_string(),
        limit: MAX_PREVIEW_RECORDS,
    });
    verify(&parser, sample, &pv, facts, metrics)
}

/// The largest char boundary at or below `i`.
fn floor(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests;
