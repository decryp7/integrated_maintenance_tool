//! Runs between lifecycle markers over the workspace (see [`crate::spans`]):
//! computed lazily on the first query, then fed only the records appended by
//! live tail; anything else that changes records (re-sorts, rewrites,
//! evictions, renames, a new profile or a regrouping) recomputes them.

use super::Workspace;
use crate::model::Record;
use crate::plugin::annotate::{AnnotationDto, AnnotationsRequest};
use crate::spans::draft::{self, Classifier};
use crate::spans::{
    summarize, to_annotation, Pairer, Per, RunSort, RunStatus, RunStyle, RunsReply, RunsRequest, SpanPreview, SpanPreviewRequest, SpanRule,
    SpanSummary, MAX_RUNS_PAGE,
};

/// The pairing state of every rule, and what it was computed from.
#[derive(Default)]
pub(super) struct SpanBook {
    /// (record generation, lane fields, profile) the state belongs to.
    key: Option<(u64, Vec<String>, usize)>,
    /// One pairer per rule of the profile, in order.
    pairers: Vec<Pairer>,
    /// Records fed so far (a prefix of the timed records).
    fed: usize,
    /// The annotations of every rule, rebuilt after each feed.
    annotations: Vec<Vec<AnnotationDto>>,
}

/// Feeds `records` (timed, in time order) to one pairer per rule.
fn feed<'s>(
    records: &[Record],
    rules: &[(&SpanRule, Classifier)],
    pairers: &mut [Pairer],
    lanes: &dyn Fn(&Record) -> String,
    source: &dyn Fn(u32) -> &'s str,
) {
    if rules.is_empty() {
        return;
    }
    for r in records {
        let Some(ts) = r.ts else { continue };
        let lane = lanes(r);
        for ((rule, classify), pairer) in rules.iter().zip(pairers.iter_mut()) {
            let value = match &rule.per {
                Per::Instance => None,
                Per::Field(f) => r.field_text(f, source(r.source)).map(|v| v.into_owned()),
            };
            let edge = classify.edge(r);
            pairer.feed(&lane, value.as_deref(), edge, r.id, ts, || r.message.clone());
        }
    }
}

impl Workspace {
    fn lane_of(&self, r: &Record) -> String {
        crate::query::group_key(&self.default_group_by, r, self.source_name(r.source))
    }

    /// Enabled rules with their classifiers.
    fn span_rules(&self) -> Vec<(&SpanRule, Classifier)> {
        self.profile
            .spans
            .iter()
            .filter(|r| r.enabled)
            .map(|r| (r, Classifier::of_rule(r)))
            .collect()
    }

    /// Brings the runs up to date and calls `f` with the annotations of each enabled rule.
    fn with_runs<R>(&self, f: impl FnOnce(&[(&SpanRule, Classifier)], &[Pairer], &[Vec<AnnotationDto>]) -> R) -> R {
        let rules = self.span_rules();
        let mut book = self.spans.lock().unwrap_or_else(|p| p.into_inner());
        let key = (
            self.generation,
            self.default_group_by.clone(),
            std::sync::Arc::as_ptr(&self.profile) as usize,
        );
        let timed = self.records.partition_point(|r| r.ts.is_some());
        if book.key.as_ref() != Some(&key) || book.fed > timed {
            *book = SpanBook {
                key: Some(key),
                pairers: rules.iter().map(|(r, _)| Pairer::new(r.restart_on_start)).collect(),
                fed: 0,
                annotations: vec![Vec::new(); rules.len()],
            };
        }
        if book.fed < timed {
            let lanes = |r: &Record| self.lane_of(r);
            let source = |id: u32| self.source_name(id);
            let from = book.fed;
            feed(&self.records[from..timed], &rules, &mut book.pairers, &lanes, &source);
            book.fed = timed;
            book.annotations = rules
                .iter()
                .zip(&book.pairers)
                .map(|((rule, _), p)| {
                    let style = RunStyle::of(rule);
                    p.runs()
                        .iter()
                        .map(|run| to_annotation(&style, run, &self.default_group_by))
                        .collect()
                })
                .collect();
        }
        f(&rules, &book.pairers, &book.annotations)
    }

