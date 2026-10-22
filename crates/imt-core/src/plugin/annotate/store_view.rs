//! [`AnnotationView`] over the store ports: annotators read a store's events through [`Query::scan`] (time order)
//! and [`Query::summary`] (source names, the instance key, the time range), as they read the in-memory
//! workspace's records. Every call is bounded: at most [`MAX_VIEW_SCAN`] events are read per call (the coverage of
//! a call that stopped there is what it found until then) and at most [`MAX_VIEW_INSTANCES`] instances listed.

use super::{AnnotationView, ViewFilter, ViewRecord, MAX_VIEW_RECORDS};
use crate::model::Record;
use crate::ports::{Filter, PageQuery, Query, StoreResult, SummaryReply, MAX_PAGE_ROWS};
use crate::profile::compile_conditions;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

/// Most events one view call reads.
pub const MAX_VIEW_SCAN: usize = 5_000_000;
/// Most instance keys [`AnnotationView::instances`] lists.
pub const MAX_VIEW_INSTANCES: usize = 10_000;

/// The view of one annotator run over a store (the summary is read once, when the view is made).
pub struct PortView<'a, S: ?Sized> {
    store: &'a S,
    summary: SummaryReply,
    names: HashMap<u32, String>,
    max_scan: usize,
}

impl<'a, S: Query + ?Sized> PortView<'a, S> {
    pub fn new(store: &'a S) -> StoreResult<PortView<'a, S>> {
        let summary = store.summary()?;
        let names = summary.sources.iter().map(|s| (s.id, s.name.clone())).collect();
        Ok(PortView {
            store,
            summary,
            names,
            max_scan: MAX_VIEW_SCAN,
        })
    }

    /// The view reading at most `max` events per call (tests).
    pub fn with_max_scan(mut self, max: usize) -> PortView<'a, S> {
        self.max_scan = max.min(MAX_VIEW_SCAN);
        self
    }

    fn source(&self, id: u32) -> &str {
        self.names.get(&id).map_or("", String::as_str)
    }

    fn view_record(&self, record: Record) -> ViewRecord {
        let source = self.source(record.source).to_string();
        let instance = crate::query::group_key(&self.summary.default_group_by, &record, &source);
        ViewRecord { record, source, instance }
    }

    /// Visits the events matching `filter` in time order until `visit` breaks or [`MAX_VIEW_SCAN`] were read.
    fn each(&self, filter: &Filter, mut visit: impl FnMut(Record) -> ControlFlow<()>) -> Result<()> {
        let mut read = 0usize;
        self.store
            .scan(filter, &mut |batch| {
                for row in batch.rows() {
                    if read == self.max_scan {
                        return ControlFlow::Break(());
                    }
                    read += 1;
                    visit(crate::row::to_record(&row))?;
                }
                ControlFlow::Continue(())
            })
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

impl<S: Query + ?Sized> AnnotationView for PortView<'_, S> {
    fn select(&self, filter: &ViewFilter, limit: usize) -> Result<Vec<ViewRecord>> {
        let when = compile_conditions(&filter.when)?;
        let limit = limit.min(MAX_VIEW_RECORDS);
        if limit == 0 {
            return Ok(Vec::new());
        }
        // Bounds leave out events without a time, as the scan's time range does.
        let range = Filter {
            from: filter.from,
            to: filter.to,
            ..Filter::default()
        };
        let matches = |r: &Record| when.iter().all(|c| c.eval(r, self.source(r.source)));
        let mut out = Vec::new();
        if filter.newest_first {
            // Pages from the newest end (events without a time first, as the grid shows them newest first).
            let (mut offset, mut read) = (0usize, 0usize);
            loop {
                let page = self
                    .store
                    .page(&PageQuery {
                        query: range.clone(),
                        offset,
                        limit: MAX_PAGE_ROWS,
                        descending: true,
                        anchor: None,
                    })
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                let n = page.rows.len();
                for e in page.rows {
                    let r = e.to_record();
                    if matches(&r) {
                        out.push(self.view_record(r));
                    }
                    read += 1;
                    if out.len() == limit || read == self.max_scan {
                        return Ok(out);
                    }
                }
                offset += n;
                if n == 0 || offset >= page.total {
                    return Ok(out);
                }
            }
        }
        self.each(&range, |r| {
            if matches(&r) {
                out.push(self.view_record(r));
                if out.len() == limit {
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        })?;
        Ok(out)
    }

    fn query(&self, query: &crate::query::Query, limit: usize) -> Result<Vec<ViewRecord>> {
        let limit = limit.min(MAX_VIEW_RECORDS);
        let mut out = Vec::new();
        if limit == 0 {
            return Ok(out);
        }
        self.each(query, |r| {
            out.push(r);
            if out.len() == limit {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })?;
        Ok(out.into_iter().map(|r| self.view_record(r)).collect())
    }

    fn instances(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let listed = self.each(&Filter::default(), |r| {
            let key = crate::query::group_key(&self.summary.default_group_by, &r, self.source(r.source));
            if seen.insert(key.clone()) {
                out.push(key);
                if out.len() == MAX_VIEW_INSTANCES {
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        });
        if let Err(e) = listed {
            log::warn!("annotators: the instances could not be listed: {e:#}");
        }
        out
    }

    fn time_range(&self) -> Option<(i64, i64)> {
        Some((self.summary.from?, self.summary.to?))
    }
}
