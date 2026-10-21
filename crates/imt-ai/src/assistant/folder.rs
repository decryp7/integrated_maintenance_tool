//! refine_folder_setup: the AI half of "Set up analysis for this folder". The engine surveys the folder
//! (`imt_core::survey`); goals for the existing use cases (profile, markers, extractors, pipeline, queries)
//! are written from the survey's facts only, and every answer becomes a candidate that the engine validates
//! and previews exactly like a heuristic one. Invalid answers end up in the skipped list, never applied.

use super::proposals::{ExtractorsProposal, MarkersProposal, PipelineProposal, ProfileProposal};
use super::tools::truncate_chars;
use super::Assistant;
use crate::access::StoreRef;
use crate::error::{AiError, Result};
use crate::llm::LlmProvider;
use imt_core::api::ContextRequest;
use imt_core::pipeline::StepCfg;
use imt_core::profile::{ExtractCfg, MarkerCfg};
use imt_core::spans::{OneOrMany, SpanCfg};
use imt_core::survey::{
    self, Candidate, FolderSurvey, Fragment, LifecycleRole, ProposalGroup, ProposalOrigin, Sampled, SetupProposal, SkippedProposal,
    SurveyControl, SurveyError, SurveyRequest,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Lines on each side of an example event sent as a use case's sample.
const SAMPLE_RADIUS: usize = 30;
const MAX_FORMAT_GROUPS: usize = 2;
const MAX_QUERIES: usize = 2;
const MAX_GOAL_ITEMS: usize = 6;

/// What the AI added: validated proposals, the ones the engine rejected, and use cases that failed.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderRefinement {
    pub proposals: Vec<SetupProposal>,
    pub skipped: Vec<SkippedProposal>,
    /// One line per use case that produced nothing ("Markers: the model ...").
    pub notes: Vec<String>,
}

fn survey_error(e: SurveyError) -> AiError {
    match e {
        SurveyError::Cancelled => AiError::Cancelled,
        other => AiError::Invalid(other.to_string()),
    }
}

/// Errors that end the whole refinement (the model is gone or the user cancelled); others only skip a step.
fn fatal(e: &AiError) -> bool {
    matches!(e, AiError::Cancelled | AiError::NoModel | AiError::NoRuntime)
}

/// Distinct items, quoted, for a goal.
fn quoted(items: impl IntoIterator<Item = String>) -> String {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|s| seen.insert(s.clone()))
        .take(MAX_GOAL_ITEMS)
        .map(|s| format!("\"{}\"", truncate_chars(&s, 120)))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Deserialize)]
struct MarkerList {
    #[serde(default)]
    marker: Vec<MarkerCfg>,
}

#[derive(Deserialize)]
struct ExtractList {
    #[serde(default)]
    extract: Vec<ExtractCfg>,
}

#[derive(Deserialize)]
struct StepList {
    #[serde(default)]
    pipeline: Vec<StepCfg>,
}

/// Collects candidates, recording a use case's failure as a note (or stopping on a fatal one).
struct Collector {
    candidates: Vec<Candidate>,
    notes: Vec<String>,
}

impl Collector {
    fn step<T>(&mut self, what: &str, r: Result<T>) -> Result<Option<T>> {
        match r {
            Ok(t) => Ok(Some(t)),
            Err(e) if fatal(&e) => Err(e),
            Err(e) => {
                self.notes.push(format!("{what}: {e}"));
                Ok(None)
            }
        }
    }

    fn push(&mut self, id: String, group: ProposalGroup, title: String, summary: String, fragments: Vec<Fragment>, requires: Vec<String>) {
        self.candidates.push(Candidate {
            id,
            group,
            origin: ProposalOrigin::Ai,
            title,
            summary,
            fragments,
            requires,
            recommended: false,
        });
    }
}

