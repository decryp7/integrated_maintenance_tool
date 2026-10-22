//! Dashboards: per-profile grids of tiles (`[[dashboard]]` in the profile,
//! see `default_profile.toml`). This module holds the configuration and its
//! validation; the workspace evaluates tiles (`Workspace::dashboard_tile`).

use crate::api::{ChartData, Facet};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Widest grid a dashboard may use.
pub const MAX_COLUMNS: u32 = 24;
/// Most tiles on one dashboard.
pub const MAX_TILES: usize = 64;
/// Most dashboards in a profile.
pub const MAX_DASHBOARDS: usize = 32;
/// Most rows a table tile returns.
pub const MAX_TABLE_ROWS: usize = 100;
/// Points of a stat tile's sparkline.
pub const SPARKLINE_POINTS: usize = 24;

/// `[[dashboard]]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardCfg {
    pub id: String,
    pub title: String,
    /// Grid columns (tiles are placed in column units).
    #[serde(default = "default_columns")]
    pub columns: u32,
    /// `[[dashboard.tile]]`.
    #[serde(default)]
    pub tile: Vec<TileCfg>,
}

fn default_columns() -> u32 {
    12
}

/// What a tile shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileKind {
    /// A chart: a profile `[[chart]]` by id, or inline `value` / `by`.
    Chart,
    /// One number from a query-language `stats` query, with a sparkline.
    Stat,
    /// A query-language result table.
    Table,
    /// The most frequent values of a field.
    Top,
    /// Anomalies of the tile's window (the live alert detectors).
    Alerts,
    /// Plugin annotations (reboots, service lifetimes...) in the window.
    Annotations,
}

/// `[[dashboard.tile]]`: position and size in grid units plus the keys of its kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TileCfg {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "type")]
    pub kind: TileKind,
    #[serde(default)]
    pub x: u32,
    #[serde(default)]
    pub y: u32,
    #[serde(default = "default_w")]
    pub w: u32,
    #[serde(default = "default_h")]
    pub h: u32,
    /// stat/table: a query-language program; chart/top/alerts: a filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// chart: id of a profile `[[chart]]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart: Option<String>,
    /// chart: inline aggregate (`count()`, `avg(cpu)`...) when there is no `chart`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// chart: inline split fields.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by: Vec<String>,
    /// chart: line, bar or stacked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// top: the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// top/table/alerts/annotations: most items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    /// stat: unit shown next to the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// A sliding window ("15m") of its own; absent = the dashboard's query
    /// (the window's filters and time range, including its live window).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
}

fn default_w() -> u32 {
    4
}
fn default_h() -> u32 {
    3
}

impl TileCfg {
    /// The window in ms, when the tile has one.
    pub fn window_ms(&self) -> Result<Option<i64>> {
        self.window
            .as_deref()
            .map(crate::workspace::duration_ms)
            .transpose()
            .with_context(|| format!("tile \"{}\": window", self.id))
    }

    /// `limit`, clamped to `1..=max` (default `default`).
    pub fn limit_or(&self, default: usize, max: usize) -> usize {
        self.limit.unwrap_or(default).clamp(1, max)
    }

    /// Checks the keys its kind needs and that it fits a grid of `columns`.
    pub fn validate(&self, columns: u32) -> Result<()> {
        let id = &self.id;
        if id.trim().is_empty() {
            bail!("a tile needs an id");
        }
        if self.w == 0 || self.h == 0 || self.h > 48 {
            bail!("tile \"{id}\": w and h must be at least 1 (h at most 48)");
        }
        if self.x.saturating_add(self.w) > columns || self.y > 1000 {
            bail!("tile \"{id}\": x + w must be at most the dashboard's {columns} columns");
        }
        self.window_ms()?;
        let program = |required: bool| -> Result<Option<crate::ql::Program>> {
            match self.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
                Some(q) => crate::ql::parse(q)
                    .map(Some)
                    .map_err(|e| anyhow!("tile \"{id}\": query: {}", e.message)),
                None if required => Err(anyhow!("tile \"{id}\": a {} tile needs a query", kind_name(self.kind))),
                None => Ok(None),
            }
        };
        match self.kind {
            TileKind::Chart => {
                program(false)?;
                match (&self.chart, &self.value) {
                    (Some(_), _) => {}
                    (None, Some(v)) => {
                        crate::chart::parse_y(v).with_context(|| format!("tile \"{id}\": value"))?;
                    }
                    (None, None) => bail!("tile \"{id}\": a chart tile needs `chart` (a [[chart]] id) or `value`"),
                }
                if let Some(s) = &self.style {
                    if !matches!(s.as_str(), "line" | "bar" | "stacked") {
                        bail!("tile \"{id}\": style must be line, bar or stacked");
                    }
                }
            }
            TileKind::Stat => {
                let p = program(true)?.unwrap_or_else(|| unreachable!("required"));
                if !p.stages.iter().any(|s| matches!(s, crate::ql::Stage::Stats { .. })) {
                    bail!("tile \"{id}\": a stat tile needs a `| stats ...` query");
                }
            }
            TileKind::Table => {
                program(true)?;
            }
            TileKind::Top => {
                program(false)?;
                if self.field.as_deref().is_none_or(|f| f.trim().is_empty()) {
                    bail!("tile \"{id}\": a top tile needs a `field`");
                }
            }
            TileKind::Alerts => {
                program(false)?;
            }
            TileKind::Annotations => {}
        }
        Ok(())
    }
}

