//! The llama-server `/completion` and `/tokenize` wire formats, as pure functions: the request bodies IMT
//! sends and the stream events it reads back. See llama.cpp `tools/server/README.md`.

use imt_ai::worker::protocol::{code, context_overflow, ChatParams, RpcError, WireFinish};
use serde_json::{json, Value};

/// Tags that must appear in the text even when the model's tokenizer marks them as special tokens: IMT's
/// prompt formats parse them (tool calls, reasoning blocks). llama-server prints a special token only
/// when it is listed in `preserved_tokens`.
pub const PRESERVED_TAGS: &[&str] = &["<tool_call>", "</tool_call>", "<think>", "</think>"];

/// Seed of the sampler when the temperature is above zero (fixed, so answers are reproducible).
pub const SEED: u64 = 1234;

/// The body of `POST /tokenize`, counting tokens exactly as `/completion` will (BOS added, special tags parsed).
pub fn tokenize_body(prompt: &str) -> Value {
    json!({"content": prompt, "add_special": true, "parse_special": true})
}

/// The number of tokens in a `/tokenize` answer.
pub fn token_count(v: &Value) -> Option<usize> {
    v.get("tokens")?.as_array().map(Vec::len)
}

/// The media marker IMT renders prompts with (llama.cpp's default, `llm::prompted::MEDIA_MARKER`).
pub const DEFAULT_MEDIA_MARKER: &str = "<__media__>";

/// The server's media marker from its `/props` (`media_marker`): llama-server builds choose their own marker and look
/// only for it, so a prompt with the default one counted no markers ("number of media markers in text (0) does not
/// match number of bitmaps (1)"). `None` when the server does not say (older builds use the default).
pub fn media_marker(props: &Value) -> Option<String> {
    props
        .get("media_marker")
        .and_then(Value::as_str)
        .filter(|m| !m.trim().is_empty() && m.len() <= 128)
        .map(str::to_string)
}

/// `prompt` with IMT's markers turned into the server's `marker`. Any text that already looks like the server's
/// marker is removed first, so only IMT's markers (one per image) remain.
pub fn with_media_marker(prompt: &str, marker: &str) -> String {
    if marker == DEFAULT_MEDIA_MARKER {
        return prompt.to_string();
    }
    let mut cleaned = prompt.to_string();
    while cleaned.contains(marker) {
        cleaned = cleaned.replace(marker, "");
    }
    cleaned.replace(DEFAULT_MEDIA_MARKER, marker)
}

/// The `prompt` of `/completion`: the text, or with images llama-server's multimodal form (the text with one
/// media marker per image, and the images' base64 in the same order).
fn prompt_of(p: &ChatParams) -> Value {
    if p.images.is_empty() {
        return json!(p.prompt);
    }
    let data: Vec<&str> = p.images.iter().map(|i| i.data.as_str()).collect();
    json!({"prompt_string": p.prompt, "multimodal_data": data})
}

/// The body of `POST /completion` for a streamed answer of at most `n_predict` tokens.
pub fn completion_body(p: &ChatParams, n_predict: usize) -> Value {
    let mut body = json!({
        "prompt": prompt_of(p),
        "n_predict": n_predict,
        "stream": true,
        "cache_prompt": true,
        "preserved_tokens": PRESERVED_TAGS,
        "return_progress": false,
        // Same sampling as IMT's compiled-in engine had: greedy, or top-k 40 / top-p 0.95 / temperature.
        "min_p": 0.0,
        "repeat_penalty": 1.0,
    });
    let Some(o) = body.as_object_mut() else {
        return body;
    };
    if p.temperature <= 0.01 {
        o.insert("temperature".into(), json!(0.0));
        o.insert("top_k".into(), json!(1));
        o.insert("samplers".into(), json!(["top_k"]));
    } else {
        o.insert("temperature".into(), json!(p.temperature));
        o.insert("top_k".into(), json!(40));
        o.insert("top_p".into(), json!(0.95));
        o.insert("seed".into(), json!(SEED));
        o.insert("samplers".into(), json!(["top_k", "top_p", "temperature"]));
    }
    if let Some(schema) = &p.json_schema {
        o.insert("json_schema".into(), schema.clone());
    }
    body
}

