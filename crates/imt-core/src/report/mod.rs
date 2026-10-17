//! Incident reports: a versioned document of sections and blocks (text,
//! events, charts, timeline snapshots, query tables, findings,
//! annotations), saved in the project and exported as self-contained HTML
//! (here) or PDF (the `imt-pdf` adapter).
//!
//! Flow: the UI edits a [`Report`]; at export the workspace resolves it into
//! a [`ResolvedReport`] (events looked up by id, every text redacted when
//! asked), which a [`ReportRenderer`] writes.

pub mod html;
pub mod markdown;
#[cfg(test)]
mod tests;

use crate::chart::ChartDef;
use crate::redact::RuleHits;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Current report format. Newer reports are read; blocks of unknown types
/// are kept as they are (and shown as unsupported).
pub const REPORT_VERSION: u32 = 1;
/// Most sections in a report.
pub const MAX_SECTIONS: usize = 100;
/// Most blocks in a section.
pub const MAX_BLOCKS: usize = 200;
/// Most event ids in one events block.
pub const MAX_EVENT_IDS: usize = 1000;
/// Most rows in a table block.
pub const MAX_TABLE_ROWS: usize = 1000;
/// Most bytes of a text field (markdown, captions...).
pub const MAX_TEXT: usize = 256 * 1024;

fn version_1() -> u32 {
    REPORT_VERSION
}

/// An incident report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    #[serde(default = "version_1")]
    pub version: u32,
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// Markdown.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub blocks: Vec<StoredBlock>,
}

/// A block as stored: known, or kept verbatim (a newer IMT's block type, or
/// a damaged one) so saving never loses it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StoredBlock {
    Known(BlockItem),
    Unknown(Value),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockItem {
    pub id: String,
    #[serde(flatten)]
    pub block: Block,
}

/// One annotation in an annotations block (a snapshot of what the timeline showed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationItem {
    pub from: i64,
    #[serde(default)]
    pub to: Option<i64>,
    pub label: String,
    #[serde(default)]
    pub kind: String,
}

/// What a block holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Block {
    /// Markdown (the subset of [`markdown`]).
    Text {
        #[serde(default)]
        markdown: String,
    },
    /// Events by workspace id (bookmarks, a selection); resolved and redacted at export.
    Events {
        #[serde(default)]
        caption: String,
        ids: Vec<u32>,
    },
    /// A chart: its definition and the SVG the UI rendered.
    Chart {
        #[serde(default)]
        caption: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        spec: Option<ChartDef>,
        #[serde(default)]
        svg: Option<String>,
    },
    /// The timeline as SVG (see the UI's timeline serializer).
    Timeline {
        #[serde(default)]
        caption: String,
        #[serde(default)]
        svg: Option<String>,
        #[serde(default)]
        from: Option<i64>,
        #[serde(default)]
        to: Option<i64>,
    },
    /// A query-language result.
    Table {
        #[serde(default)]
        caption: String,
        #[serde(default)]
        query: String,
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
    /// A health-check finding.
    Finding {
        #[serde(default)]
        severity: String,
        title: String,
        #[serde(default)]
        detail: String,
    },
    Annotations {
        #[serde(default)]
        caption: String,
        items: Vec<AnnotationItem>,
    },
}

impl Block {
    /// The type name, as in the JSON `type`.
    pub fn kind(&self) -> &'static str {
        match self {
            Block::Text { .. } => "text",
            Block::Events { .. } => "events",
            Block::Chart { .. } => "chart",
            Block::Timeline { .. } => "timeline",
            Block::Table { .. } => "table",
            Block::Finding { .. } => "finding",
            Block::Annotations { .. } => "annotations",
        }
    }
}

fn check_text(what: &str, text: &str) -> Result<()> {
    if text.len() > MAX_TEXT {
        bail!("{what} is longer than {} KB", MAX_TEXT / 1024);
    }
    Ok(())
}

impl Report {
    /// Reads a report from untrusted JSON (the UI, a project, a bundle):
    /// checks the limits and sanitises its SVG snapshots.
    pub fn parse(value: Value) -> Result<Report> {
        let mut report: Report = serde_json::from_value(value).context("not a valid report")?;
        report.check()?;
        Ok(report)
    }

