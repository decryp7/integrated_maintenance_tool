//! Import problems, aggregated per kind (ADR 0014 §5): counts and a few sample lines, never one entry
//! per bad line. Importers report them in [`crate::importer::Imported::problems`]; they cross to the host at
//! the end of an import (ADR 0017).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How urgent a problem (or a finding of the engine's health check) is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// Needs attention now; the data is likely wrong or incomplete.
    Critical,
    /// Worth a look; part of the data may be affected.
    Warning,
    /// For information only.
    Info,
}

/// Most sample line numbers kept per problem.
pub const MAX_SAMPLES: usize = 20;

/// What went wrong while reading a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProblemKind {
    /// A line longer than the line limit was skipped.
    LineTooLong,
    /// An event reached the event size limit; its further continuation lines were dropped.
    EventTooLarge,
    /// A line that is not an event (e.g. not JSON in a JSON-lines file) was skipped.
    LineNotParsed,
    /// Events without a readable timestamp (kept, marked untimed).
    TimestampUnparsed,
    /// A field value that does not fit its declared type (kept as text).
    FieldInvalid,
    /// Bytes that are not valid in the file's encoding (replaced).
    Encoding,
    /// A file no importer recognises (not imported).
    UnknownFormat,
    /// An archive that ends early or is damaged (the readable part was imported).
    TruncatedArchive,
    /// An event split at a chunk boundary of a parallel import.
    EventSplit,
    /// A line that looks like the start of an event (a date, time or syslog header) but that no text format
    /// of the profile reads; it was kept as a continuation of the event above (ADR 0020).
    UnmatchedLine,
    /// Events whose `[[route]]` label would have made more logical sources than a file may have; they stay
    /// in the file's own source (ADR 0020).
    RouteLimit,
}

/// Every kind.
const ALL_KINDS: [ProblemKind; 11] = [
    ProblemKind::LineTooLong,
    ProblemKind::EventTooLarge,
    ProblemKind::LineNotParsed,
    ProblemKind::TimestampUnparsed,
    ProblemKind::FieldInvalid,
    ProblemKind::Encoding,
    ProblemKind::UnknownFormat,
    ProblemKind::TruncatedArchive,
    ProblemKind::EventSplit,
    ProblemKind::UnmatchedLine,
    ProblemKind::RouteLimit,
];

impl ProblemKind {
    /// Stable id, as stored in the `diagnostics` table.
    pub fn id(self) -> &'static str {
        match self {
            ProblemKind::LineTooLong => "lineTooLong",
            ProblemKind::EventTooLarge => "eventTooLarge",
            ProblemKind::LineNotParsed => "lineNotParsed",
            ProblemKind::TimestampUnparsed => "timestampUnparsed",
            ProblemKind::FieldInvalid => "fieldInvalid",
            ProblemKind::Encoding => "encoding",
            ProblemKind::UnknownFormat => "unknownFormat",
            ProblemKind::TruncatedArchive => "truncatedArchive",
            ProblemKind::EventSplit => "eventSplit",
            ProblemKind::UnmatchedLine => "unmatchedLine",
            ProblemKind::RouteLimit => "routeLimit",
        }
    }

    /// Parses a stable id as returned by [`ProblemKind::id`].
    pub fn from_id(id: &str) -> Option<ProblemKind> {
        ALL_KINDS.into_iter().find(|k| k.id() == id)
    }

    /// How urgent this kind of problem is.
    pub fn severity(self) -> Severity {
        match self {
            ProblemKind::LineTooLong
            | ProblemKind::EventTooLarge
            | ProblemKind::TimestampUnparsed
            | ProblemKind::UnknownFormat
            | ProblemKind::TruncatedArchive
            | ProblemKind::UnmatchedLine => Severity::Warning,
            ProblemKind::LineNotParsed
            | ProblemKind::FieldInvalid
            | ProblemKind::Encoding
            | ProblemKind::EventSplit
            | ProblemKind::RouteLimit => Severity::Info,
        }
    }
}

/// One kind of problem in one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// What went wrong.
    pub kind: ProblemKind,
    /// How many times it happened.
    pub count: u64,
    /// 1-based line numbers of the first occurrences (at most [`MAX_SAMPLES`]).
    pub sample_lines: Vec<u32>,
}

