//! Masking of secrets and personal data while text is copied into a bundle.
//!
//! Same idea as the pipeline's `mask` step (regex -> replacement), applied to
//! raw text. Every rule counts its matches so the manifest can say what was
//! removed without revealing it.

use crate::plan::RedactConfig;
use anyhow::{Context, Result};
use regex::{Regex, RegexBuilder};
use std::collections::BTreeMap;

/// Redaction counts per rule name.
pub type Counts = BTreeMap<String, usize>;

enum Kind {
    /// Plain regex replacement (`$1` expansion).
    Replace(String),
    /// Credit-card-like digit runs, replaced only when the Luhn check passes.
    Luhn,
}

struct Rule {
    name: String,
    regex: Regex,
    kind: Kind,
}

pub struct Redactor {
    rules: Vec<Rule>,
}

const MASK: &str = "[REDACTED]";

fn builtin(name: &str, pattern: &str, replace: &str) -> Rule {
    Rule {
        name: name.into(),
        // The patterns are constants covered by tests.
        regex: RegexBuilder::new(pattern)
            .build()
            .unwrap_or_else(|e| unreachable!("built-in pattern {name}: {e}")),
        kind: Kind::Replace(replace.into()),
    }
}

impl Redactor {
    pub fn new(cfg: &RedactConfig) -> Result<Redactor> {
        let mut rules = Vec::new();
        if cfg.built_in {
            rules.push(builtin(
                "bearer-token",
                r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{8,}",
                "Bearer [REDACTED]",
            ));
            rules.push(builtin("jwt", r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*", MASK));
            rules.push(builtin(
                "api-key",
                r#"(?i)((?:api[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret|secret|token)["']?\s*[=:]\s*["']?)[A-Za-z0-9._~+/=-]{8,}"#,
                "${1}[REDACTED]",
            ));
            rules.push(builtin(
                "connection-string",
                r#"(?i)((?:AccountKey|SharedAccessKey|SharedAccessSignature|User\s*Password)\s*=\s*)[^;\s"']+"#,
                "${1}[REDACTED]",
            ));
            rules.push(builtin(
                "password",
                r#"(?i)((?:password|passwd|pwd)["']?\s*[=:]\s*["']?)[^\s;,&"']+"#,
                "${1}[REDACTED]",
            ));
            rules.push(builtin(
                "email",
                r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}\b",
                "[email]",
            ));
            rules.push(Rule {
                name: "credit-card".into(),
                regex: Regex::new(r"\b\d(?:[ -]?\d){12,18}\b").context("credit-card pattern")?,
                kind: Kind::Luhn,
            });
            if cfg.ip_addresses {
                rules.push(builtin(
                    "ipv4",
                    r"\b(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\b",
                    "[ipv4]",
                ));
                rules.push(builtin(
                    "ipv6",
                    r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b|\b(?:[0-9a-f]{1,4}:){1,6}(?::[0-9a-f]{1,4}){1,6}\b|\B::1\b",
                    "[ipv6]",
                ));
            }
        }
        for p in &cfg.patterns {
            let regex = imt_core::user_regex::compile(&p.regex, false).with_context(|| format!("redaction pattern '{}'", p.name))?;
            rules.push(Rule {
                name: p.name.clone(),
                regex,
                kind: Kind::Replace(p.replace.clone()),
            });
        }
        Ok(Redactor { rules })
    }

    /// True when there is nothing to apply (files can be copied untouched).
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Redacts the string values of a JSON document, keeping it valid JSON
    /// (numbers are never touched).
    pub fn apply_json(&self, v: &serde_json::Value, counts: &mut Counts) -> serde_json::Value {
        use serde_json::Value;
        match v {
            Value::String(s) => Value::String(self.apply(s, counts)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.apply_json(x, counts)).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), self.apply_json(x, counts))).collect()),
            other => other.clone(),
        }
    }

    /// Applies every rule in order; returns the new text and adds to `counts`.
    pub fn apply(&self, text: &str, counts: &mut Counts) -> String {
        let mut out = text.to_string();
        for rule in &self.rules {
            let mut n = 0usize;
            out = rule
                .regex
                .replace_all(&out, |caps: &regex::Captures| match &rule.kind {
                    Kind::Replace(r) => {
                        n += 1;
                        let mut s = String::new();
                        caps.expand(r, &mut s);
                        s
                    }
                    Kind::Luhn => {
                        let whole = caps[0].to_string();
                        if luhn_ok(&whole) {
                            n += 1;
                            MASK.to_string()
                        } else {
                            whole
                        }
                    }
                })
                .into_owned();
            if n > 0 {
                *counts.entry(rule.name.clone()).or_insert(0) += n;
            }
        }
        out
    }
}

/// Luhn checksum over the digits of `s`; 13 to 19 digits.
pub fn luhn_ok(s: &str) -> bool {
    let digits: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| {
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 {
                    x - 9
                } else {
                    x
                }
            } else {
                *d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}
