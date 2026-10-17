//! Applying a live refresh to the record set and everything derived from it:
//! the merge that keeps ids stable, per-source counts, retention, instance
//! tracking, live alerts, status, sliding windows and incremental patterns.

use super::alerts::{Batch, InstanceSeen};
use super::index::{same_record, sort_key};
use super::{
    group_key, wall_ms, Change, Delta, Mode, PatternCache, TailFileStatus, TailReport, TailStatus, TailTotals, TimeWindow, WatchMode,
    Workspace,
};
use crate::model::Record;
use crate::patterns::Pattern;
use crate::query::Query;
use anyhow::Result;
use rayon::prelude::*;
use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::time::Instant;

/// Most records a patterns query mines (as for the other analyses).
const MAX_MINED: usize = 2_000_000;

impl Workspace {
    /// Removes and inserts records. Records rewritten with the same sort key
    /// keep their id; new records after the newest existing one are appended;
    /// otherwise only the tail from the first affected position is re-merged
    /// and renumbered (never a full sort).
    pub(super) fn apply_delta(&mut self, delta: Delta) -> Change {
        let old_len = self.records.len();
        let Delta { remove, mut insert } = delta;
        let doomed = |r: &Record| remove.iter().any(|(s, l)| r.source == *s && l.is_none_or(|l| r.line >= l));
        let positions: Vec<usize> = if remove.is_empty() {
            Vec::new()
        } else {
            self.records
                .par_iter()
                .enumerate()
                .filter(|(_, r)| doomed(r))
                .map(|(i, _)| i)
                .collect()
        };
        insert.par_sort_by_key(sort_key);

        // Same record re-read (its sort key unchanged): rewrite in place.
        let mut taken = vec![false; insert.len()];
        let mut replaced = Vec::new();
        let mut removed = Vec::new();
        if !positions.is_empty() {
            let by_key: HashMap<(i64, u32, u32), usize> = insert.iter().enumerate().map(|(i, r)| (sort_key(r), i)).collect();
            for p in positions {
                match by_key.get(&sort_key(&self.records[p])) {
                    Some(&k) if !taken[k] => {
                        taken[k] = true;
                        if !same_record(&self.records[p], &insert[k]) {
                            let mut new = std::mem::replace(&mut insert[k], Record::new(0));
                            new.id = p as u32;
                            self.field_index.remove(&self.records[p]);
                            self.field_index.add(&new);
                            self.records[p] = new;
                            replaced.push(p as u32);
                        }
                    }
                    _ => removed.push(p),
                }
            }
        }
        let fresh: Vec<Record> = insert.into_iter().zip(taken).filter(|(_, t)| !t).map(|(r, _)| r).collect();

        let mut change = Change {
            old_len,
            cut: None,
            replaced,
            inserted: Vec::new(),
        };
        if fresh.is_empty() && removed.is_empty() {
            return self.after_change(change);
        }
        let first_new = fresh.first().map(|n| {
            let key = sort_key(n);
            self.records.partition_point(|r| sort_key(r) <= key)
        });
        let cut = removed.first().copied().into_iter().chain(first_new).min().unwrap_or(old_len);
        for r in &fresh {
            self.field_index.add(r);
        }
        if cut == old_len {
            // Pure append: existing ids are untouched.
            for (i, mut r) in fresh.into_iter().enumerate() {
                r.id = (old_len + i) as u32;
                change.inserted.push(r.id);
                self.records.push(r);
            }
            return self.after_change(change);
        }
        let mut tail = self.records.split_off(cut);
        if !removed.is_empty() {
            let gone: BTreeSet<usize> = removed.iter().map(|p| p - cut).collect();
            let mut i = 0;
            tail.retain(|r| {
                let keep = !gone.contains(&i);
                i += 1;
                if !keep {
                    self.field_index.remove(r);
                }
                keep
            });
        }
        // Merge (existing records first on equal keys) and renumber from the cut.
        let mut fresh = fresh.into_iter().peekable();
        let mut tail = tail.into_iter().peekable();
        loop {
            let take_new = match (tail.peek(), fresh.peek()) {
                (Some(t), Some(n)) => sort_key(n) < sort_key(t),
                (None, Some(_)) => true,
                (_, None) => false,
            };
            let next = if take_new { fresh.next() } else { tail.next() };
            let Some(mut r) = next else { break };
            r.id = self.records.len() as u32;
            if take_new {
                change.inserted.push(r.id);
            }
            self.records.push(r);
        }
        change.replaced.retain(|&id| (id as usize) < cut);
        change.cut = Some(cut);
        self.after_change(change)
    }

