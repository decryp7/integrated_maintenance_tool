//! The name book: the entity table (grouped spellings) of a workspace plus what was learned about the names.
//!
//! Two kinds of learned knowledge, kept apart so one never overwrites the other:
//!
//! - *AI aliases* ([`AiEntry`]): a human name and aliases a local model suggested for a group, cached with the model's
//!   version (see [`super::alias_job`]). The user can remove single aliases; a removed alias is not suggested again.
//! - *User facts* ([`UserFact`]): the user's own confirmations and corrections (an accepted or rejected match, an
//!   alias, a merge, a split, a rename). They outrank AI aliases in matching and are never touched by the alias
//!   job. Each can be undone ([`NameBook::undo`], [`NameBook::remove_fact`]) and is kept for this workspace or
//!   globally ([`UserFact::global`]; the caller stores global facts in one file and passes them to
//!   [`NameBook::adopt_global`]).
//!
//! The book is a versioned JSON file; a missing or corrupt file falls back to an empty book.

use super::group::{NameGroup, Spelling};
use crate::catalogue::Scope;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// File format version.
pub const BOOK_VERSION: u32 = 1;
/// Most user facts kept.
pub const MAX_FACTS: usize = 10_000;
/// Longest user-written term or name (characters).
pub const MAX_TERM_CHARS: usize = 128;

/// A model's suggestion for one group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiEntry {
    /// The model and version that produced it; a different model makes the entry stale.
    pub model: String,
    /// The human name (`Service Host`).
    pub human: String,
    pub aliases: Vec<String>,
}

/// What the user did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FactKind {
    /// `term` means the group (an accepted "Did you mean", or a typed alias).
    Alias { group: String, term: String },
    /// `term` does not mean the group (a rejected match).
    Reject { group: String, term: String },
    /// The two groups are one thing.
    Merge { a: String, b: String },
    /// `spelling` is not part of `group`.
    Split { group: String, spelling: String },
    /// The group is called `name`.
    Rename { group: String, name: String },
}

/// One thing the user told us, with an id for undoing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserFact {
    pub id: u64,
    pub kind: FactKind,
    /// Kept for every workspace, not only this one.
    pub global: bool,
}

/// The entity table with its learned names.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameBook {
    pub version: u32,
    base: Vec<NameGroup>,
    ai: BTreeMap<String, AiEntry>,
    /// `group id` + `\t` + alias of AI aliases the user removed.
    removed_ai: BTreeSet<String>,
    facts: Vec<UserFact>,
    next_id: u64,
    #[serde(skip)]
    view: Vec<NameGroup>,
    #[serde(skip)]
    prepared: Vec<Prepared>,
}

/// A group's matching keys, computed once per refresh so a search does no string building per group.
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    /// The group's id.
    pub id: String,
    /// The words of the key.
    pub tokens: Vec<String>,
    /// The words joined without separators.
    pub joined: String,
    /// Each spelling squashed to letters and digits.
    pub spellings: Vec<String>,
}

impl Default for NameBook {
    fn default() -> Self {
        NameBook {
            version: BOOK_VERSION,
            base: Vec::new(),
            ai: BTreeMap::new(),
            removed_ai: BTreeSet::new(),
            facts: Vec::new(),
            next_id: 1,
            view: Vec::new(),
            prepared: Vec::new(),
        }
    }
}

fn clean(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty() && t.chars().count() <= MAX_TERM_CHARS && !t.chars().any(char::is_control)).then(|| t.to_string())
}

impl NameBook {
    /// A book over the groups of the current catalogue; AI aliases and user facts are kept.
    pub fn rebuild(&mut self, groups: Vec<NameGroup>) {
        self.base = groups;
        self.refresh();
    }

    /// The groups with the user's merges, splits and renames applied.
    pub fn groups(&self) -> &[NameGroup] {
        &self.view
    }

    /// The group with `id`.
    pub fn group(&self, id: &str) -> Option<&NameGroup> {
        self.view.iter().find(|g| g.id() == id)
    }

