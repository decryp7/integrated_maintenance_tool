//! The incident summary (ADR 0039): one call turns the open data into a [`Report`] the existing report pipeline
//! (ADR 0007) edits, exports as HTML or PDF and redacts. The engine computes every fact here: the time span and
//! levels, key moments, the top anomalies, exceptions and error patterns with sample events as evidence, the sources
//! and hosts with most errors, import problems, matched known issues and the project's notes. Nothing is written
//! by a model; the optional AI narrative is a separate, marked block the caller adds.

use crate::analysis::{Anomaly, AnomalyKind};
use crate::api::{QueryRequest, Severity};
use crate::known_issues::IssueMatch;
use crate::ports::{Store, StoreResult};
use crate::ql::QlResult;
use crate::query::Query;
use crate::report::{AnnotationItem, Block, BlockItem, Report, Section, StoredBlock, MAX_EVENT_IDS, MAX_TABLE_ROWS, REPORT_VERSION};
use serde::{Deserialize, Serialize};

/// Entries per ranking, at most.
pub const MAX_TOP: usize = 10;
const DEFAULT_TOP: usize = 5;
const MAX_MOMENTS: usize = 20;
const MAX_TITLE: usize = 200;
const MAX_NOTES: usize = 64 * 1024;
const HOST_FIELDS: [&str; 5] = ["host", "hostname", "machinename", "machine", "computer"];

/// What the user asks of the incident summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IncidentOptions {
    /// The new report's id (the caller makes it; it must not be empty).
    pub id: String,
    pub title: String,
    /// Only events from this time (epoch ms).
    pub from: Option<i64>,
    /// Only events up to this time (epoch ms).
    pub to: Option<i64>,
    /// Entries per ranking (1 to 10; 5 when absent).
    pub top: Option<usize>,
    /// The project's notes as Markdown (the shell reads them; the engine only places them).
    pub notes: String,
    /// Matched known issues (from `match_known_issues`).
    #[serde(skip)]
    pub issues: Vec<IssueMatch>,
    /// When the report is made (epoch ms).
    pub now: i64,
}