/// Problems of one source, collected while it is read.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problems {
    by_kind: BTreeMap<ProblemKind, Problem>,
}

impl Problems {
    /// Records one occurrence of `kind` at `line` (1-based); only the first [`MAX_SAMPLES`] lines are kept.
    pub fn add(&mut self, kind: ProblemKind, line: u32) {
        let p = self.by_kind.entry(kind).or_insert_with(|| Problem {
            kind,
            count: 0,
            sample_lines: Vec::new(),
        });
        p.count += 1;
        if p.sample_lines.len() < MAX_SAMPLES {
            p.sample_lines.push(line);
        }
    }

    /// Occurrences of `kind` recorded so far.
    pub fn count(&self, kind: ProblemKind) -> u64 {
        self.by_kind.get(&kind).map_or(0, |p| p.count)
    }

    /// Whether no problem has been recorded.
    pub fn is_empty(&self) -> bool {
        self.by_kind.is_empty()
    }

    /// Adds the problems of another part of the same source (a chunk read in parallel); the samples stay
    /// the first [`MAX_SAMPLES`] lines.
    pub fn merge(&mut self, other: Problems) {
        for (kind, p) in other.by_kind {
            let mine = self.by_kind.entry(kind).or_insert_with(|| Problem {
                kind,
                count: 0,
                sample_lines: Vec::new(),
            });
            mine.count += p.count;
            mine.sample_lines.extend(p.sample_lines);
            mine.sample_lines.sort_unstable();
            mine.sample_lines.dedup();
            mine.sample_lines.truncate(MAX_SAMPLES);
        }
    }

    /// One [`Problem`] per recorded kind, ordered by kind.
    pub fn into_vec(self) -> Vec<Problem> {
        self.by_kind.into_values().collect()
    }

    /// Problems sent as a list (from another process): counts add up, samples stay bounded.
    pub fn from_vec(list: Vec<Problem>) -> Problems {
        let mut out = Problems::default();
        for mut p in list {
            p.sample_lines.truncate(MAX_SAMPLES);
            let mut one = Problems::default();
            one.by_kind.insert(p.kind, p);
            out.merge(one);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_are_counted_with_bounded_samples() {
        let mut p = Problems::default();
        assert!(p.is_empty());
        for line in 1..=30 {
            p.add(ProblemKind::LineTooLong, line);
        }
        p.add(ProblemKind::TimestampUnparsed, 7);
        assert_eq!(p.count(ProblemKind::LineTooLong), 30);
        assert_eq!(p.count(ProblemKind::EventTooLarge), 0);
        let v = p.into_vec();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].sample_lines.len(), MAX_SAMPLES);
        assert_eq!(v[1].sample_lines, [7]);
        for k in ALL_KINDS {
            assert_eq!(ProblemKind::from_id(k.id()), Some(k));
            // The id is the JSON name.
            assert_eq!(serde_json::to_value(k).unwrap(), k.id());
        }
        assert_eq!(ProblemKind::from_id("nope"), None);
        assert_eq!(ProblemKind::LineNotParsed.severity(), Severity::Info);
        assert_eq!(ProblemKind::LineTooLong.severity(), Severity::Warning);
        assert_eq!(ProblemKind::UnmatchedLine.severity(), Severity::Warning);
        assert_eq!(ProblemKind::RouteLimit.severity(), Severity::Info);
    }

    #[test]
    fn a_list_from_another_process_is_merged_and_bounded() {
        let mut p = Problems::default();
        p.add(ProblemKind::LineNotParsed, 3);
        p.add(ProblemKind::LineNotParsed, 9);
        let mut list = p.clone().into_vec();
        list.push(Problem {
            kind: ProblemKind::LineNotParsed,
            count: 5,
            sample_lines: (1..=40).collect(),
        });
        let back = Problems::from_vec(list);
        assert_eq!(back.count(ProblemKind::LineNotParsed), 7);
        assert_eq!(back.into_vec()[0].sample_lines.len(), MAX_SAMPLES);
        assert!(Problems::from_vec(Vec::new()).is_empty());
    }
}
