//! The catalogue: the names a store knows, kept so a lookup never scans events. Two kinds of names:
//!
//! - *field values*: the distinct values of low-cardinality fields (per field, capped at [`MAX_VALUES_PER_FIELD`]),
//! - *entities* ([`crate::entities`]): executables, files, paths, hosts, exception types... found in message and
//!   exception text.
//!
//! Each name has a count of events, first and last seen (time and event id). A store builds the catalogue per source
//! in the background after an import and keeps the rows next to the source: a source that is removed, cleared or
//! re-imported takes its rows with it, and the rows of a source are visible exactly when its events are, so a reload
//! keeps answering from the old catalogue until the new sources are ready. The store also keeps a trigram index over
//! the names ([`trigram_keys`]) for typo-tolerant lookups.
//!
//! Bounds (per source): at most [`MAX_NAMES`] distinct names ([`Tally`] drops new names when full and counts the
//! drops), at most [`MAX_GUIDS`] GUIDs listed (all are counted), [`MAX_VALUES_PER_FIELD`] values per field, fields
//! with more than [`MAX_FIELD_DISTINCT`] distinct values are not catalogued (ids, times), values and names at most
//! [`crate::entities::MAX_NAME_CHARS`] characters. Memory while building: the tally, about 200 bytes per name.

use crate::entities::{EntityKind, Extracted, MAX_NAME_CHARS};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

/// Most distinct names (entities and field values) a source keeps.
pub const MAX_NAMES: usize = 50_000;
/// Most GUIDs listed per source (the rest are only counted).
pub const MAX_GUIDS: usize = 200;
/// Most values of one field kept.
pub const MAX_VALUES_PER_FIELD: usize = 200;
/// A field with more distinct values than this is not catalogued.
pub const MAX_FIELD_DISTINCT: usize = 500;
/// Most events of a source whose text is read.
pub const MAX_EVENTS_SCANNED: usize = 5_000_000;
/// Most candidates a lookup returns before ranking.
pub const MAX_CANDIDATES: usize = 2_000;
/// Most source names listed with a candidate.
pub const MAX_SEEN_IN: usize = 5;

/// What a name is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Scope {
    Entity(EntityKind),
    /// A value of the field.
    Field(String),
}

impl Scope {
    /// The stored form: the entity kind's id, or `field:<name>`.
    pub fn encode(&self) -> String {
        match self {
            Scope::Entity(k) => k.id().to_string(),
            Scope::Field(f) => format!("field:{f}"),
        }
    }

    pub fn decode(s: &str) -> Option<Scope> {
        match s.strip_prefix("field:") {
            Some(f) => Some(Scope::Field(f.to_string())),
            None => EntityKind::from_id(s).map(Scope::Entity),
        }
    }
}

/// One name with where and how often it was seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub scope: Scope,
    pub name: String,
    /// Events that carry it.
    pub count: u64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub first_id: u64,
    pub last_id: u64,
}

/// A name a lookup found, before ranking (see [`crate::find`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub scope: Scope,
    pub name: String,
    pub count: u64,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    /// Names of (at most [`MAX_SEEN_IN`]) sources that have it.
    pub seen_in: Vec<String>,
}

/// A lookup: the words as the user wrote them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueRequest {
    pub text: String,
}

/// The catalogue's answer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueReply {
    /// Whether every ready source is catalogued; when not, callers fall back to reading facets.
    pub built: bool,
    /// Sources not catalogued yet.
    pub pending: u64,
    /// Names that share letters with the text, at most [`MAX_CANDIDATES`], most alike first.
    pub candidates: Vec<Candidate>,
}

/// Letters and digits of `s` only, lower-cased.
pub fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The distinct three-letter groups of a squashed name, sorted.
pub fn trigram_keys(squashed: &str) -> Vec<String> {
    let c: Vec<char> = squashed.chars().collect();
    let set: BTreeSet<String> = c.windows(3).map(|w| w.iter().collect()).collect();
    set.into_iter().collect()
}

/// The groups a lookup of `text` asks for: those of the whole text and of each of its words.
pub fn search_keys(text: &str) -> Vec<String> {
    let mut set: BTreeSet<String> = trigram_keys(&squash(text)).into_iter().collect();
    let mut words = Vec::new();
    crate::words::words(text, &mut |w| words.push(w));
    for w in words {
        set.extend(trigram_keys(&squash(&w)));
    }
    set.into_iter().collect()
}

