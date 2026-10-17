//! Moving one source's records in time (clock-skew correction) with the merge
//! live tail uses: ids before the first affected position stay, the rest is
//! merged in O(n) (two sorted runs) and renumbered, never fully re-sorted.

use super::Delta;
use crate::model::Record;
use crate::workspace::Workspace;
use std::collections::BTreeSet;

impl Workspace {
    /// Shifts the timed records of `source` by `delta_ms` (untimed ones stay
    /// last). Returns the first id that changed, if any.
    pub(in crate::workspace) fn reshift(&mut self, source: u32, delta_ms: i64) -> Option<u32> {
        if delta_ms == 0 {
            return None;
        }
        let insert: Vec<Record> = self
            .records
            .iter()
            .filter(|r| r.source == source)
            .map(|r| {
                let mut moved = r.clone();
                moved.ts = crate::skew::shift(moved.ts, delta_ms);
                moved
            })
            .collect();
        let change = self.apply_delta(Delta {
            remove: vec![(source, None)],
            insert,
        });
        self.recount(&BTreeSet::from([source]));
        change.cut.filter(|_| change.disturbs()).map(|c| c as u32)
    }
}