impl<P: LlmProvider> Assistant<P> {
    /// AI proposals for the folder, grounded in the engine's survey and validated by the engine.
    /// `base_toml` is the active profile as the user wrote it.
    pub fn refine_folder_setup<'s>(
        &self,
        store: impl Into<StoreRef<'s>>,
        base_toml: &str,
        req: &SurveyRequest,
    ) -> Result<FolderRefinement> {
        let store: StoreRef<'_> = store.into();
        if self.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let control = SurveyControl::new();
        let (survey, sampled) = store.with(|s| survey::survey_only(s, req, &control)).map_err(survey_error)?;
        let heuristic_kinds: HashSet<String> = store.with(|s| {
            let p = s.active_profile().unwrap_or_else(|_| imt_core::Profile::default_profile());
            survey::propose(&survey, &p)
                .iter()
                .flat_map(|c| c.fragments.iter())
                .filter_map(|f| match f {
                    Fragment::Marker { marker } => Some(marker.kind.clone()),
                    _ => None,
                })
                .chain(p.marker.iter().map(|m| m.kind.clone()))
                .chain(survey.markers.iter().map(|m| m.kind.clone()))
                .collect()
        });
        let mut c = Collector {
            candidates: Vec::new(),
            notes: Vec::new(),
        };
        self.formats(store, &survey, &mut c)?;
        self.markers(store, base_toml, &survey, &sampled, &heuristic_kinds, &mut c)?;
        self.extractors(store, base_toml, &survey, &sampled, &mut c)?;
        self.noise(store, base_toml, &survey, &sampled, &mut c)?;
        self.queries(store, &survey, &mut c)?;
        if self.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let candidates = std::mem::take(&mut c.candidates);
        let (proposals, skipped) = store
            .with(|s| survey::validate(s, base_toml, &survey, &sampled, candidates, &control))
            .map_err(survey_error)?;
        Ok(FolderRefinement {
            proposals,
            skipped,
            notes: c.notes,
        })
    }

    fn formats(&self, store: StoreRef<'_>, survey: &FolderSurvey, c: &mut Collector) -> Result<()> {
        for g in survey
            .unrecognised
            .iter()
            .filter(|g| !g.sample_lines.is_empty())
            .take(MAX_FORMAT_GROUPS)
        {
            let file = g.sample_file.clone().unwrap_or_else(|| format!("sample.{}", g.extension));
            let r = self.propose_profile(store, &g.sample_lines.join("\n"), &file);
            let Some(p) = c.step(&format!("Format of .{} files", g.extension), r)? else {
                continue;
            };
            let regexes = new_formats(&p);
            if regexes.is_empty() {
                c.notes
                    .push(format!("Format of .{} files: the model proposed no new text format", g.extension));
                continue;
            }
            c.push(
                format!("format:{}:ai", g.extension),
                ProposalGroup::Formats,
                format!("Read .{} files with the AI's format", g.extension),
                if p.rationale.is_empty() {
                    format!("A text format for {} file(s) like {file}.", g.file_count)
                } else {
                    p.rationale.clone()
                },
                regexes.into_iter().map(|regex| Fragment::ParseRegex { regex }).collect(),
                vec![],
            );
        }
        Ok(())
    }

    fn markers(
        &self,
        store: StoreRef<'_>,
        base_toml: &str,
        survey: &FolderSurvey,
        sampled: &Sampled,
        taken: &HashSet<String>,
        c: &mut Collector,
    ) -> Result<()> {
        let open: Vec<_> = survey.lifecycle.iter().filter(|l| l.covered_by.is_none()).collect();
        let Some(first) = open.first() else { return Ok(()) };
        let Some(sample) = sample_around(store, sampled, &first.example) else {
            return Ok(());
        };
        let goal = format!(
            "Mark when a service starts, stops or crashes, one marker per kind, with lifecycle meaning. These messages look like lifecycle events and no marker shows them yet: {}.",
            quoted(open.iter().map(|l| format!("{:?}: {}", l.role, l.example)))
        );
        let r = self.propose_markers(store, &goal, &sample, Some(base_toml));
        let Some(p) = c.step("Markers", r)? else { return Ok(()) };
        let mut taken = taken.clone();
        let (mut starts, mut stops, mut requires) = (Vec::new(), Vec::new(), Vec::new());
        for mut m in parse_list::<MarkerList>(&p.toml).map(|l| l.marker).unwrap_or_default() {
            if taken.contains(&m.kind) {
                m.kind = format!("{}-ai", m.kind);
            }
            if !taken.insert(m.kind.clone()) {
                continue;
            }
            m.lifecycle = true;
            let id = format!("ai:marker:{}", m.kind);
            let text = format!("{} {}", m.kind, m.label.as_deref().unwrap_or_default());
            match survey::classify_lifecycle(&text) {
                Some(LifecycleRole::Start) => {
                    starts.push(m.kind.clone());
                    requires.push(id.clone());
                }
                Some(LifecycleRole::Stop) => {
                    stops.push(m.kind.clone());
                    requires.push(id.clone());
                }
                _ => {}
            }
            let label = m.label.clone().unwrap_or_else(|| m.kind.clone());
            c.push(
                id,
                ProposalGroup::Markers,
                format!("Mark {label}"),
                markers_summary(&p, &m),
                vec![Fragment::Marker { marker: m }],
                vec![],
            );
        }
        if !starts.is_empty() && !stops.is_empty() {
            c.push(
                "ai:span:ai-run".into(),
                ProposalGroup::Markers,
                "Show runs between the AI's start and stop markers".into(),
                format!("A band on each lane from {} to {}.", starts.join(", "), stops.join(", ")),
                vec![Fragment::Span {
                    span: SpanCfg {
                        kind: "ai-run".into(),
                        label: Some("Service run (AI)".into()),
                        start: OneOrMany::Many(starts),
                        stop: OneOrMany::Many(stops),
                        per: "instance".into(),
                        color: None,
                        max_duration: None,
                        restart_on_start: true,
                        enabled: true,
                    },
                }],
                requires,
            );
        }
        Ok(())
    }

    fn extractors(&self, store: StoreRef<'_>, base_toml: &str, survey: &FolderSurvey, sampled: &Sampled, c: &mut Collector) -> Result<()> {
        let Some(first) = survey.numbers.first() else { return Ok(()) };
        let Some(sample) = sample_around(store, sampled, &first.example) else {
            return Ok(());
        };
        let goal = format!(
            "Extract the numbers written in these messages as typed fields with clear names (durations in ms, sizes in MB): {}.",
            quoted(survey.numbers.iter().map(|n| n.example.clone()))
        );
        let r = self.propose_extractors(store, &goal, &sample, Some(base_toml));
        let Some(p) = c.step("Extractors", r)? else { return Ok(()) };
        for e in parse_list::<ExtractList>(&p.toml).map(|l| l.extract).unwrap_or_default() {
            c.push(
                format!("ai:extract:{}", e.name),
                ProposalGroup::Extractors,
                format!("Extract {}", e.name),
                extract_summary(&p, &e),
                vec![Fragment::Extractor { extractor: e }],
                vec![],
            );
        }
        Ok(())
    }

    fn noise(&self, store: StoreRef<'_>, base_toml: &str, survey: &FolderSurvey, sampled: &Sampled, c: &mut Collector) -> Result<()> {
        let Some(first) = survey.noise.first() else { return Ok(()) };
        let Some(sample) = sample_around(store, sampled, &first.example) else {
            return Ok(());
        };
        let goal = format!(
            "Reduce noise: drop or collapse (dedupe) these frequent low-value messages, and nothing else: {}.",
            quoted(survey.noise.iter().map(|n| n.example.clone()))
        );
        let r = self.propose_pipeline(store, &goal, &sample, Some(base_toml));
        let Some(p) = c.step("Noise reduction", r)? else { return Ok(()) };
        for (i, step) in parse_list::<StepList>(&p.toml)
            .map(|l| l.pipeline)
            .unwrap_or_default()
            .into_iter()
            .enumerate()
        {
            c.push(
                format!("ai:noise:{i}"),
                ProposalGroup::Noise,
                format!("AI pipeline step: {}", step.step),
                pipeline_summary(&p),
                vec![Fragment::Pipeline { step }],
                vec![],
            );
        }
        Ok(())
    }

    fn queries(&self, store: StoreRef<'_>, survey: &FolderSurvey, c: &mut Collector) -> Result<()> {
        for (i, t) in survey.errors.iter().take(MAX_QUERIES).enumerate() {
            let question = format!("Show the events like this error: {}", truncate_chars(&t.example, 200));
            let r = self.nl_to_query(store, &question);
            let Some(q) = c.step("Saved queries", r)? else { continue };
            c.push(
                format!("ai:query:{i}"),
                ProposalGroup::Queries,
                format!("Save the AI's query for “{}”", truncate_chars(&t.template, 60)),
                q.explanation.clone(),
                vec![Fragment::SavedQuery {
                    name: format!("AI: {}", truncate_chars(&t.template, 50)),
                    ql: q.ql,
                }],
                vec![],
            );
        }
        Ok(())
    }
}

