//! "Find related logs": the events that belong to a screenshot (ADR 0025). The model only reads the image into
//! [`Clues`] and [`ShownTime`]s; this engine operation decides the time ([`time`]), searches through a small
//! port ([`RelatedSource`], implemented over the store ports and the in-memory store alike) and ranks what it
//! finds ([`rank`]) with a reason for every link.
//!
//! - Time: a time shown on the screen, else the image's EXIF capture time, else a time in its file name, else its
//!   file's modification time, read in the profile's time zone; a time without a date takes the date from the
//!   other evidence (or from the data when it covers one day). A clock offset of the screenshot's machine (a source
//!   whose name matches a machine on the screen) is applied like on its logs.
//! - Search: the events around that time, and every event holding a strong clue (exception name, error code, id,
//!   error text) anywhere in time, so a clock that disagrees is noticed instead of hiding the matches.
//! - Rank: time proximity plus the clues each event matches; the top N with their reasons.

mod port;
pub mod rank;
pub mod time;

pub use port::{LogStoreSource, QuerySource, RelatedSource};

use crate::model::Level;
use crate::ports::{Filter, StoreResult};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Default and largest number of links returned.
pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 100;
/// Default time window around the image's time.
pub const DEFAULT_WINDOW_MS: i64 = 120_000;
/// Largest window accepted.
pub const MAX_WINDOW_MS: i64 = 6 * 3_600_000;
/// Events read around the image's time, and per searched clue.
const WINDOW_EVENTS: usize = 2_000;
const CLUE_EVENTS: usize = 200;
/// Clues searched one by one (the strongest first).
const MAX_SEARCHES: usize = 12;
/// Most items per clue list and characters per clue.
const MAX_CLUES: usize = 16;
const MAX_CLUE_CHARS: usize = 300;

/// What the screenshot shows that can be searched for (from the model, already bounded by the caller).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Clues {
    pub window_title: Option<String>,
    pub application: Option<String>,
    pub error_texts: Vec<String>,
    pub error_codes: Vec<String>,
    pub exception_names: Vec<String>,
    pub machine_names: Vec<String>,
    pub user_names: Vec<String>,
    pub server_names: Vec<String>,
    pub ids: Vec<String>,
    pub versions: Vec<String>,
}

fn bound_text(s: &str) -> Option<String> {
    let t: String = s.chars().filter(|c| !c.is_control()).take(MAX_CLUE_CHARS).collect();
    let t = t.trim().to_string();
    (t.chars().count() >= 2).then_some(t)
}

fn bound_list(v: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v.iter().filter_map(|s| bound_text(s)) {
        if out.len() < MAX_CLUES && !out.iter().any(|o| o.eq_ignore_ascii_case(&s)) {
            out.push(s);
        }
    }
    out
}

impl Clues {
    /// The same clues with every list and string bounded (requests come from the webview or a model).
    pub fn bounded(&self) -> Clues {
        Clues {
            window_title: self.window_title.as_deref().and_then(bound_text),
            application: self.application.as_deref().and_then(bound_text),
            error_texts: bound_list(&self.error_texts),
            error_codes: bound_list(&self.error_codes),
            exception_names: bound_list(&self.exception_names),
            machine_names: bound_list(&self.machine_names),
            user_names: bound_list(&self.user_names),
            server_names: bound_list(&self.server_names),
            ids: bound_list(&self.ids),
            versions: bound_list(&self.versions),
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == Clues::default()
    }
}

/// A time as the screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShownTime {
    pub text: String,
    pub has_date: bool,
}

/// An EXIF capture time: `YYYY-MM-DD HH:MM:SS` local to the device, with its UTC offset when known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExifStamp {
    pub local: String,
    pub offset: Option<String>,
}

/// Everything that can tell when the screenshot was taken.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TimeEvidence {
    pub shown: Vec<ShownTime>,
    pub exif: Option<ExifStamp>,
    pub file_name: Option<String>,
    /// The image file's modification time (epoch ms).
    pub file_modified_ms: Option<i64>,
}

