//! `PromptedProvider`: turns any raw text-generation engine into an `LlmProvider` by rendering the
//! prompt for the model family, teaching it the tool protocol, hiding thinking and protocol markup
//! from the stream, and parsing tool calls. All engine-independent, so it is fully testable.

use super::prompt_format::PromptFormat;
use super::provider::LlmProvider;
use super::tool_protocol::{parse_output, render_message, tools_prompt, StreamFilter};
use super::types::*;
use crate::error::Result;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    Stop,
    Length,
    Cancelled,
}

pub struct GenParams {
    pub max_tokens: usize,
    pub temperature: f32,
    pub json_schema: Option<Value>,
    pub cancel: Option<Arc<AtomicBool>>,
    /// The images of the prompt, in the order of their [`MEDIA_MARKER`]s.
    pub images: Vec<ImageInput>,
}

/// Where an image goes in a rendered prompt: llama.cpp's multimodal marker (`mtmd_default_marker`), which the
/// engine replaces with the image's tokens. It is removed from every text first, so a log line or a web page that
/// contains it cannot claim an image slot.
pub const MEDIA_MARKER: &str = "<__media__>";

/// `text` without any [`MEDIA_MARKER`].
pub fn strip_media_markers(text: &str) -> String {
    let mut s = text.to_string();
    while s.contains(MEDIA_MARKER) {
        s = s.replace(MEDIA_MARKER, "");
    }
    s
}

pub struct Generated {
    pub text: String,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub finish: Finish,
}

/// A raw engine: prompt text in, text out. `on_text` receives decoded pieces as they are produced.
/// Contract: returns `AiError::ContextOverflow` when the prompt does not fit, stops between tokens when
/// `cancel` is set (`Finish::Cancelled`), and never panics.
pub trait TextEngine: Send + Sync {
    fn name(&self) -> String;
    fn context_length(&self) -> usize;
    /// Whether the engine reads images (`GenParams::images`).
    fn vision(&self) -> bool {
        false
    }
    fn generate(&self, prompt: &str, params: &GenParams, on_text: &mut dyn FnMut(&str)) -> Result<Generated>;
}

pub const DEFAULT_MAX_TOKENS: usize = 1024;
pub const DEFAULT_TEMPERATURE: f32 = 0.2;

pub struct PromptedProvider<E: TextEngine> {
    engine: E,
    format: PromptFormat,
}

impl<E: TextEngine> PromptedProvider<E> {
    pub fn new(engine: E, format: PromptFormat) -> Self {
        PromptedProvider { engine, format }
    }
}

