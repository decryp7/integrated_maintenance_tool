//! propose_markers and propose_extractors: `[[marker]]` and `[[extract]]` entries for a goal. The engine
//! compiles the candidate profile and counts how many sample events each entry matches.

use super::pipeline::{base_profile, preview, regex_hint, related_messages, sample_lines, step_schema};
use super::proposals::{ExtractorReport, ExtractorsProposal, MarkerReport, MarkersProposal};
use super::regex::valid_group_name;
use super::Assistant;
use crate::access::StoreRef;
use crate::error::{AiError, Result};
use crate::llm::LlmProvider;
use imt_core::profile::{ExtractCfg, MarkerCfg, MARKER_ICONS};
use imt_core::user_regex;
use serde::Deserialize;
use serde_json::{json, Map, Value};

const MARKER_SYSTEM: &str = "You design timeline markers for a log analysis tool. A marker highlights events that match its conditions. \
Conditions (when) must all match: {\"field\": \"@message\" or a field name, \"op\": eq|ne|contains|notContains|matches|exists|missing|gt|lt|in, \"value\": text}. \
Pseudo-fields: @level @message @exception. To mark by level use field @level, op eq and value Error (or Warning, Fatal). Answer with JSON only.";

const EXTRACT_SYSTEM: &str = "You design field extractors for a log analysis tool. An extractor applies a Rust regex with named groups to a field (default @message) and turns each group into a typed field. \
Field types: string, number, integer, bool, bytes, duration. Answer with JSON only.";

const SHAPES: &[&str] = &["circle", "square", "diamond", "hexagon"];

fn marker_schema() -> Value {
    let cond = step_schema()["properties"]["steps"]["items"]["properties"]["when"]["items"].clone();
    let text = json!({"type": "string"});
    json!({"type": "object", "properties": {
        "markers": {"type": "array", "items": {"type": "object", "properties": {
            "kind": text, "label": text, "color": text, "icon": {"type": "string", "enum": MARKER_ICONS}, "badge": text,
            "shape": {"type": "string", "enum": SHAPES}, "description": text, "when": {"type": "array", "items": cond}
        }, "required": ["kind", "label", "when"]}},
        "rationale": text}, "required": ["markers", "rationale"]})
}

fn extract_schema() -> Value {
    let cond = step_schema()["properties"]["steps"]["items"]["properties"]["when"]["items"].clone();
    let text = json!({"type": "string"});
    json!({"type": "object", "properties": {
        "extractors": {"type": "array", "items": {"type": "object", "properties": {
            "name": text, "from": text, "regex": text, "when": {"type": "array", "items": cond},
            "fields": {"type": "object", "additionalProperties": {"type": "object", "properties": {
                "type": {"type": "string", "enum": ["string", "number", "integer", "bool", "bytes", "duration", "auto"]},
                "unit": text, "unit_from": text, "to": text}, "required": ["type"]}}
        }, "required": ["name", "regex"]}},
        "rationale": text}, "required": ["extractors", "rationale"]})
}

#[derive(Deserialize)]
struct MarkerAnswer {
    #[serde(default)]
    markers: Vec<Value>,
    #[serde(default)]
    rationale: String,
}

#[derive(Deserialize)]
struct ExtractAnswer {
    #[serde(default)]
    extractors: Vec<Value>,
    #[serde(default)]
    rationale: String,
}

fn valid_color(c: &str) -> bool {
    c.strip_prefix('#')
        .is_some_and(|h| matches!(h.len(), 3 | 6) && h.chars().all(|c| c.is_ascii_hexdigit()))
}