/// Where the image's time came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeSource {
    /// A date and time shown on the screen.
    Screen,
    /// A time of day shown on the screen, on the date of other evidence.
    ScreenTimeOfDay,
    Exif,
    FileName,
    FileModified,
}

/// The time the screenshot was taken, as decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageTime {
    /// Epoch ms, in the logs' clock (after any clock offset of the screenshot's machine).
    pub at_ms: i64,
    /// How exact it is (± ms): a time shown without seconds is ± 60 s.
    pub precision_ms: i64,
    pub source: TimeSource,
    /// The evidence as written (the shown text, the file name...).
    pub text: String,
}

/// The request of "find related logs".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RelatedRequest {
    pub clues: Clues,
    pub time: TimeEvidence,
    /// The profile's `parse.assume_timezone` (how times without a zone are read).
    pub assume_timezone: String,
    /// Links wanted (0 = [`DEFAULT_LIMIT`], at most [`MAX_LIMIT`]).
    pub limit: usize,
    /// ± window around the image's time (0 = [`DEFAULT_WINDOW_MS`]).
    pub window_ms: i64,
    /// The screen shows an error: error events rank a little higher.
    pub prefer_errors: bool,
}

/// An event as related-logs ranks and shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedEvent {
    /// The event id (the in-memory store's record ids fit `u32`; the database engine's are `u64`).
    pub id: u64,
    pub ts: Option<i64>,
    pub level: Level,
    pub message: String,
    pub exception: Option<String>,
    pub source: u32,
    pub source_name: String,
    /// The event's lane (instance group).
    pub group: String,
    /// A few fields (bounded): the ones a title or a name can match.
    pub fields: Map<String, Value>,
}

/// What the title or application name matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum TitleTarget {
    Source { name: String },
    Lane { lane: String },
    Field { field: String, value: String },
}

/// Which kind of clue a text matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClueKind {
    ExceptionName,
    ErrorCode,
    Id,
    ErrorText,
    Machine,
    User,
    Server,
    Version,
}

/// Why an event is linked to the image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Reason {
    /// `delta_ms` = event time − image time.
    Time {
        delta_ms: i64,
    },
    Title {
        text: String,
        target: TitleTarget,
    },
    Text {
        clue: String,
        clue_kind: ClueKind,
        partial: bool,
    },
}

impl Reason {
    /// English text for logs, exports and the model ("time ±1.4 s", "title matches lane OrderWorker").
    pub fn label(&self) -> String {
        match self {
            Reason::Time { delta_ms } => format!("time {}", rank::format_delta(*delta_ms)),
            Reason::Title { text, target } => match target {
                TitleTarget::Source { name } => format!("title '{text}' matches source {name}"),
                TitleTarget::Lane { lane } => format!("title '{text}' matches lane {lane}"),
                TitleTarget::Field { field, value } => format!("title '{text}' matches {field} {value}"),
            },
            Reason::Text { clue, partial, .. } => {
                if *partial {
                    format!("text close to '{clue}'")
                } else {
                    format!("text '{clue}'")
                }
            }
        }
    }
}

/// One linked event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedLink {
    pub event: RelatedEvent,
    pub score: f64,
    pub reasons: Vec<Reason>,
}

/// What the user should know about the result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RelatedNote {
    /// No readable time: the links come from text and title only.
    NoTime,
    /// The image's time is outside the loaded logs.
    OutsideData { from: Option<i64>, to: Option<i64> },
    /// The best text matches are this far from the image's time: the clocks likely disagree. The existing
    /// clock-offset correction of `source_id` can fix it.
    ClockDisagreement {
        delta_ms: i64,
        source_id: u32,
        source_name: String,
    },
    /// The clock offset of the screenshot's machine (from its logs) was applied to the image's time.
    OffsetApplied { source_name: String, offset_ms: i64 },
    /// The model found nothing searchable on the image.
    NothingToSearch,
}

