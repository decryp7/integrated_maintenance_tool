//! The tools of the chat model: the read-only engine tools, the propose_* tools (which return
//! engine-validated proposals for the user to apply; nothing here changes anything) and any extra tool
//! hosts (the browser tools).

use crate::access::StoreRef;
use crate::assistant::{Assistant, EngineTools, Proposal, ToolHost, ToolOutcome};
use crate::llm::{LlmProvider, ToolSpec};
use debrief_core::api::ContextRequest;
use serde::Deserialize;
use serde_json::{json, Value};

/// Tools dropped on small context windows to leave room for the conversation.
const COMPACT_DROPS: &[&str] = &["check_query", "field_stats", "neighbours"];
const SAMPLE_RADIUS: usize = 30;

pub struct ChatHost<'a, P: LlmProvider> {
    pub(crate) engine: EngineTools<'a>,
    assistant: &'a Assistant<P>,
    store: StoreRef<'a>,
    sample_id: Option<u32>,
    compact: bool,
    extra: Vec<Box<dyn ToolHost + 'a>>,
}

fn spec(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type": "object", "properties": properties, "required": required}),
    }
}

pub fn propose_specs() -> Vec<ToolSpec> {
    let rid = json!({"type": "integer", "description": "An event in the file to use as the sample (default: the selected event)"});
    let goal = json!({"type": "string", "description": "What the user wants, in plain words"});
    vec![
        spec(
            "propose_regex",
            "ONLY when the user asks you to write a regex: propose one for example log lines. The user reviews it; nothing is changed.",
            json!({"example_lines": {"type": "array", "items": {"type": "string"}}, "intent": {"type": "string"}}),
            &["example_lines"],
        ),
        spec(
            "propose_query",
            "ONLY when the user asks you to write or suggest a query for them. To answer a question use run_query instead.",
            json!({"question": {"type": "string"}}),
            &["question"],
        ),
        spec(
            "propose_parser",
            "Propose how to parse the file an event came from (format, timestamp, regex).",
            json!({"record_id": rid}),
            &[],
        ),
        spec(
            "propose_pipeline",
            "Propose pipeline steps (drop, mask, dedupe, ...) validated on a sample of the file.",
            json!({"goal": goal, "record_id": rid}),
            &["goal"],
        ),
        spec(
            "propose_markers",
            "Propose timeline markers validated on a sample of the file.",
            json!({"goal": goal, "record_id": rid}),
            &["goal"],
        ),
        spec(
            "propose_extractors",
            "Propose field extractors validated on a sample of the file.",
            json!({"goal": goal, "record_id": rid}),
            &["goal"],
        ),
    ]
}

#[derive(Deserialize)]
struct RegexArgs {
    example_lines: Vec<String>,
    #[serde(default)]
    intent: Option<String>,
}

#[derive(Deserialize)]
struct QuestionArgs {
    question: String,
}

#[derive(Deserialize)]
struct SampleArgs {
    #[serde(default)]
    goal: String,
    #[serde(default)]
    record_id: Option<u32>,
}

impl<'a, P: LlmProvider> ChatHost<'a, P> {
    pub fn new(
        assistant: &'a Assistant<P>,
        store: impl Into<StoreRef<'a>>,
        engine: EngineTools<'a>,
        sample_id: Option<u32>,
        compact: bool,
    ) -> Self {
        ChatHost {
            engine,
            assistant,
            store: store.into(),
            sample_id,
            compact,
            extra: vec![],
        }
    }

    pub fn with_extra(mut self, tools: Box<dyn ToolHost + 'a>) -> Self {
        self.extra.push(tools);
        self
    }

    /// Lines around an event (the sample for profile, pipeline, marker and extractor proposals) and the file's name.
    fn sample(&self, record_id: Option<u32>) -> Result<(String, String), Box<ToolOutcome>> {
        let id = record_id
            .or(self.sample_id)
            .ok_or_else(|| Box::new(ToolOutcome::err("select an event in the file first, or pass record_id")))?;
        let req = ContextRequest {
            record_id: id,
            before: SAMPLE_RADIUS,
            after: SAMPLE_RADIUS,
        };
        let ctx = self
            .store
            .with(|s| s.source_context(&req))
            .map_err(|e| Box::new(ToolOutcome::err(format!("cannot read the file for event {id}: {e:#}"))))?;
        let text = ctx.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
        Ok((text, ctx.source_name))
    }

