//! Read-only engine tools for the model, and the budget that keeps their output small enough for a
//! 4k-8k context. Tool results are clipped (lists capped, long strings shortened), never trusted
//! to be small. Nothing here mutates the workspace.

use super::citations::Citation;
use super::proposals::Proposal;
use crate::access::StoreRef;
use crate::llm::ToolSpec;
use debrief_core::api::{ContextRequest, FieldStatsRequest, MomentRequest, QueryRequest};
use debrief_core::ports::Store;
use debrief_core::ports::{EventsRequest, StoreError};
use debrief_core::ql::QlResult;
use debrief_core::query::Query;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Output limits for one tool result, derived from the model's context length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolLimits {
    pub max_chars: usize,
    pub max_items: usize,
    pub max_str: usize,
}

impl ToolLimits {
    /// A sixth of the context (about 3.5 characters per token), within sane bounds.
    pub fn for_context(context_tokens: usize) -> Self {
        let max_chars = (context_tokens as f64 / 6.0 * 3.5).clamp(1200.0, 16000.0) as usize;
        ToolLimits {
            max_chars,
            max_items: (max_chars / 300).clamp(3, 20),
            max_str: (max_chars / 8).clamp(120, 600),
        }
    }
}

pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…[truncated]", s.chars().take(max).collect::<String>())
    }
}

/// Caps arrays and strings and the nesting depth of a JSON value.
pub fn clip_json(v: &Value, l: &ToolLimits, depth: usize) -> Value {
    match v {
        Value::String(s) => Value::String(truncate_chars(s, l.max_str)),
        Value::Array(a) if depth > 6 => json!(format!("[{} items]", a.len())),
        Value::Array(a) => {
            let mut out: Vec<Value> = a.iter().take(l.max_items).map(|x| clip_json(x, l, depth + 1)).collect();
            if a.len() > l.max_items {
                out.push(json!({"_more": a.len() - l.max_items}));
            }
            Value::Array(out)
        }
        Value::Object(_) if depth > 6 => json!("{…}"),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), clip_json(x, l, depth + 1))).collect()),
        other => other.clone(),
    }
}

/// Record ids mentioned in a tool result (what the answer may cite).
pub fn collect_ids(v: &Value, out: &mut Vec<u32>) {
    const KEYS: &[&str] = &["id", "ids", "sampleIds", "firstError", "recordId", "record_id", "sample_ids"];
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                if KEYS.contains(&k.as_str()) {
                    match x {
                        Value::Number(n) => out.extend(n.as_u64().and_then(|n| u32::try_from(n).ok())),
                        Value::Array(a) => out.extend(a.iter().filter_map(Value::as_u64).filter_map(|n| u32::try_from(n).ok())),
                        _ => {}
                    }
                }
                collect_ids(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_ids(x, out)),
        _ => {}
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub record_ids: Vec<u32>,
    pub queries: Vec<String>,
}

impl Evidence {
    pub fn merge(&mut self, o: &Evidence) {
        for id in &o.record_ids {
            if !self.record_ids.contains(id) {
                self.record_ids.push(*id);
            }
        }
        for q in &o.queries {
            if !self.queries.contains(q) {
                self.queries.push(q.clone());
            }
        }
    }
}

/// A web page the answer relied on (shown separately from log evidence).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSource {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Default)]
pub struct ToolOutcome {
    pub text: String,
    pub is_error: bool,
    /// One line for the UI ("12 events", "3 patterns").
    pub summary: String,
    pub evidence: Evidence,
    pub proposal: Option<Proposal>,
    pub web: Vec<WebSource>,
}

impl ToolOutcome {
    pub fn ok(text: String, summary: impl Into<String>) -> Self {
        ToolOutcome {
            text,
            summary: summary.into(),
            ..Default::default()
        }
    }
    pub fn err(message: impl Into<String>) -> Self {
        let m = message.into();
        ToolOutcome {
            text: format!("error: {m}"),
            is_error: true,
            summary: m,
            ..Default::default()
        }
    }
}

/// What the tool loop calls. Hosts: the engine tools alone, or the chat host adding proposals and web tools.
pub trait ToolHost {
    fn specs(&self) -> Vec<ToolSpec>;
    fn call(&mut self, name: &str, input: &Value) -> ToolOutcome;
}

pub struct EngineTools<'a> {
    store: StoreRef<'a>,
    limits: ToolLimits,
    /// What each event the model saw looked like (id → content hash), to re-check citations at the end of a
    /// turn: the store may change between tool calls (live tail) and ids may shift when it re-sorts.
    seen: RefCell<HashMap<u32, u64>>,
}

