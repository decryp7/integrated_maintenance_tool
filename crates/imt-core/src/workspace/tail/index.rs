//! Derived state kept up to date incrementally: field statistics, the
//! matching-id caches of active queries and the pattern miner, plus the
//! record-set delta (remove some, insert some) that keeps existing ids stable
//! wherever the time order allows.

use crate::api::FieldInfo;
use crate::model::{value_number, value_text, Record};
use crate::patterns::{MinerConfig, Pattern, PatternMiner};
use crate::query::{CompiledQuery, Query};
use crate::workspace::cache_key::CacheKey;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Distinct values remembered per field.
pub const MAX_DISTINCT: usize = 10_000;
/// Queries whose matching ids are cached (and updated on append).
const MATCH_CACHE_SIZE: usize = 8;

#[derive(Debug, Default, Clone)]
struct FieldStat {
    count: usize,
    non_numeric: usize,
    values: HashMap<String, usize>,
    /// Records carrying the field, by source id (ids are dense and never reused).
    by_source: Vec<u32>,
}

/// Per-field counts, numeric-ness and distinct values, updatable record by record.
#[derive(Debug, Default, Clone)]
pub struct FieldIndex {
    stats: HashMap<String, FieldStat>,
}

impl FieldIndex {
    pub fn build(records: &[Record]) -> FieldIndex {
        let mut index = FieldIndex::default();
        for r in records {
            index.add(r);
        }
        index
    }

    pub fn add(&mut self, r: &Record) {
        for (k, v) in &r.fields {
            let s = match self.stats.get_mut(k) {
                Some(s) => s,
                None => self.stats.entry(k.clone()).or_default(),
            };
            s.count += 1;
            let src = r.source as usize;
            if s.by_source.len() <= src {
                s.by_source.resize(src + 1, 0);
            }
            s.by_source[src] += 1;
            if value_number(v).is_none() {
                s.non_numeric += 1;
            }
            let text = value_text(v);
            if let Some(n) = s.values.get_mut(text.as_ref()) {
                *n += 1;
            } else if s.values.len() < MAX_DISTINCT {
                s.values.insert(text.into_owned(), 1);
            }
        }
    }

    pub fn remove(&mut self, r: &Record) {
        for (k, v) in &r.fields {
            let Some(s) = self.stats.get_mut(k) else { continue };
            s.count = s.count.saturating_sub(1);
            if let Some(n) = s.by_source.get_mut(r.source as usize) {
                *n = n.saturating_sub(1);
            }
            if value_number(v).is_none() {
                s.non_numeric = s.non_numeric.saturating_sub(1);
            }
            let text = value_text(v);
            if let Some(n) = s.values.get_mut(text.as_ref()) {
                *n -= 1;
                if *n == 0 {
                    s.values.remove(text.as_ref());
                }
            }
            if s.count == 0 {
                self.stats.remove(k);
            }
        }
    }

    /// Records of source `source` that carry `field`.
    pub fn count_in(&self, field: &str, source: u32) -> usize {
        self.stats
            .get(field)
            .and_then(|s| s.by_source.get(source as usize))
            .map_or(0, |n| *n as usize)
    }

    /// The field list shown in the summary: most frequent first.
    pub fn infos(&self, derived: &HashSet<&str>) -> Vec<FieldInfo> {
        let mut fields: Vec<FieldInfo> = self
            .stats
            .iter()
            .map(|(name, s)| FieldInfo {
                derived: derived.contains(name.as_str()),
                distinct: s.values.len(),
                count: s.count,
                numeric: s.non_numeric == 0,
                name: name.clone(),
            })
            .collect();
        fields.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        fields
    }
}

/// The order records are kept in (and ids follow).
pub fn sort_key(r: &Record) -> (i64, u32, u32) {
    (r.ts.unwrap_or(i64::MAX), r.source, r.line)
}

/// Whether two versions of a record show the same thing.
pub fn same_record(a: &Record, b: &Record) -> bool {
    a.ts == b.ts
        && a.level == b.level
        && a.line == b.line
        && a.source == b.source
        && a.markers == b.markers
        && a.message == b.message
        && a.template == b.template
        && a.exception == b.exception
        && a.fields == b.fields
}

