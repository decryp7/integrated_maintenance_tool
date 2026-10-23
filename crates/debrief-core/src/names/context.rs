//! Context, like a person has it: what the user is looking at makes some candidates more likely than others.
//!
//! [`SearchContext`] holds the selected time window, the open sources, the text of the event in the Inspector, the
//! names around it and the recent searches. [`relevance`] scores a group 0..1 by how many of the present signals
//! support it. [`Cooccurrence`] counts which names appear in the same events (or the same process or path family):
//! it joins spellings of one thing and breaks ties between two candidates by their closeness to the user's anchors.
//! Bounded: [`MAX_RECENT`], [`MAX_ANCHORS`], [`MAX_PAIRS`].

use super::group::NameGroup;
use super::resolve::squash;
use std::collections::HashMap;

/// Most recent searches remembered.
pub const MAX_RECENT: usize = 20;
/// Most anchor names used.
pub const MAX_ANCHORS: usize = 32;
/// Most name pairs counted.
pub const MAX_PAIRS: usize = 100_000;
/// Longest inspected text read (characters).
pub const MAX_INSPECTED_CHARS: usize = 8192;

/// What the user is looking at.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchContext {
    /// The selected time window (inclusive), when one is set.
    pub window: Option<(i64, i64)>,
    /// Names of the open or selected sources.
    pub sources: Vec<String>,
    /// The message (and fields) of the event in the Inspector.
    pub inspected: Option<String>,
    /// Names that appear around the user's work (the entities of the inspected event, earlier answers).
    pub anchors: Vec<String>,
    /// The latest searches, newest last.
    pub recent: Vec<String>,
}

impl SearchContext {
    /// Notes a search, keeping the newest [`MAX_RECENT`].
    pub fn remember(&mut self, text: &str) {
        let t = text.trim();
        if t.is_empty() {
            return;
        }
        self.recent.retain(|r| r != t);
        self.recent.push(t.to_string());
        if self.recent.len() > MAX_RECENT {
            self.recent.remove(0);
        }
    }

    /// Whether any signal is set.
    pub fn is_empty(&self) -> bool {
        self.window.is_none() && self.sources.is_empty() && self.inspected.is_none() && self.anchors.is_empty() && self.recent.is_empty()
    }
}

/// How well the context supports `g`, 0..1 (0 when no signal is set), with a short phrase for each signal that did.
pub fn relevance(g: &NameGroup, ctx: &SearchContext) -> (f64, Vec<&'static str>) {
    let mut present = 0u32;
    let mut hits = 0u32;
    let mut reasons = Vec::new();
    if let Some((from, to)) = ctx.window {
        present += 1;
        if let (Some(a), Some(b)) = (g.first_ts, g.last_ts) {
            if a <= to && b >= from {
                hits += 1;
                reasons.push("active in the selected time window");
            }
        }
    }
    if !ctx.sources.is_empty() {
        present += 1;
        if g.seen_in.iter().any(|s| ctx.sources.contains(s)) {
            hits += 1;
            reasons.push("seen in the open source");
        }
    }
    if let Some(text) = &ctx.inspected {
        present += 1;
        let t: String = squash(&text.chars().take(MAX_INSPECTED_CHARS).collect::<String>());
        if g.spellings.iter().any(|s| t.contains(&squash(&s.name))) || t.contains(&squash(&g.key)) {
            hits += 1;
            reasons.push("mentioned in the event you are inspecting");
        }
    }
    if !ctx.recent.is_empty() {
        present += 1;
        let key = squash(&g.key);
        if ctx.recent.iter().any(|r| {
            let r = squash(r);
            !r.is_empty() && (key.contains(&r) || r.contains(&key))
        }) {
            hits += 1;
            reasons.push("like a recent search");
        }
    }
    if present == 0 {
        return (0.0, reasons);
    }
    (f64::from(hits) / f64::from(present), reasons)
}

/// Which names appear in the same events.
#[derive(Debug, Clone, Default)]
pub struct Cooccurrence {
    singles: HashMap<String, u32>,
    pairs: HashMap<(String, String), u32>,
}

impl Cooccurrence {
    /// Counts the names of one event (each once; case-insensitive; at most 64 are used).
    pub fn add_event(&mut self, names: &[String]) {
        let mut v: Vec<String> = names.iter().take(64).map(|n| n.to_lowercase()).collect();
        v.sort();
        v.dedup();
        for (i, a) in v.iter().enumerate() {
            *self.singles.entry(a.clone()).or_default() += 1;
            for b in &v[i + 1..] {
                if self.pairs.len() < MAX_PAIRS || self.pairs.contains_key(&(a.clone(), b.clone())) {
                    *self.pairs.entry((a.clone(), b.clone())).or_default() += 1;
                }
            }
        }
    }