fn kind_name(k: TileKind) -> &'static str {
    match k {
        TileKind::Chart => "chart",
        TileKind::Stat => "stat",
        TileKind::Table => "table",
        TileKind::Top => "top",
        TileKind::Alerts => "alerts",
        TileKind::Annotations => "annotations",
    }
}

impl DashboardCfg {
    pub fn validate(&self) -> Result<()> {
        let id = &self.id;
        if id.trim().is_empty() {
            bail!("a [[dashboard]] needs an id");
        }
        if !(1..=MAX_COLUMNS).contains(&self.columns) {
            bail!("dashboard \"{id}\": columns must be 1 to {MAX_COLUMNS}");
        }
        if self.tile.len() > MAX_TILES {
            bail!("dashboard \"{id}\": at most {MAX_TILES} tiles");
        }
        let mut seen = std::collections::HashSet::new();
        for t in &self.tile {
            t.validate(self.columns).with_context(|| format!("dashboard \"{id}\""))?;
            if !seen.insert(t.id.as_str()) {
                bail!("dashboard \"{id}\": tile id \"{}\" is used twice", t.id);
            }
        }
        Ok(())
    }
}

/// Validates every dashboard of a profile.
pub fn validate_all(dashboards: &[DashboardCfg]) -> Result<()> {
    if dashboards.len() > MAX_DASHBOARDS {
        bail!("at most {MAX_DASHBOARDS} [[dashboard]] sections");
    }
    let mut seen = std::collections::HashSet::new();
    for d in dashboards {
        d.validate()?;
        if !seen.insert(d.id.as_str()) {
            bail!("dashboard id \"{}\" is used twice", d.id);
        }
    }
    Ok(())
}

/// `dashboard_tile`: one tile evaluated over a base query.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TileRequest {
    pub tile: TileCfg,
    /// The window's query (filters, time range or live window).
    #[serde(default)]
    pub query: crate::query::Query,
}

/// One point of a sparkline: bucket start and value (none when empty).
pub type SparkPoint = (i64, Option<f64>);

