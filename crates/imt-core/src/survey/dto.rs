//! Wire types of the folder survey and its proposals (mirrored in `app/src/app/ai/folder-setup/folder-setup.types.ts`).

use crate::condition::ConditionCfg;
use crate::model::Level;
use crate::pipeline::StepCfg;
use crate::profile::{ChartCfg, ExtractCfg, MarkerCfg};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What the UI asks for. Every limit is optional and clamped by the engine (see [`super::SurveyLimits`]).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurveyRequest {
    #[serde(default)]
    pub max_events_per_group: Option<usize>,
    #[serde(default)]
    pub max_events: Option<usize>,
    #[serde(default)]
    pub max_millis: Option<u64>,
}

/// Facts about the opened files: the only input of the heuristics and of the AI refinement.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSurvey {
    pub total_events: usize,
    pub sampled_events: usize,
    /// A limit (events, groups or time) cut the sample short.
    pub truncated: bool,
    pub elapsed_ms: u64,
    pub groups: Vec<SourceGroup>,
    pub unrecognised: Vec<UnrecognisedGroup>,
    pub fields: Vec<FieldSummary>,
    pub instance_candidates: Vec<InstanceCandidate>,
    pub lifecycle: Vec<LifecycleCandidate>,
    pub errors: Vec<TemplateSummary>,
    pub noise: Vec<NoiseCandidate>,
    pub numbers: Vec<NumberCandidate>,
    pub markers: Vec<MarkerCoverage>,
}

/// Files read by one importer that belong to one logical log (rolled files share it).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceGroup {
    pub id: String,
    pub importer: String,
    pub name: String,
    pub files: usize,
    pub events: usize,
    pub sampled: usize,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