fn parse_list<T: serde::de::DeserializeOwned>(toml_text: &str) -> Option<T> {
    toml::from_str(toml_text).ok()
}

fn markers_summary(p: &MarkersProposal, m: &MarkerCfg) -> String {
    let n = p.markers.iter().find(|r| r.kind == m.kind).map(|r| r.matches).unwrap_or(0);
    let why = m.description.clone().unwrap_or_else(|| p.rationale.clone());
    format!("{why} ({n} of {} events in the model's sample)", p.sampled)
}

fn extract_summary(p: &ExtractorsProposal, e: &ExtractCfg) -> String {
    let n = p.extractors.iter().find(|r| r.name == e.name).map(|r| r.matches).unwrap_or(0);
    format!("{} ({n} of {} events in the model's sample)", p.rationale, p.sampled)
}

fn pipeline_summary(p: &PipelineProposal) -> String {
    format!(
        "{} (on the model's sample: {} kept, {} dropped, {} changed)",
        p.rationale, p.kept, p.dropped, p.changed
    )
}

/// Text formats of a proposed profile that the active one does not have, as `[parse] regexes`.
fn new_formats(p: &ProfileProposal) -> Vec<String> {
    let Ok(proposed) = imt_core::Profile::from_toml(&p.toml) else {
        return vec![];
    };
    let defaults = imt_core::Profile::default_profile();
    let known: HashSet<&String> = defaults.parse.regexes.iter().chain(&defaults.parse.output_templates).collect();
    proposed
        .parse
        .regexes
        .iter()
        .filter(|r| !known.contains(r))
        .cloned()
        .chain(
            proposed
                .parse
                .output_templates
                .iter()
                .filter(|t| !known.contains(t))
                .map(|t| imt_core::template::output_template_to_regex(t)),
        )
        .collect()
}

