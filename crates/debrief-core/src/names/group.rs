//! Grouping the spellings of one name into one entity with variants.
//!
//! `CompanyA.Graphic.Builder.exe`, `graphicbuilder.exe` and `GraphicBuilder.exe` become one [`NameGroup`] whose
//! canonical spelling is the most frequent one and whose variants carry their own counts. Names group by their
//! normalised words (see [`super::normalise`]) within a scope (an entity kind or a field). A group whose first word is
//! unknown (a vendor) also joins the group that is the same name without it. Names that co-occur in the same events
//! and look alike can be joined too ([`merge_cooccurring`]).

use super::context::Cooccurrence;
use super::normalise::{normalise, vendor_prefixes};
use super::segment::Vocabulary;
use crate::catalogue::{Candidate, Scope};
use crate::entities::EntityKind;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Most spellings kept per group.
pub const MAX_SPELLINGS: usize = 24;

/// One way the logs wrote the name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spelling {
    pub name: String,
    pub count: u64,
}

/// One entity: all the spellings that mean the same thing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameGroup {
    pub scope: Scope,
    /// The normalised words joined by a space: the group's identity (`graphic builder`).
    pub key: String,
    /// Title-cased words: `Graphic Builder`.
    pub display: String,
    /// The most frequent spelling first.
    pub spellings: Vec<Spelling>,
    /// Events with any spelling.
    pub count: u64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub seen_in: Vec<String>,
}

impl NameGroup {
    /// The most frequent spelling.
    pub fn canonical(&self) -> &str {
        self.spellings.first().map(|s| s.name.as_str()).unwrap_or(&self.display)
    }

    /// Stable id of the group within its scope.
    pub fn id(&self) -> String {
        format!("{}|{}", self.scope.encode(), self.key)
    }

    /// What the group is, for words: the entity kind's id or the field name.
    pub fn kind_label(&self) -> String {
        match &self.scope {
            Scope::Entity(k) => k.id().to_string(),
            Scope::Field(f) => f.clone(),
        }
    }

    pub(super) fn absorb(&mut self, other: NameGroup) {
        self.count += other.count;
        self.first_ts = min_opt(self.first_ts, other.first_ts);
        self.last_ts = max_opt(self.last_ts, other.last_ts);
        for s in other.seen_in {
            if !self.seen_in.contains(&s) {
                self.seen_in.push(s);
            }
        }
        for s in other.spellings {
            match self.spellings.iter_mut().find(|x| x.name == s.name) {
                Some(x) => x.count += s.count,
                None => self.spellings.push(s),
            }
        }
        self.finish();
    }

    fn finish(&mut self) {
        self.spellings
            .sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        self.spellings.truncate(MAX_SPELLINGS);
        self.seen_in.sort();
        self.seen_in.truncate(crate::catalogue::MAX_SEEN_IN);
    }
}

fn min_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) | (None, x) => x,
    }
}

fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) | (None, x) => x,
    }
}

/// Whether names of this scope are grouped (names for things; not addresses, URLs, ids or codes).
pub fn groupable(scope: &Scope) -> bool {
    match scope {
        Scope::Entity(k) => matches!(
            k,
            EntityKind::Executable | EntityKind::Library | EntityKind::File | EntityKind::Exception
        ),
        Scope::Field(_) => true,
    }
}