/// A tile's data, by kind.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TileData {
    Chart {
        data: ChartData,
    },
    Stat {
        /// The value when it is a number.
        value: Option<f64>,
        /// The value as text (numbers formatted by the UI from `value`).
        text: String,
        /// The column it came from (`count()`, an alias...).
        label: String,
        unit: Option<String>,
        sparkline: Vec<SparkPoint>,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
        truncated: bool,
    },
    Top {
        facet: Facet,
    },
    Alerts {
        items: Vec<crate::analysis::Anomaly>,
    },
    Annotations {
        items: Vec<crate::plugin::annotate::AnnotationDto>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(kind: TileKind) -> TileCfg {
        TileCfg {
            id: "t".into(),
            title: String::new(),
            kind,
            x: 0,
            y: 0,
            w: 4,
            h: 3,
            query: None,
            chart: None,
            value: None,
            by: vec![],
            style: None,
            field: None,
            limit: None,
            unit: None,
            window: None,
        }
    }

    fn err(t: &TileCfg) -> String {
        format!("{:#}", t.validate(12).unwrap_err())
    }

    #[test]
    fn tiles_need_the_keys_of_their_kind() {
        let mut chart = tile(TileKind::Chart);
        assert!(err(&chart).contains("needs `chart`"));
        chart.chart = Some("events-by-level".into());
        chart.validate(12).unwrap();
        chart.chart = None;
        chart.value = Some("avg()".into());
        assert!(err(&chart).contains("value"));
        chart.value = Some("avg(cpu)".into());
        chart.style = Some("pie".into());
        assert!(err(&chart).contains("style"));
        chart.style = Some("bar".into());
        chart.query = Some("level >= warning".into());
        chart.validate(12).unwrap();

        let mut stat = tile(TileKind::Stat);
        assert!(err(&stat).contains("needs a query"));
        stat.query = Some("  ".into());
        assert!(err(&stat).contains("needs a query"));
        stat.query = Some("level = error".into());
        assert!(err(&stat).contains("stats"));
        stat.query = Some("level = error | stats count(".into());
        assert!(err(&stat).contains("query:"));
        stat.query = Some("level = error | stats count()".into());
        stat.validate(12).unwrap();

        let mut table = tile(TileKind::Table);
        assert!(err(&table).contains("needs a query"));
        table.query = Some("| stats count() by source".into());
        table.validate(12).unwrap();

        let mut top = tile(TileKind::Top);
        assert!(err(&top).contains("field"));
        top.field = Some("SourceContext".into());
        top.validate(12).unwrap();

        tile(TileKind::Alerts).validate(12).unwrap();
        tile(TileKind::Annotations).validate(12).unwrap();
    }

    #[test]
    fn tiles_fit_the_grid_and_windows_parse() {
        let mut t = tile(TileKind::Annotations);
        t.x = 10;
        assert!(err(&t).contains("columns"));
        t.x = 0;
        t.w = 0;
        assert!(err(&t).contains("at least 1"));
        t.w = 2;
        t.window = Some("soon".into());
        assert!(err(&t).contains("window"));
        t.window = Some("15m".into());
        assert_eq!(t.window_ms().unwrap(), Some(900_000));
        t.id = " ".into();
        assert!(err(&t).contains("needs an id"));
        assert_eq!(tile(TileKind::Top).limit_or(10, 50), 10);
        let mut l = tile(TileKind::Top);
        l.limit = Some(500);
        assert_eq!(l.limit_or(10, 50), 50);
        for k in [
            TileKind::Chart,
            TileKind::Stat,
            TileKind::Table,
            TileKind::Top,
            TileKind::Alerts,
            TileKind::Annotations,
        ] {
            assert!(!kind_name(k).is_empty());
        }
    }

    #[test]
    fn dashboards_validate_ids_columns_and_tiles() {
        let toml = r#"
[[dashboard]]
id = "d"
title = "D"
[[dashboard.tile]]
id = "a"
type = "annotations"
[[dashboard.tile]]
id = "b"
type = "top"
field = "x"
x = 4
"#;
        #[derive(Deserialize)]
        struct P {
            dashboard: Vec<DashboardCfg>,
        }
        let mut p: P = toml::from_str(toml).unwrap();
        assert_eq!(p.dashboard[0].columns, 12);
        assert_eq!((p.dashboard[0].tile[1].w, p.dashboard[0].tile[1].h), (4, 3));
        validate_all(&p.dashboard).unwrap();

        let mut d = p.dashboard[0].clone();
        d.tile[1].id = "a".into();
        assert!(d.validate().unwrap_err().to_string().contains("used twice"));
        d.columns = 0;
        assert!(d.validate().unwrap_err().to_string().contains("columns"));
        d.columns = 12;
        d.id = String::new();
        assert!(d.validate().unwrap_err().to_string().contains("needs an id"));
        d.id = "d".into();
        d.tile = vec![tile(TileKind::Annotations); MAX_TILES + 1];
        assert!(d.validate().unwrap_err().to_string().contains("at most"));
        d.tile = vec![tile(TileKind::Top)];
        assert!(format!("{:#}", d.validate().unwrap_err()).contains("dashboard \"d\""));

        p.dashboard.push(p.dashboard[0].clone());
        assert!(validate_all(&p.dashboard).unwrap_err().to_string().contains("used twice"));
        let many = vec![p.dashboard[0].clone(); MAX_DASHBOARDS + 1];
        assert!(validate_all(&many).unwrap_err().to_string().contains("at most"));
    }
}