fn clock(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map_or_else(|| ms.to_string(), |t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
}

fn severity_word(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "critical",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

struct Builder {
    n: usize,
    sections: Vec<Section>,
}

impl Builder {
    fn section(&mut self, title: &str, blocks: Vec<Block>) {
        if blocks.is_empty() {
            return;
        }
        let id = format!("s{}", self.sections.len() + 1);
        let blocks = blocks
            .into_iter()
            .map(|block| {
                self.n += 1;
                StoredBlock::Known(BlockItem {
                    id: format!("b{}", self.n),
                    block,
                })
            })
            .collect();
        self.sections.push(Section {
            id,
            title: title.to_string(),
            blocks,
        });
    }
}

fn events(caption: &str, ids: impl IntoIterator<Item = u32>) -> Option<Block> {
    let mut ids: Vec<u32> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    ids.truncate(MAX_EVENT_IDS);
    (!ids.is_empty()).then(|| Block::Events {
        caption: caption.to_string(),
        ids,
    })
}

fn table(caption: &str, query: &str, columns: &[&str], mut rows: Vec<Vec<serde_json::Value>>) -> Option<Block> {
    rows.truncate(MAX_TABLE_ROWS);
    (!rows.is_empty()).then(|| Block::Table {
        caption: caption.to_string(),
        query: query.to_string(),
        columns: columns.iter().map(|c| (*c).to_string()).collect(),
        rows,
    })
}

fn request(opts: &IncidentOptions, ql: &str) -> QueryRequest {
    QueryRequest {
        query: Query {
            ql: ql.to_string(),
            from: opts.from,
            to: opts.to,
            ..Query::default()
        },
    }
}

/// A `stats` ranking as a table block (empty when the query fails or finds nothing: the report just omits it).
fn ranking(store: &dyn Store, opts: &IncidentOptions, caption: &str, ql: &str) -> Option<Block> {
    match store.run_query(&request(opts, ql)) {
        Ok(QlResult::Table { columns, rows }) => {
            let cols: Vec<&str> = columns.iter().map(String::as_str).collect();
            table(caption, ql, &cols, rows)
        }
        _ => None,
    }
}

/// The incident summary of the open data.
pub fn build_incident_report(store: &dyn Store, opts: &IncidentOptions) -> StoreResult<Report> {
    let id = opts.id.trim();
    if id.is_empty() {
        return Err(crate::ports::StoreError::Invalid("a report needs an id".into()));
    }
    let top = opts.top.unwrap_or(DEFAULT_TOP).clamp(1, MAX_TOP);
    let summary = store.summary()?;
    let mut b = Builder {
        n: 0,
        sections: Vec::new(),
    };

    // Summary: the span, the levels and the sources, as plain facts.
    let mut facts = Vec::new();
    let (from, to) = (opts.from.or(summary.from), opts.to.or(summary.to));
    match (from, to) {
        (Some(f), Some(t)) => facts.push(format!("**Period:** {} to {}", clock(f), clock(t))),
        _ => facts.push("**Period:** the events carry no timestamps".into()),
    }
    facts.push(format!("**Events:** {} ({} without a timestamp)", summary.total, summary.untimed));
    let levels: Vec<String> = summary
        .levels
        .iter()
        .filter(|(_, n)| **n > 0)
        .map(|(l, n)| format!("{l:?} {n}"))
        .collect();
    if !levels.is_empty() {
        facts.push(format!("**Levels:** {}", levels.join(", ")));
    }
    facts.push(format!("**Sources:** {}", summary.sources.len()));
    let intro = vec![Block::Text {
        markdown: facts.join("  \n"),
    }];
    b.section("Summary", intro);

    // Analyses: each is optional; one failing analysis leaves its section out and never the whole report.
    let anomalies: Vec<Anomaly> = store.anomalies(&request(opts, "")).map(|a| a.result).unwrap_or_default();
    let groups = store.exception_groups(&request(opts, "")).map(|a| a.result).unwrap_or_default();
    let patterns = store
        .patterns(&request(opts, "level >= error"))
        .map(|a| a.result)
        .unwrap_or_default();

    // Key moments: anomaly windows and the first time each exception type appeared, in time order.
    let mut moments: Vec<AnnotationItem> = anomalies
        .iter()
        .map(|a| AnnotationItem {
            from: a.from,
            to: (a.to > a.from).then_some(a.to),
            label: a.message.clone(),
            kind: "anomaly".into(),
        })
        .collect();
    moments.extend(groups.iter().filter_map(|g| {
        g.first_ts.map(|t| AnnotationItem {
            from: t,
            to: None,
            label: format!("First {}", g.type_name),
            kind: "exception".into(),
        })
    }));
    moments.sort_by_key(|m| m.from);
    moments.truncate(MAX_MOMENTS);
    if !moments.is_empty() {
        b.section(
            "Key moments",
            vec![Block::Annotations {
                caption: "Anomaly windows and the first occurrence of each exception type".into(),
                items: moments,
            }],
        );
    }

    // Anomalies, worst first.
    let mut ranked: Vec<&Anomaly> = anomalies.iter().collect();
    ranked.sort_by(|a, c| c.score.total_cmp(&a.score));
    let mut blocks = Vec::new();
    for a in ranked.iter().take(top) {
        blocks.push(Block::Finding {
            severity: if a.kind == AnomalyKind::Crash { "critical" } else { "warning" }.into(),
            title: a.message.clone(),
            detail: format!(
                "{:?}, {} to {}{}",
                a.kind,
                clock(a.from),
                clock(a.to),
                a.instance.as_ref().map_or(String::new(), |i| format!(", {i}"))
            ),
        });
    }
    blocks.extend(events(
        "Evidence: events that show the anomalies",
        ranked.iter().take(top).flat_map(|a| a.sample_ids.clone()),
    ));
    b.section("Top anomalies", blocks);

    // Exceptions, most frequent first.
    let mut ranked: Vec<_> = groups.iter().collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.count));
    let rows = ranked
        .iter()
        .take(top)
        .map(|g| {
            vec![
                serde_json::json!(g.type_name),
                serde_json::json!(g.message),
                serde_json::json!(g.count),
                serde_json::json!(g.first_ts.map(clock)),
                serde_json::json!(g.last_ts.map(clock)),
                serde_json::json!(g.top_frame),
            ]
        })
        .collect();
    let mut blocks: Vec<Block> = table(
        "Most frequent exceptions",
        "exception groups",
        &["type", "message", "count", "first", "last", "top frame"],
        rows,
    )
    .into_iter()
    .collect();
    blocks.extend(events(
        "Evidence: one event of each exception group",
        ranked.iter().take(top).flat_map(|g| g.sample_ids.first().copied()),
    ));
    b.section("Top exceptions", blocks);

    // Error patterns, most frequent first.
    let mut ranked: Vec<_> = patterns.iter().collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.count));
    let rows = ranked
        .iter()
        .take(top)
        .map(|p| {
            vec![
                serde_json::json!(p.template),
                serde_json::json!(p.count),
                serde_json::json!(p.first_ts.map(clock)),
            ]
        })
        .collect();
    let mut blocks: Vec<Block> = table(
        "Most frequent error messages",
        "patterns of level >= error",
        &["message pattern", "count", "first"],
        rows,
    )
    .into_iter()
    .collect();
    blocks.extend(events(
        "Evidence: one event of each pattern",
        ranked.iter().take(top).flat_map(|p| p.sample_ids.first().copied()),
    ));
    b.section("Top problems", blocks);

    // Affected sources and hosts.
    let mut blocks = Vec::new();
    blocks.extend(ranking(
        store,
        opts,
        "Sources with most errors",
        &format!("level >= error | stats count() as errors by @source | sort errors desc | head {top}"),
    ));
    if let Some(f) = summary
        .fields
        .iter()
        .find(|f| HOST_FIELDS.contains(&f.name.to_ascii_lowercase().as_str()))
    {
        blocks.extend(ranking(
            store,
            opts,
            &format!("Hosts with most errors ({})", f.name),
            &format!(
                "level >= error | stats count() as errors by {} | sort errors desc | head {top}",
                f.name
            ),
        ));
    }
    b.section("Affected sources and hosts", blocks);

    // Import problems: what could not be read, so the reader knows what the data lacks.
    let blocks: Vec<Block> = store
        .problems()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.count > 0)
        .take(top * 2)
        .map(|p| Block::Finding {
            severity: severity_word(p.severity).into(),
            title: format!("{}: {} ({})", p.source_name, p.kind.id(), p.count),
            detail: p.detail.unwrap_or_default(),
        })
        .collect();
    b.section("Import problems", blocks);

    // Known issues, with the fix and the evidence.
    let mut blocks = Vec::new();
    for m in opts.issues.iter().take(top * 2) {
        blocks.push(Block::Finding {
            severity: "info".into(),
            title: format!("{} ({} events)", m.issue.title, m.count),
            detail: format!("Fix: {}\n\nFrom knowledge file {}. Query: `{}`", m.issue.fix, m.issue.file, m.query),
        });
    }
    blocks.extend(events(
        "Evidence: events that match known issues",
        opts.issues.iter().take(top * 2).flat_map(|m| m.sample_ids.clone()),
    ));
    b.section("Known issues", blocks);

    let notes: String = opts.notes.chars().take(MAX_NOTES).collect();
    if !notes.trim().is_empty() {
        b.section("Notes", vec![Block::Text { markdown: notes }]);
    }

    let title = if opts.title.trim().is_empty() {
        "Incident summary"
    } else {
        opts.title.trim()
    };
    Ok(Report {
        version: REPORT_VERSION,
        id: id.to_string(),
        title: title.chars().take(MAX_TITLE).collect(),
        summary: format!(
            "{} events, {} sources; generated by the engine from the open data.",
            summary.total,
            summary.sources.len()
        ),
        created_at: opts.now,
        updated_at: opts.now,
        sections: b.sections,
    })
}