/// Removes `<think>…</think>` blocks (an unterminated one removes the rest).
pub fn strip_think(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("<think>") {
        out.push_str(&rest[..i]);
        match rest[i..].find("</think>") {
            Some(j) => rest = &rest[i + j + "</think>".len()..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Drops a leading `<think>…</think>` block from a stream, deciding as soon as enough text arrives.
struct ThinkFilter {
    state: ThinkState,
}

enum ThinkState {
    Deciding(String),
    InThink(String),
    Pass,
}

impl ThinkFilter {
    fn new() -> Self {
        ThinkFilter {
            state: ThinkState::Deciding(String::new()),
        }
    }

    fn feed(&mut self, piece: &str, out: &mut dyn FnMut(&str)) {
        match &mut self.state {
            ThinkState::Pass => out(piece),
            ThinkState::Deciding(buf) => {
                buf.push_str(piece);
                let t = buf.trim_start();
                if t.starts_with("<think>") {
                    let b = std::mem::take(buf);
                    self.state = ThinkState::InThink(b);
                    self.feed("", out);
                } else if !"<think>".starts_with(t) {
                    let b = std::mem::take(buf);
                    self.state = ThinkState::Pass;
                    out(&b);
                }
            }
            ThinkState::InThink(buf) => {
                buf.push_str(piece);
                if let Some(j) = buf.find("</think>") {
                    let rest = buf[j + "</think>".len()..].trim_start().to_string();
                    self.state = ThinkState::Pass;
                    if !rest.is_empty() {
                        out(&rest);
                    }
                }
            }
        }
    }

    fn finish(&mut self, out: &mut dyn FnMut(&str)) {
        if let ThinkState::Deciding(buf) = &mut self.state {
            if !buf.is_empty() {
                out(buf);
            }
        }
        self.state = ThinkState::Pass;
    }
}

impl<E: TextEngine> LlmProvider for PromptedProvider<E> {
    fn id(&self) -> &str {
        "embedded"
    }

    fn display_name(&self) -> String {
        self.engine.name()
    }

    fn context_length(&self) -> usize {
        self.engine.context_length()
    }

    fn vision(&self) -> bool {
        self.engine.vision()
    }

    fn chat(&self, req: &ChatRequest, on_delta: &mut dyn FnMut(&str)) -> Result<ChatResponse> {
        let format = self.format;
        prompted_chat(req, self.engine.vision(), on_delta, &mut |prepared, sink| {
            // llama.cpp's multimodal marker stands where each image goes, in the order of `params.images`.
            let mut images: Vec<ImageInput> = Vec::new();
            let turns: Vec<(&str, String)> = prepared
                .turns
                .iter()
                .map(|t| {
                    if t.images.is_empty() {
                        return (t.role, t.text.clone());
                    }
                    images.extend(t.images.iter().cloned());
                    (t.role, format!("{}\n{}", MEDIA_MARKER.repeat(t.images.len()), t.text))
                })
                .collect();
            let prompt = format.render(&prepared.system, &turns);
            let params = GenParams {
                max_tokens: prepared.max_tokens,
                temperature: prepared.temperature,
                json_schema: prepared.json_schema.clone(),
                cancel: prepared.cancel.clone(),
                images,
            };
            self.engine.generate(&prompt, &params, sink)
        })
    }
}

/// One turn of a prepared conversation: the role as the model sees it, its text (tool calls and results inlined, no
/// media markers) and the images shown with it.
#[derive(Debug, Clone)]
pub struct PreparedTurn {
    pub role: &'static str,
    pub text: String,
    pub images: Vec<ImageInput>,
}

/// A request ready for an engine: the system prompt (with the tool protocol when tools are offered), the turns and the
/// generation parameters. Every engine (the built-in raw-prompt one and Pitwall's chat API) runs the same preparation, so
/// they behave the same (ADR 0059).
#[derive(Debug, Clone)]
pub struct Prepared {
    pub system: String,
    pub turns: Vec<PreparedTurn>,
    pub max_tokens: usize,
    pub temperature: f32,
    pub json_schema: Option<Value>,
    pub cancel: Option<Arc<AtomicBool>>,
    /// Whether the tool protocol is in the system prompt (and tool calls are parsed from the answer).
    pub with_tools: bool,
}

impl Prepared {
    /// Whether any turn carries images.
    pub fn has_images(&self) -> bool {
        self.turns.iter().any(|t| !t.images.is_empty())
    }
}

/// An engine's generation for [`prompted_chat`]: streams raw text through the sink and returns all of it.
pub type Generate<'a> = dyn FnMut(&Prepared, &mut dyn FnMut(&str)) -> Result<Generated> + 'a;

/// Prepares `req` for an engine (tool protocol, no media markers in any text, images per turn).
pub fn prepare(req: &ChatRequest) -> Prepared {
    let with_tools = !req.tools.is_empty() && req.json_schema.is_none();
    let mut system = strip_media_markers(&req.system);
    if with_tools {
        system.push_str("\n\n");
        system.push_str(&tools_prompt(&req.tools));
    }
    let turns = req
        .messages
        .iter()
        .map(|m| {
            let (role, text) = render_message(m);
            PreparedTurn {
                role,
                text: strip_media_markers(&text),
                images: m.images.clone(),
            }
        })
        .collect();
    Prepared {
        system,
        turns,
        max_tokens: if req.max_tokens == 0 { DEFAULT_MAX_TOKENS } else { req.max_tokens },
        temperature: req.temperature.unwrap_or(DEFAULT_TEMPERATURE),
        json_schema: req.json_schema.clone(),
        cancel: req.cancel.clone(),
        with_tools,
    }
}

/// Runs a chat on any engine that generates text for a [`Prepared`] conversation: refuses images without `vision`,
/// hides thinking and protocol markup from the stream, parses tool calls and maps the finish reason. `generate` streams
/// the raw text through its sink and returns the whole of it.
pub fn prompted_chat(req: &ChatRequest, vision: bool, on_delta: &mut dyn FnMut(&str), generate: &mut Generate<'_>) -> Result<ChatResponse> {
    let prepared = prepare(req);
    if prepared.has_images() && !vision {
        return Err(crate::error::AiError::NoVision);
    }
    let with_tools = prepared.with_tools;
    let json_mode = prepared.json_schema.is_some();
    let mut think = ThinkFilter::new();
    let mut tags = StreamFilter::new();
    let generated = {
        let mut sink = |piece: &str| {
            if json_mode {
                on_delta(piece);
            } else if with_tools {
                think.feed(piece, &mut |p| tags.feed(p, on_delta));
            } else {
                think.feed(piece, on_delta);
            }
        };
        generate(&prepared, &mut sink)?
    };
    if !json_mode {
        if with_tools {
            think.finish(&mut |p| tags.feed(p, on_delta));
            tags.finish(on_delta);
        } else {
            think.finish(on_delta);
        }
    }
    let cleaned = if json_mode {
        generated.text.trim().to_string()
    } else {
        strip_think(&generated.text)
    };
    let (text, tool_calls) = if with_tools { parse_output(&cleaned) } else { (cleaned, vec![]) };
    let stop_reason = match generated.finish {
        Finish::Cancelled => StopReason::Cancelled,
        _ if !tool_calls.is_empty() => StopReason::ToolUse,
        Finish::Length => StopReason::MaxTokens,
        Finish::Stop => StopReason::EndTurn,
    };
    Ok(ChatResponse {
        text,
        tool_calls,
        stop_reason,
        usage: Some(Usage {
            input_tokens: generated.prompt_tokens,
            output_tokens: generated.completion_tokens,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    struct Fake {
        output: String,
        finish: Finish,
        prompts: Mutex<Vec<String>>,
        schema_seen: Mutex<Option<Value>>,
        vision: bool,
        images_seen: Mutex<usize>,
    }

    fn fake(output: &str, finish: Finish) -> Fake {
        Fake {
            output: output.into(),
            finish,
            prompts: Mutex::new(vec![]),
            schema_seen: Mutex::new(None),
            vision: false,
            images_seen: Mutex::new(0),
        }
    }

    impl TextEngine for Fake {
        fn name(&self) -> String {
            "fake".into()
        }
        fn context_length(&self) -> usize {
            2048
        }
        fn vision(&self) -> bool {
            self.vision
        }
        fn generate(&self, prompt: &str, p: &GenParams, on_text: &mut dyn FnMut(&str)) -> Result<Generated> {
            self.prompts.lock().unwrap().push(prompt.to_string());
            *self.schema_seen.lock().unwrap() = p.json_schema.clone();
            *self.images_seen.lock().unwrap() = p.images.len();
            for chunk in self.output.as_bytes().chunks(5) {
                on_text(&String::from_utf8_lossy(chunk));
            }
            Ok(Generated {
                text: self.output.clone(),
                prompt_tokens: 10,
                completion_tokens: 3,
                finish: self.finish,
            })
        }
    }

    fn run(engine: Fake, req: ChatRequest) -> (ChatResponse, String, Fake) {
        let p = PromptedProvider::new(engine, PromptFormat::ChatMl { no_think: false });
        let mut streamed = String::new();
        let r = p.chat(&req, &mut |d| streamed.push_str(d)).unwrap();
        (r, streamed, p.engine)
    }

    fn tool() -> ToolSpec {
        ToolSpec {
            name: "run_query".into(),
            description: "q".into(),
            input_schema: json!({"type":"object"}),
        }
    }

    #[test]
    fn plain_answer_streams_and_reports_usage() {
        let req = ChatRequest {
            system: "be brief".into(),
            messages: vec![ChatMessage::user("hello")],
            ..ChatRequest::default()
        };
        let (r, streamed, e) = run(fake("Hi there, friend.", Finish::Stop), req);
        assert_eq!((r.text.as_str(), streamed.as_str()), ("Hi there, friend.", "Hi there, friend."));
        assert_eq!(r.stop_reason, StopReason::EndTurn);
        assert_eq!(
            r.usage,
            Some(Usage {
                input_tokens: 10,
                output_tokens: 3
            })
        );
        assert!(e.prompts.lock().unwrap()[0].contains("be brief") && !e.prompts.lock().unwrap()[0].contains("<tools>"));
        let p = PromptedProvider::new(fake("", Finish::Stop), PromptFormat::Phi);
        assert_eq!(
            (p.id(), p.display_name(), p.context_length()),
            ("embedded", "fake".to_string(), 2048)
        );
    }

    #[test]
    fn tool_calls_are_parsed_and_hidden_from_the_stream() {
        let out = "Let me check.\n<tool_call>{\"name\":\"run_query\",\"arguments\":{\"ql\":\"level=error\"}}</tool_call>";
        let req = ChatRequest {
            messages: vec![ChatMessage::user("why")],
            tools: vec![tool()],
            ..ChatRequest::default()
        };
        let (r, streamed, e) = run(fake(out, Finish::Stop), req);
        assert_eq!(r.stop_reason, StopReason::ToolUse);
        assert_eq!(r.tool_calls[0].name, "run_query");
        assert_eq!(r.text, "Let me check.");
        assert_eq!(streamed.trim(), "Let me check.");
        assert!(e.prompts.lock().unwrap()[0].contains("<tools>"));
    }

    #[test]
    fn thinking_is_hidden_in_stream_and_result() {
        let req = ChatRequest {
            messages: vec![ChatMessage::user("x")],
            ..ChatRequest::default()
        };
        let (r, streamed, _) = run(fake("<think>\nhmm, let me see\n</think>\n\nThe answer.", Finish::Stop), req.clone());
        assert_eq!((r.text.as_str(), streamed.as_str()), ("The answer.", "The answer."));
        let (r, streamed, _) = run(fake("  <thi", Finish::Stop), req.clone());
        assert_eq!((r.text.as_str(), streamed.as_str()), ("<thi", "  <thi"));
        let (r, streamed, _) = run(fake("<think>never closed", Finish::Length), req);
        assert_eq!((r.text.as_str(), streamed.as_str()), ("", ""));
        assert_eq!(r.stop_reason, StopReason::MaxTokens);
        assert_eq!(strip_think("a<think>b</think>c<think>d"), "ac");
    }

    #[test]
    fn json_mode_passes_the_schema_and_skips_filters() {
        let schema = json!({"type":"object"});
        let req = ChatRequest {
            messages: vec![ChatMessage::user("x")],
            json_schema: Some(schema.clone()),
            tools: vec![tool()],
            ..ChatRequest::default()
        };
        let (r, streamed, e) = run(fake("{\"a\": 1}", Finish::Stop), req);
        assert_eq!(r.text, "{\"a\": 1}");
        assert_eq!(streamed, "{\"a\": 1}");
        assert!(r.tool_calls.is_empty());
        assert_eq!(e.schema_seen.lock().unwrap().clone(), Some(schema));
        assert!(!e.prompts.lock().unwrap()[0].contains("<tools>"));
    }

    fn image() -> ImageInput {
        ImageInput::new(ImageMime::Png, vec![1, 2, 3], 4, 3)
    }

    #[test]
    fn images_go_where_their_markers_are_and_nowhere_else() {
        let req = ChatRequest {
            system: format!("sys {MEDIA_MARKER}"),
            messages: vec![
                ChatMessage::user(format!("a log line with {MEDIA_MARKER} in it")),
                ChatMessage::assistant("ok"),
                ChatMessage::user_with_images("what is <__media<__media__>__>this?", vec![image(), image()]),
            ],
            ..ChatRequest::default()
        };
        let mut engine = fake("A dialog.", Finish::Stop);
        engine.vision = true;
        let (r, _, e) = run(engine, req.clone());
        assert_eq!(r.text, "A dialog.");
        let prompt = e.prompts.lock().unwrap()[0].clone();
        // Two markers, both in the last user message; none from the system text or the log line.
        assert_eq!(prompt.matches(MEDIA_MARKER).count(), 2, "{prompt}");
        assert!(prompt.contains(&format!("{MEDIA_MARKER}{MEDIA_MARKER}\nwhat is this?")), "{prompt}");
        assert!(prompt.contains("a log line with  in it"));
        assert_eq!(*e.images_seen.lock().unwrap(), 2);
        assert_eq!(strip_media_markers("x<__media__>y"), "xy");
        // A model without vision refuses images instead of ignoring them.
        let p = PromptedProvider::new(fake("", Finish::Stop), PromptFormat::ChatMl { no_think: false });
        assert!(!p.vision());
        assert_eq!(p.chat(&req, &mut |_| {}).unwrap_err(), crate::error::AiError::NoVision);
        let img = image();
        assert_eq!(
            (img.mime().as_str(), img.bytes(), img.width(), img.height()),
            ("image/png", &[1u8, 2, 3][..], 4, 3)
        );
        assert_eq!(ImageMime::Jpeg.as_str(), "image/jpeg");
    }

    #[test]
    fn finish_reasons() {
        let req = ChatRequest {
            messages: vec![ChatMessage::user("x")],
            ..ChatRequest::default()
        };
        assert_eq!(run(fake("a", Finish::Cancelled), req.clone()).0.stop_reason, StopReason::Cancelled);
        assert_eq!(run(fake("a", Finish::Length), req).0.stop_reason, StopReason::MaxTokens);
    }
}
