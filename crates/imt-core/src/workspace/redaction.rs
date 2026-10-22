//! The redacted view and redaction previews over the workspace's events.

use super::Workspace;
use crate::redact::view::{sample, MAX_SAMPLES, MAX_SCANNED};
pub use crate::redact::view::{RedactionPreviewRequest, RedactionSample, RedactionSummary, RedactionSummaryRequest};
use crate::redact::{hits_of, Counts, Redactor, Salt};
use anyhow::{bail, Result};
use std::borrow::Cow;
use std::sync::Arc;

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
            bail!(crate::redact::view::NO_RULES);
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
