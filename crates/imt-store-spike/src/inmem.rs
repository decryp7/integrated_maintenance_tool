//! The baseline: the shipped in-memory [`Workspace`], called through the real [`LogStore`] trait.

use crate::port::{Column, FacetOut, Filter, PageOut, SearchOut, SpikeStore, StatsOut, TimelineOut};
use anyhow::Result;
use imt_core::api::{FacetRequest, FieldStatsRequest, PageRequest, TimelineRequest};
use imt_core::query::{FieldFilter, Query};
use imt_core::LogStore;

pub struct InMemory<S: LogStore> {
    pub store: S,
}

fn query(f: &Filter) -> Query {
    Query {
        levels: f.levels.clone(),
        text: f.text.clone().unwrap_or_default(),
        filters: f
            .field
            .iter()
            .map(|(c, v)| FieldFilter {
                field: c.field().into(),
                op: "eq".into(),
                value: Some(v.clone()),
            })
            .collect(),
        ..Query::default()
    }
}

impl<S: LogStore> SpikeStore for InMemory<S> {
    fn page(&self, filter: &Filter, offset: u64, limit: usize) -> Result<PageOut> {
        let req = PageRequest {
            query: query(filter),
            offset: offset as usize,
            limit,
            descending: false,
            anchor: None,
        };
        let p = self.store.page(&req)?;
        Ok(PageOut {
            total: p.total as u64,
            ids: p.rows.iter().map(|r| r.id as u64).collect(),
            message_bytes: p.rows.iter().map(|r| r.message.len()).sum(),
        })
    }

    fn facet(&self, column: Column, filter: &Filter, limit: usize) -> Result<FacetOut> {
        let f = self.store.facet(&FacetRequest {
            query: query(filter),
            field: column.field().into(),
            limit,
        })?;
        Ok(FacetOut {
            total: f.total as u64,
            distinct: f.distinct as u64,
            top: f.values.into_iter().map(|v| (v.value, v.count as u64)).collect(),
        })
    }

    fn timeline(&self, filter: &Filter, buckets: usize) -> Result<TimelineOut> {
        let t = self.store.timeline(&TimelineRequest {
            query: query(filter),
            buckets,
            always_show_lifecycle: true,
        })?;
        Ok(TimelineOut {
            from: t.from,
            to: t.to,
            histogram: t.histogram.iter().map(|b| b.map(u64::from)).collect(),
            lanes: t.lanes.len() + t.truncated_lanes,
        })
    }

    fn field_stats(&self, column: Column, filter: &Filter) -> Result<StatsOut> {
        let s = self.store.field_stats(&FieldStatsRequest {
            query: query(filter),
            field: column.field().into(),
            buckets: crate::port::HISTOGRAM_BINS,
        })?;
        let n = s.numeric.as_ref();
        Ok(StatsOut {
            count: s.count,
            missing: s.missing,
            distinct: s.distinct as u64,
            min: n.map_or(f64::NAN, |n| n.min),
            max: n.map_or(f64::NAN, |n| n.max),
            mean: n.map_or(f64::NAN, |n| n.mean),
            p50: n.map_or(f64::NAN, |n| n.p50),
            p95: n.map_or(f64::NAN, |n| n.p95),
            p99: n.map_or(f64::NAN, |n| n.p99),
            histogram: n.map(|n| n.histogram.iter().map(|b| b.count).collect()).unwrap_or_default(),
        })
    }

    /// No inverted index: the search box's substring scan over message, exception and template.
    fn search(&self, text: &str, limit: usize) -> Result<SearchOut> {
        let f = Filter {
            text: Some(text.into()),
            ..Filter::default()
        };
        let p = self.page(&f, 0, limit)?;
        Ok(SearchOut {
            hits: p.total,
            fetched: p.ids.len(),
            ids: p.ids,
        })
    }
}
