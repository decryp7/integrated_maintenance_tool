//! Evaluation of dashboard tiles: each reuses an existing engine query
//! (charts, the query language, facets, anomalies, annotations).

use super::Workspace;
use crate::api::{ChartRequest, FacetRequest, QueryRequest};
use crate::chart::ChartDef;
use crate::dashboard::{TileCfg, TileData, TileKind, TileRequest, MAX_TABLE_ROWS, SPARKLINE_POINTS};
use crate::model::value_number;
use crate::plugin::annotate::AnnotationsRequest;
use crate::ql::QlResult;
use crate::query::{Query, TimeWindow};
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

/// The tile's base query: the dashboard's, or its own sliding window.
fn tile_query(tile: &TileCfg, base: &Query) -> Result<Query> {
    let mut q = base.clone();
    if let Some(ms) = tile.window_ms()? {
        q.from = None;
        q.to = None;
        q.window = Some(TimeWindow::Live { last_ms: ms });
    }
    Ok(q)
}

/// `base` with the tile's query-language text added to its own: the base
/// program, then the tile's filter as a `where` stage, then its stages.
fn with_ql(mut base: Query, ql: Option<&str>) -> Query {
    let tile = ql.map(str::trim).unwrap_or("");
    if base.ql.trim().is_empty() || tile.is_empty() {
        if base.ql.trim().is_empty() {
            base.ql = tile.to_string();
        }
        return base;
    }
    let (filter, stages) = split_stages(tile);
    let mut text = base.ql.trim().to_string();
    if !filter.is_empty() {
        text = format!("{text} | where {filter}");
    }
    if !stages.is_empty() {
        text = format!("{text} | {stages}");
    }
    base.ql = text;
    base
}

/// Splits a query-language text at its first `|` outside quotes.
fn split_stages(text: &str) -> (&str, &str) {
    let mut quote: Option<char> = None;
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'' | '`') => quote = Some(c),
            (None, '|') => return (text[..i].trim(), text[i + 1..].trim()),
            (None, _) => {}
        }
    }
    (text.trim(), "")
}

/// The last column of the first row of a table (the value of a `stats`).
fn first_value(result: QlResult) -> (String, Option<Value>) {
    match result {
        QlResult::Table { columns, rows } => {
            let label = columns.last().cloned().unwrap_or_default();
            (label, rows.into_iter().next().and_then(|r| r.into_iter().last()))
        }
        QlResult::Events { ids, .. } => ("events".into(), Some(Value::from(ids.len()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_queries_combine_with_the_base() {
        let base = Query {
            ql: "level >= warning".into(),
            from: Some(1),
            ..Default::default()
        };
        assert_eq!(with_ql(base.clone(), None).ql, "level >= warning");
        assert_eq!(with_ql(base.clone(), Some("x = 1")).ql, "level >= warning | where x = 1");
        assert_eq!(
            with_ql(base.clone(), Some("| stats count()")).ql,
            "level >= warning | stats count()"
        );
        assert_eq!(
            with_ql(base.clone(), Some("m = \"a|b\" | stats count() by m")).ql,
            "level >= warning | where m = \"a|b\" | stats count() by m"
        );
        assert_eq!(with_ql(Query::default(), Some(" a ")).ql, "a");
        for text in [
            "level >= warning | where x = 1",
            "level >= warning | stats count()",
            "level >= warning | where m = \"a|b\" | stats count() by m",
        ] {
            crate::ql::parse(text).unwrap();
        }
        let mut tile: TileCfg = toml::from_str("id='t'\ntype='annotations'\nwindow='1m'").unwrap();
        let q = tile_query(&tile, &base).unwrap();
        assert_eq!((q.from, q.window), (None, Some(TimeWindow::Live { last_ms: 60_000 })));
        tile.window = None;
        assert_eq!(tile_query(&tile, &base).unwrap().from, Some(1));
        assert_eq!(
            first_value(QlResult::Events {
                ids: vec![1, 2],
                fields: None
            })
            .1,
            Some(Value::from(2))
        );
        let empty = QlResult::Table {
            columns: vec![],
            rows: vec![],
        };
        assert_eq!(first_value(empty), (String::new(), None));
    }
}
