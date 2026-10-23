//! The catalogue of an in-memory workspace: built on request from its records (the browser and test backend has no
//! background thread or tables). Same answer shape as the DuckDB store's persistent catalogue.

use super::Workspace;
use crate::api::FacetRequest;
use crate::catalogue::*;
use crate::entities::extract;
use crate::ports::Filter;
use std::collections::HashMap;

impl Workspace {
    /// Names that look like `req.text`: entities in the text of at most [`MAX_EVENTS_SCANNED`] records and the
    /// values of fields with few distinct values. Always built.
    pub fn catalogue(&self, req: &CatalogueRequest) -> CatalogueReply {
        let mut tallies: HashMap<u32, Tally> = HashMap::new();
        for r in self.records.iter().take(MAX_EVENTS_SCANNED) {
            let mut texts = vec![r.message.as_str()];
            if let Some(e) = r.exception.as_deref() {
                texts.push(e);
            }
            tallies
                .entry(r.source)
                .or_default()
                .add_event(&extract(&texts), u64::from(r.id), r.ts);
        }
        let mut rows: Vec<(String, Entry)> = Vec::new();
        for (source, t) in tallies {
            let name = self.source_name(source).to_string();
            rows.extend(t.finish().into_iter().map(|e| (name.clone(), e)));
        }
        for f in self
            .fields
            .iter()
            .filter(|f| !f.numeric && f.distinct > 0 && f.distinct <= MAX_FIELD_DISTINCT)
        {
            let Ok(facet) = self.facet(&FacetRequest {
                query: Filter::default(),
                field: f.name.clone(),
                limit: MAX_VALUES_PER_FIELD,
            }) else {
                continue;
            };
            for v in facet.values {
                rows.push((
                    String::new(),
                    Entry {
                        scope: Scope::Field(f.name.clone()),
                        name: v.value,
                        count: v.count as u64,
                        first_ts: None,
                        last_ts: None,
                        first_id: 0,
                        last_id: 0,
                    },
                ));
            }
        }
        let text: String = req.text.trim().chars().take(crate::find::MAX_TEXT_CHARS).collect();
        let mut candidates = candidates_from(&rows, &text);
        for c in &mut candidates {
            c.seen_in.retain(|s| !s.is_empty());
        }
        CatalogueReply {
            built: true,
            pending: 0,
            candidates,
        }
    }
}
