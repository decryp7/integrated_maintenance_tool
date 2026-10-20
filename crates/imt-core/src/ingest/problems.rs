//! Import problems, aggregated per kind (ADR 0014 §5): counts and a few sample lines, never one entry
//! per bad line.

use crate::api::Severity;
use serde::Serialize;
use std::collections::BTreeMap;

/// Most sample line numbers kept per problem.
pub const MAX_SAMPLES: usize = 20;

/// What went wrong while reading a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
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
}

impl ProblemKind {
    /// Stable id, as stored in the `diagnostics` table.
    pub fn id(self) -> &'static str {
        match self {
            ProblemKind::LineTooLong => "lineTooLong",
            ProblemKind::EventTooLarge => "eventTooLarge",
            ProblemKind::LineNotParsed => "lineNotParsed",
            ProblemKind::TimestampUnparsed => "timestampUnparsed",
        }
    }

    pub fn from_id(id: &str) -> Option<ProblemKind> {
        [
            ProblemKind::LineTooLong,
            ProblemKind::EventTooLarge,
            ProblemKind::LineNotParsed,
            ProblemKind::TimestampUnparsed,
        ]
        .into_iter()
        .find(|k| k.id() == id)
    }

    pub fn severity(self) -> Severity {
        match self {
            ProblemKind::LineTooLong | ProblemKind::EventTooLarge | ProblemKind::TimestampUnparsed => Severity::Warning,
            ProblemKind::LineNotParsed => Severity::Info,
        }
    }
}

/// One kind of problem in one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub kind: ProblemKind,
    pub count: u64,
    /// 1-based line numbers of the first occurrences (at most [`MAX_SAMPLES`]).
    pub sample_lines: Vec<u32>,
}

/// Problems of one source, collected while it is read.
#[derive(Debug, Default, Clone)]
pub struct Problems {
    by_kind: BTreeMap<ProblemKind, Problem>,
}

impl Problems {
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

    pub fn count(&self, kind: ProblemKind) -> u64 {
        self.by_kind.get(&kind).map_or(0, |p| p.count)
    }

    pub fn is_empty(&self) -> bool {
        self.by_kind.is_empty()
    }

    pub fn into_vec(self) -> Vec<Problem> {
        self.by_kind.into_values().collect()
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
        for k in [
            ProblemKind::LineTooLong,
            ProblemKind::EventTooLarge,
            ProblemKind::LineNotParsed,
            ProblemKind::TimestampUnparsed,
        ] {
            assert_eq!(ProblemKind::from_id(k.id()), Some(k));
        }
        assert_eq!(ProblemKind::from_id("nope"), None);
        assert_eq!(ProblemKind::LineNotParsed.severity(), Severity::Info);
        assert_eq!(ProblemKind::LineTooLong.severity(), Severity::Warning);
    }
}