/// A content hash of an event (time, level, message, source), independent of its id.
fn fingerprint(v: &Value) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for k in ["ts", "level", "message", "sourceName", "line"] {
        v.get(k).map(Value::to_string).hash(&mut h);
    }
    h.finish()
}

fn spec(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type": "object", "properties": properties, "required": required}),
    }
}

fn args<T: for<'de> Deserialize<'de>>(input: &Value) -> Result<T, Box<ToolOutcome>> {
    serde_json::from_value(input.clone()).map_err(|e| Box::new(ToolOutcome::err(format!("invalid arguments: {e}"))))
}

#[derive(Deserialize)]
struct QlArg {
    ql: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct OptQl {
    #[serde(default)]
    ql: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct MomentArg {
    from: i64,
    to: i64,
}

#[derive(Deserialize)]
struct IdsArg {
    ids: Vec<u32>,
}

#[derive(Deserialize)]
struct FieldArg {
    field: String,
    #[serde(default)]
    ql: String,
}

#[derive(Deserialize)]
struct NeighbourArg {
    record_id: u32,
    #[serde(default)]
    before: Option<usize>,
    #[serde(default)]
    after: Option<usize>,
}

fn query_of(ql: &str) -> Query {
    Query {
        ql: ql.trim().to_string(),
        ..Query::default()
    }
}

impl<'a> EngineTools<'a> {
    /// The store is borrowed per tool call only (see [`StoreRef`]), never across generation.
    pub fn new(store: impl Into<StoreRef<'a>>, limits: ToolLimits) -> Self {
        EngineTools {
            store: store.into(),
            limits,
            seen: RefCell::default(),
        }
    }

    pub fn limits(&self) -> ToolLimits {
        self.limits
    }

    pub fn tool_specs() -> Vec<ToolSpec> {
        let ql = json!({"type": "string", "description": "Debrief query language, NOT SQL. Examples: level >= error | MachineName = WEB02 and message contains \"timeout\" | level >= warning | stats count() as n by MachineName | sort n desc"});
        vec![
            spec(
                "run_query",
                "Run a query-language query over the logs. Returns matching event ids (and a few events) or a table for `stats` queries.",
                json!({"ql": ql, "limit": {"type": "integer"}}),
                &["ql"],
            ),
            spec(
                "check_query",
                "Check that a query is valid without running it.",
                json!({"ql": ql}),
                &["ql"],
            ),
            spec(
                "patterns",
                "Most frequent message templates (variable parts masked as <*>), optionally filtered by a query.",
                json!({"ql": ql, "limit": {"type": "integer"}}),
                &[],
            ),
            spec(
                "exception_groups",
                "Exceptions grouped by fingerprint with counts, first/last time and sample event ids.",
                json!({"ql": ql}),
                &[],
            ),
            spec(
                "overview",
                "The automatic health check: the most important findings first.",
                json!({}),
                &[],
            ),
            spec(
                "moment",
                "What happened between two times (epoch milliseconds): levels, instances, metrics, anomalies.",
                json!({"from": {"type": "integer"}, "to": {"type": "integer"}}),
                &["from", "to"],
            ),
            spec(
                "get_events",
                "Full details of events by id (at most 50).",
                json!({"ids": {"type": "array", "items": {"type": "integer"}}}),
                &["ids"],
            ),
            spec(
                "field_stats",
                "Distribution of one field (top values, numeric statistics).",
                json!({"field": {"type": "string"}, "ql": ql}),
                &["field"],
            ),
            spec(
                "neighbours",
                "The original log lines around an event.",
                json!({"record_id": {"type": "integer"}, "before": {"type": "integer"}, "after": {"type": "integer"}}),
                &["record_id"],
            ),
        ]
    }

    /// Clips `value` to the budget. Instead of cutting the text (which would break the JSON), the list and
    /// string limits are tightened until it fits; the text cut is only the last resort.
    fn finish(&self, value: Value, summary: String, queries: Vec<String>) -> ToolOutcome {
        let mut limits = self.limits;
        let mut clipped = clip_json(&value, &limits, 0);
        let mut text = clipped.to_string();
        for _ in 0..8 {
            if text.chars().count() <= self.limits.max_chars {
                break;
            }
            limits.max_items = (limits.max_items / 2).max(1);
            limits.max_str = (limits.max_str * 2 / 3).max(40);
            clipped = clip_json(&value, &limits, 0);
            text = clipped.to_string();
        }
        let mut ids = Vec::new();
        collect_ids(&clipped, &mut ids);
        ids.sort_unstable();
        ids.dedup();
        ids.truncate(40);
        ToolOutcome {
            text: truncate_chars(&text, self.limits.max_chars),
            summary,
            evidence: Evidence { record_ids: ids, queries },
            ..Default::default()
        }
    }

    /// One-line description of an event for grounding ("Error 10:42:01 Failed to capture payment").
    pub fn event_summary(&self, id: u32) -> Option<String> {
        self.store.with(|s| self.summary_in(s, id))
    }

    fn summary_in(&self, s: &dyn Store, id: u32) -> Option<String> {
        let v = self.event_in(s, id)?;
        let level = v.get("level").and_then(Value::as_str).unwrap_or("?");
        let ts = v
            .get("ts")
            .and_then(Value::as_i64)
            .map(crate::chat::format_utc)
            .unwrap_or_else(|| "no time".into());
        let msg = v.get("message").and_then(Value::as_str).unwrap_or("");
        let src = v.get("sourceName").and_then(Value::as_str).unwrap_or("");
        Some(format!("#{id} [{level}] {ts} {} ({src})", truncate_chars(msg, 140)))
    }

    /// Re-checks record citations against the store as it is now, by content: a cited event that no longer
    /// exists, or whose id now names a different event, is marked unverified. Returns how many changed.
    pub fn recheck_citations(&self, citations: &mut [Citation]) -> usize {
        let before = self.seen.borrow().clone();
        self.store.with(|s| {
            let mut changed = 0;
            for c in citations.iter_mut() {
                if let Citation::Record { id, verified } = c {
                    let still = match self.event_in(s, *id) {
                        None => false,
                        Some(v) => before.get(id).is_none_or(|fp| *fp == fingerprint(&v)),
                    };
                    if !still && *verified {
                        *verified = false;
                        changed += 1;
                    }
                }
            }
            changed
        })
    }

    fn event_in(&self, s: &dyn Store, id: u32) -> Option<Value> {
        let req = EventsRequest {
            ids: vec![debrief_core::batch::EventId(u64::from(id))],
            group_by: Vec::new(),
        };
        let row = s.events(&req).ok()?.into_iter().next()?;
        let mut v = serde_json::to_value(&row).ok()?;
        if let Some(o) = v.as_object_mut() {
            for k in ["template", "group", "source", "tsState", "byteOffset"] {
                o.remove(k);
            }
        }
        self.seen.borrow_mut().entry(id).or_insert_with(|| fingerprint(&v));
        Some(v)
    }

    fn run_query(&self, s: &dyn Store, a: QlArg) -> ToolOutcome {
        let limit = a.limit.unwrap_or(20).clamp(1, 50);
        match s.run_query(&QueryRequest { query: query_of(&a.ql) }) {
            Err(StoreError::Query(e)) => ToolOutcome::err(format!(
                "query error: {} (at byte {}); fix the query and try again",
                e.message, e.offset
            )),
            Err(e) => ToolOutcome::err(format!("the query cannot run: {e}")),
            Ok(QlResult::Events { ids, .. }) => {
                let shown: Vec<u32> = ids.iter().copied().take(limit).collect();
                let sample: Vec<Value> = shown
                    .iter()
                    .take(5.min(self.limits.max_items))
                    .filter_map(|id| self.event_in(s, *id))
                    .collect();
                self.finish(
                    json!({"kind": "events", "total": ids.len(), "ids": shown, "sample": sample}),
                    format!("{} events", ids.len()),
                    vec![a.ql],
                )
            }
            Ok(QlResult::Table { columns, rows }) => {
                let total = rows.len();
                let rows: Vec<&Vec<Value>> = rows.iter().take(limit).collect();
                self.finish(
                    json!({"kind": "table", "columns": columns, "rows": rows, "totalRows": total}),
                    format!("{total} rows"),
                    vec![a.ql],
                )
            }
        }
    }

    fn get_events(&self, s: &dyn Store, a: IdsArg) -> ToolOutcome {
        let mut ids = a.ids;
        ids.dedup();
        ids.truncate(50);
        let requested = ids.len();
        let mut out = Vec::new();
        let mut used = 0usize;
        for id in ids {
            let Some(v) = self.event_in(s, id) else { continue };
            let v = clip_json(&v, &self.limits, 1);
            used += v.to_string().len();
            if used > self.limits.max_chars && !out.is_empty() {
                break;
            }
            out.push(v);
        }
        let shown = out.len();
        let found_ids: Vec<u32> = out
            .iter()
            .filter_map(|v| v.get("id").and_then(Value::as_u64))
            .filter_map(|n| u32::try_from(n).ok())
            .collect();
        let mut o = self.finish(
            json!({"events": out, "requested": requested, "returned": shown}),
            format!("{shown} of {requested} events"),
            vec![],
        );
        o.evidence.record_ids = found_ids;
        o
    }
}

impl ToolHost for EngineTools<'_> {
    fn specs(&self) -> Vec<ToolSpec> {
        Self::tool_specs()
    }

    /// One tool execution holds the store for its duration only.
    fn call(&mut self, name: &str, input: &Value) -> ToolOutcome {
        let store = self.store;
        store.with(|s| self.call_in(s, name, input))
    }
}

impl EngineTools<'_> {
    fn call_in(&self, s: &dyn Store, name: &str, input: &Value) -> ToolOutcome {
        macro_rules! parse {
            ($t:ty) => {
                match args::<$t>(input) {
                    Ok(a) => a,
                    Err(e) => return *e,
                }
            };
        }
        match name {
            "run_query" => self.run_query(s, parse!(QlArg)),
            "check_query" => {
                let a = parse!(QlArg);
                match s.check_query(&a.ql) {
                    Ok(None) => ToolOutcome::ok(json!({"valid": true}).to_string(), "valid"),
                    Ok(Some(e)) => ToolOutcome::ok(
                        json!({"valid": false, "error": e.message, "offset": e.offset}).to_string(),
                        "invalid",
                    ),
                    Err(e) => ToolOutcome::err(format!("the query cannot be checked: {e}")),
                }
            }
            "patterns" => {
                let a = parse!(OptQl);
                match s.patterns(&QueryRequest { query: query_of(&a.ql) }).map(|a| a.result) {
                    Ok(p) => {
                        let n = p.len();
                        let top: Vec<_> = p.into_iter().take(a.limit.unwrap_or(15).clamp(1, 30)).collect();
                        self.finish(
                            json!({"total": n, "patterns": top}),
                            format!("{n} patterns"),
                            vec![a.ql].into_iter().filter(|q| !q.is_empty()).collect(),
                        )
                    }
                    Err(e) => ToolOutcome::err(format!("{e:#}")),
                }
            }
            "exception_groups" => {
                let a = parse!(OptQl);
                match s.exception_groups(&QueryRequest { query: query_of(&a.ql) }).map(|a| a.result) {
                    Ok(g) => {
                        let n = g.len();
                        self.finish(json!({"total": n, "groups": g}), format!("{n} exception groups"), vec![])
                    }
                    Err(e) => ToolOutcome::err(format!("{e:#}")),
                }
            }
            "overview" => match s.overview(&QueryRequest { query: Query::default() }).map(|a| a.result) {
                Ok(f) => {
                    let n = f.len();
                    self.finish(json!({"findings": f}), format!("{n} findings"), vec![])
                }
                Err(e) => ToolOutcome::err(format!("{e:#}")),
            },
            "moment" => {
                let a = parse!(MomentArg);
                match s
                    .moment(&MomentRequest {
                        query: Query::default(),
                        from: a.from,
                        to: a.to,
                    })
                    .map(|a| a.result)
                {
                    Ok(m) => {
                        let total = m.total;
                        self.finish(
                            serde_json::to_value(&m).unwrap_or(Value::Null),
                            format!("{total} events in the window"),
                            vec![],
                        )
                    }
                    Err(e) => ToolOutcome::err(format!("{e:#}")),
                }
            }
            "get_events" => self.get_events(s, parse!(IdsArg)),
            "field_stats" => {
                let a = parse!(FieldArg);
                match s.field_stats(&FieldStatsRequest {
                    query: query_of(&a.ql),
                    field: a.field.clone(),
                    buckets: 10,
                }) {
                    Ok(s) => self.finish(
                        serde_json::to_value(&s).unwrap_or(Value::Null),
                        format!("stats of {}", a.field),
                        vec![],
                    ),
                    Err(e) => ToolOutcome::err(format!("{e:#}")),
                }
            }
            "neighbours" => {
                let a = parse!(NeighbourArg);
                let req = ContextRequest {
                    record_id: a.record_id,
                    before: a.before.unwrap_or(5).min(10),
                    after: a.after.unwrap_or(5).min(10),
                };
                match s.source_context(&req) {
                    Ok(c) => {
                        let id = a.record_id;
                        let mut o = self.finish(
                            serde_json::to_value(&c).unwrap_or(Value::Null),
                            format!("lines around event {id}"),
                            vec![],
                        );
                        o.evidence.record_ids = vec![id];
                        o
                    }
                    Err(e) => ToolOutcome::err(format!("{e:#}")),
                }
            }
            other => ToolOutcome::err(format!("unknown tool '{other}'")),
        }
    }
}
