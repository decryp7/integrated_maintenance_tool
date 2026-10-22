//! Search everything: one search box text looked up in every kind of thing a window holds (events, sources,
//! fields and their values, marker kinds, exception groups, notes and knowledge files), a few results of each kind with the matched
//! words marked, for the palette that jumps to them.
//!
//! Pure over a small port ([`SearchSource`]: summary, page, facet) and the notes and knowledge the app hands in
//! ([`Corpus`]). Bounded: at most [`MAX_PER_KIND`] results of a kind, snippets of
//! [`snippet::SNIPPET_CHARS`] characters, at most [`MAX_VALUE_FIELDS`] facet requests, and a time budget
//! ([`Control`]): when it runs out the kinds not yet searched say so ([`KindOutcome::TimedOut`]) instead of
//! making the user wait. A store failure fails only its kinds ([`KindOutcome::Failed`]). A newer search of the
//! window cancels this one ([`Control::cancelled`]); a cancelled run answers nothing.
//!
//! Words match like the grid's smart search ([`crate::words`]): every word of the text, the last one as a prefix.

mod dto;
mod snippet;
#[cfg(test)]
mod tests;

pub use dto::*;
pub use snippet::{contains_any, needles, snippet, MAX_SCAN_CHARS, SNIPPET_CHARS};

use crate::api::{FacetRequest, FieldInfo};
use crate::exceptions::ExceptionGroup;
use crate::ports::{Analysis, EventView, Filter, PageQuery, PageReply, Query, StoreError, StoreResult, SummaryReply};
use crate::query::TextMode;
use crate::words::WordQuery;
use std::time::{Duration, Instant};

/// Longest search text (in characters).
pub const MAX_TEXT_CHARS: usize = 256;
/// Results of a kind when the request does not say.
pub const DEFAULT_PER_KIND: usize = 8;
/// Most results of a kind.
pub const MAX_PER_KIND: usize = 50;
/// How long a search may take before the kinds not yet searched are skipped.
pub const BUDGET: Duration = Duration::from_secs(2);
/// Fields whose values are searched (one facet request each), in the summary's order.
pub const MAX_VALUE_FIELDS: usize = 24;
/// A field with more distinct values than this is searched by name only (its values are ids, numbers, times...).
pub const MAX_DISTINCT_VALUES: usize = 500;

/// What search everything reads from the store: the summary (sources, fields, marker kinds), a page of events
/// matching words, the values of a field and the exception groups. Every store with [`Query`] and [`Analysis`] offers it.
pub trait SearchSource {
    fn summary(&self) -> StoreResult<SummaryReply>;
    fn page(&self, req: &PageQuery) -> StoreResult<PageReply>;
    fn facet(&self, req: &FacetRequest) -> StoreResult<crate::api::Facet>;
    /// The exception groups of every event.
    fn exception_groups(&self) -> StoreResult<Vec<ExceptionGroup>>;
}

impl<T: Query + Analysis + ?Sized> SearchSource for T {
    fn summary(&self) -> StoreResult<SummaryReply> {
        Query::summary(self)
    }
    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        Query::page(self, req)
    }
    fn facet(&self, req: &FacetRequest) -> StoreResult<crate::api::Facet> {
        Query::facet(self, req)
    }
    fn exception_groups(&self) -> StoreResult<Vec<ExceptionGroup>> {
        Analysis::exception_groups(self, &crate::api::QueryRequest { query: Default::default() }).map(|a| a.result)
    }
}

/// When a search must stop: its deadline, and whether a newer search replaced it.
pub struct Control<'a> {
    deadline: Instant,
    cancelled: &'a (dyn Fn() -> bool + Sync),
}

impl<'a> Control<'a> {
    /// A search that may run for `budget` from now and stops once `cancelled` says so.
    pub fn new(budget: Duration, cancelled: &'a (dyn Fn() -> bool + Sync)) -> Control<'a> {
        Control {
            deadline: Instant::now() + budget,
            cancelled,
        }
    }

    pub fn cancelled(&self) -> bool {
        (self.cancelled)()
    }

    pub fn timed_out(&self) -> bool {
        Instant::now() >= self.deadline
    }
}

/// Why a search did not answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SearchAllError {
    /// The text is too long; the message says the limit.
    #[error("{0}")]
    Invalid(String),
    /// A newer search replaced this one.
    #[error("cancelled")]
    Cancelled,
}

/// A request read: its words, the strings to mark and how many results of each kind.
struct Plan {
    text: String,
    words: WordQuery,
    needles: Vec<String>,
    per_kind: usize,
}

