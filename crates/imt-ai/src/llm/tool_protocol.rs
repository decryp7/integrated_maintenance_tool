//! Tool calling for local models, independent of any chat template: tools are described in the
//! system prompt and the model answers with `<tool_call>{"name":..,"arguments":{..}}</tool_call>`
//! (the Hermes convention that Qwen and Llama 3.1 instruct models follow). Pure and testable.

use super::types::{ChatMessage, Role, ToolCall, ToolSpec};
use serde_json::{json, Value};

const OPEN: &str = "<tool_call>";
const CLOSE: &str = "</tool_call>";
/// Longest tool-call JSON we try to parse.
const MAX_CALL_LEN: usize = 32 * 1024;

/// The system-prompt section that teaches the model the protocol.
pub fn tools_prompt(tools: &[ToolSpec]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let defs: Vec<String> = tools
        .iter()
        .map(|t| json!({"name": t.name, "description": t.description, "parameters": t.input_schema}).to_string())
        .collect();
    format!(
        "You can call tools to look at the logs. Available tools (JSON schemas):\n<tools>\n{}\n</tools>\n\
         To call a tool, reply with exactly one line per call and nothing else:\n\
         {OPEN}{{\"name\": \"<tool name>\", \"arguments\": {{...}}}}{CLOSE}\n\
         Tool results arrive in <tool_response> tags. When you have enough evidence, answer in plain text.",
        defs.join("\n")
    )
}

/// How a message is shown to the model: (role, content) with calls and results inlined.
pub fn render_message(m: &ChatMessage) -> (&'static str, String) {
    match m.role {
        Role::User => ("user", m.content.clone()),
        Role::Assistant => {
            let mut s = m.content.clone();
            for c in &m.tool_calls {
                if !s.is_empty() {
                    s.push('\n');
                }
                s.push_str(&format!("{OPEN}{}{CLOSE}", json!({"name": c.name, "arguments": c.input})));
            }
            ("assistant", s)
        }
        Role::Tool => ("user", format!("<tool_response>\n{}\n</tool_response>", m.content)),
    }
}

/// Splits model output into visible text and tool calls. Malformed call JSON becomes a call whose
/// input is `{"_invalid_json": "<raw>"}` so the loop can tell the model what went wrong.
pub fn parse_output(text: &str) -> (String, Vec<ToolCall>) {
    // Some models (or tokenizers that hide the opening tag as a special token) emit `{...}</tool_call>` only.
    let repaired;
    let text = match (text.find(OPEN), text.find(CLOSE)) {
        (None, Some(close)) => match text[..close].find('{') {
            Some(brace) => {
                repaired = format!("{}{OPEN}{}", &text[..brace], &text[brace..]);
                repaired.as_str()
            }
            None => text,
        },
        _ => text,
    };
    let mut visible = String::new();
    let mut calls = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        visible.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        let (body, next) = match after.find(CLOSE) {
            Some(end) => (&after[..end], &after[end + CLOSE.len()..]),
            None => (after, ""),
        };
        calls.push(parse_call(body, calls.len()));
        rest = next;
    }
    visible.push_str(rest);
    (visible.trim().to_string(), calls)
}

/// Parses the first JSON value in `text`, ignoring anything after it, and closes brackets and strings that a
/// model left open (a missing `}` or a cut-off answer is common in small models).
pub fn lenient_json(text: &str) -> Option<Value> {
    let text = text.trim();
    if let Some(Ok(v)) = serde_json::Deserializer::from_str(text).into_iter::<Value>().next() {
        return Some(v);
    }
    // Close what is open: track strings and brackets outside strings.
    let (mut stack, mut in_str, mut esc) = (Vec::new(), false, false);
    for c in text.chars() {
        if in_str {
            match (esc, c) {
                (true, _) => esc = false,
                (false, '\\') => esc = true,
                (false, '"') => in_str = false,
                _ => {}
            }
        } else {
            match c {
                '"' => in_str = true,
                '{' => stack.push('}'),
                '[' => stack.push(']'),
                '}' | ']' => {
                    stack.pop();
                }
                _ => {}
            }
        }
    }
    let mut fixed = text.to_string();
    if in_str {
        fixed.push('"');
    }
    while let Some(c) = stack.pop() {
        fixed.push(c);
    }
    serde_json::Deserializer::from_str(&fixed)
        .into_iter::<Value>()
        .next()
        .and_then(Result::ok)
}