/// Files no importer recognised, grouped by extension, with the first lines of one of them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnrecognisedGroup {
    pub extension: String,
    pub file_count: usize,
    pub files: Vec<String>,
    pub reason: String,
    pub sample_file: Option<String>,
    pub sample_lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Number,
    Bool,
    Text,
    Mixed,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueCount {
    pub value: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSummary {
    pub name: String,
    pub kind: FieldKind,
    pub present: usize,
    /// Share of the sampled events that carry the field (0..1).
    pub coverage: f64,
    pub distinct: usize,
    /// More distinct values than were counted.
    pub distinct_capped: bool,
    pub top: Vec<ValueCount>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// Where an instance key's values come from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum KeyOrigin {
    /// An existing field.
    Field,
    /// Text in the message; an extractor with this regex produces the field.
    Message { regex: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceCandidate {
    pub field: String,
    pub origin: KeyOrigin,
    pub lanes: usize,
    pub coverage: f64,
    pub score: f64,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LifecycleRole {
    Start,
    Stop,
    Crash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TemplateKind {
    /// The events carry this message template (Serilog).
    Serilog,
    /// Mined from the messages (Drain).
    Mined,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleCandidate {
    pub role: LifecycleRole,
    pub template: String,
    pub template_kind: TemplateKind,
    pub count: usize,
    pub example: String,
    /// The existing marker kind most of these events already carry.
    pub covered_by: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSummary {
    pub template: String,
    pub template_kind: TemplateKind,
    pub count: usize,
    pub level: Level,
    pub share: f64,
    pub example: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NoiseReason {
    /// The template names a known low-value activity (health check, heartbeat, ...).
    Keyword { word: String },
    /// The template repeats at a near-constant interval.
    Periodic { interval_ms: i64 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoiseCandidate {
    pub template: String,
    pub template_kind: TemplateKind,
    pub count: usize,
    pub share: f64,
    pub level: Level,
    pub reason: NoiseReason,
    pub example: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NumberKind {
    Number,
    Duration,
    Bytes,
}

/// A number written inside messages, e.g. `took 23 ms` or `CPU 23.3%`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberCandidate {
    /// Proposed field name.
    pub name: String,
    /// The text before the number, as written.
    pub label: String,
    pub unit: Option<String>,
    pub number_kind: NumberKind,
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub example: String,
    /// Extractor regex with one named group (`name`).
    pub regex: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkerCoverage {
    pub kind: String,
    pub label: String,
    pub lifecycle: bool,
    pub matches: usize,
}

// ---- proposals ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProposalGroup {
    Formats,
    Instances,
    Markers,
    Extractors,
    Noise,
    Charts,
    Queries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProposalOrigin {
    Heuristic,
    Ai,
}

/// One typed change to the profile (or, for saved queries, to the project).
// A handful per proposal, sent over IPC: boxing the larger profile sections would not pay off.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Fragment {
    /// A text format added to `[parse] regexes`.
    ParseRegex {
        regex: String,
    },
    /// `[instance] key`.
    InstanceKey {
        key: Vec<String>,
    },
    Extractor {
        extractor: ExtractCfg,
    },
    Marker {
        marker: MarkerCfg,
    },
    Span {
        span: crate::spans::SpanCfg,
    },
    Pipeline {
        step: StepCfg,
    },
    Chart {
        chart: ChartCfg,
    },
    Dashboard {
        dashboard: crate::dashboard::DashboardCfg,
    },
    /// A named query-language filter, saved with the project (not in the profile).
    SavedQuery {
        name: String,
        ql: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanePreview {
    pub name: String,
    pub count: usize,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeExample {
    pub before: String,
    pub after: String,
}

/// What a proposal does to the sampled events, computed by the engine.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ProposalPreview {
    Parse {
        parsed: usize,
        lines: usize,
        with_timestamp: usize,
        examples: Vec<String>,
    },
    Lanes {
        lanes: Vec<LanePreview>,
        total_lanes: usize,
        unassigned: usize,
    },
    Matches {
        matches: usize,
        sampled: usize,
        examples: Vec<String>,
    },
    Extracted {
        matches: usize,
        sampled: usize,
        examples: Vec<Map<String, Value>>,
    },
    Runs {
        runs: usize,
        closed: usize,
        open: usize,
        no_stop: usize,
        lanes: usize,
    },
    Pipeline {
        sampled: usize,
        kept: usize,
        dropped: usize,
        changed: usize,
        examples: Vec<ChangeExample>,
    },
    Chart {
        series: Vec<String>,
        points: usize,
    },
    Dashboard {
        tiles: usize,
    },
    Query {
        matches: usize,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupProposal {
    pub id: String,
    pub group: ProposalGroup,
    pub origin: ProposalOrigin,
    pub title: String,
    pub summary: String,
    pub fragments: Vec<Fragment>,
    /// Ids of proposals this one needs (selected together).
    pub requires: Vec<String>,
    /// The fragments as profile TOML, for review.
    pub toml: String,
    pub preview: ProposalPreview,
    /// Selected by default in the wizard.
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedProposal {
    pub id: String,
    pub group: ProposalGroup,
    pub origin: ProposalOrigin,
    pub title: String,
    pub reason: String,
}

/// The survey with its validated proposals and the ones that were dropped.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSetup {
    pub survey: FolderSurvey,
    pub proposals: Vec<SetupProposal>,
    pub skipped: Vec<SkippedProposal>,
}

/// A proposal before validation.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub id: String,
    pub group: ProposalGroup,
    pub origin: ProposalOrigin,
    pub title: String,
    pub summary: String,
    pub fragments: Vec<Fragment>,
    pub requires: Vec<String>,
    pub recommended: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeRequest {
    pub fragments: Vec<Fragment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffOp {
    Same,
    Add,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub op: DiffOp,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueryFragment {
    pub name: String,
    pub ql: String,
}

/// The profile that applying the selected fragments gives, validated, with its diff.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposedProfile {
    pub toml: String,
    pub diff: Vec<DiffLine>,
    pub added: usize,
    pub removed: usize,
    pub saved_queries: Vec<SavedQueryFragment>,
}

/// A condition on a template: `@template eq` for Serilog templates, else a regex on the message.
pub fn template_condition(template: &str, kind: TemplateKind) -> ConditionCfg {
    match kind {
        TemplateKind::Serilog => ConditionCfg {
            field: "@template".into(),
            op: "eq".into(),
            value: Some(crate::condition::CondValue::Text(template.into())),
        },
        TemplateKind::Mined => ConditionCfg {
            field: "@message".into(),
            op: "matches".into(),
            value: Some(crate::condition::CondValue::Text(super::templates::template_regex(template))),
        },
    }
}
