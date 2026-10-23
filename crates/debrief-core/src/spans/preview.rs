//! The Runs editor's preview (pure): a draft rule checked against the profile, then its runs paired over the
//! timed events fed in time order. The in-memory workspace and the store feed the same previewer, so both
//! answer the same [`SpanPreview`].

use super::draft::{self, Checked};
use super::{book, summarize, to_annotation, Pairer, RunStatus, RunStyle, SpanIssue, SpanPreview, SpanPreviewRequest};
use crate::model::Record;
use crate::profile::CompiledProfile;

/// Pairs a draft's runs over the events it is fed.
pub struct SpanPreviewer {
    errors: Vec<SpanIssue>,
    warnings: Vec<String>,
    checked: Option<(Checked, Pairer)>,
    group_by: Vec<String>,
}

impl SpanPreviewer {
    /// The draft checked against `compiled` (its effective markers); `group_by` is the instance key the runs'
    /// lanes are shown with.
    pub fn new(req: &SpanPreviewRequest, compiled: &CompiledProfile, group_by: Vec<String>) -> SpanPreviewer {
        let (errors, warnings, checked) = draft::check(&req.draft, &compiled.profile, &compiled.markers);
        SpanPreviewer {
            errors,
            warnings,
            checked: checked.map(|c| {
                let p = Pairer::new(c.rule.restart_on_start);
                (c, p)
            }),
            group_by,
        }
    }

    /// Whether the draft is valid (only then are events worth feeding).
    pub fn wants_events(&self) -> bool {
        self.checked.is_some()
    }

    /// One timed event, in time order (`lane` is its instance key).
    pub fn add(&mut self, r: &Record, source: &str, lane: &str) {
        if let Some((c, pairer)) = &mut self.checked {
            book::feed_one(r, source, lane, &[(&c.rule, c.classifier.clone())], std::slice::from_mut(pairer));
        }
    }

    pub fn finish(self) -> SpanPreview {
        let Some((checked, pairer)) = self.checked else {
            return SpanPreview {
                errors: self.errors,
                warnings: self.warnings,
                ..Default::default()
            };
        };
        let style = RunStyle::of(&checked.rule);
        let mut all: Vec<_> = pairer.runs().iter().map(|run| to_annotation(&style, run, &self.group_by)).collect();
        all.sort_by_key(|a| a.from);
        let summary = summarize(&checked.rule, &all, pairer.dropped_runs + pairer.ignored_events);
        let anomalies = all
            .iter()
            .filter(|a| {
                a.run
                    .as_ref()
                    .is_some_and(|r| r.too_long || matches!(r.status, RunStatus::NoStop | RunStatus::NoStart))
            })
            .take(super::dto::PREVIEW_RUNS)
            .cloned()
            .collect();
        all.truncate(super::dto::PREVIEW_RUNS);
        SpanPreview {
            errors: self.errors,
            warnings: self.warnings,
            summary: Some(summary),
            runs: all,
            anomalies,
            plan: Some(checked.plan),
        }
    }
}
