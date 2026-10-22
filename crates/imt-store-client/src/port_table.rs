//! The one declarative source of the store ports on the wire (ADR 0040).
//!
//! [`store_port_table!`] lists, per port trait, every method that crosses to the store worker. Three callbacks
//! expand the table: [`crate::protocol::StoreOp`] (the operation enum), the client's `impl Port for RemoteStore`
//! blocks ([`crate::client`]) and the worker's dispatch (`imt-store-worker`). A new port method is **one row here**
//! plus the trait method and the engine's implementation; nothing else lists operations.
//!
//! Row kinds, `plain { ... }` (one request DTO or none, one reply DTO, no code to write):
//!
//! ```text
//! call     Op: fn method(req: &Request) -> Reply;   // an ordinary call (CALL_TIMEOUT)
//! long     Op: fn method(req: &Request) -> Reply;   // a long call (LONG_TIMEOUT)
//! analysis Op: fn method(req: &Request) -> Reply;   // cancellable, streams progress, takes the thread's QueryControl
//! ```
//!
//! `custom { ... }` rows name hand-written handlers where the method is not a plain DTO call (import sessions,
//! several arguments, state the client remembers): `Op: fn method(args) -> Reply = client remote_x, server serve_x;`.
//!
//! The wire format is the order of the rows ([`WIRE_VERSION`], [`wire_names`]): append new rows at the end of the
//! table (inside any trait) only when nothing else changes; any reorder or removal bumps [`WIRE_VERSION`].

/// The worker protocol's version. Bump it when an operation is removed, reordered or its DTO changes shape; the
/// app and its worker ship together, and the client refuses a worker that reports another version.
pub const WIRE_VERSION: u32 = 1;

/// The names the table's types resolve through (the callbacks expand in other modules and crates).
pub mod ty {
    pub use imt_core::analysis::{Anomaly, Comparison, FieldStats, RequestSpan, WaterfallRow};
    pub use imt_core::api::{
        ChartData, ChartRequest, CompareRequest, CompletionRequest, ContextRequest, CorrelateRequest, ExportRequest, ExportResult,
        ExtractPreview, ExtractPreviewRequest, Facet, FacetRequest, FieldStatsRequest, Finding, MomentRequest, MomentSummary, ParsePreview,
        ParsePreviewRequest, QueryRequest, RequestsRequest, SourceContext, Timeline, TimelineRequest, TraceRequest, TraceSummary,
        WorkspaceState,
    };
    pub use imt_core::batch::{EventId, SourceId};
    pub use imt_core::exceptions::ExceptionGroup;
    pub use imt_core::package::ProjectManifest;
    pub use imt_core::parser::ParserSetup;
    pub use imt_core::patterns::Pattern;
    pub use imt_core::ports::{
        Analysis, Analyzed, Console, DiagnosticRow, Diagnostics, EventView, EventsRequest, Filter, FixEvaluation, ImportSession,
        IndexStatus, Ingest, Inspect, Maintain, MaintenanceAction, MaintenanceReport, PageQuery, PageReply, ParserChange, ParserDef,
        ParserUse, PartialMatches, Positions, Profiles, Projects, Query, QuickFix, Redaction, Search, SearchReply, SearchRequest,
        SegmentsPage, SegmentsRequest, Skew, SqlExportRequest, SqlExportResult, SqlIds, SqlIdsRequest, SqlPage, SqlPageRequest, SqlRequest,
        SqlResult, SqlSchema, StoreError, StoreFields, StoreIndex, StoreMemory, StoreOverview, StoreResult, SummaryReply, Tail, TailAppend,
        TailFile,
    };
    pub use imt_core::ql::{Completion, QlError, QlResult, QueryControl};
    pub use imt_core::redact::copy::{RedactedExport, RedactedExportRequest};
    pub use imt_core::redact::{RedactionSummary, RedactionSummaryRequest};
    pub use imt_core::skew::{ClockOffsets, OffsetChange, SetOffsetRequest, SuggestRequest, SuggestResult};
    pub use imt_core::spans::infer::{SpanEvaluation, SpanProposal, SpanRuleCheck, SpanRuleRequest};
    pub use imt_core::spans::{RunsReply, RunsRequest, SpanPreview, SpanPreviewRequest};
    pub use imt_core::Profile;
    pub use std::path::{Path, PathBuf};
}

