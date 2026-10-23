//! The values behind a pattern's `<*>` placeholders (ADR 0046): the most frequent ones with their counts, so a
//! template becomes evidence (which user, which id) without a second query.
//!
//! Each placeholder keeps a bounded **Space-Saving** summary (Metwally et al., 2005): at most [`COUNTERS`] values; when a
//! new value arrives and all counters are in use, the smallest counter is taken over, so counts of the heavy hitters are
//! right within a bound and memory per placeholder is constant. While no value was ever dropped the summary is exact.

use serde::{Deserialize, Serialize};

/// Values tracked per placeholder.
pub const COUNTERS: usize = 16;
/// Values returned per placeholder.
pub const TOP_VALUES: usize = 8;
/// Longest value kept (characters); longer values are cut.
pub const MAX_VALUE_CHARS: usize = 120;

/// A value and how many events of the pattern carried it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceholderValue {
    pub value: String,
    pub count: u64,
}

/// The values of one `<*>` of a template (the `index`-th in the template text, from 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placeholder {
    pub index: usize,
    /// The most frequent values, most frequent first.
    pub values: Vec<PlaceholderValue>,
    /// Events that filled this placeholder.
    pub total: u64,
    /// Whether `values` lists every distinct value with exact counts (no value was ever dropped).
    pub exact: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Summary {
    counters: Vec<(Box<str>, u64)>,
    total: u64,
    evicted: bool,
}

fn clip(value: &str) -> &str {
    match value.char_indices().nth(MAX_VALUE_CHARS) {
        Some((i, _)) => &value[..i],
        None => value,
    }
}

impl Summary {
    /// A summary that already saw `count` events with `value` (a literal that just became a placeholder).
    pub(super) fn seeded(value: &str, count: u64) -> Summary {
        let mut s = Summary::default();
        s.add_many(value, count);
        s
    }

    pub(super) fn add(&mut self, value: &str) {
        self.add_many(value, 1);
    }

    fn add_many(&mut self, value: &str, n: u64) {
        let value = clip(value);
        self.total += n;
        if let Some(c) = self.counters.iter_mut().find(|(v, _)| &**v == value) {
            c.1 += n;
        } else if self.counters.len() < COUNTERS {
            self.counters.push((value.into(), n));
        } else if let Some(min) = self.counters.iter_mut().min_by_key(|(_, c)| *c) {
            // Space-Saving: the smallest counter is taken over by the new value and keeps its count.
            *min = (value.into(), min.1 + n);
            self.evicted = true;
        }
    }

    pub(super) fn finish(&self, index: usize) -> Placeholder {
        let mut values: Vec<PlaceholderValue> = self
            .counters
            .iter()
            .map(|(v, c)| PlaceholderValue {
                value: v.to_string(),
                count: *c,
            })
            .collect();
        values.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
        values.truncate(TOP_VALUES);
        Placeholder {
            index,
            values,
            total: self.total,
            exact: !self.evicted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_exact_while_the_values_fit() {
        let mut s = Summary::default();
        for v in ["a", "b", "a", "c", "a", "b"] {
            s.add(v);
        }
        let p = s.finish(2);
        assert_eq!(p.index, 2);
        assert_eq!((p.total, p.exact), (6, true));
        assert_eq!(
            p.values.iter().map(|v| (v.value.as_str(), v.count)).collect::<Vec<_>>(),
            [("a", 3), ("b", 2), ("c", 1)]
        );
    }

    #[test]
    fn heavy_hitters_survive_a_flood_of_distinct_values() {
        let mut s = Summary::default();
        for i in 0..2_000 {
            let other = format!("id-{i}");
            s.add(if i % 2 == 0 { "hot" } else { &other });
        }
        let p = s.finish(0);
        assert!(!p.exact);
        assert_eq!(p.values[0].value, "hot");
        assert!(p.values[0].count >= 1_000, "{:?}", p.values[0]);
        assert!(p.values.len() <= TOP_VALUES);
        assert_eq!(p.total, 2_000);
    }

    #[test]
    fn seeding_and_clipping() {
        let mut s = Summary::seeded("old", 5);
        s.add("new");
        let p = s.finish(0);
        assert_eq!((p.values[0].value.as_str(), p.values[0].count, p.total), ("old", 5, 6));
        let long = "é".repeat(MAX_VALUE_CHARS + 30);
        let mut s = Summary::default();
        s.add(&long);
        assert_eq!(s.finish(0).values[0].value.chars().count(), MAX_VALUE_CHARS);
    }
}