/// The candidates for `text` among `rows` (an entry with the name of its source), as a store without an index would
/// find them: names sharing letter groups with the text (all of them when it is shorter than a group: names
/// starting with it), grouped by scope and name with summed counts, most alike first, at most [`MAX_CANDIDATES`].
pub fn candidates_from(rows: &[(String, Entry)], text: &str) -> Vec<Candidate> {
    let squashed = squash(text);
    if squashed.is_empty() {
        return Vec::new();
    }
    let keys: BTreeSet<String> = search_keys(text).into_iter().collect();
    let mut by_name: HashMap<(Scope, String), (usize, Candidate)> = HashMap::new();
    for (source, e) in rows {
        let norm = squash(&e.name);
        let overlap = if keys.is_empty() {
            if !norm.starts_with(&squashed) {
                continue;
            }
            0
        } else {
            let overlap = trigram_keys(&norm).iter().filter(|t| keys.contains(*t)).count();
            if overlap == 0 {
                continue;
            }
            overlap
        };
        let slot = by_name.entry((e.scope.clone(), e.name.clone())).or_insert_with(|| {
            (
                overlap,
                Candidate {
                    scope: e.scope.clone(),
                    name: e.name.clone(),
                    count: 0,
                    first_ts: None,
                    last_ts: None,
                    seen_in: Vec::new(),
                },
            )
        });
        let c = &mut slot.1;
        c.count += e.count;
        c.first_ts = min_opt(c.first_ts, e.first_ts);
        c.last_ts = max_opt(c.last_ts, e.last_ts);
        if !c.seen_in.contains(source) {
            c.seen_in.push(source.clone());
        }
    }
    let mut v: Vec<(usize, Candidate)> = by_name.into_values().collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.count.cmp(&a.1.count)).then_with(|| a.1.name.cmp(&b.1.name)));
    v.truncate(MAX_CANDIDATES);
    v.into_iter()
        .map(|(_, mut c)| {
            c.seen_in.sort();
            c.seen_in.truncate(MAX_SEEN_IN);
            c
        })
        .collect()
}

/// Builds the catalogue rows of one source.
#[derive(Debug, Default)]
pub struct Tally {
    entries: HashMap<(Scope, String), Entry>,
    /// Names refused because the tally was full.
    pub dropped: u64,
    /// GUIDs seen (events with a GUID, per GUID).
    pub guids: u64,
    guids_listed: usize,
}

impl Tally {
    /// Counts `name` of `scope` seen in the event `id` at `ts` (`n` events).
    pub fn add(&mut self, scope: Scope, name: &str, id: u64, ts: Option<i64>, n: u64) {
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return;
        }
        let key = (scope, name.to_lowercase());
        let full = self.entries.len() >= MAX_NAMES;
        match self.entries.get_mut(&key) {
            Some(e) => {
                e.count += n;
                e.first_ts = min_opt(e.first_ts, ts);
                e.last_ts = max_opt(e.last_ts, ts);
                e.first_id = e.first_id.min(id);
                e.last_id = e.last_id.max(id);
            }
            None if full => self.dropped += 1,
            None => {
                let e = Entry {
                    scope: key.0.clone(),
                    name: name.to_string(),
                    count: n,
                    first_ts: ts,
                    last_ts: ts,
                    first_id: id,
                    last_id: id,
                };
                self.entries.insert(key, e);
            }
        }
    }

    /// Counts the entities of one event. GUIDs are counted always and listed up to [`MAX_GUIDS`].
    pub fn add_event(&mut self, found: &Extracted, id: u64, ts: Option<i64>) {
        self.guids += found.guids as u64;
        for e in &found.entities {
            if e.kind == EntityKind::Guid {
                let known = self.entries.contains_key(&(Scope::Entity(EntityKind::Guid), e.name.to_lowercase()));
                if !known {
                    if self.guids_listed >= MAX_GUIDS {
                        continue;
                    }
                    self.guids_listed += 1;
                }
            }
            self.add(Scope::Entity(e.kind), &e.name, id, ts, 1);
        }
    }

    /// The rows, most events first then by name (deterministic).
    pub fn finish(self) -> Vec<Entry> {
        let mut v: Vec<Entry> = self.entries.into_values().collect();
        v.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.scope.cmp(&b.scope))
        });
        v
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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

#[cfg(test)]
mod tests;
