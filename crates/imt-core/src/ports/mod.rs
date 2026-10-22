//! The store's ports (ADR 0014 §1), split by responsibility so a consumer depends only on what it
//! uses: [`Ingest`], [`Query`], [`Positions`], [`Analysis`], [`Profiles`], [`Diagnostics`], [`Search`], and for the
//! Store pane [`Inspect`], [`Maintain`] and [`Console`] (ADR 0030).
//!
//! Contract of every port:
//! - methods take `&self`; implementations are `Send + Sync` and synchronise internally (readers run
//!   while an import writes);
//! - requests and replies are owned values (nothing borrows from the store);
//! - errors are a typed [`StoreError`]; an operation an adapter does not offer yet fails with
//!   [`StoreError::Unsupported`] naming the phase that adds it, never with a panic.
//!
//! [`Store`] is all of them together (plus [`Projects`]): what one window of the app holds.

mod console;
mod dto;
mod inspect;
mod live;
mod session;

pub use console::{
    SqlColumn, SqlNote, SqlPage, SqlPageRequest, SqlRequest, SqlResult, SqlSchema, SqlView, SqlViewColumn, MAX_CELL_CHARS, MAX_SQL_LEN,
    SQL_MAX_NOTES, SQL_MAX_RESULT_BYTES, SQL_MAX_ROWS, SQL_MEMORY_LIMIT, SQL_NOTE_CHARS, SQL_PAGE_ROWS, SQL_THREADS, SQL_TIME_LIMIT,
    SQL_VIEWS_VERSION,
};
pub use console::{
    SqlExportRequest, SqlExportResult, SqlIds, SqlIdsRequest, SQL_EXPORT_MAX_BYTES, SQL_EXPORT_MAX_ROWS, SQL_EXPORT_TIME_LIMIT,
};
pub use inspect::{
    parse_size, FieldStorage, FieldType, LastImport, MaintenanceAction, MaintenanceReport, RunningJob, SegmentRow, SegmentsPage,
    SegmentsRequest, StoreActivity, StoreField, StoreFields, StoreIndex, StoreMemory, StoreOverview, WorkerMemory, IMPORT_BUDGET_BYTES,
    MAX_FIELD_ROWS, MAX_SEGMENT_ROWS, QUERY_BUDGET_BYTES, SEGMENT_SOURCES,
};

pub use dto::{
    content_hash, mark_search_matches, Analyzed, Coverage, DiagnosticFix, DiagnosticRow, EventView, EventsRequest, FixEvaluation,
    ImportReport, IndexStatus, PageQuery, PageReply, ParserChange, ParserDef, ParserRef, ParserUse, PartialMatches, QuickFix, SearchMatch,
    SearchReply, SearchRequest, SummaryReply, MAX_PAGE_ROWS,
};
pub use live::{NewEvents, TailAppend, TailFile, TailRead};
pub use session::ImportSession;

use crate::analysis::{Anomaly, Comparison, FieldStats, RequestSpan, WaterfallRow};
use crate::api::{
    ChartData, ChartRequest, CompareRequest, CompletionRequest, ContextRequest, CorrelateRequest, ExportRequest, ExportResult,
    ExtractPreview, ExtractPreviewRequest, Facet, FacetRequest, FieldStatsRequest, Finding, MomentRequest, MomentSummary, ParsePreview,
    ParsePreviewRequest, QueryRequest, RequestsRequest, SourceContext, Timeline, TimelineRequest, TraceRequest, TraceSummary,
    WorkspaceState,
};
use crate::batch::{EventBatch, EventId, SourceId};
use crate::exceptions::ExceptionGroup;
use crate::patterns::Pattern;
use crate::profile::Profile;
use crate::ql::{Completion, QlError, QlResult, QueryControl};
use std::ops::ControlFlow;
use std::path::PathBuf;

/// The event filter of every query (time, levels, text, fields, instances, markers, sources, QL).
pub use crate::query::Query as Filter;

