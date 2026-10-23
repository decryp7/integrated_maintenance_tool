//! Find: what the assistant (and any caller that wants "tell me about graphic builder") searches with. Pure over
//! [`SearchSource`], so every store offers it.
//!
//! Two operations:
//!
//! - [`resolve_terms`] turns a user's words into concrete values the logs hold: field values and source names that
//!   match exactly, by their parts (`graphic builder` is in `CompanyA.Graphic.Builder.exe`), as prefixes, or
//!   approximately (trigrams: `grafic bilder`), each with the field and the number of events.
//! - [`find`] looks for events and never ends at zero without trying more: a defined ladder of steps, widest last
//!   ([`FindStep`]): the words as a phrase (separators and `camelCase` ignored, see [`crate::words::phrase_in`]),
//!   all words in any order, every word as a part of a word, and finally the events carrying the best matching value.
//!   The reply says which step found the events, where each event matched, and a note the assistant can pass on.
//!
//! Bounded: at most [`MAX_LIMIT`] hits per call (paged with `offset`), at most [`MAX_VALUES`] value matches, messages
//! clipped to [`MESSAGE_CHARS`] characters.

use crate::api::FacetRequest;
use crate::catalogue::{CatalogueRequest, Scope};
use crate::ports::{EventView, Filter, PageQuery, StoreResult};
use crate::query::{FieldFilter, TextMode};
use crate::search_all::{SearchSource, MAX_DISTINCT_VALUES, MAX_VALUE_FIELDS};
use crate::words::{phrase_in, WordQuery};
use serde::Serialize;
use std::collections::HashSet;

/// Most hits per call.
pub const MAX_LIMIT: usize = 50;
/// Hits per call when the request does not say.
pub const DEFAULT_LIMIT: usize = 20;
/// Most value matches returned.
pub const MAX_VALUES: usize = 12;
/// Longest message of a hit (in characters).
pub const MESSAGE_CHARS: usize = 300;
/// Longest text searched (in characters).
pub const MAX_TEXT_CHARS: usize = 256;
/// The pseudo field of a source's name.
pub const SOURCE_FIELD: &str = "@source";
/// The pseudo field of an entity found in message or exception text.
pub const ENTITY_FIELD: &str = "@entity";
/// Share of the search's trigrams a value must hold to be an approximate match.
const FUZZY_SHARE: f64 = 0.5;
/// Fewest trigrams a search needs before approximate matching is tried.
const FUZZY_MIN_TRIGRAMS: usize = 4;

/// How a value matched the words, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Via {
    /// Same letters and digits, ignoring case and separators.
    Exact,
    /// The words are consecutive parts of the value (`graphic builder` in `CompanyA.Graphic.Builder.exe`).
    Parts,
    /// Every word starts a part of the value.
    Prefix,
    /// Most of the three-letter groups of the words are in the value (typos).
    Fuzzy,
}

/// A value the logs hold that the words refer to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueMatch {
    /// The field (`@source` for a source's name).
    pub field: String,
    pub value: String,
    /// Events with the value.
    pub count: u64,
    pub via: Via,
    /// The source's id, for `@source`.
    #[serde(skip)]
    pub source: Option<u32>,
    /// What an entity found in event text is (`field` is then [`ENTITY_FIELD`]); none for field values and sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::entities::EntityKind>,
    /// First and last time the value was seen (entities only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<i64>,
    /// Names of sources that have it (at most a few; entities and field values from the catalogue).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub seen_in: Vec<String>,
}

impl ValueMatch {
    /// What the value is: its field, or the entity kind.
    pub fn label(&self) -> String {
        match self.kind {
            Some(k) => k.id().to_string(),
            None => self.field.clone(),
        }
    }

    /// `field = value`, or `kind value` for an entity.
    pub fn describe(&self) -> String {
        match self.kind {
            Some(k) => format!("{} {}", k.id(), self.value),
            None => format!("{} = {}", self.field, self.value),
        }
    }
}

/// What the search box asks for its "Did you mean" line: the user's words, and at most `limit` values (0 = a few).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindValuesRequest {
    pub text: String,
    #[serde(default)]
    pub limit: usize,
}

