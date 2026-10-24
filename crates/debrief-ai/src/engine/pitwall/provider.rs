//! `PitwallProvider`: the chat port over Pitwall's OpenAI-compatible API (ADR 0059). The request goes through the same
//! preparation and parsing as the built-in engine (`llm::prompted_chat`): Debrief's tool protocol in the system prompt,
//! thinking and markup hidden from the stream. Pitwall applies the model's chat template.

use super::client::{Client, PitwallError, StreamEnd};
use super::http::Timeouts;
use crate::error::Result;
use crate::llm::{prompted_chat, ChatRequest, ChatResponse, Finish, Generated, LlmProvider, Prepared};
use base64::Engine as _;
use serde_json::{json, Value};

/// How urgent a request is for Pitwall's queue (`X-Pitwall-Priority`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// Something the user waits for (chats, use cases).
    Interactive,
    /// Work nobody waits for (name aliases).
    Background,
}

impl Priority {
    pub fn header(self) -> &'static str {
        match self {
            Priority::Interactive => "interactive",
            Priority::Background => "background",
        }
    }
}

/// What happened to a request, for the engine's state (Pitwall served it, or went away).
pub type Outcome = std::sync::Arc<dyn Fn(std::result::Result<&StreamEnd, &PitwallError>) + Send + Sync>;

/// One way to Pitwall: the server, the model asked for and what Debrief knows about it.
pub struct PitwallProvider {
    pub(super) client: Client,
    /// `debrief` (the user's preset) or `auto:chat,<n>k`.
    pub(super) spec: String,
    /// Whether `spec` is a capability selection (`auto:`), so `vision` can be added for images.
    pub(super) auto: bool,
    /// The model Pitwall chose for `spec` when Debrief last asked.
    pub(super) model: String,
    pub(super) context: usize,
    pub(super) vision: bool,
    pub(super) priority: Priority,
    pub(super) timeouts: Timeouts,
    pub(super) outcome: Option<Outcome>,
}

impl PitwallProvider {
    /// The same way to Pitwall with another priority.
    pub fn with_priority(&self, priority: Priority) -> PitwallProvider {
        PitwallProvider {
            client: self.client.clone(),
            spec: self.spec.clone(),
            auto: self.auto,
            model: self.model.clone(),
            context: self.context,
            vision: self.vision,
            priority,
            timeouts: self.timeouts,
            outcome: self.outcome.clone(),
        }
    }

    /// The model asked for: `spec`, with `vision` added to a capability selection when images are sent.
    pub(super) fn model_for(&self, images: bool) -> String {
        if images && self.auto {
            format!("{},vision", self.spec)
        } else {
            self.spec.clone()
        }
    }
}

/// The chat completion body for a prepared conversation.
pub fn request_body(p: &Prepared, model: &str) -> Value {
    let mut messages = vec![json!({"role": "system", "content": p.system})];
    for t in &p.turns {
        let content = if t.images.is_empty() {
            json!(t.text)
        } else {
            let mut parts: Vec<Value> = t
                .images
                .iter()
                .map(|i| {
                    let data = base64::engine::general_purpose::STANDARD.encode(i.bytes());
                    json!({"type": "image_url", "image_url": {"url": format!("data:{};base64,{data}", i.mime().as_str())}})
                })
                .collect();
            parts.push(json!({"type": "text", "text": t.text}));
            Value::Array(parts)
        };
        messages.push(json!({"role": t.role, "content": content}));
    }
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "max_tokens": p.max_tokens,
        "temperature": p.temperature,
    });
    if let Some(schema) = &p.json_schema {
        body["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "answer", "schema": schema, "strict": true}});
    }
    body
}

fn finish_of(reason: Option<&str>) -> Finish {
    match reason {
        Some("length") => Finish::Length,
        _ => Finish::Stop,
    }
}

impl LlmProvider for PitwallProvider {
    fn id(&self) -> &str {
        "pitwall"
    }

    fn display_name(&self) -> String {
        format!("{} (Pitwall)", self.model)
    }

    fn context_length(&self) -> usize {
        self.context
    }

    fn vision(&self) -> bool {
        self.vision
    }

    fn chat(&self, req: &ChatRequest, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
        prompted_chat(req, self.vision, on_delta, &mut |p, sink| {
            let body = request_body(p, &self.model_for(p.has_images()));
            let mut text = String::new();
            let result = self
                .client
                .chat_stream(&body, self.priority.header(), self.timeouts, p.cancel.clone(), &mut |t| {
                    text.push_str(t);
                    sink(t);
                });
            if let Some(o) = &self.outcome {
                o(result.as_ref());
            }
            match result {
                Ok(end) => Ok(Generated {
                    text,
                    prompt_tokens: end.prompt_tokens,
                    completion_tokens: end.completion_tokens,
                    finish: finish_of(end.finish_reason.as_deref()),
                }),
                Err(PitwallError::Cancelled) => Ok(Generated {
                    text,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    finish: Finish::Cancelled,
                }),
                Err(e) => Err(e.into()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{prepare, ChatMessage};

    #[test]
    fn the_body_has_system_turns_images_and_schema() {
        let img = crate::llm::ImageInput::new(crate::llm::ImageMime::Png, vec![1, 2, 3], 1, 1);
        let req = ChatRequest {
            system: "sys".into(),
            messages: vec![ChatMessage::user_with_images("look", vec![img]), ChatMessage::assistant("ok")],
            json_schema: Some(json!({"type": "object"})),
            max_tokens: 9,
            temperature: Some(0.5),
            ..ChatRequest::default()
        };
        let b = request_body(&prepare(&req), "auto:chat,8k,vision");
        assert_eq!(b["model"], "auto:chat,8k,vision");
        assert_eq!(b["messages"][0], json!({"role": "system", "content": "sys"}));
        assert_eq!(b["messages"][1]["content"][0]["image_url"]["url"], "data:image/png;base64,AQID");
        assert_eq!(b["messages"][1]["content"][1], json!({"type": "text", "text": "look"}));
        assert_eq!(b["messages"][2], json!({"role": "assistant", "content": "ok"}));
        assert_eq!((b["stream"].as_bool(), b["max_tokens"].as_u64()), (Some(true), Some(9)));
        assert_eq!(b["response_format"]["json_schema"]["schema"], json!({"type": "object"}));
        assert_eq!((finish_of(Some("length")), finish_of(None)), (Finish::Length, Finish::Stop));
        assert_eq!(
            (Priority::Interactive.header(), Priority::Background.header()),
            ("interactive", "background")
        );
    }
}