    /// Events with both names, as a share of the events of the rarer one: 0..1 (0 for unknown names).
    pub fn strength(&self, a: &str, b: &str) -> f64 {
        let (a, b) = (a.to_lowercase(), b.to_lowercase());
        if a == b {
            return 0.0;
        }
        let key = if a < b { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) };
        let both = f64::from(self.pairs.get(&key).copied().unwrap_or(0));
        let rarer = f64::from(self.singles.get(&a).copied().unwrap_or(0).min(self.singles.get(&b).copied().unwrap_or(0)));
        if rarer == 0.0 {
            0.0
        } else {
            both / rarer
        }
    }

    /// The strongest closeness of any spelling of `g` to any of `anchors`, with the anchor that gave it.
    pub fn closeness<'a>(&self, g: &NameGroup, anchors: &'a [String]) -> Option<(f64, &'a str)> {
        let mut best: Option<(f64, &str)> = None;
        for a in anchors.iter().take(MAX_ANCHORS) {
            for s in &g.spellings {
                let st = self.strength(&s.name, a);
                if st > 0.0 && best.is_none_or(|b| st > b.0) {
                    best = Some((st, a.as_str()));
                }
            }
        }
        best
    }

    pub fn is_empty(&self) -> bool {
        self.singles.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Scope;
    use crate::entities::EntityKind;
    use crate::names::group::Spelling;

    fn grp(name: &str, ts: (i64, i64), src: &str) -> NameGroup {
        NameGroup {
            scope: Scope::Entity(EntityKind::Executable),
            key: name.into(),
            display: name.into(),
            spellings: vec![Spelling {
                name: format!("{name}.exe"),
                count: 1,
            }],
            count: 1,
            first_ts: Some(ts.0),
            last_ts: Some(ts.1),
            seen_in: vec![src.into()],
        }
    }

    #[test]
    fn empty_context_scores_zero() {
        let c = SearchContext::default();
        assert!(c.is_empty());
        assert_eq!(relevance(&grp("a", (1, 2), "x"), &c), (0.0, vec![]));
    }

    #[test]
    fn signals_rank_candidates() {
        let mut c = SearchContext {
            window: Some((100, 200)),
            sources: vec!["app.log".into()],
            inspected: Some("crash in Tray Agent.exe at 10:00".into()),
            ..SearchContext::default()
        };
        c.remember("tray agent");
        let near = grp("tray agent", (150, 160), "app.log");
        let far = grp("graphic builder", (900, 950), "other.log");
        let (a, why) = relevance(&near, &c);
        let (b, _) = relevance(&far, &c);
        assert!(a > 0.9 && b < 0.1, "{a} {b}");
        assert_eq!(why.len(), 4);
        assert!(!c.is_empty());
    }

    #[test]
    fn remember_is_bounded_and_deduplicated() {
        let mut c = SearchContext::default();
        for i in 0..40 {
            c.remember(&format!("q{i}"));
        }
        c.remember("q39");
        c.remember("  ");
        assert_eq!(c.recent.len(), MAX_RECENT);
        assert_eq!(c.recent.last().unwrap(), "q39");
    }

    #[test]
    fn cooccurrence_strength_and_closeness() {
        let mut co = Cooccurrence::default();
        assert!(co.is_empty());
        let ev = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        co.add_event(&ev(&["A.exe", "b.exe", "A.exe"]));
        co.add_event(&ev(&["a.exe", "b.exe"]));
        co.add_event(&ev(&["a.exe", "c.exe"]));
        assert!((co.strength("a.exe", "B.EXE") - 1.0).abs() < 1e-9);
        assert!((co.strength("a.exe", "c.exe") - 1.0).abs() < 1e-9);
        assert_eq!(co.strength("a.exe", "a.exe"), 0.0);
        assert_eq!(co.strength("a.exe", "zzz"), 0.0);
        let g = grp("b", (1, 1), "x");
        let anchors = vec!["a.exe".to_string(), "q".to_string()];
        assert_eq!(co.closeness(&g, &anchors).map(|c| c.1), Some("a.exe"));
        assert_eq!(co.closeness(&grp("zz", (1, 1), "x"), &anchors), None);
    }
}