/// Passes the table to the macro `$cb`, with `$ctx` (a bracketed token list) first.
#[macro_export]
macro_rules! store_port_table {
    ($cb:ident, [$($ctx:tt)*]) => {
        $cb! { [$($ctx)*]
            Ingest {
                plain {
                    call Clear: fn clear() -> ();
                }
                custom {
                    Open: fn open(paths: &[PathBuf]) -> ImportSession = client remote_open, server serve_open;
                    Reload: fn reload() -> ImportSession = client remote_reload, server serve_reload;
                    Refresh: fn refresh(changed: &[PathBuf]) -> ImportSession = client remote_refresh, server serve_refresh;
                    RemoveSource: fn remove_source(id: SourceId) -> () = client remote_remove_source, server serve_remove_source;
                    Restore: fn restore(state: &WorkspaceState) -> ImportSession = client remote_restore, server serve_restore;
                    AppendTail: fn append_tail(changed: &[PathBuf]) -> TailAppend = client remote_append_tail, server serve_append_tail;
                    SetSourceEncoding: fn set_source_encoding(source: SourceId, encoding: Option<&str>) -> ImportSession
                        = client remote_set_source_encoding, server serve_set_source_encoding;
                }
            }
            Query {
                plain {
                    call Summary: fn summary() -> SummaryReply;
                    call Page: fn page(req: &PageQuery) -> PageReply;
                    call Events: fn events(req: &EventsRequest) -> Vec<EventView>;
                    call Timeline: fn timeline(req: &TimelineRequest) -> Timeline;
                    call Facet: fn facet(req: &FacetRequest) -> Facet;
                    analysis Chart: fn chart(req: &ChartRequest) -> ChartData;
                    analysis FieldStats: fn field_stats(req: &FieldStatsRequest) -> FieldStats;
                    call SourceContext: fn source_context(req: &ContextRequest) -> SourceContext;
                    call CompleteQuery: fn complete_query(req: &CompletionRequest) -> Vec<Completion>;
                    analysis Export: fn export(req: &ExportRequest) -> ExportResult;
                }
                custom {
                    RunQuery: fn run_query_with(req: &QueryRequest, control: &QueryControl) -> QlResult
                        = client remote_run_query, server serve_run_query;
                    CheckQuery: fn check_query(text: &str) -> Option<QlError> = client remote_check_query, server serve_check_query;
                }
            }
            Positions {
                plain {
                    call Positioned: fn positioned() -> u64;
                }
                custom {
                    IdsAt: fn ids_at(from: u64, len: usize) -> Vec<EventId> = client remote_ids_at, server serve_ids_at;
                    PositionOf: fn position_of(id: EventId) -> Option<u64> = client remote_position_of, server serve_position_of;
                }
            }
            Analysis {
                plain {
                    analysis Patterns: fn patterns(req: &QueryRequest) -> Analyzed<Vec<Pattern>>;
                    analysis ExceptionGroups: fn exception_groups(req: &QueryRequest) -> Analyzed<Vec<ExceptionGroup>>;
                    analysis Anomalies: fn anomalies(req: &QueryRequest) -> Analyzed<Vec<Anomaly>>;
                    analysis Compare: fn compare(req: &CompareRequest) -> Analyzed<Comparison>;
                    analysis Correlate: fn correlate(req: &CorrelateRequest) -> Analyzed<Vec<EventId>>;
                    analysis Traces: fn traces(req: &TraceRequest) -> Analyzed<(Vec<TraceSummary>, Vec<WaterfallRow>)>;
                    analysis Requests: fn requests(req: &RequestsRequest) -> Analyzed<Vec<RequestSpan>>;
                    analysis Moment: fn moment(req: &MomentRequest) -> Analyzed<MomentSummary>;
                    analysis Overview: fn overview(req: &QueryRequest) -> Analyzed<Vec<Finding>>;
                    analysis Runs: fn runs(req: &RunsRequest) -> RunsReply;
                }
                custom {}
            }
            Profiles {
                plain {
                    call ActiveProfile: fn active_profile() -> Profile;
                    call ValidateProfile: fn validate_profile(profile: &Profile) -> ();
                    analysis PreviewParse: fn preview_parse(req: &ParsePreviewRequest) -> ParsePreview;
                    analysis PreviewExtractor: fn preview_extractor(req: &ExtractPreviewRequest) -> ExtractPreview;
                    call Parsers: fn parsers() -> Vec<ParserUse>;
                    call ClassifyParser: fn classify_parser(parser: &ParserDef) -> ParserChange;
                    analysis PreviewSpans: fn preview_spans(req: &SpanPreviewRequest) -> SpanPreview;
                    analysis ProposeSpanRule: fn propose_span_rule(req: &SpanRuleRequest) -> SpanProposal;
                    analysis EvaluateSpanRule: fn evaluate_span_rule(req: &SpanRuleCheck) -> Analyzed<SpanEvaluation>;
                }
                custom {
                    SetProfile: fn set_profile(profile: Profile) -> ImportSession = client remote_set_profile, server serve_set_profile;
                    SetParsers: fn set_parsers(setup: &ParserSetup) -> Vec<SourceId> = client remote_set_parsers, server serve_set_parsers;
                    RereadParsers: fn reread_parsers() -> ImportSession = client remote_reread_parsers, server serve_reread_parsers;
                    AssignParser: fn assign_parser(source: SourceId, parser: Option<&str>) -> ImportSession
                        = client remote_assign_parser, server serve_assign_parser;
                }
            }
            Diagnostics {
                plain {
                    call Problems: fn problems() -> Vec<DiagnosticRow>;
                    analysis EvaluateFix: fn evaluate_fix(fix: &QuickFix) -> FixEvaluation;
                }
                custom {}
            }
            Search {
                plain {
                    call Search: fn search(req: &SearchRequest) -> SearchReply;
                    call IndexStatus: fn index_status() -> IndexStatus;
                    analysis PartialMatches: fn partial_matches(filter: &Filter) -> PartialMatches;
                }
                custom {}
            }
            Projects {
                plain {
                    call SaveProject: fn save_project(manifest: &ProjectManifest) -> ProjectManifest;
                }
                custom {
                    ExportProject: fn export_project(manifest: &ProjectManifest, out: &Path) -> u64
                        = client remote_export_project, server serve_export_project;
                }
            }
            Inspect {
                plain {
                    call StoreOverview: fn store_overview() -> StoreOverview;
                    call StoreSegments: fn store_segments(req: &SegmentsRequest) -> SegmentsPage;
                    call StoreFields: fn store_fields() -> StoreFields;
                    call StoreIndex: fn store_index() -> StoreIndex;
                }
                custom {
                    StoreMemory: fn store_memory() -> StoreMemory = client remote_store_memory, server serve_store_memory;
                }
            }
            Maintain {
                plain {}
                custom {
                    StoreMaintain: fn maintain(action: MaintenanceAction) -> MaintenanceReport
                        = client remote_maintain, server serve_maintain;
                }
            }
            Console {
                plain {
                    call SqlSchema: fn sql_schema() -> SqlSchema;
                    analysis SqlRun: fn sql_run(req: &SqlRequest) -> SqlResult;
                    call SqlPage: fn sql_page(req: &SqlPageRequest) -> SqlPage;
                    call SqlIds: fn sql_ids(req: &SqlIdsRequest) -> SqlIds;
                    analysis SqlExport: fn sql_export(req: &SqlExportRequest) -> SqlExportResult;
                }
                custom {}
            }
            Redaction {
                plain {
                    analysis RedactionSummary: fn redaction_summary(req: &RedactionSummaryRequest) -> RedactionSummary;
                    long ExportRedactedProject: fn export_redacted_project(req: &RedactedExportRequest) -> RedactedExport;
                }
                custom {}
            }
            Skew {
                plain {
                    call ClockOffsets: fn clock_offsets() -> ClockOffsets;
                    long SetClockOffsets: fn set_clock_offsets(offsets: &ClockOffsets) -> ();
                    long SetSourceOffset: fn set_source_offset(req: &SetOffsetRequest) -> OffsetChange;
                    analysis SuggestOffsets: fn suggest_offsets(req: &SuggestRequest) -> SuggestResult;
                }
                custom {}
            }
            Tail {
                plain {
                    call TailFiles: fn tail_files() -> Vec<TailFile>;
                }
                custom {
                    OpenAtEnd: fn open_at_end(paths: &[PathBuf]) -> ImportSession = client remote_open_at_end, server serve_open_at_end;
                }
            }
            ;
            // Operations without a port method (the import session's handle); the worker serves them by hand.
            wire_only {
                SessionProgress = server serve_session_progress;
                SessionCancel = server serve_session_cancel;
                SessionWait = server serve_session_wait;
            }
        }
    };
}

