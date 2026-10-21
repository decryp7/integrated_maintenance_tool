//! Resolving a report for export over the in-memory workspace (the reference implementation; the store
//! resolves through [`crate::report::resolve`] with its events looked up by id).

use super::Workspace;
use crate::report::{resolve, Report, ResolvedReport};
use anyhow::Result;

impl Workspace {
    /// The report with its events looked up and, with `redact`, every text passed through the profile's
    /// redaction rules with a fresh salt.
    pub fn resolve_report(&self, report: &Report, redact: bool) -> Result<ResolvedReport> {
        let redactor = if redact { Some(self.export_redactor()?) } else { None };
        resolve::resolve(report, redactor.as_ref(), &mut |ids| {
            Ok(ids
                .iter()
                .map(|&id| {
                    self.records
                        .get(id as usize)
                        .filter(|x| x.id == id)
                        .map(|r| (r.clone(), self.source_name(r.source).to_string()))
                })
                .collect())
        })
    }
}
