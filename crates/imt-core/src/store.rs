//! The storage seam between the UI shell and the engine that holds the records.
//! [`Workspace`] is the in-memory implementation; another backend (for example
//! DuckDB) can be swapped in by implementing [`LogStore`].

use crate::analysis::{Anomaly, Comparison, FieldStats, RequestSpan, WaterfallRow};
use crate::api::{
    ChartData, ChartRequest, ContextRequest, ExportRequest, ExportResult, ExtractPreview, ExtractPreviewRequest, Facet, FacetRequest, Page,
    PageRequest, SourceContext, Timeline, TimelineRequest, WorkspaceState, WorkspaceSummary,
};
use crate::api::{
    CompareRequest, CompletionRequest, CorrelateRequest, FieldStatsRequest, MomentRequest, MomentSummary, QueryRequest, RequestsRequest,
    TraceRequest, TraceSummary,
};
use crate::exceptions::ExceptionGroup;
use crate::patterns::Pattern;
use crate::profile::Profile;
use crate::ql::{Completion, QlError, QlResult};
use crate::workspace::Workspace;
use anyhow::Result;
use std::path::PathBuf;

/// Everything the UI shell needs from a log store.
pub trait LogStore: Send + Sync {
    /// Opens files and folders (recursively), skipping files already loaded.
    fn open(&mut self, paths: &[PathBuf]) -> Result<()>;

    /// Re-imports every opened root with the current profile.
    fn reload(&mut self) -> Result<()>;

    /// Forgets all opened files and records.
    fn clear(&mut self);

    /// Drops one source (by id) and its records; folder scans will not bring it back.
    fn remove_source(&mut self, id: u32);

    /// Replaces the profile and re-imports everything.
    fn set_profile(&mut self, profile: Profile) -> Result<()>;

    /// The active profile.
    fn profile(&self) -> &Profile;

    /// Sources, fields, totals and chart definitions of what is loaded.
    fn summary(&self) -> WorkspaceSummary;

    /// One page of the records matching a query.
    fn page(&self, req: &PageRequest) -> Result<Page<'_>>;

    /// Instance lanes and histogram for a query.
    fn timeline(&self, req: &TimelineRequest) -> Result<Timeline>;

    /// Value counts of one field over the records matching a query.
    fn facet(&self, req: &FacetRequest) -> Result<Facet>;

    /// Time-bucketed series for a chart definition.
    fn chart(&self, req: &ChartRequest) -> Result<ChartData>;

    /// Runs one extractor of an (unsaved) profile against the current records.
    fn preview_extractor(&self, req: &ExtractPreviewRequest) -> ExtractPreview;

    /// Checks that a profile compiles against the installed plugins.
    fn validate_profile(&self, profile: Profile) -> Result<()>;

    /// Writes the records matching a query with a registered exporter.
    fn export(&self, req: &ExportRequest) -> Result<ExportResult>;

    /// The original lines around an event.
    fn source_context(&self, req: &ContextRequest) -> Result<SourceContext>;

    /// Opened paths and removed files (for saving a project).
    fn state(&self) -> WorkspaceState;

    /// Re-imports changed files and imports new files under opened folders.
    fn refresh_files(&mut self, changed: &[PathBuf]) -> Result<usize>;

    /// Rebuilds the workspace from a saved state; returns paths that no longer exist.
    fn restore(&mut self, profile: Profile, state: &WorkspaceState) -> Result<Vec<String>>;

    /// Runs a query-language program (`stats`, `sort`, ...) over the matching records.
    fn run_query(&self, req: &QueryRequest) -> std::result::Result<QlResult, QlError>;

    /// Parse error of a query-language text, if any.
    fn check_query(&self, text: &str) -> Option<QlError>;

    /// Completions at a cursor position in a query-language text.
    fn complete_query(&self, req: &CompletionRequest) -> Vec<Completion>;

    /// Message templates of the matching records, most frequent first.
    fn patterns(&self, req: &QueryRequest) -> Result<Vec<Pattern>>;

    /// Exceptions of the matching records grouped by fingerprint.
    fn exception_groups(&self, req: &QueryRequest) -> Result<Vec<ExceptionGroup>>;

    /// Error spikes, crashes, silences, outliers, rate changes and memory growth.
    fn anomalies(&self, req: &QueryRequest) -> Result<Vec<Anomaly>>;

    /// What changed between two sets of records (e.g. before/after a deploy).
    fn compare(&self, req: &CompareRequest) -> Result<Comparison>;

    /// Distribution of one field over the matching records.
    fn field_stats(&self, req: &FieldStatsRequest) -> Result<FieldStats>;

