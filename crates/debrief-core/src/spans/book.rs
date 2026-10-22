//! Runs over any event stream: one pairer per enabled rule, fed timed events in time order, and the
//! Runs table built from their annotations. Pure: the in-memory store and the database store both use it.

use super::draft::Classifier;
use super::{summarize, to_annotation, Pairer, Per, RunSort, RunStatus, RunStyle, RunsReply, RunsRequest, SpanRule, MAX_RUNS_PAGE};
use crate::model::Record;
use crate::plugin::annotate::AnnotationDto;
use crate::profile::CompiledProfile;

/// Enabled rules of a profile with their classifiers.
pub fn enabled_rules(profile: &CompiledProfile) -> Vec<(&SpanRule, Classifier)> {
    profile
        .spans
        .iter()
        .filter(|r| r.enabled)
        .map(|r| (r, Classifier::of_rule(r)))
        .collect()
}

/// A pairer per rule.
pub fn pairers(rules: &[(&SpanRule, Classifier)]) -> Vec<Pairer> {
    rules.iter().map(|(r, _)| Pairer::new(r.restart_on_start)).collect()
}

/// Feeds one timed event (`source`: its source name, `lane`: its instance key) to every rule's pairer.
pub fn feed_one(r: &Record, source: &str, lane: &str, rules: &[(&SpanRule, Classifier)], pairers: &mut [Pairer]) {
    let Some(ts) = r.ts else { return };
    for ((rule, classify), pairer) in rules.iter().zip(pairers.iter_mut()) {
        let value = match &rule.per {
            Per::Instance => None,
            Per::Field(f) => r.field_text(f, source).map(|v| v.into_owned()),
        };
        let edge = classify.edge(r);
        pairer.feed(lane, value.as_deref(), edge, r.id, ts, || r.message.clone());
    }
}

/// The annotations of every rule's runs.
pub fn annotations(rules: &[(&SpanRule, Classifier)], pairers: &[Pairer], group_by: &[String]) -> Vec<Vec<AnnotationDto>> {
    rules
        .iter()
        .zip(pairers)
        .map(|((rule, _), p)| {
            let style = RunStyle::of(rule);
            p.runs().iter().map(|run| to_annotation(&style, run, group_by)).collect()
        })
        .collect()
}

pub(crate) fn overlaps(a: &AnnotationDto, from: Option<i64>, to: Option<i64>) -> bool {
    from.is_none_or(|f| a.to >= f) && to.is_none_or(|t| a.from <= t)
}

/// The Runs table: a sorted page of runs and every span kind's summary over the selection.
pub fn runs_reply(
    profile: &CompiledProfile,
    rules: &[(&SpanRule, Classifier)],
    pairers: &[Pairer],
    anns: &[Vec<AnnotationDto>],
    req: &RunsRequest,
) -> RunsReply {
    let instances: std::collections::HashSet<&String> = req.instances.iter().take(super::dto::MAX_REQUEST_INSTANCES).collect();
    let selected = |a: &AnnotationDto| {
        overlaps(a, req.from, req.to) && (instances.is_empty() || a.instance.as_ref().is_some_and(|i| instances.contains(i)))
    };
    let mut summaries = Vec::new();
    let mut rows: Vec<&AnnotationDto> = Vec::new();
    for (((rule, _), pairer), list) in rules.iter().zip(pairers).zip(anns) {
        let chosen: Vec<&AnnotationDto> = list.iter().filter(|a| selected(a)).collect();
        summaries.push(summarize(rule, chosen.iter().copied(), pairer.dropped_runs + pairer.ignored_events));
        if req.kind.as_ref().is_none_or(|k| *k == rule.kind) {
            rows.extend(
                chosen
                    .into_iter()
                    .filter(|a| req.statuses.is_empty() || a.run.as_ref().is_some_and(|r| req.statuses.contains(&r.status))),
            );
        }
    }
    // Switched-off rules are listed (so the table can say so) without runs.
    for rule in profile.spans.iter().filter(|r| !r.enabled) {
        summaries.push(summarize(rule, std::iter::empty(), 0));
    }
    sort_runs(&mut rows, req.sort, req.descending);
    let total = rows.len();
    let limit = req.limit.unwrap_or(200).clamp(1, MAX_RUNS_PAGE);
    RunsReply {
        rules: profile.profile.span.clone(),
        summaries,
        runs: rows.into_iter().skip(req.offset).take(limit).cloned().collect(),
        total,
        offset: req.offset.min(total),
    }
}

fn status_rank(a: &AnnotationDto) -> (u8, bool) {
    a.run.as_ref().map_or((0, false), |r| {
        let rank = match r.status {
            RunStatus::NoStart => 0,
            RunStatus::NoStop => 1,
            RunStatus::Open => 2,
            RunStatus::Closed => 3,
        };
        (rank, !r.too_long)
    })
}

fn sort_runs(rows: &mut [&AnnotationDto], sort: RunSort, descending: bool) {
    let duration = |a: &AnnotationDto| a.run.as_ref().map_or(0, |r| r.duration_ms);
    let stop = |a: &AnnotationDto| a.run.as_ref().and_then(|r| r.stop.as_ref().map(|s| s.ts)).unwrap_or(i64::MAX);
    rows.sort_by(|a, b| {
        let primary = match sort {
            RunSort::Start => a.from.cmp(&b.from),
            RunSort::Stop => stop(a).cmp(&stop(b)),
            RunSort::Duration => duration(a).cmp(&duration(b)),
            RunSort::Status => status_rank(a).cmp(&status_rank(b)),
            RunSort::Lane => a.instance.cmp(&b.instance),
        };
        let primary = if descending { primary.reverse() } else { primary };
        primary.then_with(|| a.from.cmp(&b.from)).then_with(|| a.kind.cmp(&b.kind))
    });
}