/// Groups the groupable candidates. Deterministic: most events first, then by key.
pub fn group(candidates: &[Candidate], vocab: &Vocabulary) -> Vec<NameGroup> {
    let usable: Vec<&Candidate> = candidates.iter().filter(|c| groupable(&c.scope)).collect();
    let vendors = vendor_prefixes(usable.iter().map(|c| c.name.as_str()));
    let mut by_key: BTreeMap<(Scope, String), NameGroup> = BTreeMap::new();
    for c in usable {
        let n = normalise(&c.name, vocab, &vendors);
        let key = n.key();
        if key.is_empty() {
            continue;
        }
        let g = NameGroup {
            scope: c.scope.clone(),
            key: key.clone(),
            display: n.display,
            spellings: vec![Spelling {
                name: c.name.clone(),
                count: c.count,
            }],
            count: c.count,
            first_ts: c.first_ts,
            last_ts: c.last_ts,
            seen_in: c.seen_in.clone(),
        };
        match by_key.get_mut(&(c.scope.clone(), key.clone())) {
            Some(existing) => existing.absorb(g),
            None => {
                by_key.insert((c.scope.clone(), key), g);
            }
        }
    }
    // A vendor that appears once (so was not detected) is an unknown first word: join the group without it.
    let keys: Vec<(Scope, String)> = by_key.keys().cloned().collect();
    for (scope, key) in keys {
        let words: Vec<&str> = key.split(' ').collect();
        // Cut one or two leading words that look like a vendor: unknown to the vocabulary, or a single letter.
        let target = (1..=2.min(words.len().saturating_sub(1)))
            .filter(|cut| words[..*cut].iter().any(|w| w.chars().count() == 1 || !vocab.is_common(w)))
            .filter(|cut| !words[..*cut].iter().any(|w| w.chars().any(|c| c.is_ascii_digit())))
            .map(|cut| words[cut..].join(" "))
            .find(|rest| by_key.contains_key(&(scope.clone(), rest.clone())));
        let Some(rest) = target else { continue };
        if let Some(g) = by_key.remove(&(scope.clone(), key)) {
            if let Some(t) = by_key.get_mut(&(scope, rest)) {
                t.absorb(g);
            }
        }
    }
    let mut v: Vec<NameGroup> = by_key.into_values().collect();
    for g in &mut v {
        g.finish();
        g.display = display_of(&g.key);
    }
    v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    v
}

fn display_of(key: &str) -> String {
    key.split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Joins groups of one scope whose names look alike and that appear in the same events: if at least
/// `min_strength` of the rarer one's events also hold the other, and their letters share at least half of their
/// trigrams. Returns the number of merges. Deterministic.
pub fn merge_cooccurring(groups: &mut Vec<NameGroup>, cooc: &Cooccurrence, min_strength: f64) -> usize {
    let mut merged = 0;
    let mut i = 0;
    while i < groups.len() {
        let mut j = i + 1;
        while j < groups.len() {
            let same_scope = groups[i].scope == groups[j].scope;
            if same_scope && similar(&groups[i].key, &groups[j].key) && together(&groups[i], &groups[j], cooc, min_strength) {
                let g = groups.remove(j);
                groups[i].absorb(g);
                merged += 1;
            } else {
                j += 1;
            }
        }
        i += 1;
    }
    groups.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    merged
}

/// Positions of the groups among the first `limit` (most events first) that have a look-alike of their scope in the
/// same range: the ones worth reading events for before [`merge_cooccurring`].
pub fn look_alikes(groups: &[NameGroup], limit: usize) -> Vec<usize> {
    let top = &groups[..groups.len().min(limit)];
    let mut out = Vec::new();
    for (i, a) in top.iter().enumerate() {
        if top
            .iter()
            .enumerate()
            .any(|(j, b)| i != j && a.scope == b.scope && similar(&a.key, &b.key))
        {
            out.push(i);
        }
    }
    out
}

fn together(a: &NameGroup, b: &NameGroup, cooc: &Cooccurrence, min: f64) -> bool {
    a.spellings
        .iter()
        .any(|x| b.spellings.iter().any(|y| cooc.strength(&x.name, &y.name) >= min))
}

fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (super::resolve::squash(a), super::resolve::squash(b));
    let ta = trigram_set(&a);
    let tb = trigram_set(&b);
    if ta.is_empty() || tb.is_empty() {
        return a == b;
    }
    let common = ta.iter().filter(|t| tb.contains(*t)).count();
    common * 2 >= ta.len().min(tb.len())
}

fn trigram_set(s: &str) -> std::collections::HashSet<[char; 3]> {
    let c: Vec<char> = s.chars().collect();
    c.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
}