/// Why a store operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// The adapter does not offer this operation yet.
    #[error("{operation} is not available in this store yet (planned for phase {phase})")]
    Unsupported {
        operation: std::borrow::Cow<'static, str>,
        phase: std::borrow::Cow<'static, str>,
    },
    /// The request is invalid (a bad regex, an unknown field type...); the message says what to fix.
    #[error("{0}")]
    Invalid(String),
    /// The query-language text is invalid (with the span to underline).
    #[error("query: {}", .0.message)]
    Query(QlError),
    #[error("not found: {0}")]
    NotFound(String),
    /// Another operation holds what this one needs (e.g. an import is running); try again later.
    #[error("busy: {0}")]
    Busy(String),
    #[error("cancelled")]
    Cancelled,
    /// Reading or writing files failed.
    #[error("{0}")]
    Io(String),
    /// The database engine failed.
    #[error("store: {0}")]
    Engine(String),
    /// The store's files are damaged (a segment missing or unreadable): retrying cannot help; the message says
    /// how to recover (open the logs again, or the project's `.imtz`).
    #[error("{0}")]
    Damaged(String),
    /// The store is open in another process (its database file is locked); retry after closing it there.
    #[error("{0}")]
    Locked(String),
}

impl StoreError {
    pub fn unsupported(operation: &'static str, phase: &'static str) -> StoreError {
        StoreError::Unsupported {
            operation: operation.into(),
            phase: phase.into(),
        }
    }

    /// Whether repeating the request later can succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            StoreError::Busy(_) | StoreError::Io(_) | StoreError::Engine(_) | StoreError::Locked(_)
        )
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e.to_string())
    }
}

pub type StoreResult<T> = std::result::Result<T, StoreError>;

/// Writing: opening, reloading and removing data. Imports run in the background as an
/// [`ImportSession`]; readers are not blocked meanwhile.
pub trait Ingest: Send + Sync {
    /// Imports files and folders (recursively); files already read are skipped.
    fn open(&self, paths: &[PathBuf]) -> StoreResult<ImportSession>;
    /// Re-imports every opened root.
    fn reload(&self) -> StoreResult<ImportSession>;
    /// Re-reads changed files and reads new files under opened folders.
    fn refresh(&self, changed: &[PathBuf]) -> StoreResult<ImportSession>;
    /// Drops one source and its events; folder scans will not bring it back.
    fn remove_source(&self, id: SourceId) -> StoreResult<()>;
    /// Forgets every source and event.
    fn clear(&self) -> StoreResult<()>;
    /// Replaces everything with a saved state.
    fn restore(&self, state: &WorkspaceState) -> StoreResult<ImportSession>;
    /// Live tail: reads what was appended to followed files (with an empty list: only what is due, such as a
    /// held-back last event of a file that stopped growing).
    fn append_tail(&self, changed: &[PathBuf]) -> StoreResult<TailAppend>;
}

