//! Paged record listing.

use super::{Page, PageRequest, RecordView, Workspace};
use crate::model::Record;
use crate::query::CompiledQuery;
use anyhow::Result;
use std::borrow::Cow;

impl Workspace {
    fn view<'a>(&'a self, r: &'a Record, cq: &CompiledQuery, skews: &std::collections::HashMap<u32, i64>) -> RecordView<'a> {
        let source_name = self.source_name(r.source);
        RecordView {
            id: r.id,
            ts: r.ts,
            level: r.level,
            message: self.view_text("@message", Cow::Borrowed(&r.message)),
            template: r.template.as_deref().map(|t| self.view_text("@template", Cow::Borrowed(t))),
            exception: r.exception.as_deref().map(|x| self.view_text("@exception", Cow::Borrowed(x))),
            fields: self.view_fields(&r.fields),
            source: r.source,
            source_name,
            line: r.line,
            group: cq.group_key(r, source_name),
            markers: self.marker_names(r.markers),
            skew_ms: r.ts.and(skews.get(&r.source).copied()),
        }
    }

    pub fn page(&self, req: &PageRequest) -> Result<Page<'_>> {
        let ids = self.matching(&req.query)?;
        let cq = self.compile(&req.query)?;
        let total = ids.len();
        let limit = req.limit.clamp(1, 5000);
        let position = |pos: usize| if req.descending { total - 1 - pos } else { pos };

        let mut anchor_index = None;
        let mut offset = req.offset;
        if let Some(anchor) = req.anchor {
            if let Ok(pos) = ids.binary_search(&anchor) {
                let idx = position(pos);
                anchor_index = Some(idx);
                offset = idx - idx % limit;
            }
        }
        let offset = offset.min(total);
        let end = (offset + limit).min(total);
        let skews = self.offsets_by_source();
        let rows = (offset..end)
            .map(|i| self.view(&self.records[ids[position(i)] as usize], &cq, &skews))
            .collect();
        Ok(Page {
            total,
            offset,
            rows,
            anchor_index,
        })
    }
}