/// One event of the `/completion` stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Delta(String),
    /// The final event.
    Done {
        finish: WireFinish,
        prompt_tokens: Option<usize>,
        completion_tokens: Option<usize>,
    },
    Error(RpcError),
}

/// Parses one line of the SSE stream: nothing for blank lines, comments and other fields; a final event
/// that carries the last piece of text gives the text first, then the end.
pub fn parse_stream_line(line: &str) -> Vec<Event> {
    let Some(data) = line.strip_prefix("data:").or_else(|| line.strip_prefix("error:")).map(str::trim) else {
        return vec![];
    };
    if data.is_empty() || data == "[DONE]" {
        return vec![];
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return vec![Event::Error(RpcError::new(
            code::ENGINE,
            "llama-server sent a malformed stream event",
        ))];
    };
    if v.get("error").is_some() {
        return vec![Event::Error(server_error(400, &v))];
    }
    let mut out = Vec::new();
    if let Some(content) = v.get("content").and_then(Value::as_str).filter(|c| !c.is_empty()) {
        out.push(Event::Delta(content.to_string()));
    }
    if v.get("stop").and_then(Value::as_bool) == Some(true) {
        let finish = match v.get("stop_type").and_then(Value::as_str) {
            Some("limit") => WireFinish::Length,
            _ => WireFinish::Stop,
        };
        let n = |k: &str| v.get(k).and_then(Value::as_u64).map(|n| n as usize);
        out.push(Event::Done {
            finish,
            prompt_tokens: n("tokens_evaluated"),
            completion_tokens: n("tokens_predicted"),
        });
    }
    out
}