/// How the record set changed, for updating caches.
#[derive(Debug, Default, Clone)]
pub struct Change {
    /// Record count before the change.
    pub old_len: usize,
    /// Ids at and after this position were renumbered (or removed).
    pub cut: Option<usize>,
    /// Ids rewritten in place (same position, new content).
    pub replaced: Vec<u32>,
    /// Ids of the inserted records, ascending.
    pub inserted: Vec<u32>,
}

impl Change {
    /// Whether existing ids or contents changed (not a pure append).
    pub fn disturbs(&self) -> bool {
        self.cut.is_some_and(|c| c < self.old_len) || !self.replaced.is_empty()
    }
}

#[derive(Debug, Clone)]
struct MatchEntry {
    /// The query as asked (window included).
    key: CacheKey,
    /// The query evaluated (window resolved to `from`).
    query: Query,
    live: bool,
    ids: Arc<Vec<u32>>,
}

/// Matching ids of the most recently used queries.
#[derive(Debug, Default)]
pub struct MatchCache {
    entries: Vec<MatchEntry>,
}

impl MatchCache {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Cached ids for `key` evaluated as `resolved`. A sliding window whose
    /// start moved forward drops the ids that fell out of it (ids follow time order).
    pub fn get(&mut self, key: CacheKey, resolved: &Query, records: &[Record]) -> Option<Arc<Vec<u32>>> {
        let i = self.entries.iter().position(|e| e.key == key)?;
        let e = &mut self.entries[i];
        if e.query.to != resolved.to {
            return None;
        }
        if e.query.from != resolved.from {
            let (Some(old), Some(new)) = (e.query.from, resolved.from) else {
                return None;
            };
            if !e.live || new < old {
                return None;
            }
            let ids = Arc::make_mut(&mut e.ids);
            let drop = ids.partition_point(|&id| records.get(id as usize).and_then(|r| r.ts).is_some_and(|t| t < new));
            ids.drain(..drop);
            e.query.from = Some(new);
        }
        let ids = e.ids.clone();
        let entry = self.entries.remove(i);
        self.entries.insert(0, entry);
        Some(ids)
    }

    pub fn put(&mut self, key: CacheKey, resolved: Query, live: bool, ids: Arc<Vec<u32>>) {
        self.entries.retain(|e| e.key != key);
        self.entries.insert(
            0,
            MatchEntry {
                key,
                query: resolved,
                live,
                ids,
            },
        );
        self.entries.truncate(MATCH_CACHE_SIZE);
    }

    /// Brings every cached result up to date by evaluating only the records the change touched.
    pub fn apply<'n>(
        &mut self,
        change: &Change,
        records: &[Record],
        compile: &dyn Fn(&Query) -> Option<CompiledQuery>,
        name: &dyn Fn(u32) -> &'n str,
    ) {
        self.entries.retain_mut(|e| {
            let Some(cq) = compile(&e.query) else { return false };
            let matches = |id: u32| records.get(id as usize).is_some_and(|r| cq.matches(r, name(r.source)));
            let ids = Arc::make_mut(&mut e.ids);
            for &id in &change.replaced {
                if change.cut.is_some_and(|c| id as usize >= c) {
                    continue;
                }
                match (ids.binary_search(&id), matches(id)) {
                    (Ok(p), false) => {
                        ids.remove(p);
                    }
                    (Err(p), true) => ids.insert(p, id),
                    _ => {}
                }
            }
            let from = match change.cut {
                Some(cut) => {
                    ids.truncate(ids.partition_point(|&id| (id as usize) < cut));
                    cut
                }
                None => change.old_len,
            };
            ids.extend((from..records.len()).map(|i| i as u32).filter(|&id| matches(id)));
            true
        });
    }
}

/// The pattern miner of the last patterns query, fed only new matching records.
pub struct PatternCache {
    key: String,
    generation: u64,
    miner: PatternMiner,
    mined: usize,
    first: Option<u32>,
    last: Option<u32>,
}