    /// Refreshes derived state after the record set changed.
    fn after_change(&mut self, change: Change) -> Change {
        if change.disturbs() {
            self.generation += 1;
        }
        self.refresh_field_infos();
        let group_by = self.pick_group_by();
        let regrouped = group_by != self.default_group_by;
        self.default_group_by = group_by;
        {
            let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
            if regrouped {
                cache.clear();
            } else {
                let compile = |q: &Query| crate::query::CompiledQuery::new(q, &self.default_group_by, &self.marker_kinds).ok();
                let name = |s: u32| self.source_name(s);
                cache.apply(&change, &self.records, &compile, &name);
            }
        }
        *self.anomaly_cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
        change
    }

    /// Something other than records changed what queries see (renamed sources).
    pub(super) fn invalidate(&mut self) {
        self.generation += 1;
        self.cache.lock().unwrap_or_else(|p| p.into_inner()).clear();
        *self.anomaly_cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.tail.instances = None;
    }

    /// Record counts and time ranges of the given sources.
    pub(super) fn recount(&mut self, sources: &BTreeSet<u32>) {
        if sources.is_empty() {
            return;
        }
        type Acc = HashMap<u32, (usize, Option<i64>, Option<i64>)>;
        let merge = |mut a: Acc, b: Acc| {
            for (k, (n, f, t)) in b {
                let e = a.entry(k).or_insert((0, None, None));
                e.0 += n;
                e.1 = e.1.into_iter().chain(f).min();
                e.2 = e.2.into_iter().chain(t).max();
            }
            a
        };
        let counts: Acc = self
            .records
            .par_iter()
            .filter(|r| sources.contains(&r.source))
            .fold(Acc::new, |mut a, r| {
                let e = a.entry(r.source).or_insert((0, None, None));
                e.0 += 1;
                if let Some(ts) = r.ts {
                    e.1 = Some(e.1.map_or(ts, |f| f.min(ts)));
                    e.2 = Some(e.2.map_or(ts, |t| t.max(ts)));
                }
                a
            })
            .reduce(Acc::new, merge);
        for s in self.sources.iter_mut().filter(|s| sources.contains(&s.id)) {
            let (n, from, to) = counts.get(&s.id).copied().unwrap_or((0, None, None));
            (s.records, s.from, s.to) = (n, from, to);
        }
    }

    /// Applies the retention limit, evicting the oldest records in bulk (once
    /// the limit is exceeded by a tenth, down to the limit).
    pub(super) fn evict(&mut self, report: &mut TailReport) {
        let retention = self.tail.options.retention;
        let n = self.records.len();
        let mut k = 0;
        if let Some(max) = retention.max_records {
            if n > max + (max / 10).max(1) {
                k = n - max;
            }
        }
        if let (Some(age), Some(newest)) = (retention.max_age_ms, self.newest_ts()) {
            let cutoff = newest - age;
            let oldest = self.records.first().and_then(|r| r.ts);
            if oldest.is_some_and(|t| t < cutoff - (age / 10).max(1)) {
                k = k.max(self.records.partition_point(|r| r.ts.is_some_and(|t| t < cutoff)));
            }
        }
        if k == 0 {
            return;
        }
        let touched: BTreeSet<u32> = self.records[..k].iter().map(|r| r.source).collect();
        for r in &self.records[..k] {
            self.field_index.remove(r);
        }
        self.records.drain(..k);
        self.records.par_iter_mut().enumerate().for_each(|(i, r)| r.id = i as u32);
        self.cache.lock().unwrap_or_else(|p| p.into_inner()).evict(k);
        *self.anomaly_cache.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.generation += 1;
        self.refresh_field_infos();
        self.default_group_by = self.pick_group_by();
        self.recount(&touched);
        self.tail.evicted += k;
        report.evicted += k;
        report.ids_changed_from = Some(0);
    }

    /// Timestamp of the newest record (records are kept in time order, untimed last).
    pub fn newest_ts(&self) -> Option<i64> {
        let end = self.records.partition_point(|r| r.ts.is_some());
        self.records[..end].last().and_then(|r| r.ts)
    }

    /// "Now" for live windows: the newest record, or the wall clock with `[live] clock = "wall"`.
    pub fn live_now(&self) -> i64 {
        if self.profile.profile.live.clock.eq_ignore_ascii_case("wall") {
            return wall_ms();
        }
        self.newest_ts().unwrap_or_else(wall_ms)
    }