fn kind_ok(k: &str) -> bool {
    !k.is_empty() && k.len() <= 40 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl<P: LlmProvider> Assistant<P> {
    pub fn propose_markers<'s>(
        &self,
        store: impl Into<StoreRef<'s>>,
        goal: &str,
        sample: &str,
        base_toml: Option<&str>,
    ) -> Result<MarkersProposal> {
        if goal.trim().is_empty() {
            return Err(AiError::Invalid("describe what should be marked".into()));
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
        let user = format!(
            "Goal: {}\n\nSample events:\n{}\nMake the condition as specific as possible: copy a distinctive phrase from the kind of event you want and use `contains` on @message; never use a single generic word that other kinds of events also contain. Icons you may use: {}. Badge is 1-3 characters. Color like #E9AE3F.\n\
             Example: {{\"markers\":[{{\"kind\":\"gateway-timeout\",\"label\":\"Gateway timeout\",\"color\":\"#3987e5\",\"icon\":\"timer\",\"description\":\"The payment gateway timed out\",\"when\":[{{\"field\":\"@message\",\"op\":\"matches\",\"value\":\"(?i)gateway.*time(d)? ?out\"}}]}}],\"rationale\":\"...\"}}\n\
             Return JSON {{\"markers\": [...], \"rationale\": \"...\"}}.",
            goal.trim(),
            sample_lines(&base_preview, self.prompt_chars() / 3),
            MARKER_ICONS.join(", ")
        );
        let goal_text = goal.to_string();
        let existing: Vec<String> = base.marker.iter().map(|m| m.kind.clone()).collect();
        let (p, _) = self.generate_validated::<MarkerAnswer, MarkersProposal>(MARKER_SYSTEM, &user, marker_schema(), |a| {
            if a.markers.is_empty() || a.markers.len() > 6 {
                return Err("give between 1 and 6 markers".into());
            }
            let mut ms: Vec<MarkerCfg> = Vec::new();
            for (i, v) in a.markers.iter().enumerate() {
                let m: MarkerCfg = serde_json::from_value(v.clone()).map_err(|e| format!("marker {}: {e}", i + 1))?;
                if !kind_ok(&m.kind) {
                    return Err(format!("marker {}: kind '{}' must be letters, digits, - or _", i + 1, m.kind));
                }
                if existing.contains(&m.kind) || ms.iter().any(|x| x.kind == m.kind) {
                    return Err(format!("marker {}: kind '{}' already exists; choose another", i + 1, m.kind));
                }
                if m.when.is_empty() {
                    return Err(format!("marker '{}' needs at least one condition in when", m.kind));
                }
                if m.color.as_deref().is_some_and(|c| !valid_color(c)) {
                    return Err(format!("marker '{}': color must look like #RGB or #RRGGBB", m.kind));
                }
                if m.badge.as_deref().is_some_and(|b| b.is_empty() || b.chars().count() > 3) {
                    return Err(format!("marker '{}': badge must be 1 to 3 characters", m.kind));
                }
                if m.icon.as_deref().is_some_and(|i| !MARKER_ICONS.contains(&i)) {
                    return Err(format!("marker '{}': unknown icon", m.kind));
                }
                if m.shape.as_deref().is_some_and(|s| !SHAPES.contains(&s)) {
                    return Err(format!("marker '{}': shape must be one of {}", m.kind, SHAPES.join(", ")));
                }
                ms.push(m);
            }
            let mut cand = base.clone();
            let mut marker = ms.clone();
            marker.append(&mut cand.marker);
            cand.marker = marker;
            let (profile_toml, pv) = preview(store, &cand, sample, "sample.log")?;
            let sampled = pv.records.len();
            let mut reports = Vec::new();
            for m in &ms {
                let n = pv.records.iter().filter(|r| r.markers.contains(&m.kind)).count();
                if n == 0 {
                    return Err(format!(
                        "marker '{}' matches none of the {sampled} sample events; use wording that really occurs in the sample.{}",
                        m.kind,
                        related_messages(&goal_text, &base_preview, 3)
                    ));
                }
                if n == sampled && sampled > 3 {
                    return Err(format!(
                        "marker '{}' matches every sample event; make the conditions more specific",
                        m.kind
                    ));
                }
                reports.push(MarkerReport {
                    kind: m.kind.clone(),
                    label: m.label.clone().unwrap_or_else(|| m.kind.clone()),
                    matches: n,
                });
            }
            #[derive(serde::Serialize)]
            struct Wrap<'a> {
                marker: &'a [MarkerCfg],
            }
            let toml = toml::to_string(&Wrap { marker: &ms }).map_err(|e| format!("cannot write the markers: {e}"))?;
            Ok(MarkersProposal {
                toml,
                profile_toml,
                markers: reports,
                sampled,
                rationale: a.rationale.trim().to_string(),
            })
        })?;
        Ok(p)
    }

    pub fn propose_extractors<'s>(
        &self,
        store: impl Into<StoreRef<'s>>,
        goal: &str,
        sample: &str,
        base_toml: Option<&str>,
    ) -> Result<ExtractorsProposal> {
        if goal.trim().is_empty() {
            return Err(AiError::Invalid("describe which values to extract".into()));
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
        let user = format!(
            "Goal: {}\n\nSample events:\n{}\n\
             {}Leave `fields` empty ({{}}): every named group becomes a typed field by itself. Example: {{\"extractors\":[{{\"name\":\"gateway-latency\",\"from\":\"@message\",\"regex\":\"took (?P<elapsed_ms>[\\\\d.]+) ms\",\"when\":[],\"fields\":{{}}}}],\"rationale\":\"...\"}}\n\
             Return JSON {{\"extractors\": [...], \"rationale\": \"...\"}}.",
            goal.trim(),
            sample_lines(&base_preview, self.prompt_chars() / 3),
            regex_hint(goal, &base_preview)
        );
        let goal_text = goal.to_string();
        let existing: Vec<String> = base.extract.iter().map(|e| e.name.clone()).collect();
        let (p, _) = self.generate_validated::<ExtractAnswer, ExtractorsProposal>(EXTRACT_SYSTEM, &user, extract_schema(), |a| {
            if a.extractors.is_empty() || a.extractors.len() > 4 {
                return Err("give between 1 and 4 extractors".into());
            }
            let mut es: Vec<ExtractCfg> = Vec::new();
            for (i, v) in a.extractors.iter().enumerate() {
                let e: ExtractCfg = serde_json::from_value(v.clone()).map_err(|e| format!("extractor {}: {e}", i + 1))?;
                if !kind_ok(&e.name) || existing.contains(&e.name) || es.iter().any(|x| x.name == e.name) {
                    return Err(format!("extractor {}: the name '{}' is invalid or already used", i + 1, e.name));
                }
                let re = user_regex::compile(&e.regex, false)
                    .map_err(|er| format!("extractor '{}': the regex does not compile: {er}", e.name))?;
                let groups: Vec<&str> = re.capture_names().flatten().collect();
                if groups.is_empty() || groups.iter().any(|g| !valid_group_name(g)) {
                    return Err(format!(
                        "extractor '{}': the regex needs named groups (?P<name>...) with simple names",
                        e.name
                    ));
                }
                es.push(e);
            }
            let mut cand = base.clone();
            cand.extract.extend(es.iter().cloned());
            let (profile_toml, pv) = preview(store, &cand, sample, "sample.log")?;
            let mut reports = Vec::new();
            for e in &es {
                let re = user_regex::compile(&e.regex, false).map_err(|er| er.to_string())?;
                let fields: Vec<String> = if e.fields.is_empty() {
                    re.capture_names().flatten().map(str::to_string).collect()
                } else {
                    e.fields.keys().cloned().collect()
                };
                let hits: Vec<&imt_core::api::ParsedPreview> = pv
                    .records
                    .iter()
                    .filter(|r| fields.iter().any(|f| r.fields.contains_key(f)))
                    .collect();
                if hits.is_empty() {
                    return Err(format!(
                        "extractor '{}' extracted nothing from the {} sample events; the regex must match the message text exactly.{}",
                        e.name,
                        pv.records.len(),
                        related_messages(&goal_text, &base_preview, 3)
                    ));
                }
                let examples: Vec<Map<String, Value>> = hits
                    .iter()
                    .take(3)
                    .map(|r| {
                        fields
                            .iter()
                            .filter_map(|f| r.fields.get(f).map(|v| (f.clone(), v.clone())))
                            .collect()
                    })
                    .collect();
                reports.push(ExtractorReport {
                    name: e.name.clone(),
                    fields,
                    matches: hits.len(),
                    examples,
                });
            }
            #[derive(serde::Serialize)]
            struct Wrap<'a> {
                extract: &'a [ExtractCfg],
            }
            let toml = toml::to_string(&Wrap { extract: &es }).map_err(|e| format!("cannot write the extractors: {e}"))?;
            Ok(ExtractorsProposal {
                toml,
                profile_toml,
                extractors: reports,
                sampled: pv.records.len(),
                rationale: a.rationale.trim().to_string(),
            })
        })?;
        Ok(p)
    }
}
