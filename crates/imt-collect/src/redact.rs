//! Masking of secrets and personal data while text is copied into a bundle.
//!
//! A thin adapter over the engine's [`imt_core::redact::Redactor`]: the
//! collector's plan picks the built-in detectors and custom patterns, and
//! keeps its own replacement texts (`[REDACTED]`, `[email]`, `[ipv4]`...).

use crate::plan::RedactConfig;
use anyhow::Result;
use imt_core::redact::{Detector, FieldScope, Replacement, Rule, Salt};

pub use imt_core::redact::{luhn_ok, Counts};

const MASK: &str = "[REDACTED]";

/// The built-ins the collector always applies (when `built_in`), in order.
const SECRETS: [Detector; 7] = [
    Detector::BearerToken,
    Detector::Jwt,
    Detector::ApiKey,
    Detector::ConnectionString,
    Detector::Password,
    Detector::Email,
    Detector::CreditCard,
];

pub struct Redactor {
    inner: imt_core::redact::Redactor,
}

fn literal(d: Detector) -> Rule {
    let text = match d {
        Detector::Email => "[email]",
        Detector::Ipv4 => "[ipv4]",
        Detector::Ipv6 => "[ipv6]",
        _ => MASK,
    };
    Rule::detector(d, Replacement::Literal(text.into()))
}

impl Redactor {
    pub fn new(cfg: &RedactConfig) -> Result<Redactor> {
        let mut rules = Vec::new();
        if cfg.built_in {
            rules.extend(SECRETS.into_iter().map(literal));
            if cfg.ip_addresses {
                rules.extend([Detector::Ipv4, Detector::Ipv6].into_iter().map(literal));
            }
        }
        for p in &cfg.patterns {
            rules.push(Rule::custom(&p.name, &p.regex, Replacement::Template(p.replace.clone()))?);
        }
        Ok(Redactor {
            inner: imt_core::redact::Redactor::new(rules, FieldScope::All, Salt::random()),
        })
    }

    /// True when there is nothing to apply (files can be copied untouched).
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Redacts the string values of a JSON document, keeping it valid JSON
    /// (numbers are never touched).
    pub fn apply_json(&self, v: &serde_json::Value, counts: &mut Counts) -> serde_json::Value {
        self.inner.apply_json(v, counts)
    }

    /// Applies every rule in order; returns the new text and adds to `counts`.
    pub fn apply(&self, text: &str, counts: &mut Counts) -> String {
        self.inner.apply(text, counts).into_owned()
    }
}