/// The step of the ladder that found the events, narrowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FindStep {
    /// The words as a phrase, ignoring case and separators.
    Phrase,
    /// Every word, in any order (the last as a prefix).
    Words,
    /// Every word as part of a longer word.
    Partial,
    /// Events carrying the best matching field value or source.
    Value,
}

/// One event found.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindHit {
    pub id: u64,
    pub ts: Option<i64>,
    pub level: crate::model::Level,
    pub source: String,
    pub message: String,
    /// Where the words are: `message`, `exception`, `template`, `source` or `field:<name>`.
    pub matched_in: Vec<String>,
}

/// The answer of [`find`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindReply {
    pub text: String,
    /// The step that found events (none: nothing found by any step).
    pub step: Option<FindStep>,
    /// Events found by that step.
    pub total: u64,
    pub offset: usize,
    pub hits: Vec<FindHit>,
    /// Values the words refer to, best first.
    pub values: Vec<ValueMatch>,
    /// The name the events mention, as written in the logs (`CompanyA.Graphic.Builder.exe`).
    pub mention: Option<String>,
    /// What happened, for the user.
    pub note: String,
}

/// Searches for `text`, trying wider steps until one finds events. `limit` 0 is [`DEFAULT_LIMIT`]. Fails only when
/// the store does.
pub fn find(store: &(impl SearchSource + ?Sized), text: &str, offset: usize, limit: usize) -> StoreResult<FindReply> {
    let text = clip(text.trim(), MAX_TEXT_CHARS);
    let limit = if limit == 0 { DEFAULT_LIMIT } else { limit.min(MAX_LIMIT) };
    let words = WordQuery::parse(&text);
    let mut reply = FindReply {
        text: text.clone(),
        step: None,
        total: 0,
        offset,
        hits: Vec::new(),
        values: Vec::new(),
        mention: None,
        note: String::new(),
    };
    if words.is_empty() {
        reply.note = "There are no words to search for.".into();
        return Ok(reply);
    }
    let all: Vec<String> = words.terms.iter().chain(&words.prefixes).cloned().collect();
    let phrase_text = if words.phrases.is_empty() && all.len() > 1 {
        format!("\"{}\"", text.replace('"', " "))
    } else {
        text.clone()
    };
    let mut steps: Vec<(FindStep, PageQuery)> = vec![];
    let page = |q: Filter| PageQuery {
        query: q,
        offset,
        limit,
        ..PageQuery::default()
    };
    let words_filter = |t: &str| Filter {
        text: t.to_string(),
        text_mode: TextMode::Words,
        ..Filter::default()
    };
    if all.len() > 1 || !words.phrases.is_empty() {
        steps.push((FindStep::Phrase, page(words_filter(&phrase_text))));
    }
    steps.push((FindStep::Words, page(words_filter(&text.replace('"', " ")))));
    // Every word anywhere in message, exception or template, as part of a longer word.
    let partial = all.iter().map(|w| format!("\"{w}\"")).collect::<Vec<_>>().join(" and ");
    steps.push((
        FindStep::Partial,
        page(Filter {
            ql: partial,
            ..Filter::default()
        }),
    ));
    reply.values = resolve_terms(store, &text, MAX_VALUES)?;
    let mut found = None;
    for (step, req) in steps {
        let p = store.page(&req)?;
        if p.total > 0 {
            found = Some((step, p.total, p.rows));
            break;
        }
    }
    if found.is_none() {
        if let Some(best) = reply.values.first().filter(|v| v.via != Via::Fuzzy) {
            let query = match best.source {
                Some(id) => Filter {
                    sources: vec![id],
                    ..Filter::default()
                },
                None if best.kind.is_some() => Filter {
                    text: format!("\"{}\"", best.value.replace('"', " ")),
                    text_mode: TextMode::Words,
                    ..Filter::default()
                },
                None => Filter {
                    filters: vec![FieldFilter {
                        field: best.field.clone(),
                        op: "eq".into(),
                        value: Some(best.value.clone()),
                    }],
                    ..Filter::default()
                },
            };
            let p = store.page(&page(query))?;
            if p.total > 0 {
                found = Some((FindStep::Value, p.total, p.rows));
            }
        }
    }
    let Some((step, total, rows)) = found else {
        reply.note = match reply.values.first() {
            Some(v) => format!("No events contain '{text}'. Did you mean {} ({})?", v.value, v.label()),
            None => format!("No events, field values or sources match '{text}', not even partly."),
        };
        return Ok(reply);
    };
    reply.step = Some(step);
    reply.total = total as u64;
    reply.hits = rows.iter().take(limit).map(|e| hit(e, &all)).collect();
    reply.mention = reply
        .hits
        .iter()
        .find_map(|h| mention(&h.message, &all))
        .or_else(|| reply.values.first().filter(|v| v.via != Via::Fuzzy).map(|v| v.value.clone()));
    let about = reply.mention.as_ref().map(|m| format!(" mentioning {m}")).unwrap_or_default();
    reply.note = match step {
        FindStep::Phrase => format!("{total} events contain '{text}'{about}."),
        FindStep::Words => format!("No exact match for '{text}'; found {total} events with all of its words{about}."),
        FindStep::Partial => format!("No exact match for '{text}'; found {total} events where its words are part of longer words{about}."),
        FindStep::Value => format!(
            "No event text matches '{text}'; found {total} events with {}.",
            reply.values.first().map(|v| v.describe()).unwrap_or_default()
        ),
    };
    if step != FindStep::Value {
        let also: Vec<String> = reply
            .values
            .iter()
            .filter(|v| v.via != Via::Fuzzy)
            .take(3)
            .map(|v| format!("{} ({} events)", v.describe(), v.count))
            .collect();
        if !also.is_empty() {
            reply.note.push_str(&format!(" Also in field values: {}.", also.join("; ")));
        }
    }
    Ok(reply)
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// An event as a hit: where the words are.
fn hit(e: &EventView, words: &[String]) -> FindHit {
    let has = |t: &str| {
        let lower = t.to_lowercase();
        let squashed = squash(t);
        words.iter().all(|w| lower.contains(w.as_str()) || squashed.contains(&squash(w)))
    };
    let mut matched_in = Vec::new();
    if has(&e.message) {
        matched_in.push("message".to_string());
    }
    if e.exception.as_deref().is_some_and(has) {
        matched_in.push("exception".into());
    }
    if e.template.as_deref().is_some_and(has) {
        matched_in.push("template".into());
    }
    if has(&e.source_name) {
        matched_in.push("source".into());
    }
    for (name, v) in &e.fields {
        if v.as_str().is_some_and(has) {
            matched_in.push(format!("field:{name}"));
        }
    }
    FindHit {
        id: e.id.0,
        ts: e.ts,
        level: e.level,
        source: e.source_name.clone(),
        message: clip(&e.message, MESSAGE_CHARS),
        matched_in,
    }
}

/// The first whitespace-separated token of `text` that holds the first word (ignoring separators), trimmed of
/// punctuation around it: the name as written (`CompanyA.Graphic.Builder.exe`).
fn mention(text: &str, words: &[String]) -> Option<String> {
    let first = squash(words.first()?);
    text.split_whitespace()
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()))
        .find(|t| !t.is_empty() && squash(t).contains(&first))
        .map(str::to_string)
}

