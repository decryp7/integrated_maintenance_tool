//! The assistant: read-only engine tools for the model, a bounded tool loop, and the use cases.
//! The model proposes; the engine validates and previews every proposal before it is returned.

mod citations;
mod folder;
mod markers;
mod narrative;
mod pipeline;
mod parser;
mod proposals;
mod query;
mod regex;
mod structured;
mod tools;

pub use citations::{parse_citations, Citation};
pub use folder::FolderRefinement;
#[allow(unused_imports)]
pub(crate) use narrative::prefetch;
pub use narrative::{Narrative, ANALYST_SYSTEM};
pub use proposals::*;
pub use tools::{clip_json, collect_ids, truncate_chars, EngineTools, Evidence, ToolHost, ToolLimits, ToolOutcome, WebSource};

use crate::error::{AiError, Result};
use crate::llm::{ChatMessage, ChatRequest, ChatResponse, LlmProvider, StopReason, ToolCall, Usage};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AssistantConfig {
    pub max_tokens: usize,
    pub temperature: f32,
    /// Most tool rounds in one question (each round may call several tools).
    pub max_tool_rounds: usize,
    pub cancel: Arc<AtomicBool>,
}

impl Default for AssistantConfig {
    fn default() -> Self {
        AssistantConfig {
            max_tokens: 1024,
            temperature: 0.2,
            max_tool_rounds: 6,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct Assistant<P: LlmProvider> {
    pub(crate) provider: P,
    pub(crate) cfg: AssistantConfig,
}

/// Progress of a tool loop, for the chat stream.
#[derive(Debug, Clone)]
pub enum LoopEvent<'a> {
    Delta(&'a str),
    ToolStarted { name: &'a str, args: String },
    ToolFinished { name: &'a str, outcome: &'a ToolOutcome },
}

pub struct LoopResult {
    pub text: String,
    pub evidence: Evidence,
    pub usage: Usage,
    pub proposals: Vec<Proposal>,
    pub web: Vec<WebSource>,
    /// Everything exchanged after the starting messages (assistant turns and tool results).
    pub new_messages: Vec<ChatMessage>,
}

impl<P: LlmProvider> Assistant<P> {
    pub fn new(provider: P, cfg: AssistantConfig) -> Self {
        Assistant { provider, cfg }
    }

    pub fn provider(&self) -> &P {
        &self.provider
    }

    pub fn config(&self) -> &AssistantConfig {
        &self.cfg
    }

    pub fn cancel(&self) {
        self.cfg.cancel.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cfg.cancel.load(Ordering::Relaxed)
    }

    /// Characters of prompt context available for material (samples, tool output), leaving room for the answer.
    pub(crate) fn prompt_chars(&self) -> usize {
        self.provider.context_length().saturating_sub(self.cfg.max_tokens + 600).max(512) * 3
    }

    pub(crate) fn request(&self, system: &str, messages: Vec<ChatMessage>) -> ChatRequest {
        ChatRequest {
            system: system.into(),
            messages,
            max_tokens: self.cfg.max_tokens,
            temperature: Some(self.cfg.temperature),
            cancel: Some(self.cfg.cancel.clone()),
            ..ChatRequest::default()
        }
    }

    /// One non-streaming call; cancellation becomes an error.
    pub(crate) fn complete(&self, req: &ChatRequest) -> Result<ChatResponse> {
        if self.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let r = self.provider.chat(req, &mut |_| {})?;
        if r.stop_reason == StopReason::Cancelled {
            return Err(AiError::Cancelled);
        }
        Ok(r)
    }

    /// The tool loop: the model may call tools for up to `max_tool_rounds` rounds; if it is still calling
    /// tools then, it gets one last turn without tools to answer from what it has.
    pub fn run_tool_loop(
        &self,
        system: &str,
        start: Vec<ChatMessage>,
        host: &mut dyn ToolHost,
        on_event: &mut dyn FnMut(LoopEvent<'_>),
    ) -> Result<LoopResult> {
        let tools = host.specs();
        let mut messages = start;
        let first_new = messages.len();
        let mut result = LoopResult {
            text: String::new(),
            evidence: Evidence::default(),
            usage: Usage::default(),
            proposals: vec![],
            web: vec![],
            new_messages: vec![],
        };
        for round in 0..=self.cfg.max_tool_rounds {
            if self.is_cancelled() {
                return Err(AiError::Cancelled);
            }
            let last = round == self.cfg.max_tool_rounds;
            let mut req = self.request(system, messages.clone());
            if !last {
                req.tools = tools.clone();
            } else {
                req.system = format!("{system}\nYou have used all tool rounds. Answer now from the evidence gathered; do not call tools.");
            }
            let resp = self.provider.chat(&req, &mut |d| on_event(LoopEvent::Delta(d)))?;
            result.usage.add(resp.usage.unwrap_or_default());
            if resp.stop_reason == StopReason::Cancelled {
                return Err(AiError::Cancelled);
            }
            if resp.tool_calls.is_empty() || last {
                if resp.text.trim().is_empty() {
                    return Err(if last {
                        AiError::ToolLoopLimit(self.cfg.max_tool_rounds)
                    } else {
                        AiError::Malformed("the model gave an empty answer".into())
                    });
                }
                if !resp.tool_calls.is_empty() {
                    return Err(AiError::ToolLoopLimit(self.cfg.max_tool_rounds));
                }
                result.text = resp.text.clone();
                messages.push(ChatMessage::assistant(resp.text));
                break;
            }
            messages.push(ChatMessage::assistant_calls(resp.text.clone(), resp.tool_calls.clone()));
            for call in &resp.tool_calls {
                let outcome = self.dispatch(host, call, on_event);
                result.evidence.merge(&outcome.evidence);
                if let Some(p) = &outcome.proposal {
                    result.proposals.push(p.clone());
                }
                for w in &outcome.web {
                    if !result.web.contains(w) {
                        result.web.push(w.clone());
                    }
                }
                messages.push(ChatMessage::tool_result(call.id.clone(), outcome.text));
            }
        }
        result.new_messages = messages.split_off(first_new);
        Ok(result)
    }

    fn dispatch(&self, host: &mut dyn ToolHost, call: &ToolCall, on_event: &mut dyn FnMut(LoopEvent<'_>)) -> ToolOutcome {
        let args = truncate_chars(&call.input.to_string(), 160);
        on_event(LoopEvent::ToolStarted { name: &call.name, args });
        let outcome = if call.name.is_empty() || call.input.get("_invalid_json").is_some() {
            let raw = call.input.get("_invalid_json").and_then(Value::as_str).unwrap_or("");
            ToolOutcome::err(format!(
                "the tool call could not be read ({}). Call a tool again with valid JSON arguments.",
                truncate_chars(raw, 120)
            ))
        } else {
            host.call(&call.name, &call.input)
        };
        on_event(LoopEvent::ToolFinished {
            name: &call.name,
            outcome: &outcome,
        });
        outcome
    }
}

impl<T: LlmProvider + ?Sized> LlmProvider for Arc<T> {
    fn id(&self) -> &str {
        (**self).id()
    }
    fn display_name(&self) -> String {
        (**self).display_name()
    }
    fn context_length(&self) -> usize {
        (**self).context_length()
    }
    fn vision(&self) -> bool {
        (**self).vision()
    }
    fn chat(&self, req: &ChatRequest, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
        (**self).chat(req, on_delta)
    }
}

impl<T: LlmProvider + ?Sized> LlmProvider for &T {
    fn id(&self) -> &str {
        (**self).id()
    }
    fn display_name(&self) -> String {
        (**self).display_name()
    }
    fn context_length(&self) -> usize {
        (**self).context_length()
    }
    fn vision(&self) -> bool {
        (**self).vision()
    }
    fn chat(&self, req: &ChatRequest, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
        (**self).chat(req, on_delta)
    }
}
