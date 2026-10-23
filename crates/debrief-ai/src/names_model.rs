//! The model behind the AI aliases of the name matcher (`debrief_core::names::alias_job`): Debrief's own local model,
//! the llama.cpp worker the assistant uses. Nothing else is involved: no address to configure, no other process,
//! nothing leaves this computer.
//!
//! Optional by design: without a model the provider is not there and the alias job skips itself.

use crate::error::AiError;
use crate::llm::{ChatMessage, ChatRequest, LlmProvider, StopReason};
use debrief_core::names::alias_job::AliasModel;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Longest answer asked of the model for one batch of names.
const MAX_TOKENS: usize = 1500;

const SYSTEM: &str = "You name software components for a log analyst. Answer with JSON only.";

/// The strict shape of an answer (grammar-constrained decoding); the job validates the content again.
fn reply_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer"},
                        "name": {"type": "string"},
                        "aliases": {"type": "array", "items": {"type": "string"}}
                    },
                    "required": ["id", "name", "aliases"]
                }
            }
        },
        "required": ["items"]
    })
}

/// An [`LlmProvider`] (the assistant's local model) as an [`AliasModel`].
pub struct LocalAliasModel {
    provider: Arc<dyn LlmProvider>,
    cancel: Arc<AtomicBool>,
}

impl LocalAliasModel {
    /// `cancel` stops a generation between tokens (the job's Cancel).
    pub fn new(provider: Arc<dyn LlmProvider>, cancel: Arc<AtomicBool>) -> LocalAliasModel {
        LocalAliasModel { provider, cancel }
    }
}

impl AliasModel for LocalAliasModel {
    fn version(&self) -> String {
        format!("local:{}", self.provider.id())
    }

    fn complete(&self, prompt: &str) -> Result<String, String> {
        let req = ChatRequest {
            system: SYSTEM.into(),
            messages: vec![ChatMessage::user(prompt)],
            max_tokens: MAX_TOKENS,
            temperature: Some(0.0),
            json_schema: Some(reply_schema()),
            cancel: Some(self.cancel.clone()),
            ..ChatRequest::default()
        };
        match self.provider.chat(&req, &mut |_| {}) {
            Ok(r) if r.stop_reason == StopReason::Cancelled => Err("cancelled".into()),
            Ok(r) => Ok(r.text),
            Err(AiError::NoModel) => Err("no AI model is set up".into()),
            Err(e) => Err(format!("the local model cannot answer: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{Reply, ScriptedProvider};
    use std::sync::atomic::Ordering;

    #[test]
    fn asks_for_strict_json_and_returns_the_text() {
        let p = Arc::new(ScriptedProvider::text(&["{\"items\":[]}"]));
        let m = LocalAliasModel::new(p.clone(), Arc::default());
        assert_eq!(m.version(), "local:scripted");
        assert_eq!(m.complete("1. [executable] gfx").unwrap(), "{\"items\":[]}");
        let req = &p.requests()[0];
        assert!(req.json_schema.is_some() && req.tools.is_empty());
        assert_eq!(req.temperature, Some(0.0));
        assert_eq!(req.messages[0].content, "1. [executable] gfx");
    }

    #[test]
    fn failures_and_cancel_are_messages() {
        let p = Arc::new(ScriptedProvider::new(vec![
            Reply::Fail(AiError::NoModel),
            Reply::Fail(AiError::Engine("boom".into())),
        ]));
        let m = LocalAliasModel::new(p, Arc::default());
        assert!(m.complete("x").unwrap_err().contains("no AI model"));
        assert!(m.complete("x").unwrap_err().contains("boom"));
        let cancel = Arc::new(AtomicBool::new(true));
        let m = LocalAliasModel::new(Arc::new(ScriptedProvider::text(&["abcdefghij"])), cancel.clone());
        assert_eq!(m.complete("x").unwrap_err(), "cancelled");
        cancel.store(false, Ordering::Relaxed);
    }
}
