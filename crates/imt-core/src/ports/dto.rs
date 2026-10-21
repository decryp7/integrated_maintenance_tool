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

/// The parser a source was read with (ADR 0026: parsers are individual TOML files matched per file; until
/// then the workspace's one profile is the parser of every source, id [`ParserRef::PROFILE`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserRef {
    pub id: String,
    pub version: String,
    /// Content hash of the parser's TOML (canonical form): a source is re-read when its parser's hash changed.
    pub hash: String,
}

impl ParserRef {
    /// The id of the interim parser: the workspace's profile.
    pub const PROFILE: &'static str = "profile";

    /// The workspace's profile as the parser of every source (the interim before ADR 0026).
    pub fn of_profile(profile: &crate::Profile) -> ParserRef {
        ParserRef {
            id: Self::PROFILE.into(),
            version: profile.name.clone(),
            hash: content_hash(&profile_toml(profile)),
        }
    }
}

/// A profile's canonical TOML (what its parser hash and the embedded copy are made of).
pub fn profile_toml(profile: &crate::Profile) -> String {
    toml::to_string(profile).unwrap_or_default()
}

/// Hex SHA-256 (the first 16 bytes) of a parser's TOML.
pub fn content_hash(text: &str) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(text.as_bytes());
    d.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// A parser's definition: its id and version and its TOML (ADR 0026; until then a profile's TOML).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserDef {
    pub id: String,
    pub version: String,
    pub toml: String,
}

/// Which sources a parser reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserUse {
    pub parser: ParserRef,
    pub sources: Vec<SourceId>,
}

/// What changing a parser would cost: the class of the change and the sources it touches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserChange {
    pub class: crate::profile_change::ChangeClass,
    /// Changed sections, e.g. `parse`, `extract`.
    pub sections: Vec<String>,
    /// The sources that would be re-derived or re-read.
    pub sources: Vec<SourceId>,
}

/// Events by id (a report's events, bookmarks, AI citations), at most [`MAX_PAGE_ROWS`]; ids that do not
/// exist (any more) are left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EventsRequest {
    pub ids: Vec<EventId>,
    /// The instance key's fields (empty: the engine's default key).
    pub group_by: Vec<String>,
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

impl EventView {
    /// The event as a record (for code written over records: redaction, reports). Ids beyond `u32` saturate
    /// until records carry `EventId`s.
    pub fn to_record(&self) -> crate::model::Record {
        crate::model::Record {
            id: u32::try_from(self.id.0).unwrap_or(u32::MAX),
            ts: self.ts,
            level: self.level,
            message: self.message.clone(),
            template: self.template.clone(),
            exception: self.exception.clone(),
            fields: self.fields.clone(),
            source: self.source.0,
            line: self.line,
            markers: 0,
        }
    }
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

/// One kind of import problem in one source (or one file that was not imported).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticRow {
    /// The source, when the file was imported.
    pub source: Option<SourceId>,
    pub source_name: String,
    /// The file (with the archive entry for an archive member).
    pub path: String,
    pub kind: ProblemKind,
    pub severity: crate::api::Severity,
    pub count: u64,
    /// At most 20 1-based line numbers of the first occurrences.
    pub sample_lines: Vec<u32>,
    /// What the engine knows more (e.g. why a file was not opened).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A fix the engine checked against the samples, ready for quick-fix evaluation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<DiagnosticFix>,
}

impl DiagnosticRow {
    pub fn new(source: SourceId, source_name: String, path: String, p: Problem) -> DiagnosticRow {
        DiagnosticRow {
            source: Some(source),
            source_name,
            path,
            severity: p.kind.severity(),
            kind: p.kind,
            count: p.count,
            sample_lines: p.sample_lines,
            detail: None,
            fix: None,
        }
    }
}

/// An engine-proposed fix of a diagnostic: a profile override and, for unreadable timestamps, the
/// verified format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticFix {
    pub title: String,
    /// TOML merged over the active profile (the `profileOverride` of a [`QuickFix`]).
    pub profile_override: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<crate::api::TimestampSuggestion>,
}

/// How much of the matching data an analysis looked at (engine review finding 16: no silent caps).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    /// Events the analysis's query matched.
    pub matched: u64,
    /// Events it looked at (fewer than `matched` when a limit stopped it).
    pub analysed: u64,
    /// The limits that bounded the result, in words (e.g. "patterns: 10,000 kept; 312 events in the
    /// overflow pattern").
    pub limits: Vec<String>,
}

impl Coverage {
    /// Whether every matching event was analysed and no limit bounded the result.
    pub fn complete(&self) -> bool {
        self.analysed == self.matched && self.limits.is_empty()
    }

    /// Both coverages together (an analysis over two queries).
    pub fn and(mut self, other: Coverage) -> Coverage {
        self.matched += other.matched;
        self.analysed += other.analysed;
        self.limits.extend(other.limits);
        self
    }
}

/// An analysis result with its [`Coverage`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analyzed<T> {
    pub result: T,
    pub coverage: Coverage,
}

/// A proposed profile override (TOML merged over the active profile) for some sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickFix {
    pub profile_override: String,
    #[serde(default)]
    pub sources: Vec<SourceId>,
}

/// How a proposed fix does on the stored events it applies to: "fixes `fixed` of `checked`".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixEvaluation {
    pub fixed: u64,
    pub checked: u64,
    /// Affected events not checked (beyond the evaluation limit).
    pub unchecked: u64,
    /// Why some problems could not be checked (e.g. skipped lines are not stored).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
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
            "/x/a.log".into(),
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
