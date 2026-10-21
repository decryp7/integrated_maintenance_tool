//! Clock-skew correction in the workspace: per-source offsets applied to the
//! stored timestamps (see [`crate::skew`] for why they are applied on load).

use super::Workspace;
use crate::model::Record;
use crate::skew::{
    align_offset, checked_offset, suggest, ClockOffsets, IdEvent, OffsetChange, OffsetSpec, SetOffsetRequest, SkewError, SuggestRequest,
    SuggestResult, MAX_TRACKED,
};
use std::collections::HashMap;

impl Workspace {
    /// Offsets by source path.
    pub fn clock_offsets(&self) -> &ClockOffsets {
        &self.offsets
    }

    /// The offset applied to a source (0 for unknown sources).
    pub fn source_offset(&self, source: u32) -> i64 {
        self.sources.iter().find(|s| s.id == source).map_or(0, |s| s.offset_ms)
    }

    /// Non-zero offsets by source id.
    pub(super) fn offsets_by_source(&self) -> HashMap<u32, i64> {
        self.sources
            .iter()
            .filter(|s| s.offset_ms != 0)
            .map(|s| (s.id, s.offset_ms))
            .collect()
    }

    /// Moves imported records (still at their original time) to the offset of `path`.
    pub(super) fn apply_offset(&self, path: &str, records: &mut [Record]) -> i64 {
        let offset = self.offsets.get(path);
        if offset != 0 {
            for r in records {
                r.ts = crate::skew::shift(r.ts, offset);
            }
        }
        offset
    }

    /// Sets a source's offset (manually or by aligning two events) and moves
    /// its records. Ids before the first moved record stay; the new ids of
    /// the tracked records are returned. Fails, changing nothing, for an
    /// unknown source or record, an untimed event, two events of one source
    /// or an offset beyond [`crate::skew::MAX_OFFSET_MS`].
    pub fn set_source_offset(&mut self, req: &SetOffsetRequest) -> Result<OffsetChange, SkewError> {
        let (source, offset) = match req.spec {
            OffsetSpec::Manual { source, offset_ms } => (source, checked_offset(offset_ms)?),
            OffsetSpec::Align { reference, target } => {
                let (r, t) = (self.timed(reference)?, self.timed(target)?);
                if r.0 == t.0 {
                    return Err(SkewError::SameSource(r.0));
                }
                let original = t.1.saturating_sub(self.source_offset(t.0));
                (t.0, align_offset(r.1, original)?)
            }
        };
        let path = self
            .sources
            .iter()
            .find(|s| s.id == source)
            .map(|s| s.path.clone())
            .ok_or(SkewError::UnknownSource(source))?;
        let previous = self.offsets.set(&path, offset)?;
        let delta = offset - previous;
        // Offsets belong to files: the logical sources a route split from the file move together.
        let moved: Vec<u32> = self.sources.iter().filter(|s| s.path == path).map(|s| s.id).collect();
        // Where each tracked record will sort after the move.
        let keys: Vec<Option<(i64, u32, u32)>> = req
            .track
            .iter()
            .take(MAX_TRACKED)
            .map(|id| {
                self.records.get(*id as usize).map(|r| {
                    let ts = if moved.contains(&r.source) {
                        crate::skew::shift(r.ts, delta)
                    } else {
                        r.ts
                    };
                    (ts.unwrap_or(i64::MAX), r.source, r.line)
                })
            })
            .collect();
        for s in self.sources.iter_mut().filter(|s| s.path == path) {
            s.offset_ms = offset;
        }
        let ids_changed_from = moved.iter().filter_map(|&s| self.reshift(s, delta)).min();
        if delta != 0 {
            self.run_annotators(None);
        }
        let tracked = keys
            .into_iter()
            .map(|k| {
                let k = k?;
                let pos = self.records.partition_point(|r| (r.ts.unwrap_or(i64::MAX), r.source, r.line) < k);
                self.records
                    .get(pos)
                    .filter(|r| (r.ts.unwrap_or(i64::MAX), r.source, r.line) == k)
                    .map(|r| r.id)
            })
            .collect();
        Ok(OffsetChange {
            source,
            offset_ms: offset,
            previous_ms: previous,
            ids_changed_from,
            tracked,
        })
    }

    /// Suggests offsets from events that share a correlation id across sources.
    pub fn suggest_offsets(&self, req: &SuggestRequest) -> SuggestResult {
        let fields = req.id_fields();
        let offsets = self.offsets_by_source();
        let events: Vec<IdEvent> = self
            .records
            .iter()
            .filter_map(|r| {
                let ts = r.ts?;
                let id = fields.iter().find_map(|f| match r.fields.get(f) {
                    Some(serde_json::Value::String(s)) => Some(s.as_str()),
                    _ => None,
                })?;
                Some(IdEvent {
                    source: r.source,
                    id,
                    original_ts: ts.saturating_sub(offsets.get(&r.source).copied().unwrap_or(0)),
                })
            })
            .collect();
        let mut result = suggest(&events, &offsets, req);
        result.fields = fields;
        result
    }

    /// (source, shown timestamp) of a record.
    fn timed(&self, id: u32) -> Result<(u32, i64), SkewError> {
        let r = self.records.get(id as usize).ok_or(SkewError::UnknownRecord(id))?;
        Ok((r.source, r.ts.ok_or(SkewError::Untimed(id))?))
    }
}