    fn refresh(&mut self) {
        let mut v = self.base.clone();
        for f in &self.facts {
            match &f.kind {
                FactKind::Merge { a, b } => {
                    let (Some(ia), Some(ib)) = (v.iter().position(|g| &g.id() == a), v.iter().position(|g| &g.id() == b)) else {
                        continue;
                    };
                    if ia != ib {
                        let other = v.remove(ib);
                        let ia = if ib < ia { ia - 1 } else { ia };
                        v[ia].absorb(other);
                    }
                }
                FactKind::Split { group, spelling } => {
                    let Some(i) = v.iter().position(|g| &g.id() == group) else {
                        continue;
                    };
                    if v[i].spellings.len() < 2 {
                        continue;
                    }
                    let Some(p) = v[i]
                        .spellings
                        .iter()
                        .position(|s| &s.name == spelling)
                        .or_else(|| v[i].spellings.iter().position(|s| s.name.eq_ignore_ascii_case(spelling)))
                    else {
                        continue;
                    };
                    let s = v[i].spellings.remove(p);
                    v[i].count = v[i].count.saturating_sub(s.count);
                    let g = NameGroup {
                        scope: v[i].scope.clone(),
                        key: s.name.to_lowercase(),
                        display: s.name.clone(),
                        count: s.count,
                        first_ts: v[i].first_ts,
                        last_ts: v[i].last_ts,
                        seen_in: v[i].seen_in.clone(),
                        spellings: vec![Spelling {
                            name: s.name,
                            count: s.count,
                        }],
                    };
                    v.push(g);
                }
                FactKind::Rename { group, name } => {
                    if let Some(g) = v.iter_mut().find(|g| &g.id() == group) {
                        g.display = name.clone();
                    }
                }
                _ => {}
            }
        }
        self.prepared = v
            .iter()
            .map(|g| Prepared {
                id: g.id(),
                tokens: g.key.split(' ').map(String::from).collect(),
                joined: super::resolve::squash(&g.key),
                spellings: g.spellings.iter().map(|s| super::resolve::squash(&s.name)).collect(),
            })
            .collect();
        self.view = v;
    }

    /// The matching keys of [`NameBook::groups`], in the same order.
    pub fn prepared(&self) -> &[Prepared] {
        &self.prepared
    }