/// The original lines around the sampled event with this message (the use cases parse raw text).
fn sample_around(store: StoreRef<'_>, sampled: &Sampled, message: &str) -> Option<String> {
    let e = sampled
        .events
        .iter()
        .find(|e| e.rec.message.starts_with(message.trim_end_matches('…')))?;
    let req = ContextRequest {
        record_id: e.rec.id,
        before: SAMPLE_RADIUS,
        after: SAMPLE_RADIUS,
    };
    let ctx = store.with(|s| s.source_context(&req)).ok()?;
    let text: Vec<&str> = ctx.lines.iter().map(|l| l.text.as_str()).collect();
    (!text.is_empty()).then(|| text.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assistant::AssistantConfig;
    use crate::llm::ScriptedProvider;
    use imt_core::ports::Store;
    use imt_core::workspace::WorkspaceStore;
    use imt_core::{Profile, Workspace, DEFAULT_PROFILE_TOML};
    use std::path::PathBuf;

    fn store() -> WorkspaceStore {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples");
        let mut ws = Workspace::new(Profile::default_profile()).unwrap();
        ws.open(&[root.join("serilog-text"), root.join("kubernetes"), root.join("log4net")])
            .unwrap();
        WorkspaceStore::new(ws)
    }

    const MARKERS: &str = r##"{"markers":[{"kind":"pod-crash","label":"Pod crash","color":"#FF4FA3","when":[{"field":"@message","op":"contains","value":"panic:"}]}],"rationale":"The consumer panics."}"##;
    const EXTRACT: &str = r#"{"extractors":[{"name":"outbox-dispatched","from":"@message","regex":"Dispatched (?P<OutboxDispatched>\\d+) outbox","when":[],"fields":{}}],"rationale":"Messages dispatched per batch."}"#;
    const QUERY_OK: &str = r#"{"ql":"level >= error","explanation":"All errors."}"#;
    const QUERY_EMPTY: &str = r#"{"ql":"@message contains \"zzzz never logged\"","explanation":"Nothing."}"#;

    fn assistant(replies: &[&str]) -> Assistant<ScriptedProvider> {
        Assistant::new(ScriptedProvider::text(replies), AssistantConfig::default())
    }

    #[test]
    fn ai_proposals_are_grounded_in_the_survey_and_validated_by_the_engine() {
        let ws = store();
        let a = assistant(&[MARKERS, EXTRACT, QUERY_OK, QUERY_EMPTY]);
        let r = a
            .refine_folder_setup(&ws as &dyn Store, DEFAULT_PROFILE_TOML, &SurveyRequest::default())
            .unwrap();
        let ids: Vec<&str> = r.proposals.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"ai:marker:pod-crash"), "{ids:?} {:?} {:?}", r.skipped, r.notes);
        assert!(
            ids.contains(&"ai:extract:outbox-dispatched"),
            "{ids:?} {:?} {:?}",
            r.skipped,
            r.notes
        );
        assert!(ids.contains(&"ai:query:0"), "{ids:?}");
        assert!(r.proposals.iter().all(|p| p.origin == ProposalOrigin::Ai && !p.recommended));
        // The engine rejects what does not work on the sample, whatever the model said.
        let empty = r.skipped.iter().find(|s| s.id == "ai:query:1").expect("the empty query is skipped");
        assert!(empty.reason.contains("finds no events"), "{}", empty.reason);
        // The goals quote the survey's facts: the uncovered crash and the embedded numbers.
        let sent = a.provider().requests();
        assert!(sent[0].messages[0].content.contains("panic:"), "{}", sent[0].messages[0].content);
        assert!(sent[1].messages[0].content.contains("Dispatched"));
    }

    #[test]
    fn a_failing_use_case_becomes_a_note_and_cancellation_stops() {
        let ws = store();
        let a = assistant(&["not json", "still not json", EXTRACT, QUERY_OK, QUERY_OK]);
        let r = a
            .refine_folder_setup(&ws as &dyn Store, DEFAULT_PROFILE_TOML, &SurveyRequest::default())
            .unwrap();
        assert!(r.notes.iter().any(|n| n.starts_with("Markers:")), "{:?}", r.notes);
        assert!(r.proposals.iter().any(|p| p.id == "ai:extract:outbox-dispatched"));
        let cancelled = assistant(&[]);
        cancelled.cancel();
        assert!(matches!(
            cancelled.refine_folder_setup(&ws as &dyn Store, DEFAULT_PROFILE_TOML, &SurveyRequest::default()),
            Err(AiError::Cancelled)
        ));
        let empty = WorkspaceStore::new(Workspace::new(Profile::default_profile()).unwrap());
        assert!(matches!(
            assistant(&[]).refine_folder_setup(&empty as &dyn Store, DEFAULT_PROFILE_TOML, &SurveyRequest::default()),
            Err(AiError::Invalid(_))
        ));
    }

    #[test]
    fn a_missing_model_ends_the_refinement() {
        let ws = store();
        let a = Assistant::new(
            ScriptedProvider::new(vec![crate::llm::Reply::Fail(AiError::NoModel)]),
            AssistantConfig::default(),
        );
        assert!(matches!(
            a.refine_folder_setup(&ws as &dyn Store, DEFAULT_PROFILE_TOML, &SurveyRequest::default()),
            Err(AiError::NoModel)
        ));
    }
}
