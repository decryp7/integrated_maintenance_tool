//! Time-bucket aggregation for charts.

use super::{ChartData, ChartRequest, LifecycleEvent, Workspace};
use crate::chart::{chart_value, parse_y, Buckets, SeriesBuilder};
use crate::model::Record;
use crate::profile::compile_conditions;
use crate::query::group_key;
use crate::row::RecordRow;
use anyhow::{anyhow, Result};

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
        let query = self.resolve_window(&req.query);
        let from = query.from.or_else(|| rows.first().and_then(|r| r.ts)).unwrap_or(0);
        let to = query.to.or_else(|| rows.last().and_then(|r| r.ts)).unwrap_or(from);
        let buckets = Buckets::of(&def, from, to, req.buckets)?;
        let bucket_ms = buckets.bucket_ms;
        let mut builder = SeriesBuilder::new(buckets);
        for r in &rows {
            let row = RecordRow {
                rec: r,
                source: self.source_name(r.source),
            };
            // `parse_y` gives a field to every aggregate except count().
            let Some(value) = chart_value(&row, field.as_deref()) else {
                continue;
            };
            let key = if def.split_by.is_empty() {
                def.title.clone()
            } else {
                group_key(&def.split_by, r, row.source)
            };
            builder.add(r.ts.unwrap_or_default(), value, key);
        }
        let series = builder.finish(&def, agg);

        let lifecycle_mask = (self.profile.lifecycle_mask | self.fatal_mask()) & self.visible_markers();
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

    fn fatal_mask(&self) -> crate::model::MarkerMask {
        self.marker_kinds
            .iter()
            .enumerate()
            .filter(|(_, k)| *k == "fatal")
            .fold(0, |m, (i, _)| m | (1 << i))
    }
}