/// Letters and digits only, lower-cased.
fn squash(s: &str) -> String {
    crate::catalogue::squash(s)
}

fn trigrams(s: &str) -> HashSet<[char; 3]> {
    let c: Vec<char> = s.chars().collect();
    c.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
}

/// How `value` matches the words of `text`, if it does.
fn via(text: &str, words: &WordQuery, wanted: &HashSet<[char; 3]>, value: &str) -> Option<Via> {
    let (a, b) = (squash(text), squash(value));
    if a.is_empty() || b.is_empty() {
        return None;
    }
    if a == b {
        return Some(Via::Exact);
    }
    if phrase_in(text, value) {
        return Some(Via::Parts);
    }
    if words.matches(&[value]) || prefix_parts(words, value) {
        return Some(Via::Prefix);
    }
    if wanted.len() >= FUZZY_MIN_TRIGRAMS {
        let have = trigrams(&b);
        let share = wanted.iter().filter(|t| have.contains(*t)).count() as f64 / wanted.len() as f64;
        if share >= FUZZY_SHARE {
            return Some(Via::Fuzzy);
        }
    }
    None
}

/// Whether every word of the search starts one of the value's words.
fn prefix_parts(words: &WordQuery, value: &str) -> bool {
    let mut have = Vec::new();
    crate::words::words(value, &mut |w| have.push(w));
    words
        .terms
        .iter()
        .chain(&words.prefixes)
        .all(|t| have.iter().any(|h| h.starts_with(t.as_str())))
}

