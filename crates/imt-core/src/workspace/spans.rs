//! Runs between lifecycle markers over the workspace (see [`crate::spans`]):
//! computed lazily on the first query, then fed only the records appended by
//! live tail; anything else that changes records (re-sorts, rewrites,
//! evictions, renames, a new profile or a regrouping) recomputes them.

use super::Workspace;
use crate::model::Record;
use crate::plugin::annotate::{AnnotationDto, AnnotationsRequest};
use crate::spans::draft::{self, Classifier};
use crate::spans::{
    book, summarize, to_annotation, Pairer, RunStatus, RunStyle, RunsReply, RunsRequest, SpanPreview, SpanPreviewRequest, SpanRule,
    SpanSummary,
};

/// The pairing state of every rule, and what it was computed from.
#[derive(Default)]
pub(super) struct SpanBook {
    /// (record generation, lane fields, profile revision) the state belongs to.
    key: Option<(u64, Vec<String>, u64)>,
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
        book::feed_one(r, source(r.source), &lanes(r), rules, pairers);
    }
}

impl Workspace {
    fn lane_of(&self, r: &Record) -> String {
        crate::query::group_key(&self.default_group_by, r, self.source_name(r.source))
    }

    /// Enabled rules with their classifiers.
    fn span_rules(&self) -> Vec<(&SpanRule, Classifier)> {
        book::enabled_rules(&self.profile)
    }

    /// Brings the runs up to date and calls `f` with the annotations of each enabled rule.
    fn with_runs<R>(&self, f: impl FnOnce(&[(&SpanRule, Classifier)], &[Pairer], &[Vec<AnnotationDto>]) -> R) -> R {
        let rules = self.span_rules();
        let mut book = self.spans.lock().unwrap_or_else(|p| p.into_inner());
        let key = (self.generation, self.default_group_by.clone(), self.profile_revision);
        let timed = self.records.partition_point(|r| r.ts.is_some());
        if book.key.as_ref() != Some(&key) || book.fed > timed {
            *book = SpanBook {
                key: Some(key),
                pairers: book::pairers(&rules),
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
            book.annotations = book::annotations(&rules, &book.pairers, &self.default_group_by);
        }
        f(&rules, &book.pairers, &book.annotations)
    }

    /// Runs overlapping the request (each an annotation with `run`).
    pub(super) fn run_annotations(&self, req: &AnnotationsRequest) -> Vec<AnnotationDto> {
        self.with_runs(|_, _, anns| {
            anns.iter()
                .flatten()
                .filter(|a| book::overlaps(a, req.from, req.to))
                .filter(|a| req.instance.as_ref().is_none_or(|i| a.instance.as_ref() == Some(i)))
                .cloned()
                .collect()
        })
    }

    /// The Runs table: a sorted page of runs and every span kind's summary over the selection.
    pub fn runs(&self, req: &RunsRequest) -> RunsReply {
        self.with_runs(|rules, pairers, anns| book::runs_reply(&self.profile, rules, pairers, anns, req))
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
