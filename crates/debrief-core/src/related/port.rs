//! What "find related logs" needs from a store, and its two adapters: the store ports of ADR 0014 ([`QuerySource`],
//! the database engine) and the in-memory [`LogStore`] ([`LogStoreSource`]) the app uses until it switches over.

use super::rank::LANE_FIELDS;
use super::RelatedEvent;
use crate::api::{PageRequest, SourceInfo};
use crate::ports::{EventView, Filter, PageQuery, Query, StoreError, StoreResult};
use crate::store::LogStore;
use serde_json::{Map, Value};

/// Longest message and exception kept per event, and most characters of a field value.
const MAX_MESSAGE: usize = 1_000;
const MAX_FIELD: usize = 200;

/// The port of "find related logs". Contract: `events` returns at most `limit` events matching `filter` in time
/// order, owned (nothing borrows from the store); errors are typed, never panics.
pub trait RelatedSource {
    fn sources(&self) -> StoreResult<Vec<SourceInfo>>;
    fn events(&self, filter: &Filter, limit: usize) -> StoreResult<Vec<RelatedEvent>>;
}

fn cut(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// The fields a title or name can match, bounded.
fn lane_fields(fields: &Map<String, Value>) -> Map<String, Value> {
    LANE_FIELDS
        .iter()
        .filter_map(|k| {
            fields.get(*k).map(|v| {
                let text = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                ((*k).to_string(), Value::String(cut(&text, MAX_FIELD)))
            })
        })
        .collect()
}

impl From<EventView> for RelatedEvent {
    fn from(e: EventView) -> Self {
        RelatedEvent {
            id: e.id.0,
            ts: e.ts,
            level: e.level,
            message: cut(&e.message, MAX_MESSAGE),
            exception: e.exception.map(|x| cut(&x, MAX_MESSAGE)),
            source: e.source.0,
            source_name: e.source_name,
            group: e.group,
            fields: lane_fields(&e.fields),
        }
    }
}

/// [`RelatedSource`] over the store's [`Query`] port.
pub struct QuerySource<'a>(pub &'a dyn Query);

impl RelatedSource for QuerySource<'_> {
    fn sources(&self) -> StoreResult<Vec<SourceInfo>> {
        Ok(self.0.summary()?.sources)
    }

    fn events(&self, filter: &Filter, limit: usize) -> StoreResult<Vec<RelatedEvent>> {
        let reply = self.0.page(&PageQuery {
            query: filter.clone(),
            limit: limit.max(1),
            ..PageQuery::default()
        })?;
        Ok(reply.rows.into_iter().take(limit).map(RelatedEvent::from).collect())
    }
}

/// [`RelatedSource`] over the in-memory [`LogStore`].
pub struct LogStoreSource<'a>(pub &'a dyn LogStore);

impl RelatedSource for LogStoreSource<'_> {
    fn sources(&self) -> StoreResult<Vec<SourceInfo>> {
        Ok(self.0.sources())
    }

    fn events(&self, filter: &Filter, limit: usize) -> StoreResult<Vec<RelatedEvent>> {
        let page = self
            .0
            .page(&PageRequest {
                query: filter.clone(),
                offset: 0,
                limit: limit.clamp(1, crate::ports::MAX_PAGE_ROWS),
                descending: false,
                anchor: None,
            })
            .map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
        Ok(page
            .rows
            .into_iter()
            .take(limit)
            .map(|r| RelatedEvent {
                id: u64::from(r.id),
                ts: r.ts,
                level: r.level,
                message: cut(&r.message, MAX_MESSAGE),
                exception: r.exception.map(|x| cut(&x, MAX_MESSAGE)),
                source: r.source,
                source_name: r.source_name.to_string(),
                group: r.group,
                fields: lane_fields(&r.fields),
            })
            .collect())
    }
}