/// Expands the table into the operation enum named `$name` (`ctx` = `[$name]`).
#[macro_export]
macro_rules! store_ops_enum {
    ([$name:ident]
     $( $trait:ident {
         plain { $( $kind:ident $pop:ident : fn $pm:ident ( $($pa:ident : & $pt:ty)? ) -> $pr:ty ; )* }
         custom { $( $cop:ident : fn $cm:ident ( $($ca:ident : $ct:ty),* ) -> $cr:ty = client $cc:ident, server $cs:ident ; )* }
     } )*
     ;
     wire_only { $( $wop:ident = server $ws:ident ; )* }
    ) => {
        /// An operation on one open store; its request and reply are the port method's DTOs (as JSON). Generated
        /// from [`store_port_table!`](crate::store_port_table), whose row order is the wire encoding.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        pub enum $name {
            $( $( $pop, )* $( $cop, )* )* $( $wop, )*
        }

        impl $name {
            /// Every operation, in wire order (tests: each one is dispatched).
            pub const ALL: &'static [$name] = &[ $( $( $name::$pop, )* $( $name::$cop, )* )* $( $name::$wop, )* ];

            /// The operation's name (logs).
            pub fn name(self) -> &'static str {
                match self {
                    $( $( $name::$pop => stringify!($pop), )* $( $name::$cop => stringify!($cop), )* )* $( $name::$wop => stringify!($wop), )*
                }
            }
        }
    };
}

/// The operation names in wire order (the wire format's snapshot; see [`WIRE_VERSION`]).
pub fn wire_names() -> Vec<&'static str> {
    crate::protocol::StoreOp::ALL.iter().map(|o| o.name()).collect()
}
