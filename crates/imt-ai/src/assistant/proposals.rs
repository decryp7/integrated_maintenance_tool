//! Typed proposals. Each one has already been checked by the engine (compiled, previewed against the
//! user's sample) before it is returned; the UI shows it with an Apply button. Nothing is applied here.

use imt_core::api::ParsedPreview;
use imt_core::regex_assist::{ExplainNode, InferredGroup};
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Proposal {
    Regex(RegexProposal),
    Profile(ProfileProposal),
    Pipeline(PipelineProposal),
    Markers(MarkersProposal),
    Extractors(ExtractorsProposal),
    Query(QueryProposal),
}

impl Proposal {
    pub fn title(&self) -> &'static str {
        match self {
            Proposal::Regex(_) => "Regular expression",
            Proposal::Profile(_) => "Parsing profile",
            Proposal::Pipeline(_) => "Pipeline steps",
            Proposal::Markers(_) => "Markers",
            Proposal::Extractors(_) => "Extractors",
            Proposal::Query(_) => "Query",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RegexSource {
    /// The model's version (renamed groups, generalised pattern) passed validation.
    Model,
    /// The engine's own inference.
    Inferred,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegexProposal {
    pub pattern: String,
    pub source: RegexSource,
    pub groups: Vec<InferredGroup>,
    /// Whether the pattern matches each example line.
    pub matches: Vec<bool>,
    pub match_count: usize,
    pub total: usize,
    pub explanation: Vec<ExplainNode>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileProposal {
    pub name: String,
    /// The complete candidate profile (TOML), ready to open in the profile editor.
    pub toml: String,
    pub importer: Option<String>,
    pub parsed: usize,
    pub with_timestamp: usize,
    pub preview: Vec<ParsedPreview>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepReport {
    pub index: usize,
    pub step: String,
    pub input: usize,
    pub output: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeExample {
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineProposal {
    /// Just the proposed `[[pipeline]]` steps.
    pub toml: String,
    /// The current profile with the steps added.
    pub profile_toml: String,
    pub steps: Vec<StepReport>,
    pub sample_total: usize,
    pub kept: usize,
    pub dropped: usize,
    pub changed: usize,
    pub examples: Vec<ChangeExample>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkerReport {
    pub kind: String,
    pub label: String,
    pub matches: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkersProposal {
    pub toml: String,
    pub profile_toml: String,
    pub markers: Vec<MarkerReport>,
    pub sampled: usize,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractorReport {
    pub name: String,
    pub fields: Vec<String>,
    pub matches: usize,
    pub examples: Vec<Map<String, Value>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractorsProposal {
    pub toml: String,
    pub profile_toml: String,
    pub extractors: Vec<ExtractorReport>,
    pub sampled: usize,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryProposal {
    pub ql: String,
    pub explanation: String,
    /// Events (or table rows) the query returns right now.
    pub estimated_matches: Option<usize>,
    /// The first answer was invalid and had to be repaired.
    pub repaired: bool,
}