    fn push(&mut self, kind: FactKind, global: bool) -> Option<u64> {
        if self.facts.len() >= MAX_FACTS {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.facts.push(UserFact { id, kind, global });
        self.refresh();
        Some(id)
    }

    /// The user accepted a match or typed an alias: `term` means `group`. Replaces an earlier rejection of the pair.
    pub fn accept(&mut self, term: &str, group: &str, global: bool) -> Option<u64> {
        let term = clean(term)?;
        let key = term.to_lowercase();
        self.facts
            .retain(|f| !matches!(&f.kind, FactKind::Reject { group: g, term: t } if g == group && t.to_lowercase() == key));
        self.push(FactKind::Alias { group: group.into(), term }, global)
    }

    /// The user rejected a match: `term` does not mean `group`. Replaces an earlier acceptance of the pair.
    pub fn reject(&mut self, term: &str, group: &str, global: bool) -> Option<u64> {
        let term = clean(term)?;
        let key = term.to_lowercase();
        self.facts
            .retain(|f| !matches!(&f.kind, FactKind::Alias { group: g, term: t } if g == group && t.to_lowercase() == key));
        self.push(FactKind::Reject { group: group.into(), term }, global)
    }

    /// The two groups are one thing.
    pub fn merge(&mut self, a: &str, b: &str, global: bool) -> Option<u64> {
        (a != b)
            .then(|| self.push(FactKind::Merge { a: a.into(), b: b.into() }, global))
            .flatten()
    }

    /// `spelling` is not part of `group`.
    pub fn split(&mut self, group: &str, spelling: &str, global: bool) -> Option<u64> {
        let spelling = clean(spelling)?;
        self.push(
            FactKind::Split {
                group: group.into(),
                spelling,
            },
            global,
        )
    }

    /// The group is called `name`.
    pub fn rename(&mut self, group: &str, name: &str, global: bool) -> Option<u64> {
        let name = clean(name)?;
        self.push(FactKind::Rename { group: group.into(), name }, global)
    }

    /// Takes back the fact with `id`.
    pub fn remove_fact(&mut self, id: u64) -> bool {
        let n = self.facts.len();
        self.facts.retain(|f| f.id != id);
        let removed = self.facts.len() != n;
        if removed {
            self.refresh();
        }
        removed
    }

    /// Takes back the newest fact.
    pub fn undo(&mut self) -> Option<UserFact> {
        let f = self.facts.pop()?;
        self.refresh();
        Some(f)
    }

    pub fn facts(&self) -> &[UserFact] {
        &self.facts
    }

    /// The terms the user said mean `group` (accepted aliases and renames), with the fact id.
    pub fn user_terms(&self, group: &str) -> Vec<(String, u64)> {
        self.facts
            .iter()
            .filter_map(|f| match &f.kind {
                FactKind::Alias { group: g, term } if g == group => Some((term.clone(), f.id)),
                FactKind::Rename { group: g, name } if g == group => Some((name.clone(), f.id)),
                _ => None,
            })
            .collect()
    }

    /// Whether any rejection exists (lets a search skip the per-group check).
    pub fn has_rejections(&self) -> bool {
        self.facts.iter().any(|f| matches!(f.kind, FactKind::Reject { .. }))
    }

    /// Whether any alias or rename exists.
    pub fn has_user_terms(&self) -> bool {
        self.facts
            .iter()
            .any(|f| matches!(f.kind, FactKind::Alias { .. } | FactKind::Rename { .. }))
    }

    /// Whether the user said `term` does not mean `group`.
    pub fn is_rejected(&self, term: &str, group: &str) -> bool {
        let key = super::resolve::squash(term);
        self.facts
            .iter()
            .any(|f| matches!(&f.kind, FactKind::Reject { group: g, term: t } if g == group && super::resolve::squash(t) == key))
    }

    /// The facts the user wants everywhere.
    pub fn global_facts(&self) -> Vec<UserFact> {
        self.facts.iter().filter(|f| f.global).cloned().collect()
    }

    /// Adds global facts from another workspace that this book does not hold yet (same kind), with new ids.
    pub fn adopt_global(&mut self, facts: &[UserFact]) -> usize {
        let mut n = 0;
        for f in facts {
            if !self.facts.iter().any(|x| x.kind == f.kind) && self.push(f.kind.clone(), true).is_some() {
                n += 1;
            }
        }
        n
    }

    // AI aliases ---------------------------------------------------------------------------------------------------

    /// Whether `group` has an entry made by `model` (nothing to ask for).
    pub fn ai_current(&self, group: &str, model: &str) -> bool {
        self.ai.get(group).is_some_and(|e| e.model == model)
    }

    /// Stores the model's entry for `group`, dropping aliases the user removed earlier.
    pub fn set_ai(&mut self, group: &str, mut entry: AiEntry) {
        entry
            .aliases
            .retain(|a| !self.removed_ai.contains(&format!("{group}\t{}", a.to_lowercase())));
        self.ai.insert(group.to_string(), entry);
    }

    /// The model's human name and aliases for `group`, minus removed ones.
    pub fn ai(&self, group: &str) -> Option<&AiEntry> {
        self.ai.get(group)
    }

    /// The user removes one AI alias (or the human name when it equals `alias`); it is not suggested again.
    pub fn remove_ai_alias(&mut self, group: &str, alias: &str) -> bool {
        let Some(e) = self.ai.get_mut(group) else { return false };
        let key = alias.to_lowercase();
        let before = e.aliases.len();
        e.aliases.retain(|a| a.to_lowercase() != key);
        let mut changed = e.aliases.len() != before;
        if e.human.to_lowercase() == key {
            e.human.clear();
            changed = true;
        }
        if changed {
            self.removed_ai.insert(format!("{group}\t{key}"));
        }
        changed
    }

    /// Forgets every AI entry (the feature was turned off, or its data should go).
    pub fn clear_ai(&mut self) {
        self.ai.clear();
    }

    /// Number of groups with an AI entry.
    pub fn ai_len(&self) -> usize {
        self.ai.len()
    }

    // Files --------------------------------------------------------------------------------------------------------

    /// Reads a book; a missing, unreadable or newer-version file gives an empty book and, when the file exists, a note.
    pub fn load(path: &Path) -> (NameBook, Option<String>) {
        let Ok(bytes) = std::fs::read(path) else {
            return (NameBook::default(), None);
        };
        match serde_json::from_slice::<NameBook>(&bytes) {
            Ok(mut b) if b.version == BOOK_VERSION => {
                b.refresh();
                (b, None)
            }
            Ok(_) => (
                NameBook::default(),
                Some("the name book was written by another version; starting empty".into()),
            ),
            Err(e) => (
                NameBook::default(),
                Some(format!("the name book is unreadable ({e}); starting empty")),
            ),
        }
    }

    /// Writes the book atomically.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }

    /// The scope of `group` and whether it can have AI aliases (entities of any kind, not field values).
    pub fn wants_ai(g: &NameGroup) -> bool {
        matches!(g.scope, Scope::Entity(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::EntityKind;

    fn g(key: &str, spellings: &[(&str, u64)]) -> NameGroup {
        NameGroup {
            scope: Scope::Entity(EntityKind::Executable),
            key: key.into(),
            display: key.into(),
            spellings: spellings
                .iter()
                .map(|(n, c)| Spelling {
                    name: n.to_string(),
                    count: *c,
                })
                .collect(),
            count: spellings.iter().map(|s| s.1).sum(),
            first_ts: None,
            last_ts: None,
            seen_in: vec![],
        }
    }

    fn book() -> NameBook {
        let mut b = NameBook::default();
        b.rebuild(vec![g("a", &[("a.exe", 5), ("A.exe", 2)]), g("b", &[("b.exe", 3)])]);
        b
    }

    const A: &str = "executable|a";
    const B: &str = "executable|b";

    #[test]
    fn merging_the_later_group_into_the_earlier_and_the_other_way_round_agree() {
        let mut b = book();
        b.merge(B, A, false);
        let groups = b.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 10);
        assert_eq!(groups[0].spellings.len(), 3);
    }

    #[test]
    fn splitting_off_a_spelling_the_group_does_not_have_changes_nothing() {
        let mut b = book();
        b.split(A, "nope.exe", false);
        assert_eq!(b.groups().len(), 2);
        assert_eq!(b.groups()[0].spellings.len(), 2);
    }

    #[test]
    fn the_book_stops_taking_facts_at_its_limit() {
        let mut b = book();
        b.facts = (0..MAX_FACTS as u64)
            .map(|id| UserFact {
                id,
                kind: FactKind::Merge { a: A.into(), b: A.into() },
                global: false,
            })
            .collect();
        assert_eq!(b.accept("anything", A, false), None);
        assert_eq!(b.facts.len(), MAX_FACTS);
    }

    #[test]
    fn accept_reject_replace_each_other_and_undo() {
        let mut b = book();
        let id = b.accept(" gfx ", A, false).unwrap();
        assert_eq!(b.user_terms(A), vec![("gfx".to_string(), id)]);
        assert!(!b.is_rejected("GFX", A));
        b.reject("gfx", A, false).unwrap();
        assert!(b.user_terms(A).is_empty());
        assert!(b.is_rejected("G-F-X", A));
        b.accept("GFX", A, true).unwrap();
        assert!(!b.is_rejected("gfx", A));
        assert_eq!(b.global_facts().len(), 1);
        assert!(b.undo().is_some());
        assert!(b.facts().is_empty());
        assert!(b.undo().is_none());
        assert!(b.accept("   ", A, false).is_none());
        assert!(b.accept(&"x".repeat(200), A, false).is_none());
        assert!(b.accept("a\u{7}b", A, false).is_none());
    }

    #[test]
    fn merge_split_rename_change_the_view_and_undo() {
        let mut b = book();
        let m = b.merge(A, B, false).unwrap();
        assert_eq!(b.groups().len(), 1);
        assert_eq!(b.groups()[0].count, 10);
        assert!(b.merge(A, A, false).is_none());
        assert!(b.remove_fact(m));
        assert!(!b.remove_fact(m));
        assert_eq!(b.groups().len(), 2);
        b.split(A, "A.exe", false).unwrap();
        assert_eq!(b.groups().len(), 3);
        assert_eq!(b.group(A).unwrap().count, 5);
        // The last spelling cannot be split off.
        b.split(B, "b.exe", false).unwrap();
        assert_eq!(b.groups().len(), 3);
        // Facts naming groups that no longer exist are ignored.
        b.merge("executable|zz", A, false).unwrap();
        b.split("executable|zz", "q", false).unwrap();
        b.split(A, "nope.exe", false).unwrap();
        b.rename(A, "Alpha", false).unwrap();
        assert_eq!(b.group(A).unwrap().display, "Alpha");
        assert_eq!(b.user_terms(A)[0].0, "Alpha");
        assert_eq!(b.groups().len(), 3);
    }

    #[test]
    fn merge_with_earlier_index() {
        let mut b = book();
        b.merge(B, A, false).unwrap();
        assert_eq!(b.groups().len(), 1);
        assert_eq!(b.groups()[0].id(), B);
    }

    #[test]
    fn ai_entries_are_cached_by_model_and_removals_stick() {
        let mut b = book();
        assert!(!b.ai_current(A, "m1"));
        b.set_ai(
            A,
            AiEntry {
                model: "m1".into(),
                human: "Alpha".into(),
                aliases: vec!["alp".into(), "Aleph".into()],
            },
        );
        assert!(b.ai_current(A, "m1"));
        assert!(!b.ai_current(A, "m2"));
        assert!(b.remove_ai_alias(A, "ALP"));
        assert!(!b.remove_ai_alias(A, "ALP"));
        assert!(b.remove_ai_alias(A, "alpha"));
        assert!(!b.remove_ai_alias(B, "x"));
        b.set_ai(
            A,
            AiEntry {
                model: "m2".into(),
                human: "Alpha".into(),
                aliases: vec!["alp".into(), "Aleph".into()],
            },
        );
        assert_eq!(b.ai(A).unwrap().aliases, vec!["Aleph"]);
        assert_eq!(b.ai_len(), 1);
        b.clear_ai();
        assert!(b.ai(A).is_none());
    }

    #[test]
    fn global_facts_are_adopted_once() {
        let mut one = book();
        one.accept("gfx", A, true).unwrap();
        one.accept("local", A, false).unwrap();
        let mut two = book();
        assert_eq!(two.adopt_global(&one.global_facts()), 1);
        assert_eq!(two.adopt_global(&one.global_facts()), 0);
        assert!(two.facts()[0].global);
    }

    #[test]
    fn files_round_trip_and_bad_files_fall_back() {
        let dir = std::env::temp_dir().join(format!("debrief-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("book.json");
        let (empty, note) = NameBook::load(&p);
        assert!(empty.groups().is_empty() && note.is_none());
        let mut b = book();
        b.accept("gfx", A, false).unwrap();
        b.save(&p).unwrap();
        let (back, note) = NameBook::load(&p);
        assert!(note.is_none());
        assert_eq!(back.groups().len(), 2);
        assert_eq!(back.facts().len(), 1);
        std::fs::write(&p, b"{not json").unwrap();
        let (bad, note) = NameBook::load(&p);
        assert!(bad.groups().is_empty() && note.unwrap().contains("unreadable"));
        std::fs::write(
            &p,
            serde_json::to_vec(&NameBook {
                version: 99,
                ..NameBook::default()
            })
            .unwrap(),
        )
        .unwrap();
        assert!(NameBook::load(&p).1.unwrap().contains("another version"));
        std::fs::remove_dir_all(&dir).ok();
        assert!(NameBook::wants_ai(&g("a", &[("a", 1)])));
    }
}