/// Maps an error answer of llama-server (`{"error": {"code", "message", "type", ...}}`) to a typed error.
pub fn server_error(status: u16, v: &Value) -> RpcError {
    let e = v.get("error").unwrap_or(v);
    let message: String = e
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("llama-server reported an error")
        .chars()
        .take(500)
        .collect();
    let kind = e.get("type").and_then(Value::as_str).unwrap_or_default();
    let n = |k: &str| e.get(k).and_then(Value::as_u64).map(|n| n as usize);
    match kind {
        "exceed_context_size_error" => context_overflow(n("n_prompt_tokens").unwrap_or(0), n("n_ctx").unwrap_or(0)),
        "authentication_error" => RpcError::new(code::ENGINE, "llama-server refused the worker's key; restart the AI engine"),
        _ if message.contains("json_schema") || message.contains("JSON schema") => {
            RpcError::new(code::INVALID_PARAMS, format!("unsupported output schema: {message}"))
        }
        "invalid_request_error" => RpcError::new(code::INVALID_PARAMS, message),
        _ if status == 503 => RpcError::new(code::ENGINE, format!("llama-server is not ready: {message}")),
        _ => RpcError::new(code::ENGINE, format!("llama-server: {message}")),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn prompts_use_the_servers_own_media_marker() {
        let props = json!({"media_marker": "<__media_ab12__>"});
        let m = media_marker(&props).unwrap();
        assert_eq!(
            with_media_marker(
                "<__media__>
what is this?",
                &m
            ),
            "<__media_ab12__>
what is this?"
        );
        // Text that already looks like the server's marker cannot claim an image slot.
        assert_eq!(
            with_media_marker(
                "<__media__>
log says <__media_ab12__> here",
                &m
            ),
            "<__media_ab12__>
log says  here"
        );
        assert_eq!(with_media_marker("<__media__>x", DEFAULT_MEDIA_MARKER), "<__media__>x");
        for bad in [
            json!({}),
            json!({"media_marker": ""}),
            json!({"media_marker": 5}),
            json!({"media_marker": "x".repeat(200)}),
        ] {
            assert_eq!(media_marker(&bad), None, "{bad}");
        }
    }

    use super::*;

    fn params(temp: f32, schema: Option<Value>) -> ChatParams {
        ChatParams {
            prompt: "<|im_start|>user\nhi".into(),
            max_tokens: 100,
            temperature: temp,
            json_schema: schema,
            images: vec![],
        }
    }

    #[test]
    fn greedy_and_sampled_bodies() {
        let g = completion_body(&params(0.0, None), 50);
        assert_eq!(
            (g["n_predict"].as_u64(), g["stream"].as_bool(), g["top_k"].as_u64()),
            (Some(50), Some(true), Some(1))
        );
        assert_eq!(g["cache_prompt"], true);
        assert!(g.get("json_schema").is_none() && g.get("seed").is_none());
        assert_eq!(g["preserved_tokens"].as_array().unwrap().len(), 4);
        let s = completion_body(&params(0.7, Some(json!({"type": "object"}))), 10);
        assert_eq!((s["top_k"].as_u64(), s["seed"].as_u64()), (Some(40), Some(SEED)));
        assert!((s["temperature"].as_f64().unwrap() - 0.7).abs() < 1e-6);
        assert!((s["top_p"].as_f64().unwrap() - 0.95).abs() < 1e-6);
        assert_eq!(s["json_schema"], json!({"type": "object"}));
        assert_eq!(tokenize_body("x")["parse_special"], true);
        assert_eq!(g["prompt"], "<|im_start|>user\nhi");
        let mut with_image = params(0.0, None);
        with_image.prompt = "<__media__>\nwhat is this?".into();
        with_image.images = vec![imt_ai::worker::protocol::WireImage {
            mime: "image/png".into(),
            data: "iVBORw0KGgo=".into(),
            width: 2,
            height: 2,
        }];
        let m = completion_body(&with_image, 8);
        assert_eq!(m["prompt"]["prompt_string"], "<__media__>\nwhat is this?");
        assert_eq!(m["prompt"]["multimodal_data"], json!(["iVBORw0KGgo="]));
        assert_eq!(token_count(&json!({"tokens": [1, 2, 3]})), Some(3));
        assert_eq!(token_count(&json!({})), None);
    }

    #[test]
    fn stream_events() {
        assert_eq!(
            parse_stream_line(r#"data: {"content":"Hi","stop":false}"#),
            [Event::Delta("Hi".into())]
        );
        for quiet in [
            r#"data: {"content":"","stop":false}"#,
            "",
            ": keep-alive",
            "data: [DONE]",
            "event: x",
        ] {
            assert!(parse_stream_line(quiet).is_empty(), "{quiet}");
        }
        assert_eq!(
            parse_stream_line(r#"data: {"content":"","stop":true,"stop_type":"eos","tokens_predicted":7,"tokens_evaluated":12}"#),
            [Event::Done {
                finish: WireFinish::Stop,
                prompt_tokens: Some(12),
                completion_tokens: Some(7)
            }]
        );
        assert_eq!(
            parse_stream_line(r#"data: {"content":"end","stop":true,"stop_type":"limit"}"#),
            [
                Event::Delta("end".into()),
                Event::Done {
                    finish: WireFinish::Length,
                    prompt_tokens: None,
                    completion_tokens: None
                }
            ]
        );
        assert!(matches!(&parse_stream_line("data: {broken")[..], [Event::Error(e)] if e.code == code::ENGINE));
        assert!(matches!(
            &parse_stream_line(r#"error: {"error":{"code":500,"message":"boom","type":"server_error"}}"#)[..],
            [Event::Error(e)] if e.message.contains("boom")
        ));
    }

    #[test]
    fn error_answers_are_typed() {
        let o = server_error(
            400,
            &json!({"error": {"code": 400, "type": "exceed_context_size_error", "n_prompt_tokens": 402, "n_ctx": 128, "message": "x"}}),
        );
        assert_eq!(o, context_overflow(402, 128));
        let s = server_error(
            400,
            &json!({"error": {"type": "invalid_request_error", "message": "Field 'json_schema': bad"}}),
        );
        assert_eq!(s.code, code::INVALID_PARAMS);
        assert!(s.message.starts_with("unsupported output schema"));
        assert_eq!(
            server_error(400, &json!({"error": {"type": "invalid_request_error", "message": "m"}})).code,
            code::INVALID_PARAMS
        );
        assert!(server_error(401, &json!({"error": {"type": "authentication_error"}}))
            .message
            .contains("key"));
        assert!(server_error(503, &json!({"error": {"message": "Loading model"}}))
            .message
            .contains("not ready"));
        let generic = server_error(500, &json!({"message": "y".repeat(2000)}));
        assert_eq!(generic.code, code::ENGINE);
        assert!(generic.message.len() < 600);
    }
}