/// Reading: pages, aggregates and batch scans over the events matching a [`Filter`].
pub trait Query: Send + Sync {
    /// Sources, totals, fields and marker kinds.
    fn summary(&self) -> StoreResult<SummaryReply>;
    /// One page of matching events, in time order (untimed last).
    fn page(&self, req: &PageQuery) -> StoreResult<PageReply>;
    /// Events by id, in the order asked (missing ids left out).
    fn events(&self, req: &EventsRequest) -> StoreResult<Vec<EventView>>;
    /// Instance lanes and the level histogram (timed events only).
    fn timeline(&self, req: &TimelineRequest) -> StoreResult<Timeline>;
    /// Value counts of one field.
    fn facet(&self, req: &FacetRequest) -> StoreResult<Facet>;
    /// Every matching event in time order, batch by batch, until `visit` breaks. The batches leave the
    /// wide `raw` column empty (re-deriving from `raw` is the store's own job, see [`Profiles`]).
    fn scan(&self, filter: &Filter, visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()>;
    fn chart(&self, req: &ChartRequest) -> StoreResult<ChartData>;
    fn field_stats(&self, req: &FieldStatsRequest) -> StoreResult<FieldStats>;
    /// The original lines around an event.
    fn source_context(&self, req: &ContextRequest) -> StoreResult<SourceContext>;
    fn run_query(&self, req: &QueryRequest) -> StoreResult<QlResult> {
        self.run_query_with(req, &QueryControl::new())
    }
    /// [`run_query`](Self::run_query), stopped (with [`StoreError::Cancelled`]) when `control` is cancelled.
    fn run_query_with(&self, req: &QueryRequest, control: &QueryControl) -> StoreResult<QlResult>;
    fn check_query(&self, text: &str) -> StoreResult<Option<QlError>>;
    fn complete_query(&self, req: &CompletionRequest) -> StoreResult<Vec<Completion>>;
    fn export(&self, req: &ExportRequest) -> StoreResult<ExportResult>;
}

/// Time-order positions of all events (unfiltered): position `p` is the `p`-th event in time order
/// (untimed last), so an unfiltered page or an anchor is a range lookup instead of a sort. Positions are
/// renumbered after imports and removals; events imported since have none yet.
pub trait Positions: Send + Sync {
    /// Ids of the events at positions `from .. from + len` (at most [`MAX_PAGE_ROWS`]), in time order.
    fn ids_at(&self, from: u64, len: usize) -> StoreResult<Vec<EventId>>;
    /// The position of an event, if it has one.
    fn position_of(&self, id: EventId) -> StoreResult<Option<u64>>;
    /// Events that have a position.
    fn positioned(&self) -> StoreResult<u64>;
}

/// Analyses that need code over event batches (patterns, exceptions, anomalies...). Each result says
/// how much of the matching data it covers ([`Coverage`]).
pub trait Analysis: Send + Sync {
    fn patterns(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Pattern>>>;
    fn exception_groups(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<ExceptionGroup>>>;
    fn anomalies(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Anomaly>>>;
    fn compare(&self, req: &CompareRequest) -> StoreResult<Analyzed<Comparison>>;
    /// Every event (whatever the query) whose field equals the value, in time order.
    fn correlate(&self, req: &CorrelateRequest) -> StoreResult<Analyzed<Vec<EventId>>>;
    fn traces(&self, req: &TraceRequest) -> StoreResult<Analyzed<(Vec<TraceSummary>, Vec<WaterfallRow>)>>;
    fn requests(&self, req: &RequestsRequest) -> StoreResult<Analyzed<Vec<RequestSpan>>>;
    fn moment(&self, req: &MomentRequest) -> StoreResult<Analyzed<MomentSummary>>;
    fn overview(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Finding>>>;
    fn runs(&self, req: &crate::spans::RunsRequest) -> StoreResult<crate::spans::RunsReply>;
}

/// The active profile, profile changes and previews.
pub trait Profiles: Send + Sync {
    fn active_profile(&self) -> StoreResult<Profile>;
    /// Checks that a profile compiles (with the installed plugins).
    fn validate_profile(&self, profile: &Profile) -> StoreResult<()>;
    /// Switches profile: a job that re-derives or re-reads what the change needs (only the sources whose parser
    /// changed).
    fn set_profile(&self, profile: Profile) -> StoreResult<ImportSession>;
    /// Which parser each source was read with (ADR 0026).
    fn parsers(&self) -> StoreResult<Vec<ParserUse>>;
    /// Replaces the parser library (the store keeps its per-source choices); nothing is read again. Returns
    /// the sources read with a parser that is no longer the one the library reads them with.
    fn set_parsers(&self, setup: &crate::parser::ParserSetup) -> StoreResult<Vec<SourceId>>;
    /// Reads again the sources [`Profiles::set_parsers`] called stale and retries the files no parser read.
    fn reread_parsers(&self) -> StoreResult<ImportSession>;
    /// What changing the library's parser `parser.id` to `parser` would do: the sources it read again.
    fn classify_parser(&self, parser: &ParserDef) -> StoreResult<ParserChange>;
    /// "Parse with…": reads one source with the library's parser `parser` (`None`: the automatic choice).
    fn assign_parser(&self, source: SourceId, parser: Option<&str>) -> StoreResult<ImportSession>;
    fn preview_parse(&self, req: &ParsePreviewRequest) -> StoreResult<ParsePreview>;
    fn preview_extractor(&self, req: &ExtractPreviewRequest) -> StoreResult<ExtractPreview>;
    /// Proposes `[[span]]` rules that pair the example start/end events, each evaluated on the data.
    fn propose_span_rule(&self, req: &crate::spans::infer::SpanRuleRequest) -> StoreResult<crate::spans::infer::SpanProposal>;
    /// The Runs editor's preview of a draft rule: checked against the profile and paired over the events.
    fn preview_spans(&self, req: &crate::spans::SpanPreviewRequest) -> StoreResult<crate::spans::SpanPreview>;
    /// What a rule (a profile override adding a `[[span]]`) does on the data; verifies AI-proposed rules.
    fn evaluate_span_rule(&self, req: &crate::spans::infer::SpanRuleCheck) -> StoreResult<Analyzed<crate::spans::infer::SpanEvaluation>>;
}

/// Import problems and engine-verified quick fixes (ADR 0014 §5).
pub trait Diagnostics: Send + Sync {
    fn problems(&self) -> StoreResult<Vec<DiagnosticRow>>;
    /// How many affected events a proposed fix would repair, checked on the stored samples.
    fn evaluate_fix(&self, fix: &QuickFix) -> StoreResult<FixEvaluation>;
}

/// Full-text search (Tantivy), returning event ids.
pub trait Search: Send + Sync {
    /// Word search (smart search, see [`crate::words`]): the hits in time order.
    fn search(&self, req: &SearchRequest) -> StoreResult<SearchReply>;
    fn index_status(&self) -> StoreResult<IndexStatus>;
    /// The background scan of a smart search: how many events matching `filter` with the search box text as
    /// a substring are not word matches. Cancelled through [`QueryControl::current`]; bounded.
    fn partial_matches(&self, filter: &Filter) -> StoreResult<PartialMatches>;
}

/// The store's folder as a project (ADR 0014 §6): `project.toml` and the `.imtz` export.
pub trait Projects: Send + Sync {
    /// The folder holding the store (and its `project.toml`).
    fn store_dir(&self) -> PathBuf;
    /// Writes `project.toml` with `manifest` and the store's opened roots; returns what was written.
    fn save_project(&self, manifest: &crate::package::ProjectManifest) -> StoreResult<crate::package::ProjectManifest>;
    /// Saves the project and writes it as an `.imtz` to `out` (no caches); returns the bytes written.
    fn export_project(&self, manifest: &crate::package::ProjectManifest, out: &std::path::Path) -> StoreResult<u64>;
    /// Lets go of the store's files (its window closed or opened another store); a later call opens them
    /// again. A store that holds nothing it could let go of does nothing.
    fn close(&self) {}
}

/// Read-only reports about the store itself (the Store pane). Queries: no side effects. A store that cannot
/// report them answers [`StoreError::Unsupported`].
pub trait Inspect: Send + Sync {
    fn store_overview(&self) -> StoreResult<StoreOverview> {
        Err(StoreError::unsupported("the store overview", "ADR 0030"))
    }
    fn store_segments(&self, _req: &SegmentsRequest) -> StoreResult<SegmentsPage> {
        Err(StoreError::unsupported("the segment list", "ADR 0030"))
    }
    fn store_fields(&self) -> StoreResult<StoreFields> {
        Err(StoreError::unsupported("the field list", "ADR 0030"))
    }
    fn store_index(&self) -> StoreResult<StoreIndex> {
        Err(StoreError::unsupported("the full-text index report", "ADR 0030"))
    }
    /// The store's memory; [`StoreMemory::worker`] and [`StoreMemory::running_jobs`] are left for the worker and
    /// the app to fill.
    fn store_memory(&self) -> StoreResult<StoreMemory> {
        Err(StoreError::unsupported("the memory report", "ADR 0030"))
    }
}

/// Maintenance of the store (Store pane actions, run as jobs). Fails with [`StoreError::Busy`] while an import
/// runs.
pub trait Maintain: Send + Sync {
    fn maintain(&self, _action: MaintenanceAction) -> StoreResult<MaintenanceReport> {
        Err(StoreError::unsupported("store maintenance", "ADR 0030"))
    }
}

/// The read-only SQL console. A refused or invalid SQL text fails with [`StoreError::Query`] (the span to
/// underline); a run over its time or memory limit with [`StoreError::Invalid`] saying which; a cancelled run
/// (the thread's [`QueryControl`]) with [`StoreError::Cancelled`].
pub trait Console: Send + Sync {
    fn sql_schema(&self) -> StoreResult<SqlSchema> {
        Err(StoreError::unsupported("the SQL console", "ADR 0030"))
    }
    fn sql_run(&self, _req: &SqlRequest) -> StoreResult<SqlResult> {
        Err(StoreError::unsupported("the SQL console", "ADR 0030"))
    }
    fn sql_page(&self, _req: &SqlPageRequest) -> StoreResult<SqlPage> {
        Err(StoreError::unsupported("the SQL console", "ADR 0030"))
    }
    /// The event ids of a kept result (*Open in Events* on all its rows); [`StoreError::NotFound`] when the result
    /// is gone, [`StoreError::Invalid`] when it has no `id` column of an integer type.
    fn sql_ids(&self, _req: &SqlIdsRequest) -> StoreResult<SqlIds> {
        Err(StoreError::unsupported("the SQL console's event ids", "ADR 0030"))
    }
    /// Runs the query again and streams its rows as CSV to the request's file (bounded by
    /// [`SQL_EXPORT_MAX_ROWS`], [`SQL_EXPORT_MAX_BYTES`] and [`SQL_EXPORT_TIME_LIMIT`]; cancelled with the thread's
    /// [`QueryControl`]). An existing file is replaced only when the request says so; a failed or cancelled export
    /// leaves no file behind.
    fn sql_export(&self, _req: &SqlExportRequest) -> StoreResult<SqlExportResult> {
        Err(StoreError::unsupported("CSV export of the SQL console", "ADR 0030"))
    }
}

/// What redaction would change in the events of a query (shown before an export), with the profile's `[redact]`
/// rules. The redacted view itself is applied to replies by their caller ([`crate::redact::view`]). Cancellable through
/// the thread's [`QueryControl`].
pub trait Redaction: Send + Sync {
    fn redaction_summary(&self, _req: &crate::redact::RedactionSummaryRequest) -> StoreResult<crate::redact::RedactionSummary> {
        Err(StoreError::unsupported("the redaction summary", "ADR 0014 6c"))
    }
}

/// Clock-skew correction (see [`crate::skew`]): per-file time offsets, kept by the store (with the project) and
/// applied to the events' timestamps, so pages, timelines, facets, analyses and search all see the corrected
/// times. A refused offset (too large, an unknown source or event) fails with [`StoreError::Invalid`] saying what
/// to fix.
pub trait Skew: Send + Sync {
    /// The offsets by file path.
    fn clock_offsets(&self) -> StoreResult<crate::skew::ClockOffsets> {
        Err(StoreError::unsupported("clock-skew correction", "6c"))
    }
    /// Replaces the offsets (a project's or a session's; out-of-range ones are dropped): files read later take
    /// them, and the events of files already read move.
    fn set_clock_offsets(&self, _offsets: &crate::skew::ClockOffsets) -> StoreResult<()> {
        Err(StoreError::unsupported("clock-skew correction", "6c"))
    }
    /// Sets the offset of a source's file (manually or by aligning two events) and moves its events.
    fn set_source_offset(&self, _req: &crate::skew::SetOffsetRequest) -> StoreResult<crate::skew::OffsetChange> {
        Err(StoreError::unsupported("clock-skew correction", "6c"))
    }
    /// Offsets that would align each source with a reference, from events sharing a correlation id (at most
    /// [`crate::skew::MAX_IDS`] ids).
    fn suggest_offsets(&self, _req: &crate::skew::SuggestRequest) -> StoreResult<crate::skew::SuggestResult> {
        Err(StoreError::unsupported("clock-skew correction", "6c"))
    }
}

/// Live tail: how the store's files were last read (a query) and opening files at their end.
pub trait Tail: Send + Sync {
    /// The followed files as the store last read them.
    fn tail_files(&self) -> StoreResult<Vec<TailFile>> {
        Err(StoreError::unsupported("the live tail state", "6c"))
    }
    /// Opens files and folders for live tail at their current end: text files are followed from there (only lines
    /// written from now on); files that cannot be read from the middle are read in full (with a note).
    fn open_at_end(&self, _paths: &[PathBuf]) -> StoreResult<ImportSession> {
        Err(StoreError::unsupported("opening files at their end", "6c"))
    }
}

/// Every port of a store: what one window of the app holds (the DuckDB store in the store worker; in process
/// in tests).
pub trait Store:
    Ingest + Query + Positions + Analysis + Profiles + Diagnostics + Search + Projects + Inspect + Maintain + Console + Redaction + Skew + Tail
{
}

impl<
        T: Ingest
            + Query
            + Positions
            + Analysis
            + Profiles
            + Diagnostics
            + Search
            + Projects
            + Inspect
            + Maintain
            + Console
            + Redaction
            + Skew
            + Tail,
    > Store for T
{
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_say_what_happened() {
        let e = StoreError::unsupported("chart", "R2c");
        assert_eq!(e.to_string(), "chart is not available in this store yet (planned for phase R2c)");
        assert!(!e.is_retryable());
        assert!(StoreError::Busy("import".into()).is_retryable());
        let io: StoreError = std::io::Error::other("disk").into();
        assert_eq!(io, StoreError::Io("disk".into()));
        assert!(io.is_retryable());
        assert_eq!(StoreError::Invalid("bad regex".into()).to_string(), "bad regex");
        assert_eq!(StoreError::NotFound("source 3".into()).to_string(), "not found: source 3");
        assert_eq!(StoreError::Cancelled.to_string(), "cancelled");
        assert_eq!(StoreError::Engine("x".into()).to_string(), "store: x");
    }
}
