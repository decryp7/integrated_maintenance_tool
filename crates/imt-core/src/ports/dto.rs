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

/// The parser a source was read with (ADR 0026: parsers are individual TOML files matched per file).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserRef {
    pub id: String,
    pub version: String,
    /// Content hash of the parser's TOML (canonical form): a source is re-read when its parser's hash changed.
    pub hash: String,
}

/// Hex SHA-256 (the first 16 bytes) of a parser's TOML.
pub fn content_hash(text: &str) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(text.as_bytes());
    d.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// A parser's definition: its id and version and its TOML (ADR 0026).
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
    /// How the event matched a smart search (set on pages of a word search; see [`mark_search_matches`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_match: Option<SearchMatch>,
}

/// How an event matched the search box's text (smart search).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchMatch {
    /// It holds every word of the text.
    Words,
    /// It holds the text only as a substring (added by the background scan).
    Partial,
}

/// Marks how each row matched the filter's search box text when it is a word search (rows of a substring,
/// regex or case-sensitive search, or without text, stay unmarked).
pub fn mark_search_matches(rows: &mut [EventView], filter: &crate::ports::Filter) {
    if filter.text_mode.is_substring() || filter.regex || filter.case_sensitive || filter.text.trim().is_empty() {
        return;
    }
    let words = crate::words::WordQuery::parse(&filter.text);
    if words.is_empty() {
        return;
    }
    for r in rows {
        let texts: Vec<&str> = [Some(r.message.as_str()), r.exception.as_deref(), r.template.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        r.search_match = Some(if words.matches(&texts) {
            SearchMatch::Words
        } else {
            SearchMatch::Partial
        });
    }
}

/// What the background scan of a smart search found: events holding the text only as a substring.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialMatches {
    /// Matching events that are not word matches ("N more partial matches").
    pub added: u64,
    /// False when the scan looked at only part of the events (a limit stopped it).
    pub complete: bool,
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

/// An engine-proposed fix of a diagnostic: a parser override and, for unreadable timestamps, the
/// verified format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticFix {
    pub title: String,
    /// `[format]` TOML merged over the source's parser (the `parserOverride` of a [`QuickFix`]).
    pub parser_override: String,
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

/// A proposed parser change (a `[format]` fragment merged over the parser of each source) for some sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickFix {
    pub parser_override: String,
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
            search_match: None,
        };
        let json = serde_json::to_value(&view).unwrap();
        assert!(json.get("searchMatch").is_none());
        let mut rows = vec![
            view.clone(),
            EventView {
                message: "runtime".into(),
                ..view.clone()
            },
            EventView {
                message: "Time out".into(),
                ..view.clone()
            },
        ];
        let filter = |text: &str, text_mode| crate::ports::Filter {
            text: text.into(),
            text_mode,
            ..Default::default()
        };
        mark_search_matches(&mut rows, &filter("time", crate::query::TextMode::Substring));
        assert!(rows.iter().all(|r| r.search_match.is_none()), "a substring search is not marked");
        mark_search_matches(&mut rows, &filter("time", crate::query::TextMode::WordsOrSubstring));
        assert_eq!(
            rows.iter().map(|r| r.search_match).collect::<Vec<_>>(),
            [Some(SearchMatch::Partial), Some(SearchMatch::Partial), Some(SearchMatch::Words)]
        );
        let mut fresh = vec![view.clone()];
        mark_search_matches(&mut fresh, &filter("--", crate::query::TextMode::Words));
        assert_eq!(fresh[0].search_match, None, "no words: nothing to mark");
        assert_eq!(serde_json::to_value(SearchMatch::Partial).unwrap(), "partial");
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
        let fix: QuickFix = serde_json::from_str(r#"{"parserOverride": "[format]"}"#).unwrap();
        assert!(fix.sources.is_empty());
    }
}