/// Index from lower-cased spelling to the position of its group in `groups`.
pub fn spelling_index(groups: &[NameGroup]) -> HashMap<(Scope, String), usize> {
    let mut m = HashMap::new();
    for (i, g) in groups.iter().enumerate() {
        for s in &g.spellings {
            m.insert((g.scope.clone(), s.name.to_lowercase()), i);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(name: &str, n: u64) -> Candidate {
        Candidate {
            scope: Scope::Entity(EntityKind::Executable),
            name: name.into(),
            count: n,
            first_ts: Some(n as i64),
            last_ts: Some(100 + n as i64),
            seen_in: vec![format!("s{n}.log")],
        }
    }

    #[test]
    fn absorbing_a_group_adds_the_counts_of_a_shared_spelling() {
        let mut a = group(&[cand("graphicbuilder.exe", 4)], &Vocabulary::builtin()).remove(0);
        let b = group(
            &[cand("graphicbuilder.exe", 6), cand("Graphic.Builder.exe", 1)],
            &Vocabulary::builtin(),
        )
        .remove(0);
        a.absorb(b);
        assert_eq!(a.spellings[0].count, 10);
        assert_eq!(a.spellings.len(), 2);
        assert_eq!(a.count, 11);
    }

    #[test]
    fn a_field_group_is_named_by_its_field() {
        let mut c = cand("alpha", 1);
        c.scope = Scope::Field("host".into());
        let g = group(&[c], &Vocabulary::builtin());
        assert_eq!(g[0].kind_label(), "host");
    }

    #[test]
    fn very_short_names_are_alike_only_when_equal() {
        assert!(similar("ab", "ab"));
        assert!(!similar("ab", "ba"));
    }

    #[test]
    fn spellings_group_into_one_entity() {
        let c = [
            cand("CompanyA.Graphic.Builder.exe", 5),
            cand("graphicbuilder.exe", 12),
            cand("GraphicBuilder.exe", 3),
            cand("graphic-builder", 1),
            cand("CompanyA.Tray.Agent.exe", 7),
            cand("other.exe", 2),
        ];
        let g = group(&c, &Vocabulary::builtin());
        let gb = g.iter().find(|g| g.key == "graphic builder").unwrap();
        assert_eq!(gb.count, 21);
        assert_eq!(gb.canonical(), "graphicbuilder.exe");
        assert_eq!(gb.spellings.len(), 4);
        assert_eq!(gb.display, "Graphic Builder");
        assert_eq!(gb.first_ts, Some(1));
        assert_eq!(gb.last_ts, Some(112));
        assert_eq!(gb.seen_in.len(), 4);
        assert_eq!(gb.id(), "executable|graphic builder");
        assert_eq!(gb.kind_label(), "executable");
        assert!(g.iter().any(|g| g.key == "tray agent"));
        assert_eq!(g.len(), 3);
    }

    #[test]
    fn single_vendor_prefix_joins_the_group_without_it() {
        let c = [cand("CompanyA.Graphic.Builder.exe", 5), cand("graphicbuilder.exe", 12)];
        let g = group(&c, &Vocabulary::builtin());
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].spellings.len(), 2);
        // A known first word is not a vendor.
        let c = [cand("Graphic.Builder.Core.exe", 5), cand("builder.core.exe", 12)];
        assert_eq!(group(&c, &Vocabulary::builtin()).len(), 2);
    }

    #[test]
    fn kinds_stay_apart_and_ids_are_not_grouped() {
        let mut dll = cand("graphicbuilder.dll", 4);
        dll.scope = Scope::Entity(EntityKind::Library);
        let mut ip = cand("10.0.0.1", 9);
        ip.scope = Scope::Entity(EntityKind::Ip);
        let g = group(&[cand("graphicbuilder.exe", 1), dll, ip, cand("....", 3)], &Vocabulary::builtin());
        assert_eq!(g.len(), 2);
        assert!(!groupable(&Scope::Entity(EntityKind::Guid)));
        assert!(groupable(&Scope::Field("ProcessName".into())));
    }

    #[test]
    fn cooccurring_lookalikes_merge() {
        let c = [cand("gfxbuilder.exe", 5), cand("gfx-bilder.exe", 4), cand("tray.exe", 3)];
        let mut g = group(&c, &Vocabulary::builtin());
        assert_eq!(g.len(), 3);
        let mut cooc = Cooccurrence::default();
        cooc.add_event(&["gfxbuilder.exe".to_string(), "gfx-bilder.exe".to_string(), "tray.exe".to_string()]);
        cooc.add_event(&["gfxbuilder.exe".to_string(), "gfx-bilder.exe".to_string()]);
        assert_eq!(merge_cooccurring(&mut g, &cooc, 0.8), 1);
        assert_eq!(g.len(), 2);
        // Unrelated names are never joined, however often they co-occur.
        assert_eq!(merge_cooccurring(&mut g, &cooc, 0.8), 0);
        let idx = spelling_index(&g);
        assert_eq!(idx.len(), 3);
    }
}
