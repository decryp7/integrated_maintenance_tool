//! Request and reply of search everything (camelCase on the wire, mirrored in `api.types.ts`), and the notes and
//! knowledge the app hands in ([`Corpus`]). Kept apart from the search itself ([`super::run`]).

use crate::batch::EventId;
use crate::model::Level;
use serde::{Deserialize, Serialize};

/// What the search box asks: its text and how many results of each kind to return.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchAllRequest {
    /// The search text, read as words like the grid's smart search (at most [`super::MAX_TEXT_CHARS`]).
    pub text: String,
    /// Results per kind: 0 is [`super::DEFAULT_PER_KIND`], at most [`super::MAX_PER_KIND`].
    pub limit: usize,
}

/// A run of a snippet: highlighted (`mark`, a matched word) or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnippetPart {
    pub text: String,
    pub mark: bool,
}

/// An event that holds every word of the text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventHit {
    pub id: EventId,
    pub ts: Option<i64>,
    pub level: Level,
    /// The source's name.
    pub source: String,
    /// The message (or the exception, when the words are there) around the first match.
    pub snippet: Vec<SnippetPart>,
}

/// A field whose name holds the words (`value` none), or one value of a field with few distinct values that does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldHit {
    pub field: String,
    pub value: Option<String>,
    /// Events with the field (with the value).
    pub count: u64,
    /// The name (or the value) with the matched words marked.
    pub snippet: Vec<SnippetPart>,
}

/// A source whose name or path holds the words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceHit {
    /// The source's id (`SourceInfo::id`), to filter the grid on.
    pub id: u32,
    pub name: String,
    pub path: String,
    pub records: u64,
    /// The path with the matched words marked.
    pub snippet: Vec<SnippetPart>,
}

/// A marker kind whose name holds the words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkerHit {
    pub kind: String,
    pub snippet: Vec<SnippetPart>,
}

/// An exception group whose type, message or top frame holds the words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionHit {
    pub fingerprint: String,
    pub type_name: String,
    /// Events in the group.
    pub count: u64,
    /// The group's first sample event, to reveal.
    pub sample: Option<EventId>,
    /// `Type: message` with the matched words marked.
    pub snippet: Vec<SnippetPart>,
}

/// Whose a note or knowledge file is: the project's (notes, project knowledge) or every project's (global knowledge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DocScope {
    Project,
    Global,
}

/// A note or knowledge file that holds the words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocHit {
    pub id: String,
    pub scope: DocScope,
    pub title: String,
    /// The event a note is about (none for other notes and for knowledge files).
    pub event: Option<EventId>,
    pub snippet: Vec<SnippetPart>,
}

/// How far the search of one kind got.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum KindOutcome {
    /// Everything of the kind was searched.
    #[default]
    Complete,
    /// The time budget ran out: the results are what was found until then.
    TimedOut,
    /// The store could not answer (it is busy, or failed); the other kinds are still searched.
    Failed { message: String },
}

/// The results of one kind: the first `items` of `total` matches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KindHits<T> {
    /// Matches found (events: every matching event; the others: those searched before a limit stopped the search).
    pub total: u64,
    pub items: Vec<T>,
    pub outcome: KindOutcome,
}

impl<T> Default for KindHits<T> {
    fn default() -> Self {
        KindHits {
            total: 0,
            items: Vec::new(),
            outcome: KindOutcome::Complete,
        }
    }
}

/// Every kind's results, in the order the UI shows them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchAllReply {
    pub events: KindHits<EventHit>,
    pub sources: KindHits<SourceHit>,
    pub fields: KindHits<FieldHit>,
    pub markers: KindHits<MarkerHit>,
    pub exceptions: KindHits<ExceptionHit>,
    pub notes: KindHits<DocHit>,
    pub knowledge: KindHits<DocHit>,
}

/// A note or knowledge file to search: what the app holds outside the store, handed in by the app (never by the
/// webview).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchDoc {
    pub id: String,
    pub scope: DocScope,
    pub title: String,
    /// The event a note is about.
    pub event: Option<EventId>,
    pub text: String,
}

/// The notes and knowledge files of the window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Corpus {
    pub notes: Vec<SearchDoc>,
    pub knowledge: Vec<SearchDoc>,
}