    fn check(&mut self) -> Result<()> {
        if self.id.trim().is_empty() {
            bail!("a report needs an id");
        }
        check_text("the title", &self.title)?;
        check_text("the summary", &self.summary)?;
        if self.sections.len() > MAX_SECTIONS {
            bail!("a report has at most {MAX_SECTIONS} sections");
        }
        for s in &mut self.sections {
            check_text("a section title", &s.title)?;
            if s.blocks.len() > MAX_BLOCKS {
                bail!("a section has at most {MAX_BLOCKS} blocks");
            }
            for b in &mut s.blocks {
                if let StoredBlock::Known(item) = b {
                    check_block(&mut item.block).with_context(|| format!("block \"{}\"", item.id))?;
                }
            }
        }
        Ok(())
    }
}

fn check_block(block: &mut Block) -> Result<()> {
    match block {
        Block::Text { markdown } => check_text("the text", markdown),
        Block::Events { caption, ids } => {
            check_text("the caption", caption)?;
            if ids.len() > MAX_EVENT_IDS {
                bail!("at most {MAX_EVENT_IDS} events per block");
            }
            Ok(())
        }
        Block::Chart { caption, title, svg, .. } => {
            check_text("the caption", caption)?;
            check_text("the title", title)?;
            sanitize_svg(svg)
        }
        Block::Timeline { caption, svg, .. } => {
            check_text("the caption", caption)?;
            sanitize_svg(svg)
        }
        Block::Table { caption, query, rows, .. } => {
            check_text("the caption", caption)?;
            check_text("the query", query)?;
            if rows.len() > MAX_TABLE_ROWS {
                bail!("at most {MAX_TABLE_ROWS} table rows");
            }
            Ok(())
        }
        Block::Finding { title, detail, .. } => {
            check_text("the title", title)?;
            check_text("the detail", detail)
        }
        Block::Annotations { caption, items } => {
            check_text("the caption", caption)?;
            if items.len() > MAX_EVENT_IDS {
                bail!("at most {MAX_EVENT_IDS} annotations per block");
            }
            Ok(())
        }
    }
}

fn sanitize_svg(svg: &mut Option<String>) -> Result<()> {
    if let Some(s) = svg {
        *s = crate::svg::sanitize(s, &crate::svg::FIGURE)?;
    }
    Ok(())
}

/// Reports of a project: each is read leniently; one that cannot be read is
/// skipped instead of failing the whole project.
pub fn tolerant_reports<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<Report>, D::Error> {
    let raw: Vec<Value> = Vec::deserialize(d).unwrap_or_default();
    Ok(raw.into_iter().filter_map(|v| Report::parse(v).ok()).collect())
}

/// One event in an exported report.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventLine {
    pub id: u32,
    pub ts: Option<i64>,
    pub level: String,
    pub source: String,
    pub message: String,
    pub exception: Option<String>,
}

/// A block with everything looked up and redacted, ready to draw.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ResolvedBlock {
    Text {
        markdown: String,
    },
    Events {
        caption: String,
        events: Vec<EventLine>,
        /// Ids no longer in the workspace.
        missing: usize,
    },
    /// A chart or timeline snapshot.
    Figure {
        caption: String,
        title: String,
        /// Sanitised SVG.
        svg: Option<String>,
    },
    Table {
        caption: String,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Finding {
        severity: String,
        title: String,
        detail: String,
    },
    Annotations {
        caption: String,
        items: Vec<AnnotationItem>,
    },
    /// A block this IMT does not know.
    Unsupported {
        kind: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedSection {
    pub title: String,
    pub blocks: Vec<ResolvedBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedReport {
    pub title: String,
    pub summary: String,
    pub generated_at: i64,
    pub redacted: bool,
    /// Matches per redaction rule (every rule, zero included) when redacted.
    pub redaction: Vec<RuleHits>,
    pub sections: Vec<ResolvedSection>,
}

/// Colours of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

/// An output format for reports.
pub trait ReportRenderer: Send + Sync {
    /// `"html"`, `"pdf"`.
    fn id(&self) -> &'static str;
    fn extension(&self) -> &'static str;
    fn render(&self, report: &ResolvedReport, theme: Theme, out: &mut dyn std::io::Write) -> Result<()>;
}

/// UTC timestamp as `2026-10-14 10:00:00.123Z` (reports are read anywhere).
pub fn format_ts(ts: Option<i64>) -> String {
    use chrono::TimeZone;
    ts.and_then(|ms| chrono::Utc.timestamp_millis_opt(ms).single())
        .map(|t| t.format("%Y-%m-%d %H:%M:%S%.3fZ").to_string())
        .unwrap_or_else(|| "—".into())
}

/// A table cell as text.
pub fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        other => crate::model::value_text(other).into_owned(),
    }
}
