//! Request and response types of the provider port. They are provider-neutral and local-only.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    /// The result of a tool call (answers one `ToolCall::id`).
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// How a sanitised image is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageMime {
    Png,
    Jpeg,
}

impl ImageMime {
    pub fn as_str(self) -> &'static str {
        match self {
            ImageMime::Png => "image/png",
            ImageMime::Jpeg => "image/jpeg",
        }
    }
}

/// An image for the model. Only `crate::vision::image::sanitize` makes one: decoded in Rust within limits,
/// downscaled and re-encoded without metadata, so a provider never sees the user's raw bytes (ADR 0025).
#[derive(Debug, Clone, PartialEq)]
pub struct ImageInput {
    mime: ImageMime,
    bytes: Arc<[u8]>,
    width: u32,
    height: u32,
}

impl ImageInput {
    pub(crate) fn new(mime: ImageMime, bytes: Vec<u8>, width: u32, height: u32) -> Self {
        ImageInput {
            mime,
            bytes: bytes.into(),
            width,
            height,
        }
    }

    pub fn mime(&self) -> ImageMime {
        self.mime
    }

    /// The encoded image (PNG or JPEG).
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Images shown with a user message (to a model that reads images). Never persisted with a chat.
    #[serde(skip)]
    pub images: Vec<ImageInput>,
    /// Tool calls made by an assistant message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For `Role::Tool`: the call this answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::User,
            content: content.into(),
            images: vec![],
            tool_calls: vec![],
            tool_call_id: None,
        }
    }
    /// A user message with images (shown to the model before the text).
    pub fn user_with_images(content: impl Into<String>, images: Vec<ImageInput>) -> Self {
        ChatMessage {
            images,
            ..ChatMessage::user(content)
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::Assistant,
            content: content.into(),
            images: vec![],
            tool_calls: vec![],
            tool_call_id: None,
        }
    }
    pub fn assistant_calls(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        ChatMessage {
            role: Role::Assistant,
            content: content.into(),
            images: vec![],
            tool_calls,
            tool_call_id: None,
        }
    }
    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::Tool,
            content: content.into(),
            images: vec![],
            tool_calls: vec![],
            tool_call_id: Some(call_id.into()),
        }
    }
}

/// A tool the model may call; `input_schema` is a JSON schema object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub system: String,
    pub messages: Vec<ChatMessage>,
    /// Hard cap on generated tokens.
    pub max_tokens: usize,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
    /// Constrain the output to JSON matching this schema (grammar-constrained decoding).
    pub json_schema: Option<Value>,
    /// Set to stop generation between tokens.
    pub cancel: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: usize,
    pub output_tokens: usize,
}

impl Usage {
    pub fn add(&mut self, o: Usage) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: StopReason,
    pub usage: Option<Usage>,
}