    /// The query with a sliding window resolved into `from`.
    pub fn resolve_window<'q>(&self, q: &'q Query) -> Cow<'q, Query> {
        match q.window {
            None => Cow::Borrowed(q),
            Some(TimeWindow::Live { last_ms }) => {
                let since = self.live_now().saturating_sub(last_ms.max(0));
                Cow::Owned(Query {
                    from: Some(q.from.map_or(since, |f| f.max(since))),
                    window: None,
                    ..q.clone()
                })
            }
        }
    }

    /// Message patterns of the matching records; after an append only the new
    /// records are added to the existing clusters.
    pub(in crate::workspace) fn mined_patterns(&self, q: &Query) -> Result<Vec<Pattern>> {
        let ids = self.matching(q)?;
        let key = serde_json::to_string(&self.resolve_window(q))?;
        let ids = &ids[..ids.len().min(MAX_MINED)];
        let mut slot = self.pattern_cache.lock().unwrap_or_else(|p| p.into_inner());
        Ok(PatternCache::patterns(&mut slot, &key, self.generation, ids, &self.records))
    }

    /// Builds the instance list the first time live tail needs it.
    pub(super) fn ensure_instances(&mut self) {
        if self.tail.instances.as_ref().is_some_and(|(by, _)| *by == self.default_group_by) {
            return;
        }
        let mut map: HashMap<String, InstanceSeen> = HashMap::new();
        for r in &self.records {
            let key = group_key(&self.default_group_by, r, self.source_name(r.source));
            let e = map.entry(key).or_default();
            e.count += 1;
            if let Some(ts) = r.ts {
                e.last_ts = e.last_ts.max(ts);
            }
        }
        self.tail.instances = Some((self.default_group_by.clone(), map));
    }

    /// Adds the inserted records to the instance list; reports instances seen for the first time.
    pub(super) fn track_instances(&mut self, inserted: &[u32], report: &mut TailReport) {
        if self.tail.instances.as_ref().is_none_or(|(by, _)| *by != self.default_group_by) {
            self.ensure_instances();
            return;
        }
        let Some((by, mut map)) = self.tail.instances.take() else { return };
        for &id in inserted {
            let Some(r) = self.records.get(id as usize) else { continue };
            let key = group_key(&by, r, self.source_name(r.source));
            let e = match map.get_mut(&key) {
                Some(e) => e,
                None => {
                    report.instances_changed.push(key.clone());
                    map.entry(key).or_insert(InstanceSeen {
                        last_ts: i64::MIN,
                        count: 0,
                    })
                }
            };
            e.count += 1;
            if let Some(ts) = r.ts {
                e.last_ts = e.last_ts.max(ts);
            }
        }
        report.instances_changed.sort();
        self.tail.instances = Some((by, map));
    }

    /// Runs the live detectors and alert rules over this batch.
    pub(super) fn live_alerts(&mut self, inserted: &[u32], report: &mut TailReport) {
        let Ok(rules) = self.profile.profile.live.compile() else { return };
        if self.records.is_empty() || (inserted.is_empty() && !rules.wall_clock) {
            return;
        }
        let mut state = std::mem::take(&mut self.tail.alerts);
        let detect = !inserted.is_empty() && state.detect_due(self.tail.options.detect_every_ms);
        let empty = HashMap::new();
        let instances = self.tail.instances.as_ref().map_or(&empty, |(_, m)| m);
        let name = |s: u32| self.source_name(s);
        let batch = Batch {
            records: &self.records,
            inserted,
            names: &name,
            group_by: &self.default_group_by,
            marker_kinds: &self.marker_kinds,
            instances,
            now: self.live_now(),
            detect,
        };
        report.alerts = state.evaluate(&rules, &batch);
        self.tail.alerts = state;
    }

    /// Per-file live state and totals. `mode` is `off` here; the shell fills in
    /// how each file is watched.
    pub fn tail_status(&self) -> TailStatus {
        let now = Instant::now();
        let files: Vec<TailFileStatus> = self
            .tail
            .states
            .iter()
            .map(|(id, st)| TailFileStatus {
                source_id: *id,
                path: st.path.to_string_lossy().into_owned(),
                name: self.source_name(*id).to_string(),
                group: st.group.clone(),
                mode: WatchMode::Off,
                offset: st.seed.offset,
                size: if matches!(st.seed.mode, Mode::Whole) {
                    st.seed.len
                } else {
                    st.seed.len.max(st.seed.offset)
                },
                lines_per_sec: st.rate.per_sec(now),
                last_event: st.last_event,
                state: st.state.clone(),
            })
            .collect();
        let totals = TailTotals {
            files: files.len(),
            tailing: files
                .iter()
                .filter(|f| matches!(f.state, super::TailFileState::Tailing | super::TailFileState::Rotated))
                .count(),
            errors: files
                .iter()
                .filter(|f| matches!(f.state, super::TailFileState::Error { .. }))
                .count(),
            bytes: files.iter().map(|f| f.offset).sum(),
            lines_per_sec: files.iter().map(|f| f.lines_per_sec).sum(),
            evicted: self.tail.evicted,
            records: self.records.len(),
        };
        TailStatus { files, totals }
    }
}

impl TailStatus {
    /// Fills in each file's watch mode.
    pub fn with_modes(mut self, mode_of: impl Fn(&Path) -> WatchMode) -> TailStatus {
        for f in &mut self.files {
            f.mode = mode_of(Path::new(&f.path));
        }
        self
    }
}
