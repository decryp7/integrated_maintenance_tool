//! Value counts of one field.

use super::{Facet, FacetRequest, FacetValue, Workspace};
use anyhow::Result;
use std::collections::HashMap;

impl Workspace {
    pub fn facet(&self, req: &FacetRequest) -> Result<Facet> {
        let ids = self.matching(&req.query)?;
        let cq = self.compile(&req.query)?;
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut total = 0;
        for &id in ids.iter() {
            let r = &self.records[id as usize];
            let source = self.source_name(r.source);
            let value = if req.field == "@group" {
                Some(cq.group_key(r, source).into())
            } else {
                r.field_text(&req.field, source)
            };
            if let Some(v) = value {
                total += 1;
                *counts.entry(v.into_owned()).or_default() += 1;
            }
        }
        let distinct = counts.len();
        let mut values: Vec<FacetValue> = counts.into_iter().map(|(value, count)| FacetValue { value, count }).collect();
        values.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
        values.truncate(req.limit.clamp(1, 1000));
        Ok(Facet {
            field: req.field.clone(),
            values,
            distinct,
            total,
        })
    }
}