    /// Runs overlapping the request (each an annotation with `run`).
    pub(super) fn run_annotations(&self, req: &AnnotationsRequest) -> Vec<AnnotationDto> {
        self.with_runs(|_, _, anns| {
            anns.iter()
                .flatten()
                .filter(|a| overlaps(a, req.from, req.to))
                .filter(|a| req.instance.as_ref().is_none_or(|i| a.instance.as_ref() == Some(i)))
                .cloned()
                .collect()
        })
    }

    /// The Runs table: a sorted page of runs and every span kind's summary over the selection.
    pub fn runs(&self, req: &RunsRequest) -> RunsReply {
        let instances: std::collections::HashSet<&String> = req.instances.iter().take(crate::spans::dto::MAX_REQUEST_INSTANCES).collect();
        let selected = |a: &AnnotationDto| {
            overlaps(a, req.from, req.to) && (instances.is_empty() || a.instance.as_ref().is_some_and(|i| instances.contains(i)))
        };
        self.with_runs(|rules, pairers, anns| {
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
            for rule in self.profile.spans.iter().filter(|r| !r.enabled) {
                summaries.push(summarize(rule, std::iter::empty(), 0));
            }
            sort_runs(&mut rows, req.sort, req.descending);
            let total = rows.len();
            let limit = req.limit.unwrap_or(200).clamp(1, MAX_RUNS_PAGE);
            RunsReply {
                rules: self.profile.profile.span.clone(),
                summaries,
                runs: rows.into_iter().skip(req.offset).take(limit).cloned().collect(),
                total,
                offset: req.offset.min(total),
            }
        })
    }

    /// The guided editor's preview: the draft checked against the profile and paired over the loaded records.
    pub fn preview_spans(&self, req: &SpanPreviewRequest) -> SpanPreview {
        let (errors, warnings, checked) = draft::check(&req.draft, &self.profile.profile, &self.profile.markers);
        let Some(checked) = checked else {
            return SpanPreview {
                errors,
                warnings,
                ..Default::default()
            };
        };
        let timed = self.records.partition_point(|r| r.ts.is_some());
        let mut pairers = vec![Pairer::new(checked.rule.restart_on_start)];
        let lanes = |r: &Record| self.lane_of(r);
        let source = |id: u32| self.source_name(id);
        feed(
            &self.records[..timed],
            &[(&checked.rule, checked.classifier.clone())],
            &mut pairers,
            &lanes,
            &source,
        );
        let style = RunStyle::of(&checked.rule);
        let mut all: Vec<AnnotationDto> = pairers[0]
            .runs()
            .iter()
            .map(|run| to_annotation(&style, run, &self.default_group_by))
            .collect();
        all.sort_by_key(|a| a.from);
        let summary = summarize(&checked.rule, &all, pairers[0].dropped_runs + pairers[0].ignored_events);
        let anomalies = all
            .iter()
            .filter(|a| {
                a.run
                    .as_ref()
                    .is_some_and(|r| r.too_long || matches!(r.status, RunStatus::NoStop | RunStatus::NoStart))
            })
            .take(crate::spans::dto::PREVIEW_RUNS)
            .cloned()
            .collect();
        all.truncate(crate::spans::dto::PREVIEW_RUNS);
        SpanPreview {
            errors,
            warnings,
            summary: Some(summary),
            runs: all,
            anomalies,
            plan: Some(checked.plan),
        }
    }

    /// Summaries of every span kind over all runs (enabled rules first).
    pub fn span_summaries(&self) -> Vec<SpanSummary> {
        self.runs(&RunsRequest {
            limit: Some(1),
            ..Default::default()
        })
        .summaries
    }
}

fn overlaps(a: &AnnotationDto, from: Option<i64>, to: Option<i64>) -> bool {
    from.is_none_or(|f| a.to >= f) && to.is_none_or(|t| a.from <= t)
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
