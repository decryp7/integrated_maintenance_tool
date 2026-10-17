//! The redacted view and redaction previews over the workspace's events.

use super::Workspace;
use crate::query::Query;
use crate::redact::{hits_of, Counts, Redactor, RuleHits, Salt};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;

/// Most events a redaction summary scans.
const MAX_SCANNED: usize = 200_000;
/// Most before/after samples in a summary.
const MAX_SAMPLES: usize = 20;

/// `preview_redaction`: a sample redacted with the profile's rules, or with
/// those of an unsaved profile.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionPreviewRequest {
    pub sample: String,
    /// A candidate profile (TOML); absent = the active profile.
    #[serde(default)]
    pub toml: Option<String>,
}

/// `redaction_summary`: what redaction would change in the events of a query
/// (shown before an export, report or bundle is written).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionSummaryRequest {
    #[serde(default)]
    pub query: Query,
    #[serde(default = "default_samples")]
    pub samples: usize,
}

fn default_samples() -> usize {
    5
}

/// One event before and after redaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionSample {
    pub record_id: u32,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionSummary {
    /// Events looked at (at most 200,000).
    pub scanned: usize,
    /// Of those, events with at least one match.
    pub changed: usize,
    /// More events matched the query than were scanned.
    pub truncated: bool,
    pub hits: Vec<RuleHits>,
    pub samples: Vec<RedactionSample>,
}

impl Workspace {
    /// A redactor for one export: the profile's rules with a fresh salt, so
    /// hashes are stable within the export and differ from other exports.
    pub fn export_redactor(&self) -> Result<Redactor> {
        Redactor::from_config(&self.profile.profile.redact, Salt::random())
    }

    /// Switches the redacted view on (pages, facets and source context are
    /// redacted until it is switched off) or off. Fails when the profile has
    /// no redaction rules.
    pub fn set_view_redaction(&mut self, on: bool) -> Result<()> {
        if !on {
            self.view_redaction = None;
            return Ok(());
        }
        let redactor = self.export_redactor()?;
        if redactor.is_empty() {
            bail!("the profile has no redaction rules: add detectors or [[redact.rule]] to its [redact] section");
        }
        self.view_redaction = Some(Arc::new(redactor));
        Ok(())
    }

    /// Recompiles the redacted view after a profile change (off when the new
    /// profile has no rules).
    pub(super) fn refresh_view_redaction(&mut self) -> Result<()> {
        if self.view_redaction.is_some() {
            let redactor = self.export_redactor()?;
            self.view_redaction = (!redactor.is_empty()).then(|| Arc::new(redactor));
        }
        Ok(())
    }

    /// `text` of field `name` as the view shows it.
    pub(super) fn view_text<'a>(&self, name: &str, text: Cow<'a, str>) -> Cow<'a, str> {
        match &self.view_redaction {
            Some(r) => r.field(name, text, &mut Counts::new()),
            None => text,
        }
    }

    /// A source line as the context view shows it (always fully redacted).
    pub(super) fn view_line(&self, text: String) -> String {
        match &self.view_redaction {
            Some(r) => r.apply(&text, &mut Counts::new()).into_owned(),
            None => text,
        }
    }

    pub(super) fn view_fields<'a>(
        &self,
        fields: &'a serde_json::Map<String, serde_json::Value>,
    ) -> Cow<'a, serde_json::Map<String, serde_json::Value>> {
        match &self.view_redaction {
            Some(r) => r.fields(fields, &mut Counts::new()),
            None => Cow::Borrowed(fields),
        }
    }

    /// Hits per rule over the events matching a query, with a few samples.
    pub fn redaction_summary(&self, req: &RedactionSummaryRequest) -> Result<RedactionSummary> {
        let redactor = self.export_redactor()?;
        let ids = self.matching(&req.query)?;
        let mut counts = Counts::new();
        let mut changed = 0;
        let mut samples = Vec::new();
        for &id in ids.iter().take(MAX_SCANNED) {
            let r = &self.records[id as usize];
            let out = redactor.record(r, &mut counts);
            if let Cow::Owned(after) = out {
                changed += 1;
                if samples.len() < req.samples.min(MAX_SAMPLES) {
                    samples.push(sample(r, &after));
                }
            }
        }
        Ok(RedactionSummary {
            scanned: ids.len().min(MAX_SCANNED),
            changed,
            truncated: ids.len() > MAX_SCANNED,
            hits: hits_of(&redactor, &counts),
            samples,
        })
    }
}

/// The message, or the properties when only they changed.
fn sample(before: &crate::model::Record, after: &crate::model::Record) -> RedactionSample {
    let (b, a) = if before.message != after.message {
        (before.message.clone(), after.message.clone())
    } else if before.exception != after.exception {
        (
            before.exception.clone().unwrap_or_default(),
            after.exception.clone().unwrap_or_default(),
        )
    } else if before.template != after.template {
        (
            before.template.clone().unwrap_or_default(),
            after.template.clone().unwrap_or_default(),
        )
    } else {
        (
            serde_json::Value::Object(before.fields.clone()).to_string(),
            serde_json::Value::Object(after.fields.clone()).to_string(),
        )
    };
    RedactionSample {
        record_id: before.id,
        before: b,
        after: a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Record;

    #[test]
    fn samples_show_what_changed() {
        let req: RedactionSummaryRequest = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(req.samples, 5);
        let mut before = Record::new(1);
        before.message = "m".into();
        before.exception = Some("x".into());
        before.template = Some("t".into());
        before.fields.insert("F".into(), "v".into());
        let changed = |f: &dyn Fn(&mut Record)| {
            let mut after = before.clone();
            f(&mut after);
            let s = sample(&before, &after);
            (s.before, s.after)
        };
        assert_eq!(changed(&|r| r.message = "M".into()), ("m".into(), "M".into()));
        assert_eq!(changed(&|r| r.exception = Some("X".into())), ("x".into(), "X".into()));
        assert_eq!(changed(&|r| r.template = Some("T".into())), ("t".into(), "T".into()));
        let (b, a) = changed(&|r| {
            r.fields.insert("F".into(), "<v>".into());
        });
        assert!(b.contains("\"v\"") && a.contains("<v>"));
    }
}
