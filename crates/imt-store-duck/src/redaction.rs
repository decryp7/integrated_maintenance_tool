//! What redaction would change in the matching events: the engine's summary ([`imt_core::redact::view::summary`])
//! over the store's own scan, with the active profile's `[redact]` rules.

use crate::DuckStore;
use imt_core::ports::{Redaction, StoreResult};
use imt_core::redact::{RedactionSummary, RedactionSummaryRequest};

impl Redaction for DuckStore {
    fn redaction_summary(&self, req: &RedactionSummaryRequest) -> StoreResult<RedactionSummary> {
        let cfg = self.inner.profile().profile.redact.clone();
        imt_core::redact::view::summary(self, &cfg, req)
    }
}
