//! Evaluation of dashboard tiles: each reuses an existing engine query
//! (charts, the query language, facets, anomalies, annotations).

use super::Workspace;
use crate::api::{ChartRequest, FacetRequest, QueryRequest};
use crate::chart::ChartDef;
use crate::dashboard::{TileCfg, TileData, TileKind, TileRequest, MAX_TABLE_ROWS, SPARKLINE_POINTS};
use crate::model::value_number;
use crate::plugin::annotate::AnnotationsRequest;
use crate::ql::QlResult;
use crate::query::Query;
use crate::tiles::{first_value, tile_query, with_ql};
use anyhow::{anyhow, Result};
use serde_json::Value;

impl Workspace {
    /// Evaluates one tile over the dashboard's base query.
    pub fn dashboard_tile(&self, req: &TileRequest) -> Result<TileData> {
        let tile = &req.tile;
        tile.validate(crate::dashboard::MAX_COLUMNS)?;
        let base = tile_query(tile, &req.query)?;
        match tile.kind {
            TileKind::Chart => self.chart_tile(tile, base),
            TileKind::Stat => self.stat_tile(tile, base),
            TileKind::Table => self.table_tile(tile, base),
            TileKind::Top => Ok(TileData::Top {
                facet: self.facet(&FacetRequest {
                    query: with_ql(base, tile.query.as_deref()),
                    field: tile.field.clone().unwrap_or_default(),
                    limit: tile.limit_or(10, 100),
                })?,
            }),
            TileKind::Alerts => {
                let mut items = self.anomalies(&QueryRequest {
                    query: with_ql(base, tile.query.as_deref()),
                })?;
                items.truncate(tile.limit_or(20, 200));
                Ok(TileData::Alerts { items })
            }
            TileKind::Annotations => {
                let q = self.resolve_window(&base);
                let mut items = self.annotations(&AnnotationsRequest {
                    from: q.from,
                    to: q.to,
                    instance: None,
                });
                items.truncate(tile.limit_or(20, 200));
                Ok(TileData::Annotations { items })
            }
        }
    }

    fn chart_tile(&self, tile: &TileCfg, base: Query) -> Result<TileData> {
        let (chart_id, def) = match &tile.chart {
            Some(id) => (Some(id.clone()), None),
            None => (
                None,
                Some(ChartDef {
                    id: tile.id.clone(),
                    title: if tile.title.is_empty() {
                        tile.id.clone()
                    } else {
                        tile.title.clone()
                    },
                    y: tile.value.clone().unwrap_or_default(),
                    bucket: "auto".into(),
                    split_by: tile.by.clone(),
                    kind: tile.style.clone().unwrap_or_else(|| "line".into()),
                    unit: tile.unit.clone().unwrap_or_default(),
                }),
            ),
        };
        let data = self.chart(&ChartRequest {
            query: with_ql(base, tile.query.as_deref()),
            chart_id,
            def,
            buckets: 120,
        })?;
        Ok(TileData::Chart { data })
    }

    fn ql(&self, query: Query) -> Result<QlResult> {
        self.run_query(&QueryRequest { query }).map_err(|e| anyhow!("{}", e.message))
    }

    fn stat_tile(&self, tile: &TileCfg, base: Query) -> Result<TileData> {
        let program = tile.query.clone().unwrap_or_default();
        let (label, value) = first_value(self.ql(with_ql(base.clone(), Some(&program)))?);
        let resolved = self.resolve_window(&base).into_owned();
        let (from, to) = match (resolved.from, resolved.to) {
            (Some(f), Some(t)) => (f, t),
            (f, t) => {
                let ids = self.matching(&resolved)?;
                let ts = |i: Option<&u32>| i.and_then(|&id| self.records[id as usize].ts);
                (f.or(ts(ids.first())).unwrap_or(0), t.or(ts(ids.last())).unwrap_or(0))
            }
        };
        let step = ((to - from) / SPARKLINE_POINTS as i64).max(1);
        let mut sparkline = Vec::with_capacity(SPARKLINE_POINTS);
        for i in 0..SPARKLINE_POINTS as i64 {
            let start = from + i * step;
            let bucket = Query {
                from: Some(start),
                to: Some(if i + 1 == SPARKLINE_POINTS as i64 {
                    to.max(start)
                } else {
                    start + step - 1
                }),
                ..resolved.clone()
            };
            let (_, v) = first_value(self.ql(with_ql(bucket, Some(&program)))?);
            sparkline.push((start, v.as_ref().and_then(value_number)));
        }
        Ok(TileData::Stat {
            value: value.as_ref().and_then(value_number),
            text: value.as_ref().map(|v| crate::model::value_text(v).into_owned()).unwrap_or_default(),
            label,
            unit: tile.unit.clone(),
            sparkline,
        })
    }

    fn table_tile(&self, tile: &TileCfg, base: Query) -> Result<TileData> {
        let limit = tile.limit_or(MAX_TABLE_ROWS, MAX_TABLE_ROWS);
        match self.ql(with_ql(base, tile.query.as_deref()))? {
            QlResult::Table { columns, mut rows } => {
                let truncated = rows.len() > limit;
                rows.truncate(limit);
                Ok(TileData::Table { columns, rows, truncated })
            }
            QlResult::Events { ids, .. } => {
                let truncated = ids.len() > limit;
                let rows = ids
                    .iter()
                    .take(limit)
                    .map(|&id| {
                        let r = &self.records[id as usize];
                        vec![
                            r.ts.map(Value::from).unwrap_or(Value::Null),
                            Value::from(r.level.as_str()),
                            Value::from(self.view_text("@message", r.message.as_str().into()).into_owned()),
                        ]
                    })
                    .collect();
                Ok(TileData::Table {
                    columns: vec!["@t".into(), "@level".into(), "@message".into()],
                    rows,
                    truncated,
                })
            }
        }
    }
}