/// Qwen3.5's native format: `<function=name><parameter=key>value</parameter></function>`.
fn parse_xml_call(body: &str) -> Option<(String, Value)> {
    let rest = body.strip_prefix("<function=")?;
    let (name, mut rest) = rest.split_once('>')?;
    let mut args = serde_json::Map::new();
    while let Some(p) = rest.find("<parameter=") {
        let after = &rest[p + "<parameter=".len()..];
        let (key, after) = after.split_once('>')?;
        let (value, next) = after.split_once("</parameter>").unwrap_or((after, ""));
        let value = value.trim_matches('\n');
        args.insert(
            key.trim().to_string(),
            serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string())),
        );
        rest = next;
    }
    Some((name.trim().to_string(), Value::Object(args)))
}

fn parse_call(body: &str, n: usize) -> ToolCall {
    let id = format!("call_{}", n + 1);
    let body = body.trim();
    if let Some((name, input)) = parse_xml_call(body) {
        return ToolCall { id, name, input };
    }
    let invalid = |raw: &str| ToolCall {
        id: id.clone(),
        name: String::new(),
        input: json!({"_invalid_json": raw.chars().take(500).collect::<String>()}),
    };
    if body.len() > MAX_CALL_LEN {
        return invalid(body);
    }
    match lenient_json(body) {
        Some(Value::Object(o)) => match o.get("name").and_then(Value::as_str) {
            Some(name) => {
                let args = o.get("arguments").or_else(|| o.get("parameters")).cloned().unwrap_or(json!({}));
                // Some models double-encode the arguments as a string.
                let args = match args {
                    Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
                    other => other,
                };
                ToolCall {
                    id,
                    name: name.to_string(),
                    input: args,
                }
            }
            None => invalid(body),
        },
        _ => invalid(body),
    }
}

/// Forwards streamed text to a callback but stops at the first `<tool_call>` and holds back a
/// possible partial tag, so the UI never flashes protocol markup.
pub struct StreamFilter {
    pending: String,
    in_call: bool,
}

impl Default for StreamFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamFilter {
    pub fn new() -> Self {
        StreamFilter {
            pending: String::new(),
            in_call: false,
        }
    }

    pub fn feed(&mut self, piece: &str, out: &mut dyn FnMut(&str)) {
        if self.in_call {
            return;
        }
        self.pending.push_str(piece);
        if let Some(i) = self.pending.find(OPEN) {
            if i > 0 {
                out(&self.pending[..i]);
            }
            self.pending.clear();
            self.in_call = true;
            return;
        }
        // Keep the longest suffix that is a prefix of the tag.
        let keep = (1..OPEN.len().min(self.pending.len() + 1))
            .rev()
            .find(|&k| self.pending.is_char_boundary(self.pending.len() - k) && OPEN.starts_with(&self.pending[self.pending.len() - k..]))
            .unwrap_or(0);
        let emit = self.pending.len() - keep;
        if emit > 0 {
            out(&self.pending[..emit]);
            self.pending.drain(..emit);
        }
    }

    /// Releases held-back text (it turned out not to be a tag).
    pub fn finish(&mut self, out: &mut dyn FnMut(&str)) {
        if !self.in_call && !self.pending.is_empty() {
            out(&self.pending);
        }
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ToolSpec {
        ToolSpec {
            name: "run_query".into(),
            description: "d".into(),
            input_schema: json!({"type":"object"}),
        }
    }

    #[test]
    fn prompt_lists_tools_and_is_empty_without() {
        assert_eq!(tools_prompt(&[]), "");
        let p = tools_prompt(&[spec()]);
        assert!(p.contains("run_query") && p.contains("<tool_call>"));
    }

    #[test]
    fn renders_roles_calls_and_results() {
        let call = ToolCall {
            id: "c".into(),
            name: "x".into(),
            input: json!({"a": 1}),
        };
        let (r, c) = render_message(&ChatMessage::assistant_calls("hi", vec![call.clone()]));
        assert_eq!(r, "assistant");
        assert!(c.starts_with("hi\n<tool_call>") && c.contains("\"arguments\":{\"a\":1}"));
        let (_, only) = render_message(&ChatMessage::assistant_calls("", vec![call]));
        assert!(only.starts_with("<tool_call>"));
        assert_eq!(render_message(&ChatMessage::user("u")), ("user", "u".to_string()));
        let (r, c) = render_message(&ChatMessage::tool_result("c", "res"));
        assert_eq!(r, "user");
        assert!(c.contains("<tool_response>") && c.contains("res"));
    }

    #[test]
    fn parses_calls_and_text() {
        let (t, calls) = parse_output("Looking.\n<tool_call>{\"name\":\"run_query\",\"arguments\":{\"ql\":\"level=error\"}}</tool_call>");
        assert_eq!(t, "Looking.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "run_query");
        assert_eq!(calls[0].input["ql"], "level=error");
        assert_eq!(calls[0].id, "call_1");
    }

    #[test]
    fn parses_unclosed_double_encoded_parameters_and_multiple() {
        let (_, calls) = parse_output(
            "<tool_call>{\"name\":\"a\",\"arguments\":\"{\\\"k\\\":1}\"}</tool_call><tool_call>{\"name\":\"b\",\"parameters\":{\"z\":2}}",
        );
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].input["k"], 1);
        assert_eq!(calls[1].name, "b");
        assert_eq!(calls[1].input["z"], 2);
        let (_, none) = parse_output("<tool_call>{\"name\":\"c\"}</tool_call>");
        assert_eq!(none[0].input, json!({}));
    }