/// The answer of "find related logs".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedResult {
    pub time: Option<ImageTime>,
    pub links: Vec<RelatedLink>,
    pub notes: Vec<RelatedNote>,
    /// Distinct events examined.
    pub examined: usize,
}

/// Escapes `s` as a quoted text term of the query language (a phrase, case-insensitive).
pub fn ql_phrase(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Finds the events related to a screenshot: the engine operation behind "Find related logs".
pub fn find_related(src: &dyn RelatedSource, req: &RelatedRequest) -> StoreResult<RelatedResult> {
    let clues = req.clues.bounded();
    let limit = match req.limit {
        0 => DEFAULT_LIMIT,
        n => n.min(MAX_LIMIT),
    };
    let window = match req.window_ms {
        w if w <= 0 => DEFAULT_WINDOW_MS,
        w => w.min(MAX_WINDOW_MS),
    };
    let sources = src.sources()?;
    let data_from = sources.iter().filter_map(|s| s.from).min();
    let data_to = sources.iter().filter_map(|s| s.to).max();
    let mut notes = Vec::new();
    let mut image_time = time::resolve(&req.time, &req.assume_timezone, (data_from, data_to));
    if let Some(t) = image_time.as_mut() {
        if let Some((name, offset)) = time::machine_offset(&clues.machine_names, &sources) {
            t.at_ms = t.at_ms.saturating_add(offset);
            notes.push(RelatedNote::OffsetApplied {
                source_name: name,
                offset_ms: offset,
            });
        }
    }
    if clues.is_empty() && image_time.is_none() {
        notes.push(RelatedNote::NothingToSearch);
        return Ok(RelatedResult {
            time: None,
            links: vec![],
            notes,
            examined: 0,
        });
    }
    let mut candidates: BTreeMap<u64, RelatedEvent> = BTreeMap::new();
    let mut add = |events: Vec<RelatedEvent>| {
        for e in events {
            candidates.entry(e.id).or_insert(e);
        }
    };
    if let Some(t) = &image_time {
        let slack = window.max(t.precision_ms * 2);
        add(src.events(
            &Filter {
                from: Some(t.at_ms.saturating_sub(slack)),
                to: Some(t.at_ms.saturating_add(slack)),
                ..Filter::default()
            },
            WINDOW_EVENTS,
        )?);
    }
    for term in rank::search_terms(&clues).into_iter().take(MAX_SEARCHES) {
        add(src.events(
            &Filter {
                ql: ql_phrase(&term),
                ..Filter::default()
            },
            CLUE_EVENTS,
        )?);
    }
    // No time: the warnings and errors of the sources the title names.
    let title_sources = rank::title_sources(&clues, &sources);
    if image_time.is_none() && !title_sources.is_empty() {
        add(src.events(
            &Filter {
                sources: title_sources,
                levels: vec![Level::Warning, Level::Error, Level::Fatal],
                ..Filter::default()
            },
            CLUE_EVENTS,
        )?);
    }
    let examined = candidates.len();
    let ranked = rank::rank(
        candidates.into_values().collect(),
        &clues,
        image_time.as_ref(),
        window,
        &sources,
        req.prefer_errors,
    );
    match &image_time {
        None => notes.push(RelatedNote::NoTime),
        Some(t) => {
            if let Some(n) = rank::clock_disagreement(&ranked, t, window) {
                notes.push(n);
            }
            let outside = data_from.is_some_and(|f| t.at_ms < f.saturating_sub(window))
                || data_to.is_some_and(|to| t.at_ms > to.saturating_add(window));
            if outside {
                notes.push(RelatedNote::OutsideData {
                    from: data_from,
                    to: data_to,
                });
            }
        }
    }
    let mut links = ranked;
    links.truncate(limit);
    Ok(RelatedResult {
        time: image_time,
        links,
        notes,
        examined,
    })
}

#[cfg(test)]
mod tests;
