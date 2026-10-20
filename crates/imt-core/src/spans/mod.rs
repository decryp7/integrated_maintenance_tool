//! Runs between lifecycle markers: `[[span]]` rules pair "XXXXX started" …
//! "XXXXX stopped" events per timeline lane (or per field value within a lane,
//! e.g. a process id in a log shared by many instances) into runs, shown as
//! bands on the timeline and listed in the Runs table.
//!
//! - [`config`]: the `[[span]]` profile section, validated with profile lines;
//! - [`pairing`]: the pure, incremental start/stop pairing;
//! - [`draft`]: the guided editor (a candidate rule, its preview and the
//!   profile changes saving it needs);
//! - [`dto`]: wire types.
//!
//! Runs are annotations (owner `profile`, annotator [`ANNOTATOR`]) carrying a
//! [`RunInfo`]; the workspace computes them lazily and feeds live-tail appends
//! incrementally (`crate::workspace` `spans`).

pub mod book;
pub mod config;
pub mod draft;
pub mod dto;
pub mod pairing;

pub use config::{OneOrMany, Per, SpanCfg, SpanField, SpanIssue, SpanRule, MAX_SPAN_RULES};
pub use dto::{
    RunInfo, RunSort, RunsReply, RunsRequest, SpanDraft, SpanPlan, SpanPreview, SpanPreviewRequest, SpanSummary, Trigger, MAX_RUNS_PAGE,
};
pub use pairing::{Edge, Pairer, Run, RunEvent, RunStatus, MAX_GROUPS, MAX_RUNS_PER_LANE};

use crate::api::Severity;
use crate::plugin::annotate::{AnnotationDto, FieldMatch};
use crate::plugin::markers::OWNER_PROFILE;
use crate::query::{FieldFilter, Query};

/// Annotator id of runs.
pub const ANNOTATOR: &str = "profile.spans";

/// What a rule contributes to how its runs are reported.
#[derive(Debug, Clone)]
pub struct RunStyle<'a> {
    pub kind: &'a str,
    pub label: &'a str,
    pub color: Option<&'a str>,
    pub max_duration_ms: Option<i64>,
    /// The field runs are paired by, if any.
    pub field: Option<&'a str>,
}

impl<'a> RunStyle<'a> {
    pub fn of(rule: &'a SpanRule) -> RunStyle<'a> {
        RunStyle {
            kind: &rule.kind,
            label: &rule.label,
            color: rule.color.as_deref(),
            max_duration_ms: rule.max_duration_ms,
            field: match &rule.per {
                Per::Instance => None,
                Per::Field(f) => Some(f),
            },
        }
    }
}

/// A run as an annotation on its lane; `group_by` are the fields of the lane keys (for the evidence query).
pub fn to_annotation(style: &RunStyle, run: &Run, group_by: &[String]) -> AnnotationDto {
    let duration_ms = (run.to - run.from).max(0);
    let too_long = run.status != RunStatus::NoStart && style.max_duration_ms.is_some_and(|max| duration_ms > max);
    let label = match run.status {
        RunStatus::Closed => style.label.to_string(),
        RunStatus::Open => format!("{} (still running)", style.label),
        RunStatus::NoStop => format!("{} (no stop seen)", style.label),
        RunStatus::NoStart => format!("{}: stop without start", style.label),
    };
    let severity = match (run.status, too_long) {
        (RunStatus::NoStop | RunStatus::NoStart, _) | (_, true) => Severity::Warning,
        _ => Severity::Info,
    };
    let group = style.field.zip(run.group.as_ref()).map(|(field, value)| FieldMatch {
        field: field.to_string(),
        value: value.clone(),
    });
    let evidence_query = Query {
        from: Some(run.from),
        to: Some(run.to.max(run.from)),
        group_by: group_by.to_vec(),
        groups: vec![run.lane.clone()],
        filters: group
            .iter()
            .map(|g| FieldFilter {
                field: g.field.clone(),
                op: "eq".into(),
                value: Some(g.value.clone()),
            })
            .collect(),
        ..Default::default()
    };
    AnnotationDto {
        annotator: ANNOTATOR.into(),
        owner: OWNER_PROFILE.into(),
        kind: style.kind.to_string(),
        from: run.from,
        to: run.to.max(run.from),
        instance: Some(run.lane.clone()),
        on: None,
        label,
        severity,
        evidence_query,
        run: Some(RunInfo {
            span: style.kind.to_string(),
            span_label: style.label.to_string(),
            status: run.status,
            start: run.start.clone(),
            stop: run.stop.clone(),
            duration_ms: if run.status == RunStatus::NoStart { 0 } else { duration_ms },
            too_long,
            group,
            color: style.color.map(str::to_string),
        }),
    }
}

/// Totals of a rule's annotations (each with `run`), lanes counted once.
pub fn summarize<'a>(rule: &SpanRule, anns: impl IntoIterator<Item = &'a AnnotationDto>, dropped: usize) -> SpanSummary {
    let mut s = SpanSummary {
        kind: rule.kind.clone(),
        label: rule.label.clone(),
        color: rule.color.clone(),
        enabled: rule.enabled,
        dropped,
        missing_markers: rule.missing.clone(),
        ..Default::default()
    };
    let mut lanes = std::collections::HashSet::new();
    for a in anns {
        if let Some(info) = &a.run {
            s.add(info, lanes.insert(a.instance.clone()));
        }
    }
    s.finish();
    s
}

#[cfg(test)]
mod tests;
