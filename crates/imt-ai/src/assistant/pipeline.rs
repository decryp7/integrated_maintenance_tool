//! propose_pipeline: `[[pipeline]]` steps for a goal such as "drop health-check noise and mask tokens".
//! The engine builds the candidate profile, compiles it, and dry-runs it on the sample step by step.

use super::proposals::{ChangeExample, PipelineProposal, StepReport};
use super::tools::truncate_chars;
use super::Assistant;
use crate::access::StoreRef;
use crate::error::{AiError, Result};
use crate::llm::LlmProvider;
use imt_core::api::{ParsePreview, ParsePreviewRequest};
use imt_core::pipeline::StepCfg;
use imt_core::Profile;
use serde::Deserialize;
use serde_json::{json, Value};

/// Steps the model may propose (`plugin` and file-based `lookup` need things the model cannot know).
pub(crate) const STEP_KINDS: &[&str] = &[
    "drop",
    "keep",
    "mask",
    "dedupe",
    "remove_fields",
    "rename",
    "set",
    "set_level",
    "truncate",
    "parse_json",
    "parse_kv",
    "shift_time",
    "calc",
];
const MAX_STEPS: usize = 8;

const SYSTEM: &str = "You design log-cleaning pipeline steps for an analysis tool. Use only the step kinds listed. \
Conditions (when) are all required to match: {\"field\": \"@message\" or a field name, \"op\": eq|ne|contains|notContains|matches|exists|missing|gt|lt|in, \"value\": text}. \
Pseudo-fields: @level @message @exception @source. Regexes use Rust syntax. Answer with JSON only.";

const CATALOGUE: &str = "Step kinds: drop (when) removes events; keep (when) keeps only matching events; mask (field, regex, replace like \"$1=***\"); \
dedupe (fields, window like \"5s\"); remove_fields (fields); rename (from, to); set (field, value); set_level (level, when); truncate (field, max); \
parse_json / parse_kv (field, prefix); shift_time (by like \"+2.4s\"); calc (field, expr) computes a field from an expression, e.g. {\"field\":\"ElapsedSec\",\"expr\":\"round(ElapsedMs / 1000, 1)\"} (operators + - * / % comparisons and/or/not; functions round floor ceil abs min max len lower upper coalesce if substr to_number).";

const EXAMPLES: &str = r#"Examples:
Goal "ignore health checks": {"steps":[{"step":"drop","when":[{"field":"@message","op":"contains","value":"/health"}]}],"rationale":"Health probes are noise."}
Goal "hide passwords": {"steps":[{"step":"mask","field":"@message","regex":"(password=)\\S+","replace":"${1}***"}],"rationale":"Masks the value after password=."}"#;

pub(crate) fn step_schema() -> Value {
    let cond = json!({"type":"object","properties":{"field":{"type":"string"},"op":{"type":"string","enum":["eq","ne","contains","notContains","matches","exists","missing","gt","lt","in"]},"value":{"type":"string"}},"required":["field","op"]});
    let text = json!({"type":"string"});
    json!({
        "type": "object",
        "properties": {
            "steps": {"type": "array", "items": {"type": "object", "properties": {
                "step": {"type": "string", "enum": STEP_KINDS},
                "when": {"type": "array", "items": cond},
                "field": text, "fields": {"type": "array", "items": text}, "regex": text, "replace": text, "window": text,
                "from": text, "to": text, "value": text, "level": text, "max": {"type": "integer"}, "prefix": text, "by": text, "expr": text
            }, "required": ["step"]}},
            "rationale": {"type": "string"}
        },
        "required": ["steps", "rationale"]
    })
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    steps: Vec<Value>,
    #[serde(default)]
    rationale: String,
}

/// The profile the proposal builds on: the given TOML, or the store's active profile.
pub(crate) fn base_profile(store: StoreRef<'_>, base_toml: Option<&str>) -> Result<Profile> {
    match base_toml {
        Some(t) => Profile::from_toml(t).map_err(|e| AiError::Invalid(format!("the current profile is not valid: {e}"))),
        None => store
            .with(|s| s.active_profile())
            .map_err(|e| AiError::Invalid(format!("the current profile cannot be read: {e}"))),
    }
}