    /// Ids of every event with `field = value`, across all sources.
    fn correlate(&self, req: &CorrelateRequest) -> Vec<u32>;

    /// Traces (no trace id) or one trace's waterfall.
    fn traces(&self, req: &TraceRequest) -> Result<(Vec<TraceSummary>, Vec<WaterfallRow>)>;

    /// Requests grouped by an id field, longest first.
    fn requests(&self, req: &RequestsRequest) -> Result<Vec<RequestSpan>>;

    /// What was happening in a time window, across every source.
    fn moment(&self, req: &MomentRequest) -> Result<MomentSummary>;

    /// Parses sample text with a candidate profile, without changing the workspace.
    fn preview_parse(&self, req: &crate::api::ParsePreviewRequest) -> crate::api::ParsePreview;
}

impl LogStore for Workspace {
    fn open(&mut self, paths: &[PathBuf]) -> Result<()> {
        Workspace::open(self, paths)
    }

    fn reload(&mut self) -> Result<()> {
        Workspace::reload(self)
    }

    fn clear(&mut self) {
        Workspace::clear(self)
    }

    fn remove_source(&mut self, id: u32) {
        Workspace::remove_source(self, id)
    }

    fn set_profile(&mut self, profile: Profile) -> Result<()> {
        Workspace::set_profile(self, profile)
    }

    fn profile(&self) -> &Profile {
        Workspace::profile(self)
    }

    fn summary(&self) -> WorkspaceSummary {
        Workspace::summary(self)
    }

    fn page(&self, req: &PageRequest) -> Result<Page<'_>> {
        Workspace::page(self, req)
    }

    fn timeline(&self, req: &TimelineRequest) -> Result<Timeline> {
        Workspace::timeline(self, req)
    }

    fn facet(&self, req: &FacetRequest) -> Result<Facet> {
        Workspace::facet(self, req)
    }

    fn chart(&self, req: &ChartRequest) -> Result<ChartData> {
        Workspace::chart(self, req)
    }

    fn preview_extractor(&self, req: &ExtractPreviewRequest) -> ExtractPreview {
        Workspace::preview_extractor(self, req)
    }

    fn validate_profile(&self, profile: Profile) -> Result<()> {
        Workspace::validate_profile(self, profile)
    }

    fn export(&self, req: &ExportRequest) -> Result<ExportResult> {
        Workspace::export(self, req)
    }

    fn source_context(&self, req: &ContextRequest) -> Result<SourceContext> {
        Workspace::source_context(self, req)
    }

    fn state(&self) -> WorkspaceState {
        Workspace::state(self)
    }

    fn refresh_files(&mut self, changed: &[PathBuf]) -> Result<usize> {
        Workspace::refresh_files(self, changed)
    }

    fn restore(&mut self, profile: Profile, state: &WorkspaceState) -> Result<Vec<String>> {
        Workspace::restore(self, profile, state)
    }

    fn run_query(&self, req: &QueryRequest) -> std::result::Result<QlResult, QlError> {
        Workspace::run_query(self, req)
    }

    fn check_query(&self, text: &str) -> Option<QlError> {
        Workspace::check_query(self, text)
    }

    fn complete_query(&self, req: &CompletionRequest) -> Vec<Completion> {
        Workspace::complete_query(self, req)
    }

    fn patterns(&self, req: &QueryRequest) -> Result<Vec<Pattern>> {
        Workspace::patterns(self, req)
    }

    fn exception_groups(&self, req: &QueryRequest) -> Result<Vec<ExceptionGroup>> {
        Workspace::exception_groups(self, req)
    }

    fn anomalies(&self, req: &QueryRequest) -> Result<Vec<Anomaly>> {
        Workspace::anomalies(self, req)
    }

    fn compare(&self, req: &CompareRequest) -> Result<Comparison> {
        Workspace::compare(self, req)
    }

    fn field_stats(&self, req: &FieldStatsRequest) -> Result<FieldStats> {
        Workspace::field_stats(self, req)
    }

    fn correlate(&self, req: &CorrelateRequest) -> Vec<u32> {
        Workspace::correlate(self, req)
    }

    fn traces(&self, req: &TraceRequest) -> Result<(Vec<TraceSummary>, Vec<WaterfallRow>)> {
        Workspace::traces(self, req)
    }

    fn requests(&self, req: &RequestsRequest) -> Result<Vec<RequestSpan>> {
        Workspace::requests(self, req)
    }

    fn moment(&self, req: &MomentRequest) -> Result<MomentSummary> {
        Workspace::moment(self, req)
    }

    fn preview_parse(&self, req: &crate::api::ParsePreviewRequest) -> crate::api::ParsePreview {
        Workspace::preview_parse(self, req)
    }
}