    #[test]
    fn malformed_calls_are_flagged_not_dropped() {
        for body in ["not json", "[1]", "{\"x\":1}"] {
            let (_, calls) = parse_output(&format!("<tool_call>{body}</tool_call>"));
            assert!(calls[0].name.is_empty());
            assert!(calls[0].input["_invalid_json"].is_string());
        }
        let big = "x".repeat(MAX_CALL_LEN + 1);
        let (_, calls) = parse_output(&format!("<tool_call>{big}</tool_call>"));
        assert!(calls[0].input["_invalid_json"].as_str().unwrap().len() <= 500);
    }

    #[test]
    fn parses_the_qwen_xml_form() {
        let (_, calls) = parse_output(
            "<tool_call>\n<function=get_events>\n<parameter=ids>\n[1, 2]\n</parameter>\n<parameter=note>\nhello\n</parameter>\n</function>\n</tool_call>",
        );
        assert_eq!(calls[0].name, "get_events");
        assert_eq!(calls[0].input["ids"], json!([1, 2]));
        assert_eq!(calls[0].input["note"], "hello");
        let (_, c) = parse_output("<tool_call><function=x></function></tool_call>");
        assert_eq!(c[0].input, json!({}));
        let (_, bad) = parse_output("<tool_call><function=x><parameter=k</tool_call>");
        assert!(bad[0].name.is_empty());
    }

    #[test]
    fn lenient_parsing_of_sloppy_calls() {
        // trailing prose, a missing closing brace, a cut-off string
        let (_, c) = parse_output("<tool_call>{\"name\":\"a\",\"arguments\":{\"k\":1}} and then some words</tool_call>");
        assert_eq!(c[0].name, "a");
        let (_, c) = parse_output("<tool_call>{\"name\":\"a\",\"arguments\":{\"k\":{\"z\":[1,2]}}");
        assert_eq!((c[0].name.as_str(), c[0].input["k"]["z"][1].clone()), ("a", json!(2)));
        let (_, c) = parse_output("<tool_call>{\"name\":\"a\",\"arguments\":{\"k\":\"cut off");
        assert_eq!(c[0].input["k"], "cut off");
        assert!(lenient_json("no json at all").is_none());
        assert!(lenient_json("").is_none());
        assert_eq!(lenient_json("[1, 2").unwrap(), json!([1, 2]));
        assert_eq!(lenient_json("{\"a\": \"q\\\"uote\"}").unwrap()["a"], "q\"uote");
    }

    #[test]
    fn a_missing_opening_tag_is_repaired() {
        let (t, c) = parse_output("Checking. {\"name\":\"run_query\",\"arguments\":{\"ql\":\"x\"}} </tool_call>");
        assert_eq!((t.as_str(), c.len(), c[0].name.as_str()), ("Checking.", 1, "run_query"));
        // a stray closing tag with nothing before it is just text
        let (t, c) = parse_output("nothing </tool_call>");
        assert!(c.is_empty() && t.contains("nothing"));
    }

    #[test]
    fn plain_text_passes_through() {
        let (t, c) = parse_output("  just an answer ");
        assert_eq!(t, "just an answer");
        assert!(c.is_empty());
    }

    fn run(pieces: &[&str]) -> String {
        let mut f = StreamFilter::new();
        let mut out = String::new();
        for p in pieces {
            f.feed(p, &mut |s| out.push_str(s));
        }
        f.finish(&mut |s| out.push_str(s));
        out
    }

    #[test]
    fn filter_hides_calls_even_when_split() {
        assert_eq!(run(&["Hello ", "wor", "ld"]), "Hello world");
        assert_eq!(run(&["Checking <tool", "_call>{\"name\":\"x\"}"]), "Checking ");
        assert_eq!(run(&["a <", "b"]), "a <b");
        assert_eq!(run(&["<tool_call>{}", " more"]), "");
        assert_eq!(run(&["x<tool_call"]), "x<tool_call");
        assert_eq!(run(&["é<", "ü"]), "é<ü");
    }
}