pub(crate) fn preview(
    store: StoreRef<'_>,
    profile: &Profile,
    sample: &str,
    name: &str,
) -> std::result::Result<(String, ParsePreview), String> {
    let toml = toml::to_string(profile).map_err(|e| format!("the profile cannot be written: {e}"))?;
    let req = ParsePreviewRequest {
        toml: toml.clone(),
        sample: sample.to_string(),
        file_name: name.to_string(),
        limit: 1000,
    };
    // Borrowed for this validation step only.
    let p = store
        .with(|s| s.preview_parse(&req))
        .map_err(|e| format!("the engine cannot preview this profile: {e}"))?;
    match p.error {
        Some(e) => Err(e),
        None => Ok((toml, p)),
    }
}

/// What the sample contains, as kinds of events with counts: the model sees every kind (also the rare ones)
/// instead of a handful of identical lines, so it can pick distinctive wording.
pub(crate) fn sample_lines(p: &ParsePreview, max_chars: usize) -> String {
    let mut kinds: Vec<(String, usize, String, String)> = Vec::new();
    for r in &p.records {
        let key: String = normalise_message(&r.message);
        match kinds.iter_mut().find(|k| k.0 == key) {
            Some(k) => k.1 += 1,
            None => kinds.push((key, 1, format!("{:?}", r.level), r.message.clone())),
        }
    }
    kinds.sort_by_key(|k| std::cmp::Reverse(k.1));
    let mut out = format!(
        "The sample has {} events of these kinds (count x [level] example message):\n",
        p.records.len()
    );
    for (_, n, level, example) in kinds.iter().take(16) {
        let line = format!("- {n} x [{level}] {}\n", truncate_chars(&example.replace('\n', " "), 140));
        if out.len() + line.len() > max_chars {
            break;
        }
        out.push_str(&line);
    }
    let mut fields: Vec<&str> = Vec::new();
    for r in &p.records {
        for k in r.fields.keys() {
            if !fields.contains(&k.as_str()) && fields.len() < 20 {
                fields.push(k);
            }
        }
    }
    out.push_str(&format!("Fields on the events: {}\n", fields.join(", ")));
    out
}

/// Messages of the sample that share words with `text` (the goal): shown to the model when its condition or
/// regex matched nothing, so it can see the wording it should have used.
pub(crate) fn related_messages(text: &str, p: &ParsePreview, n: usize) -> String {
    let words: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 4)
        .map(str::to_string)
        .collect();
    let mut seen: Vec<String> = Vec::new();
    let mut scored: Vec<(usize, String)> = Vec::new();
    for r in &p.records {
        let key = normalise_message(&r.message);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let low = r.message.to_lowercase();
        let score = words.iter().filter(|w| low.contains(w.as_str())).count();
        if score > 0 {
            scored.push((score, truncate_chars(&r.message.replace('\n', " "), 160)));
        }
    }
    scored.sort_by_key(|s| std::cmp::Reverse(s.0));
    let top: Vec<String> = scored.into_iter().take(n).map(|(_, m)| format!("`{m}`")).collect();
    if top.is_empty() {
        String::new()
    } else {
        format!(" Messages in the sample that look related: {}.", top.join(", "))
    }
}

