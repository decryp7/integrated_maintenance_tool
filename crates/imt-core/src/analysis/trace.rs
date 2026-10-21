//! Following one request through the logs: correlation by id, spans built from
//! trace fields and a request waterfall.

use super::util::truncate;
use crate::model::Level;
use crate::row::RowRef;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

const TRACE_FIELDS: [&str; 3] = ["TraceId", "trace_id", "@tr"];
const SPAN_FIELDS: [&str; 3] = ["SpanId", "span_id", "@sp"];
const PARENT_FIELDS: [&str; 3] = ["ParentSpanId", "ParentId", "parent_span_id"];
/// An event's id as the DTOs carry it (saturating until 6c widens them).
fn sid(r: &dyn RowRef) -> u32 {
    u32::try_from(r.id()).unwrap_or(u32::MAX)
}

const DURATION_FIELDS: [&str; 4] = ["ElapsedMs", "Elapsed", "DurationMs", "Duration"];

/// Every field [`spans`] reads (a store reads only these).
pub const SPAN_INPUT_FIELDS: [&str; 15] = [
    "TraceId",
    "trace_id",
    "@tr",
    "SpanId",
    "span_id",
    "@sp",
    "ParentSpanId",
    "ParentId",
    "parent_span_id",
    "ElapsedMs",
    "Elapsed",
    "DurationMs",
    "Duration",
    "RequestPath",
    "SourceContext",
];

/// First non-empty value among `names`.
fn first_text(r: &dyn RowRef, names: &[&str]) -> Option<String> {
    names
        .iter()
        .filter_map(|n| r.text_of(n))
        .map(|v| v.into_owned())
        .find(|v| !v.is_empty())
}

/// Ids of the events whose `key_field` equals `value`, in time order
/// (events without a timestamp last).
pub fn correlate<R: RowRef>(records: impl IntoIterator<Item = R>, key_field: &str, value: &str) -> Vec<u32> {
    let mut hits: Vec<(i64, u32)> = records
        .into_iter()
        .filter(|r| r.text_of(key_field).is_some_and(|v| v == value))
        .map(|r| (r.ts().unwrap_or(i64::MAX), sid(&r)))
        .collect();
    hits.sort_unstable();
    hits.into_iter().map(|(_, id)| id).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    pub parent: Option<String>,
    pub name: String,
    /// Epoch ms of the first event of the span.
    pub start: i64,
    pub duration_ms: f64,
    pub events: u32,
    pub errors: u32,
    /// First event of the span, for navigation.
    pub first_id: u32,
}

#[derive(Default)]
struct SpanBuilder {
    /// Each with the time of the event it came from: the earliest event that has one wins.
    parent: Option<(i64, String)>,
    start: i64,
    end: i64,
    explicit: Option<f64>,
    events: u32,
    errors: u32,
    first_id: u32,
    request_path: Option<(i64, String)>,
    context: Option<(i64, String)>,
    text: String,
}

/// Keeps the value of the earliest event (the first one seen among events at the same time).
fn earliest(slot: &mut Option<(i64, String)>, ts: i64, value: impl FnOnce() -> Option<String>) {
    if slot.as_ref().is_none_or(|(t, _)| ts < *t) {
        if let Some(v) = value() {
            *slot = Some((ts, v));
        }
    }
}

/// Builds spans from events carrying a trace id and a span id. The name is the
/// request path, else the logger, else the first message. The duration is the
/// largest `Elapsed`-style value logged in the span, else last minus first event.
///
/// Events stream through (nothing is kept per event); their order does not matter: each span takes what it
/// reports from its earliest events, as if they were sorted by time.
pub fn spans<R: RowRef>(records: impl IntoIterator<Item = R>) -> Vec<Span> {
    let mut builders: BTreeMap<(String, String), SpanBuilder> = BTreeMap::new();
    for r in records {
        let Some(ts) = r.ts() else { continue };
        let r: &dyn RowRef = &r;
        let (Some(trace), Some(span)) = (first_text(r, &TRACE_FIELDS), first_text(r, &SPAN_FIELDS)) else {
            continue;
        };
        let text = || truncate(r.template().unwrap_or(r.message()), 80);
        let b = builders.entry((trace, span)).or_insert_with(|| SpanBuilder {
            start: ts,
            end: ts,
            first_id: sid(r),
            text: text(),
            ..SpanBuilder::default()
        });
        if ts < b.start {
            b.start = ts;
            b.first_id = sid(r);
            b.text = text();
        }
        b.end = b.end.max(ts);
        b.events += 1;
        b.errors += u32::from(r.level() >= Level::Error);
        earliest(&mut b.parent, ts, || first_text(r, &PARENT_FIELDS));
        earliest(&mut b.request_path, ts, || first_text(r, &["RequestPath"]));
        earliest(&mut b.context, ts, || first_text(r, &["SourceContext"]));
        for f in DURATION_FIELDS {
            if let Some(d) = r.number_of(f) {
                b.explicit = Some(b.explicit.map_or(d, |e| e.max(d)));
            }
        }
    }
    let mut out: Vec<Span> = builders
        .into_iter()
        .map(|((trace_id, span_id), b)| Span {
            trace_id,
            span_id,
            parent: b.parent.map(|(_, p)| p),
            name: b.request_path.or(b.context).map_or(b.text, |(_, n)| n),
            start: b.start,
            duration_ms: b.explicit.unwrap_or((b.end - b.start) as f64),
            events: b.events,
            errors: b.errors,
            first_id: b.first_id,
        })
        .collect();
    out.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.span_id.cmp(&b.span_id)));
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaterfallRow {
    pub span_id: String,
    pub parent: Option<String>,
    pub depth: u32,
    pub name: String,
    pub start_offset_ms: i64,
    pub duration_ms: f64,
    pub errors: u32,
}

