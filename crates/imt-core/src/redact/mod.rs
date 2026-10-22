//! Redaction of secrets and personal data: e-mail and IP addresses, tokens,
//! passwords, card numbers, user names in paths, custom patterns...
//!
//! One [`Redactor`] serves every place text leaves the engine redacted: the
//! redacted view (pages, facets, source context), exports (CSV, CLEF),
//! reports and `.imtz` bundles, and the diagnostics collector. Rules run in
//! order; each counts its matches so a manifest or a preview can say what was
//! removed without revealing it.

mod config;
mod detectors;
#[cfg(test)]
mod tests;
pub mod view;

pub use config::{FieldScope, Mode, RedactCfg, RedactRuleCfg};
pub use detectors::{luhn_ok, Detector};
pub use view::{view_redactor, RedactionPreviewRequest, RedactionSample, RedactionSummary, RedactionSummaryRequest};

use crate::model::Record;
use anyhow::{Context, Result};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::BTreeMap;

/// Matches per rule name.
pub type Counts = BTreeMap<String, usize>;

/// The per-export secret mixed into hashes: equal values hash equally with
/// one salt (correlatable within an export) and differently with another.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Salt([u8; 16]);

impl Salt {
    /// A fresh random salt (OS randomness; falls back to the clock and the
    /// process's hash keys if the OS refuses).
    pub fn random() -> Salt {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).unwrap_or_else(|_| bytes = fallback_entropy());
        Salt(bytes)
    }

    /// A known salt (tests, previews).
    pub fn fixed(bytes: [u8; 16]) -> Salt {
        Salt(bytes)
    }
}

impl std::fmt::Debug for Salt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Salt(..)")
    }
}

fn fallback_entropy() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let a = h.finish();
    h.write_u64(a);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a.to_le_bytes());
    out[8..].copy_from_slice(&h.finish().to_le_bytes());
    out
}

/// What replaces a match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replacement {
    /// Mask, hash or drop the value, labelled `<label>`.
    Mode(Mode),
    /// This text replaces the value.
    Literal(String),
    /// `$1` / `$name` expansion replaces the whole match.
    Template(String),
}

/// One compiled rule.
pub struct Rule {
    name: String,
    label: String,
    regex: Regex,
    /// Capture group whose text is replaced (`value`); else the whole match.
    value_group: Option<usize>,
    detector: Option<Detector>,
    replacement: Replacement,
}

impl Rule {
    /// A built-in detector.
    pub fn detector(d: Detector, replacement: Replacement) -> Rule {
        // The patterns are constants covered by tests.
        let regex = Regex::new(d.pattern()).unwrap_or_else(|e| unreachable!("built-in pattern {}: {e}", d.name()));
        Rule::build(d.name().into(), d.label().into(), regex, Some(d), replacement)
    }

    /// A user pattern, compiled with the limits of [`crate::user_regex`].
    pub fn custom(name: &str, pattern: &str, replacement: Replacement) -> Result<Rule> {
        if name.trim().is_empty() {
            anyhow::bail!("a redaction rule needs a name");
        }
        let regex = crate::user_regex::compile(pattern, false).with_context(|| format!("redaction rule \"{name}\""))?;
        Ok(Rule::build(name.into(), name.into(), regex, None, replacement))
    }

    fn build(name: String, label: String, regex: Regex, detector: Option<Detector>, replacement: Replacement) -> Rule {
        let value_group = regex.capture_names().position(|n| n == Some("value"));
        Rule {
            name,
            label,
            regex,
            value_group,
            detector,
            replacement,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Applies an ordered list of rules to the parts of events in its scope.
pub struct Redactor {
    rules: Vec<Rule>,
    scope: FieldScope,
    salt: Salt,
}

impl Redactor {
    pub fn new(rules: Vec<Rule>, scope: FieldScope, salt: Salt) -> Redactor {
        Redactor { rules, scope, salt }
    }

    /// The redactor a profile's `[redact]` section describes.
    pub fn from_config(cfg: &RedactCfg, salt: Salt) -> Result<Redactor> {
        let mut rules: Vec<Rule> = Detector::ALL
            .iter()
            .filter(|d| cfg.detectors.contains(d))
            .map(|&d| Rule::detector(d, Replacement::Mode(cfg.mode)))
            .collect();
        for r in &cfg.rule {
            let replacement = match &r.replace {
                Some(t) => Replacement::Template(t.clone()),
                None => Replacement::Mode(cfg.mode),
            };
            rules.push(Rule::custom(&r.name, &r.regex, replacement)?);
        }
        Ok(Redactor::new(rules, FieldScope::parse(&cfg.fields), salt))
    }

    /// True when there is nothing to apply.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn scope(&self) -> &FieldScope {
        &self.scope
    }

    /// Rule names, in the order they run.
    pub fn rule_names(&self) -> Vec<&str> {
        self.rules.iter().map(|r| r.name.as_str()).collect()
    }

    /// Applies every rule in order; adds to `counts`.
    pub fn apply<'a>(&self, text: &'a str, counts: &mut Counts) -> Cow<'a, str> {
        let mut out = Cow::Borrowed(text);
        for rule in &self.rules {
            let mut n = 0usize;
            if let Some(changed) = self.apply_rule(rule, &out, &mut n) {
                out = Cow::Owned(changed);
            }
            if n > 0 {
                *counts.entry(rule.name.clone()).or_insert(0) += n;
            }
        }
        out
    }