/// The engine's own regex for the kind of event closest to the goal, so the model names and adjusts groups
/// instead of writing a regex from scratch.
pub(crate) fn regex_hint(goal: &str, p: &ParsePreview) -> String {
    let words: Vec<String> = goal
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 4)
        .map(str::to_string)
        .collect();
    let mut best: Option<(usize, String)> = None;
    for r in &p.records {
        let low = r.message.to_lowercase();
        let score = words.iter().filter(|w| low.contains(w.as_str())).count();
        if score > 0 && best.as_ref().is_none_or(|b| score > b.0) {
            best = Some((score, normalise_message(&r.message)));
        }
    }
    let Some((_, key)) = best else { return String::new() };
    let messages: Vec<String> = p
        .records
        .iter()
        .filter(|r| normalise_message(&r.message) == key && !r.message.contains('\n'))
        .map(|r| r.message.clone())
        .take(10)
        .collect();
    match imt_core::regex_assist::infer(&messages) {
        Ok(inf) if messages.len() >= 2 && inf.matches.iter().all(|m| *m) => {
            let groups: Vec<String> = inf
                .groups
                .iter()
                .map(|g| {
                    format!(
                        "- {} = a {}, for example {}",
                        g.name,
                        g.kind,
                        g.examples.first().cloned().unwrap_or_default()
                    )
                })
                .collect();
            format!("For the kind of event closest to the goal (for example `{}`) the engine built this regex, which matches all {} such events:\n{}\nIts groups:\n{}\nRename the groups meaningfully and drop the ones you do not need; keep the rest unchanged.\n", truncate_chars(&messages[0], 140), messages.len(), inf.pattern, groups.join("\n"))
        }
        _ => String::new(),
    }
}

/// A message with its numbers and identifiers blanked, so events of one kind share a key.
fn normalise_message(m: &str) -> String {
    let mut out = String::new();
    let mut prev_digit = false;
    for ch in m.chars().take(70) {
        if ch.is_ascii_digit() {
            if !prev_digit {
                out.push('#');
            }
            prev_digit = true;
        } else {
            out.push(ch);
            prev_digit = false;
        }
    }
    out
}

/// Step kinds the goal's wording demands (each group: any one of them will do).
pub(crate) fn required_steps(goal: &str) -> Vec<(Vec<&'static str>, &'static str)> {
    let g = goal.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| g.contains(w));
    let mut need: Vec<(Vec<&'static str>, &'static str)> = Vec::new();
    if has(&[
        "drop ",
        "remove the",
        "remove all",
        "ignore ",
        "discard",
        "exclude",
        "filter out",
        "delete the",
        "throw away",
    ]) && !has(&["field", "column"])
    {
        need.push((
            vec!["drop", "keep"],
            "drop events (use a `drop` step with a `when` condition, or a `keep` step)",
        ));
    }
    if has(&["mask", "redact", "anonymi", "obfuscate", "censor"]) {
        need.push((vec!["mask"], "mask values (use a `mask` step with a `regex` and a `replace`)"));
    }
    if has(&["truncate", "shorten", "cut off", "at most ", "limit the length"]) {
        need.push((vec!["truncate"], "shorten messages (use a `truncate` step with `max`)"));
    }
    if has(&["dedupe", "duplicate", "repeated", "collapse", "deduplicate"]) {
        need.push((vec!["dedupe"], "collapse repeats (use a `dedupe` step)"));
    }
    if has(&["rename "]) {
        need.push((vec!["rename"], "rename a field (use a `rename` step with `from` and `to`)"));
    }
    if has(&["remove the field", "remove field", "drop the field", "drop field"]) {
        need.push((vec!["remove_fields"], "remove fields (use a `remove_fields` step)"));
    }
    need
}

fn same(a: &imt_core::api::ParsedPreview, b: &imt_core::api::ParsedPreview) -> bool {
    a.message == b.message && a.level == b.level && a.fields == b.fields && a.exception == b.exception
}