/// The values of fields (and the names of sources) the words refer to, best match first, then most events;
/// approximate matches only when nothing matches better. At most `limit` (and [`MAX_VALUES`]).
pub fn resolve_terms(store: &(impl SearchSource + ?Sized), text: &str, limit: usize) -> StoreResult<Vec<ValueMatch>> {
    let text = clip(text.trim(), MAX_TEXT_CHARS);
    let words = WordQuery::parse(&text);
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let wanted = trigrams(&squash(&text));
    let summary = store.summary()?;
    let mut out: Vec<ValueMatch> = Vec::new();
    for s in &summary.sources {
        if let Some(v) = via(&text, &words, &wanted, &s.name) {
            out.push(ValueMatch {
                field: SOURCE_FIELD.into(),
                value: s.name.clone(),
                count: s.records as u64,
                via: v,
                source: Some(s.id),
                kind: None,
                first_seen: None,
                last_seen: None,
                seen_in: Vec::new(),
            });
        }
    }
    let catalogue = store.catalogue(&CatalogueRequest { text: text.clone() })?;
    if catalogue.built {
        // The catalogue holds field values and the names found in event text; no facet is read.
        for c in &catalogue.candidates {
            let (field, kind) = match &c.scope {
                Scope::Entity(k) => (ENTITY_FIELD.to_string(), Some(*k)),
                Scope::Field(f) => (f.clone(), None),
            };
            if let Some(via) = via(&text, &words, &wanted, &c.name) {
                out.push(ValueMatch {
                    field,
                    value: c.name.clone(),
                    count: c.count,
                    via,
                    source: None,
                    kind,
                    first_seen: kind.and(c.first_ts),
                    last_seen: kind.and(c.last_ts),
                    seen_in: c.seen_in.clone(),
                });
            }
        }
    } else {
        facet_values(store, &summary, &text, &words, &wanted, &mut out)?;
    }
    let best = out.iter().map(|m| m.via).min();
    out.retain(|m| m.via != Via::Fuzzy || best == Some(Via::Fuzzy));
    out.sort_by(|a, b| a.via.cmp(&b.via).then(b.count.cmp(&a.count)).then_with(|| a.value.cmp(&b.value)));
    out.truncate(limit.clamp(1, MAX_VALUES));
    Ok(out)
}

/// Without a built catalogue: the values of the low-cardinality fields, read as facets.
fn facet_values(
    store: &(impl SearchSource + ?Sized),
    summary: &crate::ports::SummaryReply,
    text: &str,
    words: &WordQuery,
    wanted: &HashSet<[char; 3]>,
    out: &mut Vec<ValueMatch>,
) -> StoreResult<()> {
    let fields = summary
        .fields
        .iter()
        .filter(|f| !f.numeric && f.distinct > 0 && f.distinct <= MAX_DISTINCT_VALUES)
        .take(MAX_VALUE_FIELDS);
    for f in fields {
        let facet = store.facet(&FacetRequest {
            query: Filter::default(),
            field: f.name.clone(),
            limit: MAX_DISTINCT_VALUES,
        })?;
        for v in &facet.values {
            if let Some(via) = via(text, words, wanted, &v.value) {
                out.push(ValueMatch {
                    field: f.name.clone(),
                    value: v.value.clone(),
                    count: v.count as u64,
                    via,
                    source: None,
                    kind: None,
                    first_seen: None,
                    last_seen: None,
                    seen_in: Vec::new(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