    fn proposal(p: Proposal, facts: String) -> ToolOutcome {
        let text = format!(
            "A {} proposal was prepared and shown to the user, who must review and apply it; nothing was changed. {facts}",
            p.title().to_lowercase()
        );
        ToolOutcome {
            text,
            summary: format!("{} ready to review", p.title()),
            proposal: Some(p),
            ..Default::default()
        }
    }

    fn propose(&mut self, name: &str, input: &Value) -> ToolOutcome {
        macro_rules! parse {
            ($t:ty) => {
                match serde_json::from_value::<$t>(input.clone()) {
                    Ok(a) => a,
                    Err(e) => return ToolOutcome::err(format!("invalid arguments: {e}")),
                }
            };
        }
        let a = self.assistant;
        match name {
            "propose_regex" => {
                let args = parse!(RegexArgs);
                match a.propose_regex(&args.example_lines, args.intent.as_deref()) {
                    Ok(p) => Self::proposal(
                        Proposal::Regex(p.clone()),
                        format!("Pattern `{}` matches {} of {} examples.", p.pattern, p.match_count, p.total),
                    ),
                    Err(e) => ToolOutcome::err(e.to_string()),
                }
            }
            "propose_query" => {
                let args = parse!(QuestionArgs);
                match a.nl_to_query(self.store, &args.question) {
                    Ok(p) => Self::proposal(
                        Proposal::Query(p.clone()),
                        format!(
                            "Query `{}` returns {} results now.",
                            p.ql,
                            p.estimated_matches.map_or("an unknown number of".into(), |n| n.to_string())
                        ),
                    ),
                    Err(e) => ToolOutcome::err(e.to_string()),
                }
            }
            "propose_parser" => {
                let args = parse!(SampleArgs);
                let (sample, file) = match self.sample(args.record_id) {
                    Ok(s) => s,
                    Err(e) => return *e,
                };
                match a.propose_parser(self.store, &sample, &file) {
                    Ok(p) => Self::proposal(
                        Proposal::Parser(p.clone()),
                        format!("It parses {} entries of the sample.", p.parsed),
                    ),
                    Err(e) => ToolOutcome::err(e.to_string()),
                }
            }
            "propose_pipeline" | "propose_markers" | "propose_extractors" => {
                let args = parse!(SampleArgs);
                let (sample, _) = match self.sample(args.record_id) {
                    Ok(s) => s,
                    Err(e) => return *e,
                };
                match name {
                    "propose_pipeline" => match a.propose_pipeline(self.store, &args.goal, &sample, None) {
                        Ok(p) => Self::proposal(
                            Proposal::Pipeline(p.clone()),
                            format!(
                                "On the sample it keeps {}, drops {} and changes {} events.",
                                p.kept, p.dropped, p.changed
                            ),
                        ),
                        Err(e) => ToolOutcome::err(e.to_string()),
                    },
                    "propose_markers" => match a.propose_markers(self.store, &args.goal, &sample, None) {
                        Ok(p) => Self::proposal(
                            Proposal::Markers(p.clone()),
                            format!("It marks {} kinds of events in the sample.", p.markers.len()),
                        ),
                        Err(e) => ToolOutcome::err(e.to_string()),
                    },
                    _ => match a.propose_extractors(self.store, &args.goal, &sample, None) {
                        Ok(p) => Self::proposal(
                            Proposal::Extractors(p.clone()),
                            format!(
                                "It extracts values from {} sample events.",
                                p.extractors.iter().map(|e| e.matches).sum::<usize>()
                            ),
                        ),
                        Err(e) => ToolOutcome::err(e.to_string()),
                    },
                }
            }
            other => ToolOutcome::err(format!("unknown tool '{other}'")),
        }
    }
}

impl<P: LlmProvider> ToolHost for ChatHost<'_, P> {
    fn specs(&self) -> Vec<ToolSpec> {
        let mut specs: Vec<ToolSpec> = EngineTools::tool_specs()
            .into_iter()
            .filter(|s| !(self.compact && COMPACT_DROPS.contains(&s.name.as_str())))
            .collect();
        specs.extend(propose_specs());
        for e in &self.extra {
            specs.extend(e.specs());
        }
        specs
    }

    fn call(&mut self, name: &str, input: &Value) -> ToolOutcome {
        if name.starts_with("propose_") {
            return self.propose(name, input);
        }
        if let Some(e) = self.extra.iter_mut().find(|e| e.specs().iter().any(|s| s.name == name)) {
            return e.call(name, input);
        }
        self.engine.call(name, input)
    }
}