impl<P: LlmProvider> Assistant<P> {
    pub fn propose_pipeline<'s>(
        &self,
        store: impl Into<StoreRef<'s>>,
        goal: &str,
        sample: &str,
        base_toml: Option<&str>,
    ) -> Result<PipelineProposal> {
        if goal.trim().is_empty() {
            return Err(AiError::Invalid("describe what the pipeline should do".into()));
        }
        let store: StoreRef<'_> = store.into();
        let base = base_profile(store, base_toml)?;
        let (_, base_preview) = preview(store, &base, sample, "sample.log").map_err(|e| {
            AiError::Invalid(format!(
                "the current profile cannot read this sample ({e}); set up the profile first"
            ))
        })?;
        if base_preview.total == 0 {
            return Err(AiError::Invalid("the sample has no events to work with".into()));
        }
        let user = format!("Goal: {}\n\nSample events after parsing:\n{}\n{CATALOGUE}\n{EXAMPLES}\nEvery thing the goal asks for needs its own step (to remove events use a drop step with a `when` condition; to change text use a mask step). \
             Return JSON {{\"steps\": [...], \"rationale\": \"...\"}} with at most {MAX_STEPS} steps.", goal.trim(), sample_lines(&base_preview, self.prompt_chars() / 3));
        let total0 = base_preview.total;
        let goal_text = goal.to_string();
        let (p, _) = self.generate_validated::<Answer, PipelineProposal>(SYSTEM, &user, step_schema(), |a| {
            if a.steps.is_empty() || a.steps.len() > MAX_STEPS {
                return Err(format!("give between 1 and {MAX_STEPS} steps"));
            }
            let mut steps: Vec<StepCfg> = Vec::new();
            for (i, v) in a.steps.iter().enumerate() {
                let s: StepCfg = serde_json::from_value(v.clone()).map_err(|e| format!("step {}: {e}", i + 1))?;
                if !STEP_KINDS.contains(&s.step.as_str()) {
                    return Err(format!(
                        "step {}: '{}' is not an allowed step kind (use one of {})",
                        i + 1,
                        s.step,
                        STEP_KINDS.join(", ")
                    ));
                }
                steps.push(s);
            }
            for (kinds, what) in required_steps(&goal_text) {
                if !steps.iter().any(|s| kinds.contains(&s.step.as_str())) {
                    return Err(format!("the goal asks to {what}, but none of your steps does that; add it"));
                }
            }
            let mut reports = Vec::new();
            let mut prev_total = total0;
            for k in 1..=steps.len() {
                let mut cand = base.clone();
                cand.pipeline.extend(steps[..k].iter().cloned());
                let (_, pv) =
                    preview(store, &cand, sample, "sample.log").map_err(|e| format!("step {} ({}) failed: {e}", k, steps[k - 1].step))?;
                reports.push(StepReport {
                    index: k,
                    step: steps[k - 1].step.clone(),
                    input: prev_total,
                    output: pv.total,
                });
                prev_total = pv.total;
            }
            let mut cand = base.clone();
            cand.pipeline.extend(steps.iter().cloned());
            let (profile_toml, fin) = preview(store, &cand, sample, "sample.log")?;
            let before: std::collections::HashMap<u32, &imt_core::api::ParsedPreview> =
                base_preview.records.iter().map(|r| (r.line, r)).collect();
            let mut changed = 0;
            let mut examples = Vec::new();
            for r in &fin.records {
                if let Some(b) = before.get(&r.line) {
                    if !same(b, r) {
                        changed += 1;
                        if examples.len() < 3 {
                            examples.push(ChangeExample {
                                before: truncate_chars(&b.message, 200),
                                after: truncate_chars(&r.message, 200),
                            });
                        }
                    }
                }
            }
            let dropped = total0.saturating_sub(fin.total);
            if dropped == 0 && changed == 0 {
                return Err(
                    "these steps change nothing in the sample; check the conditions and field names against the sample events".into(),
                );
            }
            #[derive(serde::Serialize)]
            struct Wrap<'a> {
                pipeline: &'a [StepCfg],
            }
            let toml = toml::to_string(&Wrap { pipeline: &steps }).map_err(|e| format!("cannot write the steps: {e}"))?;
            Ok(PipelineProposal {
                toml,
                profile_toml,
                steps: reports,
                sample_total: total0,
                kept: fin.total,
                dropped,
                changed,
                examples,
                rationale: a.rationale.trim().to_string(),
            })
        })?;
        Ok(p)
    }
}
