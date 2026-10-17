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

    /// Lifts a plugin's quarantine after repeated crashes.
    fn reset_plugin(&self, id: &str);

    /// Progress and cancellation of the current import (usable without the store lock).
    fn progress(&self) -> std::sync::Arc<crate::workspace::ImportProgress>;

    /// The automatic health check: findings, most severe first.
    fn overview(&self, req: &QueryRequest) -> Result<Vec<crate::api::Finding>>;

    /// Settings of the plugins this store runs, under the same crash
    /// containment (and health record) as the rest of their contributions.
    /// A store without plugins has none.
    fn plugin_settings(&self) -> Vec<crate::settings::PluginSettings> {
        Vec::new()
    }

    /// Opens files and folders; with `start_at_end` text files are tailed from
    /// their current end, skipping their history.
    fn open_with(&mut self, paths: &[PathBuf], options: &crate::workspace::OpenOptions) -> Result<()> {
        let _ = options;
        self.open(paths)
    }

    /// Live tail: reads what changed in `changed` (appends, rotations,
    /// truncations, deletions, new files) and reports it. Stores without
    /// incremental reading re-import and report nothing finer.
    fn refresh_tail(&mut self, changed: &[PathBuf]) -> Result<crate::workspace::TailReport> {
        self.refresh_files(changed).map(|_| crate::workspace::TailReport::default())
    }

    /// Per-file live tail state and totals.
    fn tail_status(&self) -> crate::workspace::TailStatus {
        crate::workspace::TailStatus::default()
    }

    /// Live tail options: deleted files, retention, carry timeout.
    fn set_tail_options(&mut self, options: crate::workspace::TailOptions) {
        let _ = options;
    }

    /// Mutes live alerts of one kind or rule until `until_ms` (wall clock, at
    /// most a day ahead): they are not raised meanwhile. Fails for an invalid
    /// mute. A store without live alerts has nothing to mute.
    fn mute_alerts(&mut self, target: crate::workspace::AlertTarget, until_ms: i64) -> Result<()> {
        let _ = (target, until_ms);
        Ok(())
    }

    /// Lifts a mute set with [`LogStore::mute_alerts`].
    fn unmute_alerts(&mut self, target: &crate::workspace::AlertTarget) {
        let _ = target;
    }

    /// Ranges and points found by plugin annotators (reboots, service
    /// uptime, ...) overlapping the request. A store without plugins has none.
    fn annotations(&self, req: &crate::plugin::annotate::AnnotationsRequest) -> Vec<crate::plugin::annotate::AnnotationDto> {
        let _ = req;
        Vec::new()
    }

    /// The event bus between the engine and its plugins, for publishing
    /// host events (e.g. [`crate::events::SnapshotTaken`]) and reading
    /// per-subscriber delivery metrics. A store without plugins has none.
    fn event_bus(&self) -> Option<std::sync::Arc<crate::events::EventBus>> {
        None
    }

    /// Deactivates the store's plugins, dependents first (on exit). Idempotent.
    fn shutdown_plugins(&self) {}

    /// Switches the redacted view on or off: while on, pages, facets and
    /// source context are redacted with the profile's `[redact]` rules.
    /// Fails when there is nothing to redact with, or the store cannot.
    fn set_view_redaction(&mut self, on: bool) -> Result<()> {
        if on {
            anyhow::bail!("this store has no redacted view");
        }
        Ok(())
    }

    /// A sample redacted with the profile's rules (or a candidate profile's).
    fn preview_redaction(&self, req: &crate::workspace::RedactionPreviewRequest) -> crate::redact::RedactionPreview {
        let cfg = match &req.toml {
            None => self.profile().redact.clone(),
            Some(t) => match Profile::from_toml(t) {
                Ok(p) => p.redact,
                Err(e) => {
                    return crate::redact::RedactionPreview {
                        before: req.sample.clone(),
                        after: req.sample.clone(),
                        hits: Vec::new(),
                        error: Some(e.to_string()),
                    }
                }
            },
        };
        crate::redact::preview(&cfg, &req.sample)
    }

    /// What redaction would change in the events of a query (before an export).
    fn redaction_summary(&self, req: &crate::workspace::RedactionSummaryRequest) -> Result<crate::workspace::RedactionSummary> {
        let _ = req;
        anyhow::bail!("this store cannot preview redaction")
    }

    /// Evaluates one dashboard tile.
    fn dashboard_tile(&self, req: &crate::dashboard::TileRequest) -> Result<crate::dashboard::TileData> {
        let _ = req;
        anyhow::bail!("this store cannot evaluate dashboards")
    }

    /// A report with its events looked up and, with `redact`, redacted.
    fn resolve_report(&self, report: &crate::report::Report, redact: bool) -> Result<crate::report::ResolvedReport> {
        let _ = (report, redact);
        anyhow::bail!("this store cannot export reports")
    }

    /// What a shareable bundle would contain.
    fn estimate_bundle(&self, opts: &crate::bundle::BundleOptions, profile_toml: &str) -> Result<crate::bundle::BundleEstimate> {
        let _ = (opts, profile_toml);
        anyhow::bail!("this store cannot write bundles")
    }

    /// Writes a shareable `.imtz` bundle.
    fn write_bundle(
        &self,
        opts: &crate::bundle::BundleOptions,
        profile_toml: &str,
        path: &std::path::Path,
    ) -> Result<crate::bundle::BundleManifest> {
        let _ = (opts, profile_toml, path);
        anyhow::bail!("this store cannot write bundles")
    }

    /// Configuration parsers for the diff view: the built-in formats plus
    /// those the store's plugins contribute.
    fn config_parsers(&self) -> crate::config::ConfigParsers {
        crate::config::ConfigParsers::builtin()
    }

    /// The tools the store's plugins contribute, for generic dispatch by the
    /// shell (usable without the store lock). A store without plugins has none.
    fn tool_host(&self) -> Option<crate::plugin::tools::ToolHost> {
        None
    }

    /// Sets a source's clock offset (manually or by aligning two events) and
    /// moves its records in time. A store without clock correction refuses.
    fn set_source_offset(&mut self, req: &crate::skew::SetOffsetRequest) -> Result<crate::skew::OffsetChange> {
        let _ = req;
        Err(anyhow::anyhow!("this store cannot correct clock skew"))
    }

    /// Suggests clock offsets from correlation ids shared across sources.
    fn suggest_offsets(&self, req: &crate::skew::SuggestRequest) -> crate::skew::SuggestResult {
        let _ = req;
        crate::skew::SuggestResult::default()
    }
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

    fn reset_plugin(&self, id: &str) {
        Workspace::reset_plugin(self, id)
    }

    fn progress(&self) -> std::sync::Arc<crate::workspace::ImportProgress> {
        Workspace::progress(self)
    }

    fn overview(&self, req: &QueryRequest) -> Result<Vec<crate::api::Finding>> {
        Workspace::overview(self, req)
    }

    fn plugin_settings(&self) -> Vec<crate::settings::PluginSettings> {
        Workspace::plugin_settings(self)
    }

    fn open_with(&mut self, paths: &[PathBuf], options: &crate::workspace::OpenOptions) -> Result<()> {
        Workspace::open_with(self, paths, options)
    }

    fn refresh_tail(&mut self, changed: &[PathBuf]) -> Result<crate::workspace::TailReport> {
        Workspace::refresh_tail(self, changed)
    }

    fn tail_status(&self) -> crate::workspace::TailStatus {
        Workspace::tail_status(self)
    }

    fn set_tail_options(&mut self, options: crate::workspace::TailOptions) {
        Workspace::set_tail_options(self, options)
    }

    fn mute_alerts(&mut self, target: crate::workspace::AlertTarget, until_ms: i64) -> Result<()> {
        Workspace::mute_alerts(self, target, until_ms)
    }

    fn unmute_alerts(&mut self, target: &crate::workspace::AlertTarget) {
        Workspace::unmute_alerts(self, target);
    }

    fn annotations(&self, req: &crate::plugin::annotate::AnnotationsRequest) -> Vec<crate::plugin::annotate::AnnotationDto> {
        Workspace::annotations(self, req)
    }

    fn event_bus(&self) -> Option<std::sync::Arc<crate::events::EventBus>> {
        Some(self.registry().events())
    }

    fn shutdown_plugins(&self) {
        self.registry().shutdown()
    }

    fn set_view_redaction(&mut self, on: bool) -> Result<()> {
        Workspace::set_view_redaction(self, on)
    }

    fn redaction_summary(&self, req: &crate::workspace::RedactionSummaryRequest) -> Result<crate::workspace::RedactionSummary> {
        Workspace::redaction_summary(self, req)
    }

    fn dashboard_tile(&self, req: &crate::dashboard::TileRequest) -> Result<crate::dashboard::TileData> {
        Workspace::dashboard_tile(self, req)
    }

    fn resolve_report(&self, report: &crate::report::Report, redact: bool) -> Result<crate::report::ResolvedReport> {
        Workspace::resolve_report(self, report, redact)
    }

    fn estimate_bundle(&self, opts: &crate::bundle::BundleOptions, profile_toml: &str) -> Result<crate::bundle::BundleEstimate> {
        Workspace::estimate_bundle(self, opts, profile_toml)
    }

    fn write_bundle(
        &self,
        opts: &crate::bundle::BundleOptions,
        profile_toml: &str,
        path: &std::path::Path,
    ) -> Result<crate::bundle::BundleManifest> {
        Workspace::write_bundle(self, opts, profile_toml, path)
    }

    fn config_parsers(&self) -> crate::config::ConfigParsers {
        crate::config::ConfigParsers::with_registry(self.registry())
    }

    fn tool_host(&self) -> Option<crate::plugin::tools::ToolHost> {
        Some(self.registry().tools())
    }

    fn set_source_offset(&mut self, req: &crate::skew::SetOffsetRequest) -> Result<crate::skew::OffsetChange> {
        Ok(Workspace::set_source_offset(self, req)?)
    }

    fn suggest_offsets(&self, req: &crate::skew::SuggestRequest) -> crate::skew::SuggestResult {
        Workspace::suggest_offsets(self, req)
    }
}
