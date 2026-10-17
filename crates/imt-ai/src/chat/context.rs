//! What the user is looking at, sent with each turn. The UI only says WHERE they are (query, time range,
//! selected ids); the engine validates every value and fetches the details itself. Nothing from the UI is
//! trusted as data.

use crate::access::StoreRef;
use crate::assistant::{truncate_chars, EngineTools};
use serde::{Deserialize, Serialize};

pub const MAX_SELECTED: usize = 20;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatContext {
    /// The active query-language filter.
    pub query: Option<String>,
    /// The time cursor / zoomed range (epoch milliseconds).
    pub time_range: Option<(i64, i64)>,
    pub selected_ids: Vec<u32>,
    pub focused_instance: Option<String>,
    /// Which view is open (grid, timeline, charts, ...).
    pub open_view: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedContext {
    /// The grounding block put in front of the user's question.
    pub text: String,
    /// Selected ids that exist (at most [`MAX_SELECTED`]).
    pub selected: Vec<u32>,
    pub query: Option<String>,
    pub time_range: Option<(i64, i64)>,
    /// Things from the UI that were dropped, and why.
    pub notes: Vec<String>,
}

fn word(s: &str, max: usize) -> String {
    truncate_chars(&s.chars().filter(|c| !c.is_control()).collect::<String>(), max)
}

/// Validates `ctx` against the engine and renders the grounding block within `max_chars`.
pub fn resolve<'s>(ctx: &ChatContext, store: impl Into<StoreRef<'s>>, tools: &EngineTools<'_>, max_chars: usize) -> ResolvedContext {
    let store: StoreRef<'_> = store.into();
    let mut r = ResolvedContext::default();
    let mut lines: Vec<String> = Vec::new();
    if let Some(v) = ctx.open_view.as_deref().filter(|v| !v.trim().is_empty()) {
        lines.push(format!("The user has the {} view open.", word(v, 30)));
    }
    if let Some(q) = ctx.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        match store.with(|s| s.check_query(q)) {
            None => {
                lines.push(format!("Active filter: `{}`", word(q, 300)));
                r.query = Some(q.to_string());
            }
            Some(e) => r
                .notes
                .push(format!("The active filter was ignored because it is not valid: {}", e.message)),
        }
    }
    if let Some((a, b)) = ctx.time_range {
        if a < b {
            lines.push(format!(
                "Time range in view: {} to {} UTC (epoch ms {a} to {b}).",
                super::format_utc(a),
                super::format_utc(b)
            ));
            r.time_range = Some((a, b));
        } else {
            r.notes
                .push("The time range was ignored because its end is not after its start.".into());
        }
    }
    if let Some(i) = ctx.focused_instance.as_deref().map(str::trim).filter(|i| !i.is_empty()) {
        lines.push(format!("Focused instance: {}", word(i, 100)));
    }
    let mut ids: Vec<u32> = ctx.selected_ids.clone();
    ids.dedup();
    if ids.len() > MAX_SELECTED {
        r.notes.push(format!("Only the first {MAX_SELECTED} selected events are used."));
        ids.truncate(MAX_SELECTED);
    }
    let mut missing = 0;
    let mut detail: Vec<String> = Vec::new();
    for id in ids {
        match tools.event_summary(id) {
            Some(s) => {
                r.selected.push(id);
                if detail.len() < 5 {
                    detail.push(s);
                }
            }
            None => missing += 1,
        }
    }
    if missing > 0 {
        r.notes.push(format!("{missing} selected event ids do not exist and were ignored."));
    }
    if !r.selected.is_empty() {
        lines.push(format!("Selected events ({}):", r.selected.len()));
        lines.extend(detail.into_iter().map(|d| format!("  {d}")));
        if r.selected.len() > 5 {
            lines.push(format!(
                "  and {} more: {}",
                r.selected.len() - 5,
                r.selected[5..].iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(" ")
            ));
        }
    }
    if !lines.is_empty() {
        r.text = truncate_chars(&format!("[What the user is looking at]\n{}", lines.join("\n")), max_chars);
    }
    r
}
