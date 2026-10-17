//! The port the assistant talks to. The real adapter is the embedded llama.cpp engine; tests use
//! the scripted fake. Nothing here knows about networks.

use super::types::{ChatRequest, ChatResponse};
use crate::error::Result;

/// Contract: `chat` streams generated text through `on_delta`, honours `req.cancel` between tokens
/// (returning `StopReason::Cancelled` with the partial text), never panics on bad input, and returns
/// `AiError::ContextOverflow` rather than truncating the prompt silently.
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> String;
    /// Tokens the loaded model can hold (prompt plus answer).
    fn context_length(&self) -> usize;
    fn chat(&self, req: &ChatRequest, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse>;
}