fn plan(req: &SearchAllRequest) -> Result<Option<Plan>, SearchAllError> {
    let text = req.text.trim();
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(SearchAllError::Invalid(format!("search for at most {MAX_TEXT_CHARS} characters")));
    }
    let words = WordQuery::parse(text);
    if words.is_empty() {
        return Ok(None);
    }
    let per_kind = match req.limit {
        0 => DEFAULT_PER_KIND,
        n => n.min(MAX_PER_KIND),
    };
    Ok(Some(Plan {
        text: text.to_string(),
        needles: needles(&words),
        words,
        per_kind,
    }))
}

impl Plan {
    /// Whether `text` holds every word (only its first [`MAX_SCAN_CHARS`] characters are read).
    fn matches(&self, text: &str) -> bool {
        self.matches_all(&[text])
    }

    /// Whether `texts` together hold every word (like a title and its text).
    fn matches_all(&self, texts: &[&str]) -> bool {
        let heads: Vec<&str> = texts
            .iter()
            .map(|t| match t.char_indices().nth(MAX_SCAN_CHARS) {
                Some((at, _)) => &t[..at],
                None => t,
            })
            .collect();
        self.words.matches(&heads)
    }

    /// Adds `item` to `hits` when there is room; counts it either way.
    fn push<T>(&self, hits: &mut KindHits<T>, item: impl FnOnce() -> T) {
        hits.total += 1;
        if hits.items.len() < self.per_kind {
            hits.items.push(item());
        }
    }
}

/// Searches everything for `req`: `None` when the text has no words (nothing to search). Fails only for a text
/// that is too long or when `control` says a newer search replaced this one; store failures and the time budget
/// are reported per kind.
pub fn run(
    store: &(impl SearchSource + ?Sized),
    corpus: &Corpus,
    req: &SearchAllRequest,
    control: &Control<'_>,
) -> Result<Option<SearchAllReply>, SearchAllError> {
    let Some(plan) = plan(req)? else {
        return Ok(None);
    };
    // Events first (what users look for most), then what the summary and the app already hold, then field
    // values (a facet request each), the slowest.
    let events = if go_on(control)? { events(store, &plan) } else { timed_out() };
    let summary = if go_on(control)? { Some(store.summary()) } else { None };
    let (sources, markers) = match &summary {
        None => (timed_out(), timed_out()),
        Some(Err(e)) => (failed(e), failed(e)),
        Some(Ok(s)) => (sources(&plan, s), markers(&plan, s)),
    };
    let notes = if go_on(control)? { docs(&plan, &corpus.notes) } else { timed_out() };
    let knowledge = if go_on(control)? {
        docs(&plan, &corpus.knowledge)
    } else {
        timed_out()
    };
    let exceptions = if go_on(control)? { exceptions(store, &plan) } else { timed_out() };
    let fields = match &summary {
        None => timed_out(),
        Some(Err(e)) => failed(e),
        Some(Ok(s)) => fields(store, &plan, &s.fields, control)?,
    };
    Ok(Some(SearchAllReply {
        events,
        sources,
        fields,
        markers,
        exceptions,
        notes,
        knowledge,
    }))
}

/// Whether the search may go on: an error once a newer search replaced it, false once its time is up.
fn go_on(control: &Control<'_>) -> Result<bool, SearchAllError> {
    if control.cancelled() {
        return Err(SearchAllError::Cancelled);
    }
    Ok(!control.timed_out())
}

fn timed_out<T>() -> KindHits<T> {
    KindHits {
        outcome: KindOutcome::TimedOut,
        ..KindHits::default()
    }
}

fn failed<T>(e: &StoreError) -> KindHits<T> {
    KindHits {
        outcome: KindOutcome::Failed { message: e.to_string() },
        ..KindHits::default()
    }
}

fn events(store: &(impl SearchSource + ?Sized), plan: &Plan) -> KindHits<EventHit> {
    let req = PageQuery {
        query: Filter {
            text: plan.text.clone(),
            text_mode: TextMode::Words,
            ..Filter::default()
        },
        limit: plan.per_kind,
        ..PageQuery::default()
    };
    match store.page(&req) {
        Ok(page) => KindHits {
            total: page.total as u64,
            items: page.rows.iter().take(plan.per_kind).map(|e| event_hit(plan, e)).collect(),
            outcome: KindOutcome::Complete,
        },
        Err(e) => failed(&e),
    }
}

