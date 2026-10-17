//! Resolving a report for export: events looked up by id, snapshots
//! re-sanitised, and every text redacted when asked.

use super::Workspace;
use crate::redact::{hits_of, Counts, Redactor};
use crate::report::{cell_text, Block, EventLine, Report, ResolvedBlock, ResolvedReport, ResolvedSection, StoredBlock};
use anyhow::Result;
use std::borrow::Cow;

impl Workspace {
    /// The report with its events looked up and, with `redact`, every text
    /// (titles, markdown, captions, cells, chart and timeline labels, events)
    /// passed through the profile's redaction rules with a fresh salt.
    pub fn resolve_report(&self, report: &Report, redact: bool) -> Result<ResolvedReport> {
        let redactor = if redact { Some(self.export_redactor()?) } else { None };
        let mut r = Resolver {
            redactor: redactor.as_ref(),
            counts: Counts::new(),
        };
        let mut sections = Vec::with_capacity(report.sections.len());
        for s in &report.sections {
            let mut blocks = Vec::with_capacity(s.blocks.len());
            for b in &s.blocks {
                blocks.push(match b {
                    StoredBlock::Known(item) => self.resolve_block(&item.block, &mut r)?,
                    StoredBlock::Unknown(v) => ResolvedBlock::Unsupported {
                        kind: v
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("unknown")
                            .chars()
                            .take(40)
                            .collect(),
                    },
                });
            }
            sections.push(ResolvedSection {
                title: r.text(&s.title),
                blocks,
            });
        }
        let title = r.text(&report.title);
        let summary = r.text(&report.summary);
        Ok(ResolvedReport {
            title,
            summary,
            generated_at: chrono::Utc::now().timestamp_millis(),
            redacted: redactor.is_some(),
            redaction: redactor.as_ref().map(|x| hits_of(x, &r.counts)).unwrap_or_default(),
            sections,
        })
    }

    fn resolve_block(&self, block: &Block, r: &mut Resolver<'_>) -> Result<ResolvedBlock> {
        Ok(match block {
            Block::Text { markdown } => ResolvedBlock::Text {
                markdown: r.text(markdown),
            },
            Block::Events { caption, ids } => {
                let mut events = Vec::new();
                let mut missing = 0;
                for &id in ids {
                    let Some(rec) = self.records.get(id as usize).filter(|x| x.id == id) else {
                        missing += 1;
                        continue;
                    };
                    let rec = match r.redactor {
                        Some(x) => x.record(rec, &mut r.counts),
                        None => Cow::Borrowed(rec),
                    };
                    events.push(EventLine {
                        id,
                        ts: rec.ts,
                        level: rec.level.as_str().into(),
                        source: self.source_name(rec.source).into(),
                        message: rec.message.clone(),
                        exception: rec.exception.clone(),
                    });
                }
                ResolvedBlock::Events {
                    caption: r.text(caption),
                    events,
                    missing,
                }
            }
            Block::Chart { caption, title, spec, svg } => ResolvedBlock::Figure {
                caption: r.text(caption),
                title: r.text(if title.is_empty() {
                    spec.as_ref().map(|s| s.title.as_str()).unwrap_or("")
                } else {
                    title
                }),
                svg: r.svg(svg.as_deref())?,
            },
            Block::Timeline { caption, svg, .. } => ResolvedBlock::Figure {
                caption: r.text(caption),
                title: "Timeline".into(),
                svg: r.svg(svg.as_deref())?,
            },
            Block::Table {
                caption, columns, rows, ..
            } => ResolvedBlock::Table {
                caption: r.text(caption),
                columns: columns.iter().map(|c| r.text(c)).collect(),
                rows: rows.iter().map(|row| row.iter().map(|v| r.text(&cell_text(v))).collect()).collect(),
            },
            Block::Finding { severity, title, detail } => ResolvedBlock::Finding {
                severity: severity.clone(),
                title: r.text(title),
                detail: r.text(detail),
            },
            Block::Annotations { caption, items } => ResolvedBlock::Annotations {
                caption: r.text(caption),
                items: items
                    .iter()
                    .map(|a| crate::report::AnnotationItem {
                        label: r.text(&a.label),
                        ..a.clone()
                    })
                    .collect(),
            },
        })
    }
}

/// Redaction state while resolving one report.
struct Resolver<'a> {
    redactor: Option<&'a Redactor>,
    counts: Counts,
}

impl Resolver<'_> {
    fn text(&mut self, t: &str) -> String {
        match self.redactor {
            Some(x) => x.apply(t, &mut self.counts).into_owned(),
            None => t.to_string(),
        }
    }

    fn svg(&mut self, svg: Option<&str>) -> Result<Option<String>> {
        svg.map(|s| crate::svg::sanitize_text(s, &crate::svg::FIGURE, &mut |t| self.text(t)))
            .transpose()
    }
}
