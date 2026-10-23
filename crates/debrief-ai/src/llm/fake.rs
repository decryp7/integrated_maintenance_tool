//! A scripted provider for tests and for UI development without a model. Replies are consumed in
//! order; every request is recorded so tests can assert on prompts.

use super::provider::LlmProvider;
use super::types::*;
use crate::error::{AiError, Result};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub enum Reply {
    Text(String),
    /// Text followed by tool calls (name, arguments).
    Tools(String, Vec<(String, Value)>),
    Fail(AiError),
}

pub struct ScriptedProvider {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<ChatRequest>>,
    context: usize,
    /// Bytes per streamed delta.
    chunk: usize,
    vision: bool,
}

impl ScriptedProvider {
    pub fn new(replies: Vec<Reply>) -> Self {
        ScriptedProvider {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(vec![]),
            context: 8192,
            chunk: 6,
            vision: false,
        }
    }

    pub fn text(replies: &[&str]) -> Self {
        Self::new(replies.iter().map(|s| Reply::Text((*s).to_string())).collect())
    }

    pub fn with_context(mut self, n: usize) -> Self {
        self.context = n;
        self
    }

    /// A scripted model that reads images (requests with images are accepted and recorded).
    pub fn with_vision(mut self) -> Self {
        self.vision = true;
        self
    }

    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }

    pub fn remaining(&self) -> usize {
        self.replies.lock().map(|r| r.len()).unwrap_or(0)
    }
}

impl LlmProvider for ScriptedProvider {
    fn id(&self) -> &str {
        "scripted"
    }

    fn display_name(&self) -> String {
        "Scripted test model".into()
    }

    fn context_length(&self) -> usize {
        self.context
    }

    fn vision(&self) -> bool {
        self.vision
    }

    fn chat(&self, req: &ChatRequest, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
        if let Ok(mut r) = self.requests.lock() {
            r.push(req.clone());
        }
        if !self.vision && req.messages.iter().any(|m| !m.images.is_empty()) {
            return Err(AiError::NoVision);
        }
        let reply = self.replies.lock().ok().and_then(|mut q| q.pop_front());
        let (text, calls) = match reply {
            Some(Reply::Text(t)) => (t, vec![]),
            Some(Reply::Tools(t, c)) => (t, c),
            Some(Reply::Fail(e)) => return Err(e),
            None => return Err(AiError::Engine("the scripted provider has no more replies".into())),
        };
        let mut sent = String::new();
        let mut rest = text.as_str();
        while !rest.is_empty() {
            let mut n = self.chunk.min(rest.len());
            while !rest.is_char_boundary(n) {
                n += 1;
            }
            if req.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Ok(ChatResponse {
                    text: sent,
                    tool_calls: vec![],
                    stop_reason: StopReason::Cancelled,
                    usage: None,
                });
            }
            on_delta(&rest[..n]);
            sent.push_str(&rest[..n]);
            rest = &rest[n..];
        }
        let tool_calls: Vec<ToolCall> = calls
            .into_iter()
            .enumerate()
            .map(|(i, (name, input))| ToolCall {
                id: format!("call_{}", i + 1),
                name,
                input,
            })
            .collect();
        let stop_reason = if tool_calls.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        };
        let usage = Some(Usage {
            input_tokens: req.messages.iter().map(|m| m.content.len() / 4).sum::<usize>() + req.system.len() / 4,
            output_tokens: text.len() / 4,
        });
        Ok(ChatResponse {
            text,
            tool_calls,
            stop_reason,
            usage,
        })
    }
}
