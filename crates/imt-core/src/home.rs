//! The Home dashboard's summary over the store ports: headline counts, the events over time by level (with the
//! error and warning trends), markers and anomalies to mark on that history, and the exception groups. Each part
//! reuses an existing store query (summary, timeline, anomalies, exception groups); this composes them (pure apart
//! from the store calls), so any store adapter gives the same summary. A query: no side effects.

use crate::analysis::Anomaly;
use crate::api::{QueryRequest, Timeline, TimelineRequest};
use crate::model::Level;
use crate::ports::{Analysis, Coverage, Query as QueryPort, StoreResult};
use crate::query::Query;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Fewest and most history buckets a request may ask for (the request is clamped into this range).
pub const MIN_HOME_BUCKETS: usize = 10;
pub const MAX_HOME_BUCKETS: usize = 240;
/// Most markers and anomalies the summary carries for the history chart (the counts are always complete).
pub const MAX_HOME_MARKS: usize = 200;

/// What the Home dashboard asks for: the base query (usually no filters, just the grouping) and how many
/// history buckets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeRequest {
    #[serde(default)]
    pub query: Query,
    #[serde(default = "default_buckets")]
    pub buckets: usize,
}

fn default_buckets() -> usize {
    60
}

impl Default for HomeRequest {
    fn default() -> Self {
        HomeRequest {
            query: Query::default(),
            buckets: default_buckets(),
        }
    }
}

/// Markers of one kind in one history bucket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeMarker {
    pub kind: String,
    /// The first marker's time (Unix ms).
    pub ts: i64,
    pub count: u32,
}

/// One anomaly to mark on the history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeAnomaly {
    pub kind: crate::analysis::AnomalyKind,
    pub from: i64,
    pub to: i64,
    pub message: String,
}

/// The Home dashboard's numbers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeSummary {
    pub sources: usize,
    pub events: usize,
    /// Error and Fatal events.
    pub errors: usize,
    pub warnings: usize,
    /// The time span of the timed events (Unix ms); `None` without timestamps.
    pub from: Option<i64>,
    pub to: Option<i64>,
    /// Width of one history bucket in ms (0 without timestamps).
    pub bucket_ms: f64,
    /// `[bucket][level index]` = events, levels in ascending severity (empty without timestamps).
    pub histogram: Vec<[u32; 6]>,
    /// Error and Fatal events per bucket.
    pub error_trend: Vec<u32>,
    /// Warning events per bucket.
    pub warning_trend: Vec<u32>,
    /// Markers per kind and bucket, by time (at most [`MAX_HOME_MARKS`]).
    pub markers: Vec<HomeMarker>,
    /// Every anomaly found.
    pub anomaly_count: usize,
    /// The anomalies to mark, by start time (at most [`MAX_HOME_MARKS`]).
    pub anomalies: Vec<HomeAnomaly>,
    pub exception_groups: usize,
    /// Events in all exception groups.
    pub exception_events: usize,
    /// How much of the data the analyses (anomalies, exceptions) covered.
    pub coverage: Coverage,
}

/// What the summary needs from the store.
pub trait HomeStore: QueryPort + Analysis {}
impl<T: QueryPort + Analysis + ?Sized> HomeStore for T {}

/// The Home dashboard's summary of everything matching `req.query`.
pub fn home_summary<S: HomeStore + ?Sized>(store: &S, req: &HomeRequest) -> StoreResult<HomeSummary> {
    let summary = store.summary()?;
    let level = |l: Level| summary.levels.get(&l).copied().unwrap_or(0);
    let analysis = QueryRequest { query: req.query.clone() };
    let anomalies = store.anomalies(&analysis)?;
    let groups = store.exception_groups(&analysis)?;
    let timed = summary.from.is_some() && summary.to.is_some();
    let timeline = if timed {
        Some(store.timeline(&TimelineRequest {
            query: req.query.clone(),
            buckets: req.buckets.clamp(MIN_HOME_BUCKETS, MAX_HOME_BUCKETS),
            always_show_lifecycle: true,
        })?)
    } else {
        None
    };
    let (bucket_ms, histogram, markers) = match timeline {
        Some(t) => {
            let markers = markers_of(&t);
            (t.bucket_ms, t.histogram, markers)
        }
        None => (0.0, Vec::new(), Vec::new()),
    };
    Ok(HomeSummary {
        sources: summary.sources.len(),
        events: summary.total,
        errors: level(Level::Error) + level(Level::Fatal),
        warnings: level(Level::Warning),
        from: summary.from,
        to: summary.to,
        bucket_ms,
        error_trend: histogram
            .iter()
            .map(|b| b[Level::Error.index()].saturating_add(b[Level::Fatal.index()]))
            .collect(),
        warning_trend: histogram.iter().map(|b| b[Level::Warning.index()]).collect(),
        histogram,
        markers,
        anomaly_count: anomalies.result.len(),
        anomalies: marks_of(&anomalies.result),
        exception_groups: groups.result.len(),
        exception_events: groups.result.iter().map(|g| g.count).sum(),
        coverage: anomalies.coverage.and(groups.coverage),
    })
}