impl PatternCache {
    /// Patterns of `ids` (matching `key`), mining only ids not seen before when
    /// the cached miner saw a prefix of the same list in the same generation.
    pub fn patterns(slot: &mut Option<PatternCache>, key: &str, generation: u64, ids: &[u32], records: &[Record]) -> Vec<Pattern> {
        let reusable = slot.as_ref().is_some_and(|c| {
            c.key == key
                && c.generation == generation
                && c.mined <= ids.len()
                && ids.first().copied() == c.first
                && (c.mined == 0 || ids.get(c.mined - 1).copied() == c.last)
        });
        if !reusable {
            *slot = Some(PatternCache {
                key: key.to_string(),
                generation,
                miner: PatternMiner::new(MinerConfig::default()),
                mined: 0,
                first: ids.first().copied(),
                last: None,
            });
        }
        let Some(c) = slot.as_mut() else { return Vec::new() };
        for &id in &ids[c.mined..] {
            if let Some(r) = records.get(id as usize) {
                c.miner.add(r);
            }
        }
        c.mined = ids.len();
        c.last = ids.last().copied();
        c.miner.clusters()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(id: u32, ts: i64, msg: &str, fields: serde_json::Value) -> Record {
        let mut r = Record::new(id + 1);
        r.id = id;
        r.ts = Some(ts);
        r.message = msg.into();
        if let serde_json::Value::Object(m) = fields {
            r.fields = m;
        }
        r
    }

    #[test]
    fn field_index_adds_and_removes_exactly() {
        let a = rec(0, 1, "a", json!({"n": 1, "s": "x"}));
        let b = rec(1, 2, "b", json!({"n": "two"}));
        let mut idx = FieldIndex::build(&[a.clone(), b.clone()]);
        let infos = idx.infos(&HashSet::from(["s"]));
        assert_eq!(infos[0].name, "n");
        assert_eq!((infos[0].count, infos[0].distinct, infos[0].numeric), (2, 2, false));
        assert!(infos[1].derived);
        let mut c = rec(2, 3, "c", json!({"n": 3}));
        c.source = 4;
        idx.add(&c);
        assert_eq!(
            (
                idx.count_in("n", 0),
                idx.count_in("n", 4),
                idx.count_in("n", 9),
                idx.count_in("x", 0)
            ),
            (2, 1, 0, 0)
        );
        idx.remove(&c);
        assert_eq!(idx.count_in("n", 4), 0);
        idx.remove(&b);
        assert_eq!(idx.count_in("n", 0), 1);
        let infos = idx.infos(&HashSet::new());
        assert!(infos[0].numeric && infos[0].distinct == 1);
        idx.remove(&a);
        assert!(idx.infos(&HashSet::new()).is_empty());
        idx.remove(&a);
        let mut many = FieldIndex::default();
        for i in 0..MAX_DISTINCT + 5 {
            many.add(&rec(0, 0, "", json!({ "k": i })));
        }
        assert_eq!(many.infos(&HashSet::new())[0].distinct, MAX_DISTINCT);
        assert!(same_record(&a, &a.clone()));
        assert!(!same_record(&a, &b));
    }

    #[test]
    fn match_cache_follows_changes() {
        let k = |name: &str| CacheKey::of(0, name).unwrap();
        let mut records: Vec<Record> = (0..4)
            .map(|i| rec(i, i as i64 * 10, if i % 2 == 0 { "hit" } else { "miss" }, json!({})))
            .collect();
        let q = Query {
            text: "hit".into(),
            ..Query::default()
        };
        let compile = |q: &Query| CompiledQuery::new(q, &[], &[]).ok();
        let name = |_: u32| "";
        let mut cache = MatchCache::default();
        cache.put(k("k"), q.clone(), false, Arc::new(vec![0, 2]));
        assert!(cache.get(k("other"), &q, &records).is_none());
        // Pure append.
        records.push(rec(4, 40, "hit", json!({})));
        cache.apply(
            &Change {
                old_len: 4,
                ..Change::default()
            },
            &records,
            &compile,
            &name,
        );
        assert_eq!(*cache.get(k("k"), &q, &records).unwrap(), vec![0, 2, 4]);
        // In place: 1 now matches, 2 no longer does.
        records[1].message = "hit".into();
        records[2].message = "miss".into();
        cache.apply(
            &Change {
                old_len: 5,
                replaced: vec![1, 2],
                ..Change::default()
            },
            &records,
            &compile,
            &name,
        );
        assert_eq!(*cache.get(k("k"), &q, &records).unwrap(), vec![0, 1, 4]);
        // Renumbered tail.
        records.insert(3, rec(3, 35, "hit", json!({})));
        for (i, r) in records.iter_mut().enumerate() {
            r.id = i as u32;
        }
        cache.apply(
            &Change {
                old_len: 5,
                cut: Some(3),
                ..Change::default()
            },
            &records,
            &compile,
            &name,
        );
        assert_eq!(*cache.get(k("k"), &q, &records).unwrap(), vec![0, 1, 3, 5]);
        // A different `to` misses; a live window that slid forward drops old ids.
        let moved = Query { to: Some(1), ..q.clone() };
        assert!(cache.get(k("k"), &moved, &records).is_none());
        let live = Query {
            from: Some(0),
            ..q.clone()
        };
        cache.put(k("live"), live.clone(), true, Arc::new(vec![0, 1, 3, 5]));
        let slid = Query {
            from: Some(30),
            ..q.clone()
        };
        assert_eq!(*cache.get(k("live"), &slid, &records).unwrap(), vec![3, 5]);
        let back = Query {
            from: Some(0),
            ..q.clone()
        };
        assert!(cache.get(k("live"), &back, &records).is_none());
        cache.put(k("fixed"), live, false, Arc::new(vec![]));
        assert!(cache.get(k("fixed"), &slid, &records).is_none());
        let bad = Query {
            text: "(".into(),
            regex: true,
            ..Query::default()
        };
        cache.put(k("bad"), bad, false, Arc::new(vec![]));
        cache.apply(&Change::default(), &records, &compile, &name);
        assert!(cache.get(k("bad"), &q, &records).is_none());
        cache.clear();
        assert!(cache.get(k("k"), &q, &records).is_none());
    }

    #[test]
    fn pattern_cache_mines_incrementally() {
        let records: Vec<Record> = (0..6)
            .map(|i| rec(i, i as i64, &format!("user {i} logged in"), json!({})))
            .collect();
        let mut slot = None;
        let all: Vec<u32> = (0..6).collect();
        let first = PatternCache::patterns(&mut slot, "k", 1, &all[..3], &records);
        assert_eq!(first[0].count, 3);
        let more = PatternCache::patterns(&mut slot, "k", 1, &all, &records);
        assert_eq!(more, crate::patterns::mine(records.iter(), &MinerConfig::default()));
        // Another generation or key starts over.
        let again = PatternCache::patterns(&mut slot, "k", 2, &all[..2], &records);
        assert_eq!(again[0].count, 2);
        assert_eq!(PatternCache::patterns(&mut slot, "j", 2, &[], &records), vec![]);
    }

    #[test]
    fn a_value_with_no_events_left_leaves_the_distinct_values_of_a_field() {
        let a = rec(0, 1, "a", json!({"s": "x"}));
        let b = rec(1, 2, "b", json!({"s": "y"}));
        let mut idx = FieldIndex::build(&[a.clone(), b]);
        assert_eq!(idx.infos(&HashSet::new())[0].distinct, 2);
        idx.remove(&a);
        let infos = idx.infos(&HashSet::new());
        assert_eq!(
            (infos[0].count, infos[0].distinct),
            (1, 1),
            "the field stays, the removed value is gone"
        );
    }

    #[test]
    fn a_window_with_an_open_end_is_not_slid_and_replaced_records_beyond_a_cut_are_left_to_the_cut() {
        let k = |name: &str| CacheKey::of(0, name).unwrap();
        let records: Vec<Record> = (0..4).map(|i| rec(i, i as i64 * 10, "hit", json!({}))).collect();
        let open = Query {
            text: "hit".into(),
            ..Query::default()
        };
        let from = Query {
            from: Some(10),
            ..open.clone()
        };
        let mut cache = MatchCache::default();
        // A cached window without a start cannot be slid to one with a start, nor the reverse.
        cache.put(k("open"), open.clone(), true, Arc::new(vec![0, 1, 2, 3]));
        assert!(cache.get(k("open"), &from, &records).is_none());
        cache.put(k("from"), from, true, Arc::new(vec![1, 2, 3]));
        assert!(cache.get(k("from"), &open, &records).is_none());
        // A record the change says was replaced but lies beyond the cut is dropped by the cut, not looked at.
        cache.put(k("cut"), open.clone(), false, Arc::new(vec![0, 1, 2, 3]));
        let compile = |q: &Query| CompiledQuery::new(q, &[], &[]).ok();
        cache.apply(
            &Change {
                old_len: 4,
                replaced: vec![3],
                cut: Some(2),
                ..Change::default()
            },
            &records[..2],
            &compile,
            &|_: u32| "",
        );
        assert_eq!(*cache.get(k("cut"), &open, &records[..2]).unwrap(), vec![0, 1]);
    }
}
