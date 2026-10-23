//! propose_regex: the engine infers a regex from the examples; the model renames groups and may
//! generalise it; the engine compiles it, counts matches on the examples and explains it.

use super::proposals::{RegexProposal, RegexSource};
use super::tools::truncate_chars;
use super::Assistant;
use crate::error::{AiError, Result};
use crate::llm::LlmProvider;
use debrief_core::regex_assist::{self, InferredGroup};
use debrief_core::user_regex;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;

const SYSTEM: &str = "You improve regular expressions for parsing log lines (Rust regex syntax). \
Use named groups (?P<name>...). Group names: letters, digits and underscore only, descriptive (duration_ms, user_id). \
The regex MUST still match every example line. Answer with JSON only.";

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    pattern: String,
    #[serde(default)]
    renames: BTreeMap<String, String>,
    #[serde(default)]
    rationale: String,
}

pub(crate) fn valid_group_name(n: &str) -> bool {
    let mut c = n.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && n.len() <= 40
        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Renames `(?P<old>` to `(?P<new>`; the new names must be valid and not collide.
fn apply_renames(pattern: &str, renames: &BTreeMap<String, String>) -> std::result::Result<String, String> {
    let mut p = pattern.to_string();
    for (old, new) in renames {
        if old == new {
            continue;
        }
        if !valid_group_name(new) {
            return Err(format!("'{new}' is not a valid group name (letters, digits and underscore only)"));
        }
        let from = format!("(?P<{old}>");
        if !p.contains(&from) {
            // A rename of a group that does not exist is ignored: small models mix up the names.
            continue;
        }
        if p.contains(&format!("(?P<{new}>")) {
            return Err(format!("the group name '{new}' is already used"));
        }
        p = p.replacen(&from, &format!("(?P<{new}>"), 1);
    }
    Ok(p)
}

impl<P: LlmProvider> Assistant<P> {
    pub fn propose_regex(&self, example_lines: &[String], intent: Option<&str>) -> Result<RegexProposal> {
        let budget = self.prompt_chars();
        let mut used = 0;
        let lines: Vec<String> = example_lines
            .iter()
            .map(|l| l.trim_end_matches(['\r', '\n']))
            .filter(|l| !l.trim().is_empty())
            .map(|l| truncate_chars(l, 400))
            .take(30)
            .take_while(|l| {
                used += l.len();
                used < budget / 2
            })
            .collect();
        if lines.is_empty() {
            return Err(AiError::Invalid("give at least one example line".into()));
        }
        let inferred = regex_assist::infer(&lines).map_err(|e| AiError::Invalid(format!("{e:#}")))?;
        let inferred_count = inferred.matches.iter().filter(|m| **m).count();
        let groups: Vec<String> = inferred
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
        let first = inferred.groups.first().map(|g| g.name.clone()).unwrap_or_else(|| "g1".into());
        let user = format!(
            "Example lines:\n{}\n\nThe engine already built this regex; it matches {inferred_count} of {} lines:\n{}\nIts capture groups:\n{}\n{}\
             Give the groups meaningful names. Return JSON: {{\"pattern\": the regex (keep it as it is unless you can improve it), \"renames\": {{\"<current group name>\": \"<new name>\"}}, \"rationale\": one sentence}}.\n\
             Example: {{\"pattern\": \"{}\", \"renames\": {{\"{first}\": \"duration_ms\"}}, \"rationale\": \"Named the first group after what it holds.\"}}",
            lines.iter().enumerate().map(|(i, l)| format!("{}: {l}", i + 1)).collect::<Vec<_>>().join("\n"),
            lines.len(),
            inferred.pattern,
            groups.join("\n"),
            intent.map(|i| format!("The user wants: {i}\n")).unwrap_or_default(),
            inferred.pattern.replace('\\', "\\\\").replace('"', "\\\""),
        );
        let schema = json!({
            "type": "object",
            "properties": {"pattern": {"type": "string"}, "renames": {"type": "object", "additionalProperties": {"type": "string"}}, "rationale": {"type": "string"}},
            "required": ["pattern", "renames", "rationale"]
        });
        let base_pattern = inferred.pattern.clone();
        let (proposal, _) = self.generate_validated::<Answer, RegexProposal>(SYSTEM, &user, schema, |a| {
            let pattern = if a.pattern.trim().is_empty() {
                base_pattern.clone()
            } else {
                a.pattern.trim().to_string()
            };
            let pattern = apply_renames(&pattern, &a.renames)?;
            let re = user_regex::compile(&pattern, false).map_err(|e| format!("the regex does not compile: {e}"))?;
            let names: Vec<&str> = re.capture_names().flatten().collect();
            if names.is_empty() {
                return Err("the regex has no named groups; use (?P<name>...)".to_string());
            }
            if let Some(bad) = names.iter().find(|n| !valid_group_name(n)) {
                return Err(format!("'{bad}' is not a valid group name"));
            }
            let matches: Vec<bool> = lines.iter().map(|l| re.is_match(l)).collect();
            let count = matches.iter().filter(|m| **m).count();
            if count < inferred_count {
                return Err(format!(
                    "it matches only {count} of {} example lines, the starting regex matched {inferred_count}",
                    lines.len()
                ));
            }
            let groups = names
                .iter()
                .map(|n| {
                    let examples: Vec<String> = lines
                        .iter()
                        .filter_map(|l| re.captures(l))
                        .filter_map(|c| c.name(n).map(|m| m.as_str().to_string()))
                        .take(3)
                        .collect();
                    let kind = inferred
                        .groups
                        .iter()
                        .find(|g| g.name == *n)
                        .map(|g| g.kind.clone())
                        .unwrap_or_else(|| "text".into());
                    InferredGroup {
                        name: n.to_string(),
                        kind,
                        examples,
                    }
                })
                .collect();
            Ok(RegexProposal {
                source: if pattern == base_pattern {
                    RegexSource::Inferred
                } else {
                    RegexSource::Model
                },
                explanation: regex_assist::explain(&pattern).unwrap_or_default(),
                pattern,
                groups,
                match_count: count,
                total: lines.len(),
                matches,
                rationale: a.rationale.trim().to_string(),
            })
        })?;
        Ok(proposal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_names() {
        assert!(valid_group_name("duration_ms") && valid_group_name("_x1"));
        assert!(!valid_group_name("1x") && !valid_group_name("a-b") && !valid_group_name("") && !valid_group_name(&"a".repeat(41)));
    }

    #[test]
    fn renames_are_validated() {
        let r = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(apply_renames("(?P<a>\\d+)", &r(&[("a", "b")])).unwrap(), "(?P<b>\\d+)");
        assert_eq!(apply_renames("(?P<a>\\d+)", &r(&[("a", "a")])).unwrap(), "(?P<a>\\d+)");
        assert_eq!(apply_renames("(?P<a>x)", &r(&[("zz", "b")])).unwrap(), "(?P<a>x)");
        assert!(apply_renames("(?P<a>x)(?P<b>y)", &r(&[("a", "b")]))
            .unwrap_err()
            .contains("already used"));
        assert!(apply_renames("(?P<a>x)", &r(&[("a", "1bad")]))
            .unwrap_err()
            .contains("valid group name"));
    }
}
