//! Clock-skew helpers that work over any store's ports: suggested offsets from events sharing a correlation id.

use super::{suggest, IdEvent, SuggestRequest, SuggestResult, MAX_IDS};
use crate::batch::FieldValue;
use crate::ports::{Filter, Query, StoreResult};
use std::collections::HashMap;
use std::ops::ControlFlow;

/// Most events read for a suggestion.
pub const MAX_SCAN: usize = 5_000_000;

/// Offsets that would align each source with a reference: the earliest event per source and correlation id (at most
/// [`MAX_IDS`] of them) is read through [`Query::scan`], their original times are the shown ones less each source's
/// current offset, and [`suggest`] pairs them. Cancellable through the thread's `QueryControl`.
pub fn suggest_over(store: &(impl Query + ?Sized), req: &SuggestRequest) -> StoreResult<SuggestResult> {
    let fields = req.id_fields();
    let offsets: HashMap<u32, i64> = store
        .summary()?
        .sources
        .iter()
        .filter(|s| s.offset_ms != 0)
        .map(|s| (s.id, s.offset_ms))
        .collect();
    // (source, id) -> earliest original time.
    let mut earliest: HashMap<(u32, String), i64> = HashMap::new();
    let mut read = 0usize;
    store.scan(&Filter::default(), &mut |batch| {
        for row in batch.rows() {
            read += 1;
            if read > MAX_SCAN {
                return ControlFlow::Break(());
            }
            let Some(ts) = row.ts() else { continue };
            let source = row.source().0;
            let id = fields.iter().find_map(|f| match row.field(f) {
                Some(FieldValue::Text(s)) => Some(s),
                _ => None,
            });
            let Some(id) = id else { continue };
            let original = ts.saturating_sub(offsets.get(&source).copied().unwrap_or(0));
            let key = (source, id.to_string());
            if let Some(t) = earliest.get_mut(&key) {
                *t = (*t).min(original);
            } else if earliest.len() < MAX_IDS {
                earliest.insert(key, original);
            }
        }
        ControlFlow::Continue(())
    })?;
    let events: Vec<IdEvent> = earliest
        .iter()
        .map(|((source, id), original_ts)| IdEvent {
            source: *source,
            id,
            original_ts: *original_ts,
        })
        .collect();
    let mut result = suggest(&events, &offsets, req);
    result.fields = fields;
    Ok(result)
}
