//! Request and response types of clock-skew correction (wire format of the
//! `set_source_offset` and `suggest_offsets` commands).

use super::{DEFAULT_ID_FIELDS, DEFAULT_MAX_DELTA_MS, DEFAULT_TOLERANCE_MS};
use serde::{Deserialize, Serialize};

/// How a source's new offset is given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OffsetSpec {
    /// An explicit offset in milliseconds (0 clears it).
    Manual { source: u32, offset_ms: i64 },
    /// Align the target event's source so the target shows at the reference
    /// event's time (the same real-world moment seen by two machines).
    Align { reference: u32, target: u32 },
}

/// `set_source_offset` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetOffsetRequest {
    #[serde(flatten)]
    pub spec: OffsetSpec,
}

/// What `set_source_offset` changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OffsetChange {
    pub source: u32,
    pub offset_ms: i64,
    pub previous_ms: i64,
    /// Ids at and after this one were renumbered (records moved in time). The log store keeps event ids
    /// stable when events move (an id is not a position there): it always answers `None`.
    pub ids_changed_from: Option<u32>,
}

/// How [`suggest`] pairs events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestRequest {
    /// Correlation-id fields; empty = [`DEFAULT_ID_FIELDS`].
    #[serde(default)]
    pub fields: Vec<String>,
    /// The source the others are aligned to; none = the best connected one.
    #[serde(default)]
    pub reference: Option<u32>,
    /// Pairs further apart than this are ignored as implausible.
    #[serde(default = "default_max_delta")]
    pub max_delta_ms: i64,
    /// Pairs within this distance of the median agree.
    #[serde(default = "default_tolerance")]
    pub tolerance_ms: i64,
}

fn default_max_delta() -> i64 {
    DEFAULT_MAX_DELTA_MS
}

fn default_tolerance() -> i64 {
    DEFAULT_TOLERANCE_MS
}

impl Default for SuggestRequest {
    fn default() -> Self {
        SuggestRequest {
            fields: Vec::new(),
            reference: None,
            max_delta_ms: DEFAULT_MAX_DELTA_MS,
            tolerance_ms: DEFAULT_TOLERANCE_MS,
        }
    }
}

impl SuggestRequest {
    /// The id fields to read, defaults applied.
    pub fn id_fields(&self) -> Vec<String> {
        if self.fields.is_empty() {
            DEFAULT_ID_FIELDS.iter().map(|s| s.to_string()).collect()
        } else {
            self.fields.iter().filter(|f| !f.trim().is_empty()).take(16).cloned().collect()
        }
    }
}

/// A suggested offset for one source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OffsetSuggestion {
    pub source: u32,
    pub reference: u32,
    /// The offset the source currently has.
    pub current_ms: i64,
    /// The absolute offset that would align it with the reference.
    pub suggested_ms: i64,
    /// Pairs within the plausible bound.
    pub pairs: usize,
    /// Pairs within the tolerance of the median.
    pub agreeing: usize,
    /// Median absolute deviation of the pairs, a measure of their spread.
    pub spread_ms: i64,
    /// 0..1: share of agreeing pairs, scaled down for few pairs.
    pub confidence: f64,
}

/// Suggestions, and the reference they were computed against.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestResult {
    pub reference: Option<u32>,
    /// Field that provided the ids.
    pub fields: Vec<String>,
    pub suggestions: Vec<OffsetSuggestion>,
    /// Sources that shared too few ids with the reference to suggest anything.
    pub insufficient: Vec<u32>,
}