/// The facts of a report as plain text for a model to write up (and the event ids it may cite), at most `max_chars`
/// characters. Only what the report holds goes in; the text is data, never instructions.
pub fn digest(report: &Report, max_chars: usize) -> (String, Vec<u32>) {
    let mut out = String::new();
    let mut ids: Vec<u32> = Vec::new();
    'all: for s in &report.sections {
        out.push_str(&format!("## {}\n", s.title));
        for b in &s.blocks {
            let StoredBlock::Known(item) = b else { continue };
            let line = match &item.block {
                Block::Text { markdown } if s.title != "Notes" => format!("{markdown}\n"),
                Block::Text { .. } => "(the investigator's notes are in the report)\n".to_string(),
                Block::Finding { severity, title, detail } => format!("- [{severity}] {title}: {detail}\n"),
                Block::Table {
                    caption, columns, rows, ..
                } => {
                    let mut t = format!("{caption} ({}):\n", columns.join(" | "));
                    for r in rows.iter().take(MAX_TOP) {
                        let cells: Vec<String> = r.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string)).collect();
                        t.push_str(&format!("- {}\n", cells.join(" | ")));
                    }
                    t
                }
                Block::Annotations { items, .. } => items.iter().map(|i| format!("- {}: {}\n", clock(i.from), i.label)).collect(),
                Block::Events { caption, ids: e } => {
                    ids.extend(e.iter().copied());
                    format!(
                        "{caption}: events {}\n",
                        e.iter().take(12).map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
                    )
                }
                _ => continue,
            };
            if out.chars().count() + line.chars().count() > max_chars {
                out.push_str("(cut: the report is longer)\n");
                break 'all;
            }
            out.push_str(&line);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    (out, ids)
}
