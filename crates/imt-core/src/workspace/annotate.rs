//! Annotators over the workspace: the read-only view they query, when they
//! run, and the stored annotations (see [`crate::plugin::annotate`]).

use super::Workspace;
use crate::plugin::annotate::{AnnotationDto, AnnotationView, AnnotationsRequest, RecordCursor, ViewFilter, ViewRecord, MAX_VIEW_RECORDS};
use crate::profile::compile_conditions;
use crate::query::Query;
use anyhow::Result;

/// [`AnnotationView`] over a workspace.
struct View<'a>(&'a Workspace);

impl View<'_> {
    fn record(&self, r: &crate::model::Record) -> ViewRecord {
        let source = self.0.source_name(r.source);
        ViewRecord {
            record: r.clone(),
            source: source.to_string(),
            instance: crate::query::group_key(&self.0.default_group_by, r, source),
        }
    }

    /// Records in time order (or reversed), those without a time last.
    fn ordered(&self, newest_first: bool) -> Box<dyn Iterator<Item = &crate::model::Record> + '_> {
        if newest_first {
            Box::new(self.0.records.iter().rev())
        } else {
            Box::new(self.0.records.iter())
        }
    }
}

impl AnnotationView for View<'_> {
    fn select(&self, filter: &ViewFilter, limit: usize) -> Result<Vec<ViewRecord>> {
        let when = compile_conditions(&filter.when)?;
        let unbounded = filter.from.is_none() && filter.to.is_none();
        let in_range = |ts: Option<i64>| {
            ts.map_or(unbounded, |ts| {
                filter.from.is_none_or(|f| ts >= f) && filter.to.is_none_or(|t| ts <= t)
            })
        };
        Ok(self
            .ordered(filter.newest_first)
            .filter(|r| in_range(r.ts))
            .filter(|r| when.iter().all(|c| c.eval(r, self.0.source_name(r.source))))
            .take(limit.min(MAX_VIEW_RECORDS))
            .map(|r| self.record(r))
            .collect())
    }

    fn query(&self, query: &Query, limit: usize) -> Result<Vec<ViewRecord>> {
        let cq = self.0.compile(query)?;
        Ok(self
            .0
            .records
            .iter()
            .filter(|r| cq.matches(r, self.0.source_name(r.source)))
            .take(limit.min(MAX_VIEW_RECORDS))
            .map(|r| self.record(r))
            .collect())
    }

    fn instances(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        self.0
            .records
            .iter()
            .map(|r| crate::query::group_key(&self.0.default_group_by, r, self.0.source_name(r.source)))
            .filter(|k| seen.insert(k.clone()))
            .collect()
    }

    fn time_range(&self) -> Option<(i64, i64)> {
        let first = self.0.records.first().and_then(|r| r.ts)?;
        Some((first, self.0.newest_ts()?))
    }
}

impl Workspace {
    /// Runs the annotators; `since` is the first new record of an append.
    pub(super) fn run_annotators(&self, since: Option<RecordCursor>) {
        if self.registry.has_annotators() {
            self.registry.run_annotators(&View(self), since);
        }
    }

    /// Re-runs the annotators after a live-tail refresh.
    pub(super) fn annotate_after_tail(&self, report: &super::TailReport, first_new: Option<u32>) {
        if report.is_empty() {
            return;
        }
        let since = match (report.ids_changed_from, first_new) {
            (None, Some(id)) => self.records.get(id as usize).map(|r| RecordCursor { id, ts: r.ts }),
            _ => None,
        };
        self.run_annotators(since);
    }

    /// Bits of the marker set that may be shown (kinds of unavailable plugins are hidden).
    pub(super) fn visible_markers(&self) -> crate::model::MarkerMask {
        self.registry.visible_markers(&self.profile.markers)
    }

    /// Ranges and points found by annotators, overlapping the request.
    pub fn annotations(&self, req: &AnnotationsRequest) -> Vec<AnnotationDto> {
        self.registry.annotations(req)
    }
}