/// The timeline's marker clusters merged over the lanes: one per kind and bucket, by time, at most
/// [`MAX_HOME_MARKS`].
fn markers_of(t: &Timeline) -> Vec<HomeMarker> {
    let mut merged: BTreeMap<(usize, &str), HomeMarker> = BTreeMap::new();
    for m in t.lanes.iter().flat_map(|l| &l.markers) {
        merged
            .entry((m.bucket, m.kind.as_str()))
            .and_modify(|e| {
                e.ts = e.ts.min(m.ts);
                e.count = e.count.saturating_add(m.count);
            })
            .or_insert_with(|| HomeMarker {
                kind: m.kind.clone(),
                ts: m.ts,
                count: m.count,
            });
    }
    let mut out: Vec<HomeMarker> = merged.into_values().collect();
    out.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.kind.cmp(&b.kind)));
    out.truncate(MAX_HOME_MARKS);
    out
}

/// The anomalies by start time, at most [`MAX_HOME_MARKS`].
fn marks_of(anomalies: &[Anomaly]) -> Vec<HomeAnomaly> {
    let mut out: Vec<HomeAnomaly> = anomalies
        .iter()
        .map(|a| HomeAnomaly {
            kind: a.kind,
            from: a.from,
            to: a.to,
            message: a.message.clone(),
        })
        .collect();
    out.sort_by_key(|a| (a.from, a.to));
    out.truncate(MAX_HOME_MARKS);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::AnomalyKind;
    use crate::api::{Lane, MarkerCluster};

    fn cluster(kind: &str, bucket: usize, ts: i64, count: u32) -> MarkerCluster {
        MarkerCluster {
            kind: kind.into(),
            bucket,
            ts,
            id: 0,
            count,
            message: String::new(),
        }
    }

    fn lane(markers: Vec<MarkerCluster>) -> Lane {
        Lane {
            key: "a".into(),
            first: 0,
            last: 0,
            total: 0,
            matched: 0,
            density: Vec::new(),
            markers,
            sources: Vec::new(),
        }
    }

    #[test]
    fn markers_merge_over_lanes_per_kind_and_bucket() {
        let t = Timeline {
            from: 0,
            to: 100,
            bucket_ms: 10.0,
            histogram: Vec::new(),
            lanes: vec![
                lane(vec![cluster("start", 1, 15, 1), cluster("crash", 5, 55, 2)]),
                lane(vec![cluster("start", 1, 12, 3)]),
            ],
            truncated_lanes: 0,
        };
        assert_eq!(
            markers_of(&t),
            vec![
                HomeMarker {
                    kind: "start".into(),
                    ts: 12,
                    count: 4
                },
                HomeMarker {
                    kind: "crash".into(),
                    ts: 55,
                    count: 2
                },
            ]
        );
    }

    #[test]
    fn markers_and_anomalies_are_capped() {
        let many: Vec<MarkerCluster> = (0..MAX_HOME_MARKS + 5).map(|i| cluster("m", i, i as i64, 1)).collect();
        let t = Timeline {
            from: 0,
            to: 1,
            bucket_ms: 1.0,
            histogram: Vec::new(),
            lanes: vec![lane(many)],
            truncated_lanes: 0,
        };
        assert_eq!(markers_of(&t).len(), MAX_HOME_MARKS);
        let anomalies: Vec<Anomaly> = (0..MAX_HOME_MARKS + 5)
            .rev()
            .map(|i| Anomaly {
                kind: AnomalyKind::ErrorSpike,
                instance: None,
                from: i as i64,
                to: i as i64 + 1,
                score: 1.0,
                message: format!("spike {i}"),
                sample_ids: Vec::new(),
            })
            .collect();
        let marks = marks_of(&anomalies);
        assert_eq!(marks.len(), MAX_HOME_MARKS);
        assert_eq!(marks[0].from, 0, "sorted by start time before the cap");
    }

    #[test]
    fn requests_default_to_sixty_buckets() {
        let req: HomeRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req.buckets, 60);
        assert_eq!(HomeRequest::default().buckets, 60);
    }
}
