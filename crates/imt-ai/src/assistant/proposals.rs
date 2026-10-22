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
    Parser(ParserProposal),
    Pipeline(PipelineProposal),
    Markers(MarkersProposal),
    Extractors(ExtractorsProposal),
    Query(QueryProposal),
}

impl Proposal {
    pub fn title(&self) -> &'static str {
        match self {
            Proposal::Regex(_) => "Regular expression",
            Proposal::Parser(_) => "Parser",
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
pub struct ParserProposal {
    pub name: String,
    /// The candidate parser (TOML, ADR 0026) for the library; empty when the library already reads the file.
    pub toml: String,
    pub importer: Option<String>,
    pub parsed: usize,
    pub with_timestamp: usize,
    pub preview: Vec<ParsedPreview>,
    pub rationale: String,
    /// Whether the engine's verdict reached the target (ADR 0032), only came close, or the library reads the file.
    pub status: ParserStatus,
    /// The engine's verdict on the final parser (on the larger sample of the file).
    pub verdict: Option<Box<imt_core::parser::verify::ParserVerdict>>,
    /// Each round of drafting and verification.
    pub rounds: Vec<ParserRound>,
    /// Profile `[[chart]]` sections for the accepted metrics (per instance); empty without metrics.
    pub charts_toml: String,
}

/// How a parser generation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ParserStatus {
    /// The verdict reached the target share of lines with an accepted timestamp rule.
    Done,
    /// The best verified parser after the rounds, below the target: the user edits it.
    BestEffort,
    /// The library already reads the file; nothing to add.
    Builtin,
}

/// One round of a parser generation.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserRound {
    pub round: usize,
    /// The verdict's score (0..1); 0 when the answer could not be used.
    pub score: f64,
    pub timestamps_accepted: bool,
    /// Failing lines reported to the model.
    pub failures: usize,
    /// What happened ("the engine's regex was used", "the answer was not JSON", ...).
    pub note: String,
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
