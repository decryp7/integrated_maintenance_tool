//! Owned request and reply types of the ports (camelCase on the wire). Requests the in-memory store
//! already has (`TimelineRequest`, `FacetRequest`, ...) are reused from [`crate::api`].

use crate::api::{FieldInfo, SourceInfo, UnrecognizedFile};
use crate::batch::{EventId, SourceId, TsState};
use crate::ingest::{Problem, ProblemKind};
use crate::model::Level;
use crate::query::Query;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Most rows of one page.
pub const MAX_PAGE_ROWS: usize = 5000;

/// A page of events: `limit` rows from `offset` in time order (or reversed), or the page that holds
/// `anchor` when it matches.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PageQuery {
    pub query: Query,
    pub offset: usize,
    pub limit: usize,
    pub descending: bool,
    pub anchor: Option<EventId>,
}

impl PageQuery {
    /// The page size, within 1..=[`MAX_PAGE_ROWS`] (0 = the default of 500).
    pub fn rows(&self) -> usize {
        if self.limit == 0 {
            500
        } else {
            self.limit.min(MAX_PAGE_ROWS)
        }
    }
}

/// One event as the grid shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventView {
    pub id: EventId,
    pub ts: Option<i64>,
    pub ts_state: TsState,
    pub level: Level,
    pub message: String,
    pub template: Option<String>,
    pub exception: Option<String>,
    pub fields: Map<String, Value>,
    pub source: SourceId,
    pub source_name: String,
    pub line: u32,
    pub byte_offset: u64,
    /// The instance key (the query's group-by fields).
    pub group: String,
    pub markers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageReply {
    /// Events matching the query.
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<EventView>,
    /// Index (in the whole result) of the anchor event, when it matches.
    pub anchor_index: Option<usize>,
}

/// What is loaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryReply {
    pub total: usize,
    /// Events without a timestamp (in the grid, not on the timeline).
    pub untimed: usize,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub levels: BTreeMap<Level, usize>,
    pub sources: Vec<SourceInfo>,
    pub unrecognized: Vec<UnrecognizedFile>,
    pub fields: Vec<FieldInfo>,
    pub default_group_by: Vec<String>,
    /// Marker kinds by bit (bit i of an event's markers = kind i).
    pub marker_kinds: Vec<String>,
    pub profile_name: String,
}

/// The result of an import job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// Sources read, with their event counts.
    pub sources: Vec<SourceInfo>,
    pub unrecognized: Vec<UnrecognizedFile>,
    pub events: usize,
    pub bytes: u64,
    pub elapsed_ms: u64,
}

/// One kind of import problem in one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRow {
    pub source: SourceId,
    pub source_name: String,
    pub kind: ProblemKind,
    pub severity: crate::api::Severity,
    pub count: u64,
    pub sample_lines: Vec<u32>,
}

impl DiagnosticRow {
    pub fn new(source: SourceId, source_name: String, p: Problem) -> DiagnosticRow {
        DiagnosticRow {
            source,
            source_name,
            severity: p.kind.severity(),
            kind: p.kind,
            count: p.count,
            sample_lines: p.sample_lines,
        }
    }
}

/// A proposed profile override (TOML merged over the active profile) for some sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickFix {
    pub profile_override: String,
    #[serde(default)]
    pub sources: Vec<SourceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixEvaluation {
    pub fixed: u64,
    pub checked: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub text: String,
    #[serde(default)]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReply {
    /// Matching events (with `exact` false: candidates of which not all were checked; an upper bound).
    pub total: u64,
    /// The first matches in time order.
    pub ids: Vec<EventId>,
    pub exact: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub indexed: u64,
    pub total: u64,
    pub ready: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_queries_are_bounded_and_camel_case() {
        let q: PageQuery = serde_json::from_str(r#"{"anchor": 42, "limit": 9000}"#).unwrap();
        assert_eq!(q.anchor, Some(EventId(42)));
        assert_eq!(q.rows(), MAX_PAGE_ROWS);
        assert_eq!(PageQuery::default().rows(), 500);
        let view = EventView {
            id: EventId(1),
            ts: None,
            ts_state: TsState::Untimed,
            level: Level::Information,
            message: "m".into(),
            template: None,
            exception: None,
            fields: Map::new(),
            source: SourceId(2),
            source_name: "s".into(),
            line: 3,
            byte_offset: 4,
            group: "g".into(),
            markers: vec![],
        };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["tsState"], "untimed");
        assert_eq!(json["byteOffset"], 4);
        assert_eq!(json["source"], 2);
        let row = DiagnosticRow::new(
            SourceId(1),
            "a.log".into(),
            Problem {
                kind: ProblemKind::LineTooLong,
                count: 2,
                sample_lines: vec![5, 9],
            },
        );
        assert_eq!(serde_json::to_value(&row).unwrap()["kind"], "lineTooLong");
        let fix: QuickFix = serde_json::from_str(r#"{"profileOverride": "[parse]"}"#).unwrap();
        assert!(fix.sources.is_empty());
    }
}