    fn apply_rule(&self, rule: &Rule, text: &str, n: &mut usize) -> Option<String> {
        let mut out = String::new();
        let mut last = 0;
        for caps in rule.regex.captures_iter(text) {
            let Some(whole) = caps.get(0) else { continue };
            let target = rule.value_group.and_then(|g| caps.get(g)).unwrap_or(whole);
            if !rule.detector.is_none_or(|d| d.accepts(target.as_str())) {
                continue;
            }
            let (start, end, replacement) = match &rule.replacement {
                Replacement::Template(t) => (whole.start(), whole.end(), expand(&caps, t)),
                Replacement::Literal(s) => (target.start(), target.end(), s.clone()),
                Replacement::Mode(m) => (target.start(), target.end(), self.token(*m, &rule.label, target.as_str())),
            };
            out.push_str(&text[last..start]);
            out.push_str(&replacement);
            last = end;
            *n += 1;
        }
        (*n > 0).then(|| {
            out.push_str(&text[last..]);
            out
        })
    }

    fn token(&self, mode: Mode, label: &str, value: &str) -> String {
        match mode {
            Mode::Mask => format!("<{label}>"),
            Mode::Drop => String::new(),
            Mode::Hash => format!("<{label}:{}>", self.hash(label, value)),
        }
    }

    /// First 5 bytes of SHA-256(salt, label, value), as hex.
    fn hash(&self, label: &str, value: &str) -> String {
        let mut h = Sha256::new();
        h.update(self.salt.0);
        h.update(label.as_bytes());
        h.update([0u8]);
        h.update(value.as_bytes());
        h.finalize()[..5].iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Redacts the string values of a JSON document, keeping it valid JSON
    /// (numbers and keys are never touched).
    pub fn apply_json(&self, v: &Value, counts: &mut Counts) -> Value {
        match v {
            Value::String(s) => Value::String(self.apply(s, counts).into_owned()),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.apply_json(x, counts)).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), self.apply_json(x, counts))).collect()),
            other => other.clone(),
        }
    }

    /// A field's or pseudo-field's text as shown: redacted when in scope.
    pub fn field<'a>(&self, name: &str, text: Cow<'a, str>, counts: &mut Counts) -> Cow<'a, str> {
        if !self.scope.covers(name) {
            return text;
        }
        match text {
            Cow::Borrowed(s) => self.apply(s, counts),
            Cow::Owned(s) => Cow::Owned(self.apply(&s, counts).into_owned()),
        }
    }

    /// The properties of an event, redacted where in scope.
    pub fn fields<'a>(&self, fields: &'a Map<String, Value>, counts: &mut Counts) -> Cow<'a, Map<String, Value>> {
        let before: usize = counts.values().sum();
        let mut out = Map::new();
        for (k, v) in fields {
            let v = if self.scope.covers(k) {
                self.apply_json(v, counts)
            } else {
                v.clone()
            };
            out.insert(k.clone(), v);
        }
        if counts.values().sum::<usize>() == before {
            Cow::Borrowed(fields)
        } else {
            Cow::Owned(out)
        }
    }

    /// An event with everything in scope redacted (borrowed when nothing matched).
    pub fn record<'a>(&self, r: &'a Record, counts: &mut Counts) -> Cow<'a, Record> {
        let before: usize = counts.values().sum();
        let message = self.field("@message", Cow::Borrowed(r.message.as_str()), counts);
        let exception = r.exception.as_deref().map(|x| self.field("@exception", Cow::Borrowed(x), counts));
        let template = r.template.as_deref().map(|t| self.field("@template", Cow::Borrowed(t), counts));
        let fields = self.fields(&r.fields, counts);
        if counts.values().sum::<usize>() == before {
            return Cow::Borrowed(r);
        }
        let mut out = r.clone();
        out.message = message.into_owned();
        out.exception = exception.map(Cow::into_owned);
        out.template = template.map(Cow::into_owned);
        out.fields = fields.into_owned();
        Cow::Owned(out)
    }
}

fn expand(caps: &Captures<'_>, template: &str) -> String {
    let mut s = String::new();
    caps.expand(template, &mut s);
    s
}

/// Hits of one rule in a preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleHits {
    pub rule: String,
    pub count: usize,
}

/// `preview_redaction`: a sample before and after, and what matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct RedactionPreview {
    pub before: String,
    pub after: String,
    /// Every active rule, in order, with its matches (zero included).
    pub hits: Vec<RuleHits>,
    /// Why the configuration does not compile, if it does not.
    pub error: Option<String>,
}

/// Redacts `sample` with `cfg` (a mask/hash preview uses a fixed salt so the
/// result is stable while the user edits).
pub fn preview(cfg: &RedactCfg, sample: &str) -> RedactionPreview {
    let redactor = match Redactor::from_config(cfg, Salt::fixed([7; 16])) {
        Ok(r) => r,
        Err(e) => {
            return RedactionPreview {
                before: sample.to_string(),
                after: sample.to_string(),
                hits: Vec::new(),
                error: Some(format!("{e:#}")),
            }
        }
    };
    let mut counts = Counts::new();
    let after = redactor.apply(sample, &mut counts).into_owned();
    RedactionPreview {
        before: sample.to_string(),
        after,
        hits: hits_of(&redactor, &counts),
        error: None,
    }
}

/// Every rule of `redactor` with its count in `counts`.
pub fn hits_of(redactor: &Redactor, counts: &Counts) -> Vec<RuleHits> {
    redactor
        .rule_names()
        .into_iter()
        .map(|rule| RuleHits {
            rule: rule.to_string(),
            count: counts.get(rule).copied().unwrap_or(0),
        })
        .collect()
}
