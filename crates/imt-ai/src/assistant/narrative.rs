//! Read-only narratives: explain an event, summarise a time window, find root causes. The engine fetches
//! the facts first (as a recorded tool exchange the model can see); the model only writes them up and may
//! call more tools. Answers carry citations, checked against what the tools actually returned.

use super::citations::{parse_citations, Citation};
use super::tools::{EngineTools, Evidence, ToolHost, ToolLimits};
use super::{Assistant, LoopEvent};
use crate::access::StoreRef;
use crate::error::{AiError, Result};
use crate::llm::{ChatMessage, LlmProvider, ToolCall, Usage};
use serde::Serialize;
use serde_json::{json, Value};

pub const ANALYST_SYSTEM: &str =
    "You are the log analysis assistant inside IMT. Use only facts from the tool results; never invent events, \
times or causes. Cite evidence as event ids (for example: event 123) or as queries in backticks. If the logs do not contain the answer, \
say \"not found in the logs\". Be concise and concrete: what happened, where, when, the likely cause, and next steps.";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Narrative {
    pub text: String,
    pub citations: Vec<Citation>,
    pub evidence: Evidence,
    pub usage: Usage,
}

/// Runs the given calls through the tools and returns them as an assistant/tool message exchange.
pub(crate) fn prefetch(host: &mut dyn ToolHost, calls: &[(&str, Value)]) -> (Vec<ChatMessage>, Evidence, bool) {
    let tool_calls: Vec<ToolCall> = calls
        .iter()
        .enumerate()
        .map(|(i, (n, v))| ToolCall {
            id: format!("pre_{}", i + 1),
            name: n.to_string(),
            input: v.clone(),
        })
        .collect();
    let mut msgs = vec![ChatMessage::assistant_calls("", tool_calls.clone())];
    let mut ev = Evidence::default();
    let mut any_ok = false;
    for c in &tool_calls {
        let o = host.call(&c.name, &c.input);
        any_ok |= !o.is_error;
        ev.merge(&o.evidence);
        msgs.push(ChatMessage::tool_result(c.id.clone(), o.text));
    }
    (msgs, ev, any_ok)
}

impl<P: LlmProvider> Assistant<P> {
    fn narrate(&self, store: StoreRef<'_>, task: String, calls: &[(&str, Value)], require_ids: bool) -> Result<Narrative> {
        let mut tools = EngineTools::new(store, ToolLimits::for_context(self.provider.context_length()));
        let (exchange, mut evidence, any_ok) = prefetch(&mut tools, calls);
        if !any_ok || (require_ids && evidence.record_ids.is_empty()) {
            return Err(AiError::NotFound("nothing in the loaded logs matches that".into()));
        }
        let mut start = vec![ChatMessage::user(task)];
        start.extend(exchange);
        let r = self.run_tool_loop(ANALYST_SYSTEM, start, &mut tools, &mut |_: LoopEvent<'_>| {})?;
        evidence.merge(&r.evidence);
        let mut citations = parse_citations(&r.text, &evidence.record_ids, &|q| store.with(|s| s.check_query(q)).is_none());
        tools.recheck_citations(&mut citations);
        Ok(Narrative {
            text: r.text,
            citations,
            evidence,
            usage: r.usage,
        })
    }

    /// What an event means, its likely cause and what to do next.
    pub fn explain_event<'s>(&self, store: impl Into<StoreRef<'s>>, record_id: u32) -> Result<Narrative> {
        self.narrate(
            store.into(),
            format!(
                "Explain event {record_id}: what it means, the likely cause and next steps. Use the surrounding lines and similar events."
            ),
            &[
                ("get_events", json!({"ids": [record_id]})),
                ("neighbours", json!({"record_id": record_id, "before": 5, "after": 3})),
            ],
            true,
        )
    }

    /// What happened between two times (epoch milliseconds): what, where, when, likely cause, next steps.
    pub fn summarize<'s>(&self, store: impl Into<StoreRef<'s>>, from: i64, to: i64) -> Result<Narrative> {
        if to <= from {
            return Err(AiError::Invalid("the end of the time range must be after its start".into()));
        }
        self.narrate(
            store.into(),
            format!(
                "Summarise what happened between {from} and {to} (epoch milliseconds): what, where, when, the likely cause and next steps."
            ),
            &[("moment", json!({"from": from, "to": to}))],
            false,
        )
    }

    /// The most likely root causes, from the health-check findings and exception groups, with evidence.
    pub fn root_causes<'s>(&self, store: impl Into<StoreRef<'s>>) -> Result<Narrative> {
        self.narrate(
            store.into(),
            "List the most likely root causes of the problems in these logs, most important first, each with its evidence.".into(),
            &[("overview", json!({})), ("exception_groups", json!({}))],
            false,
        )
    }
}
