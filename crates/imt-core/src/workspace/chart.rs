//! Time-bucket aggregation for charts.

use super::{ChartData, ChartRequest, ChartSeries, LifecycleEvent, Workspace};
use crate::chart::{nice_bucket, parse_bucket, parse_y, Acc, Agg};
use crate::model::{Level, Record};
use crate::profile::compile_conditions;
use crate::query::group_key;
use anyhow::{anyhow, Result};
use std::collections::HashMap;

const MAX_SERIES: usize = 8;

impl Workspace {
    pub fn chart(&self, req: &ChartRequest) -> Result<ChartData> {
        let (def, when) = match (&req.def, &req.chart_id) {
            (Some(def), _) => (def.clone(), Vec::new()),
            (None, Some(id)) => {
                let defs = self.chart_defs();
                let i = defs
                    .iter()
                    .position(|d| &d.id == id)
                    .ok_or_else(|| anyhow!("unknown chart \"{id}\""))?;
                (defs[i].clone(), compile_conditions(&self.profile.profile.chart[i].when)?)
            }
            (None, None) => return Err(anyhow!("chart request needs chartId or def")),
        };
        let (agg, field) = parse_y(&def.y)?;
        let ids = self.matching(&req.query)?;
        let cq = self.compile(&req.query)?;

        let rows: Vec<&Record> = ids
            .iter()
            .map(|&id| &self.records[id as usize])
            .filter(|r| r.ts.is_some() && when.iter().all(|c| c.eval(r, self.source_name(r.source))))
            .collect();
        let from = req.query.from.or_else(|| rows.first().and_then(|r| r.ts)).unwrap_or(0);
        let to = req.query.to.or_else(|| rows.last().and_then(|r| r.ts)).unwrap_or(from);
        let span = (to - from).max(1);
        let bucket_ms = match parse_bucket(&def.bucket)? {
            Some(b) => b.max(span / 5000),
            None => nice_bucket(span, req.buckets.clamp(10, 2000)),
        };
        let start = from - from.rem_euclid(bucket_ms);
        let n = ((to - start) / bucket_ms + 1) as usize;

        let mut series: HashMap<String, Vec<Acc>> = HashMap::new();
        let mut totals: HashMap<String, Acc> = HashMap::new();
        for r in &rows {
            let source = self.source_name(r.source);
            let value = match (&field, agg) {
                (_, Agg::Count) => 1.0,
                (Some(f), _) => match r.field_number(f) {
                    Some(v) => v,
                    None => continue,
                },
                (None, _) => continue,
            };
            let key = if def.split_by.is_empty() {
                def.title.clone()
            } else {
                group_key(&def.split_by, r, source)
            };
            let b = ((r.ts.unwrap() - start) / bucket_ms) as usize;
            if b >= n {
                continue;
            }
            series.entry(key.clone()).or_insert_with(|| vec![Acc::default(); n])[b].add(value);
            totals.entry(key).or_default().add(value);
        }

        // Keep the biggest series; fold the rest into "Other".
        let mut keys: Vec<String> = series.keys().cloned().collect();
        if def.split_by.len() == 1 && def.split_by[0] == "@level" {
            keys.sort_by_key(|k| Level::parse(k).map(|l| l.index()).unwrap_or(99));
        } else {
            keys.sort_by(|a, b| totals[b].count.cmp(&totals[a].count).then_with(|| a.cmp(b)));
        }
        let mut out = Vec::new();
        let mut other: Option<Vec<Acc>> = None;
        for (i, k) in keys.into_iter().enumerate() {
            let accs = series.remove(&k).unwrap();
            if i < MAX_SERIES - 1 || totals.len() <= MAX_SERIES {
                out.push((k, accs));
            } else {
                let o = other.get_or_insert_with(|| vec![Acc::default(); n]);
                for (a, b) in o.iter_mut().zip(&accs) {
                    a.merge(b);
                }
            }
        }
        if let Some(o) = other {
            out.push(("Other".into(), o));
        }
        let series = out
            .into_iter()
            .map(|(key, accs)| ChartSeries {
                key,
                points: accs
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        (
                            start + i as i64 * bucket_ms,
                            a.value(agg).or(if agg == Agg::Count { Some(0.0) } else { None }),
                        )
                    })
                    .collect(),
            })
            .collect();

        let lifecycle_mask = self.profile.lifecycle_mask | self.fatal_mask();
        let lifecycle = self
            .records
            .iter()
            .filter(|r| r.markers & lifecycle_mask != 0 && r.ts.is_some_and(|ts| ts >= from && ts <= to))
            .filter(|r| cq.structural(r, self.source_name(r.source), None))
            .take(500)
            .map(|r| LifecycleEvent {
                kind: self.marker_kinds[(r.markers & lifecycle_mask).trailing_zeros() as usize].clone(),
                ts: r.ts.unwrap(),
                group: cq.group_key(r, self.source_name(r.source)),
                id: r.id,
            })
            .collect();

        Ok(ChartData {
            def,
            from,
            to,
            bucket_ms,
            series,
            lifecycle,
        })
    }

    fn fatal_mask(&self) -> u32 {
        self.marker_kinds
            .iter()
            .enumerate()
            .filter(|(_, k)| *k == "fatal")
            .fold(0, |m, (i, _)| m | (1 << i))
    }
}