/// The spans of one trace as depth-first rows (parents before children,
/// siblings by start time). Spans whose parent is missing become roots.
pub fn waterfall(spans: &[Span], trace_id: &str) -> Vec<WaterfallRow> {
    let mut mine: Vec<&Span> = spans.iter().filter(|s| s.trace_id == trace_id).collect();
    mine.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.span_id.cmp(&b.span_id)));
    let Some(origin) = mine.first().map(|s| s.start) else {
        return Vec::new();
    };
    let by_id: HashMap<&str, &Span> = mine.iter().map(|s| (s.span_id.as_str(), *s)).collect();
    let mut children: HashMap<&str, Vec<&Span>> = HashMap::new();
    let mut roots: Vec<&Span> = Vec::new();
    for s in &mine {
        match s.parent.as_deref() {
            Some(p) if by_id.contains_key(p) && p != s.span_id => children.entry(p).or_default().push(s),
            _ => roots.push(s),
        }
    }
    let mut rows = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    // Every span that reaches a root is visited from it; the rest are caught in a parent cycle and are
    // surfaced after the real roots, in start order (linear, engine review finding 19).
    let walk = roots.into_iter().chain(mine.iter().copied());
    for root in walk {
        if seen.contains(root.span_id.as_str()) {
            continue;
        }
        let mut stack = vec![(root, 0u32)];
        while let Some((s, depth)) = stack.pop() {
            if !seen.insert(s.span_id.as_str()) {
                continue;
            }
            rows.push(WaterfallRow {
                span_id: s.span_id.clone(),
                parent: s.parent.clone(),
                depth,
                name: s.name.clone(),
                start_offset_ms: s.start - origin,
                duration_ms: s.duration_ms,
                errors: s.errors,
            });
            if let Some(kids) = children.get(s.span_id.as_str()) {
                stack.extend(kids.iter().rev().map(|k| (*k, depth + 1)));
            }
        }
    }
    rows
}

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSpan {
    pub id: String,
    pub first: i64,
    pub last: i64,
    pub events: u32,
    pub errors: u32,
    pub instance: String,
}

/// Groups events by a request/correlation id for logs without span ids.
/// `id_field` may list alternatives separated by `|` (`RequestId|CorrelationId`).
/// Input is `(event, instance key)`; events without a timestamp or id are skipped.
pub fn request_durations<R: RowRef, K: AsRef<str>>(records: impl IntoIterator<Item = (R, K)>, id_field: &str) -> Vec<RequestSpan> {
    let names: Vec<&str> = id_field.split('|').map(str::trim).filter(|n| !n.is_empty()).collect();
    let mut by_id: BTreeMap<String, RequestSpan> = BTreeMap::new();
    for (r, instance) in records {
        let (Some(ts), Some(id)) = (r.ts(), first_text(&r, &names)) else {
            continue;
        };
        let e = by_id.entry(id.clone()).or_insert_with(|| RequestSpan {
            id,
            first: ts,
            last: ts,
            events: 0,
            errors: 0,
            instance: instance.as_ref().to_string(),
        });
        e.first = e.first.min(ts);
        e.last = e.last.max(ts);
        e.events += 1;
        e.errors += u32::from(r.level() >= Level::Error);
    }
    let mut out: Vec<RequestSpan> = by_id.into_values().collect();
    out.sort_by(|a, b| a.first.cmp(&b.first).then_with(|| a.id.cmp(&b.id)));
    out
}

/// Most requests listed.
pub const MAX_REQUESTS: usize = 500;

/// Requests longest first (ties by id), at most [`MAX_REQUESTS`].
pub fn rank_requests(out: &mut Vec<RequestSpan>) {
    out.sort_by(|a, b| (b.last - b.first).cmp(&(a.last - a.first)).then_with(|| a.id.cmp(&b.id)));
    out.truncate(MAX_REQUESTS);
}
