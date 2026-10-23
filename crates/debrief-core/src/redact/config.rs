//! The profile's `[redact]` section (see `default_profile.toml`).

use super::detectors::Detector;
use serde::{Deserialize, Serialize};

/// What replaces a detected value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// `<email>`.
    #[default]
    Mask,
    /// `<email:1a2b3c4d5e>`: a salted hash, so equal values stay equal (and
    /// can be correlated) within one export, but not across exports.
    Hash,
    /// Removed.
    Drop,
}

/// `[[redact.rule]]`: a custom pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactRuleCfg {
    pub name: String,
    /// A regex; with a group named `value` only that group is replaced.
    pub regex: String,
    /// Literal replacement with `$1` / `$name` expansion of the whole match;
    /// absent = the section's `mode` with the rule name as the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replace: Option<String>,
}

/// `[redact]`: which detectors and rules run, how values are replaced and
/// where. The redacted view, exports, reports and bundles all use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactCfg {
    #[serde(default)]
    pub detectors: Vec<Detector>,
    #[serde(default)]
    pub mode: Mode,
    /// `@message`, `@exception`, `@template`, field names, or `*` for every
    /// field.
    #[serde(default = "default_fields")]
    pub fields: Vec<String>,
    /// Exports, reports and bundles are redacted unless the user unticks it.
    #[serde(default)]
    pub on_export: bool,
    #[serde(default)]
    pub rule: Vec<RedactRuleCfg>,
}

fn default_fields() -> Vec<String> {
    vec!["@message".into(), "@exception".into()]
}

impl Default for RedactCfg {
    fn default() -> Self {
        RedactCfg {
            detectors: Vec::new(),
            mode: Mode::Mask,
            fields: default_fields(),
            on_export: false,
            rule: Vec::new(),
        }
    }
}

/// Which parts of an event are redacted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldScope {
    /// Message, exception, template and every string field.
    All,
    Only {
        message: bool,
        exception: bool,
        template: bool,
        fields: Vec<String>,
    },
}

impl FieldScope {
    /// Parses the profile's `fields` list.
    pub fn parse(names: &[String]) -> FieldScope {
        if names.iter().any(|n| n.trim() == "*") {
            return FieldScope::All;
        }
        let mut message = false;
        let mut exception = false;
        let mut template = false;
        let mut fields = Vec::new();
        for n in names.iter().map(|n| n.trim()) {
            match n {
                "@message" => message = true,
                "@exception" => exception = true,
                "@template" => template = true,
                "" => {}
                other => fields.push(other.to_string()),
            }
        }
        FieldScope::Only {
            message,
            exception,
            template,
            fields,
        }
    }

    /// True when a field or pseudo-field (`@message`, ...) is redacted.
    pub fn covers(&self, name: &str) -> bool {
        match self {
            FieldScope::All => true,
            FieldScope::Only {
                message,
                exception,
                template,
                fields,
            } => match name {
                "@message" => *message,
                "@exception" => *exception,
                // A message's template can hold the same literal text.
                "@template" => *template || *message,
                other => fields.iter().any(|f| f == other),
            },
        }
    }
}
