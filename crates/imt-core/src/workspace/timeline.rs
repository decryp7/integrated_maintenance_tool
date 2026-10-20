//! Per-instance lanes and the level histogram.

use super::{Lane, MarkerCluster, Timeline, TimelineRequest, Workspace, MAX_LANE_SOURCES};
use crate::model::Record;
use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashMap;

const MAX_LANES: usize = 500;

impl Workspace {
    pub fn timeline(&self, req: &TimelineRequest) -> Result<Timeline> {
        let cq = self.compile(&req.query)?;
        let buckets = req.buckets.clamp(10, 2000);
        let visible = self.visible_markers();
        let lifecycle = if req.always_show_lifecycle {
            self.profile.lifecycle_mask & visible
        } else {
            0
        };

        // Structural pass: which records belong to the visible instances.
        let candidates: Vec<(&Record, String)> = self
            .records
            .par_iter()
            .filter(|r| r.ts.is_some())
            .filter_map(|r| {
                let source = self.source_name(r.source);
                let key = cq.group_key(r, source);
                cq.structural(r, source, Some(&key)).then_some((r, key))
            })
            .collect();

        let query = self.resolve_window(&req.query);
        let (from, to) = match (query.from, query.to) {
            (Some(f), Some(t)) => (f, t),
            (f, t) => {
                let min = candidates.iter().filter_map(|(r, _)| r.ts).min();
                let max = candidates.iter().filter_map(|(r, _)| r.ts).max();
                (f.or(min).unwrap_or(0), t.or(max).unwrap_or(0))
            }
        };
        let span = (to - from).max(1);
        let bucket_ms = span as f64 / buckets as f64;
        let bucket_of = |ts: i64| (((ts - from) as f64 / bucket_ms) as usize).min(buckets - 1);

        let mut histogram = vec![[0u32; 6]; buckets];
        let mut lane_index: HashMap<String, usize> = HashMap::new();
        let mut lanes: Vec<Lane> = Vec::new();
        let mut clusters: Vec<HashMap<(usize, usize), MarkerCluster>> = Vec::new();

        for (r, key) in candidates {
            let ts = r.ts.unwrap();
            let li = *lane_index.entry(key).or_insert_with_key(|k| {
                lanes.push(Lane {
                    key: k.clone(),
                    first: ts,
                    last: ts,
                    total: 0,
                    matched: 0,
                    density: vec![0; buckets],
                    markers: Vec::new(),
                    sources: Vec::new(),
                });
                clusters.push(HashMap::new());
                lanes.len() - 1
            });
            let lane = &mut lanes[li];
            lane.first = lane.first.min(ts);
            lane.last = lane.last.max(ts);
            lane.total += 1;
            if lane.sources.len() < MAX_LANE_SOURCES && !lane.sources.contains(&r.source) {
                lane.sources.push(r.source);
            }
            let b = bucket_of(ts);
            let matched = cq.content(r, self.source_name(r.source));
            let mask = if matched {
                lane.matched += 1;
                lane.density[b] += 1;
                histogram[b][r.level.index()] += 1;
                r.markers & visible
            } else {
                r.markers & lifecycle
            };
            if mask != 0 {
                let kind = mask.trailing_zeros() as usize;
                clusters[li]
                    .entry((kind, b))
                    .and_modify(|c| c.count += 1)
                    .or_insert_with(|| MarkerCluster {
                        kind: self.marker_kinds[kind].clone(),
                        bucket: b,
                        ts,
                        id: r.id,
                        count: 1,
                        message: r.message.chars().take(240).collect(),
                    });
            }
        }
        for (lane, c) in lanes.iter_mut().zip(clusters) {
            lane.markers = c.into_values().collect();
            lane.markers.sort_by_key(|m| (m.ts, m.kind.clone()));
            lane.sources.sort_unstable();
        }
        lanes.sort_by(|a, b| a.first.cmp(&b.first).then_with(|| a.key.cmp(&b.key)));
        let truncated_lanes = lanes.len().saturating_sub(MAX_LANES);
        lanes.truncate(MAX_LANES);

        Ok(Timeline {
            from,
            to,
            bucket_ms,
            histogram,
            lanes,
            truncated_lanes,
        })
    }
}