/// An event's hit: the snippet of the message, or of the exception or template when the words are there and not
/// in the message.
fn event_hit(plan: &Plan, e: &EventView) -> EventHit {
    let shown = [Some(e.message.as_str()), e.exception.as_deref(), e.template.as_deref()]
        .into_iter()
        .flatten()
        .find(|t| contains_any(t, &plan.needles))
        .unwrap_or(&e.message);
    EventHit {
        id: e.id,
        ts: e.ts,
        level: e.level,
        source: e.source_name.clone(),
        snippet: snippet(shown, &plan.needles),
    }
}

fn sources(plan: &Plan, summary: &SummaryReply) -> KindHits<SourceHit> {
    let mut hits = KindHits::default();
    for s in &summary.sources {
        if plan.matches_all(&[&s.path, &s.name]) {
            plan.push(&mut hits, || SourceHit {
                id: s.id,
                name: s.name.clone(),
                path: s.path.clone(),
                records: s.records as u64,
                snippet: snippet(&s.path, &plan.needles),
            });
        }
    }
    hits
}

fn markers(plan: &Plan, summary: &SummaryReply) -> KindHits<MarkerHit> {
    let mut hits = KindHits::default();
    for kind in &summary.marker_kinds {
        if plan.matches(kind) {
            plan.push(&mut hits, || MarkerHit {
                kind: kind.clone(),
                snippet: snippet(kind, &plan.needles),
            });
        }
    }
    hits
}

/// Exception groups whose type, message or top frame hold the words (most frequent first, as the engine lists them).
fn exceptions(store: &(impl SearchSource + ?Sized), plan: &Plan) -> KindHits<ExceptionHit> {
    let groups = match store.exception_groups() {
        Ok(groups) => groups,
        Err(e) => return failed(&e),
    };
    let mut hits = KindHits::default();
    for g in &groups {
        let top = g.top_frame.as_deref().unwrap_or_default();
        if plan.matches_all(&[&g.type_name, &g.message, top]) {
            plan.push(&mut hits, || ExceptionHit {
                fingerprint: g.fingerprint.clone(),
                type_name: g.type_name.clone(),
                count: g.count as u64,
                sample: g.sample_ids.first().copied().map(|id| crate::batch::EventId(u64::from(id))),
                snippet: snippet(&format!("{}: {}", g.type_name, g.message), &plan.needles),
            });
        }
    }
    hits
}

fn docs(plan: &Plan, docs: &[SearchDoc]) -> KindHits<DocHit> {
    let mut hits = KindHits::default();
    for d in docs {
        if plan.matches_all(&[&d.title, &d.text]) {
            plan.push(&mut hits, || DocHit {
                id: d.id.clone(),
                scope: d.scope,
                title: d.title.clone(),
                event: d.event,
                snippet: snippet(&d.text, &plan.needles),
            });
        }
    }
    hits
}

/// Field names, then the values of up to [`MAX_VALUE_FIELDS`] fields with few distinct values (checking the
/// budget before each facet request).
fn fields(
    store: &(impl SearchSource + ?Sized),
    plan: &Plan,
    fields: &[FieldInfo],
    control: &Control<'_>,
) -> Result<KindHits<FieldHit>, SearchAllError> {
    let mut hits = KindHits::default();
    for f in fields {
        if plan.matches(&f.name) {
            plan.push(&mut hits, || FieldHit {
                field: f.name.clone(),
                value: None,
                count: f.count as u64,
                snippet: snippet(&f.name, &plan.needles),
            });
        }
    }
    let searched = fields
        .iter()
        .filter(|f| !f.numeric && f.distinct > 0 && f.distinct <= MAX_DISTINCT_VALUES)
        .take(MAX_VALUE_FIELDS);
    for f in searched {
        if !go_on(control)? {
            hits.outcome = KindOutcome::TimedOut;
            break;
        }
        let facet = match store.facet(&FacetRequest {
            query: Filter::default(),
            field: f.name.clone(),
            limit: MAX_DISTINCT_VALUES,
        }) {
            Ok(facet) => facet,
            Err(e) => {
                // The names found stay; the values of the other fields are not searched.
                hits.outcome = KindOutcome::Failed { message: e.to_string() };
                break;
            }
        };
        for v in &facet.values {
            if plan.matches(&v.value) {
                plan.push(&mut hits, || FieldHit {
                    field: f.name.clone(),
                    value: Some(v.value.clone()),
                    count: v.count as u64,
                    snippet: snippet(&v.value, &plan.needles),
                });
            }
        }
    }
    Ok(hits)
}
