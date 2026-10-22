//! A `Store` that delegates every call to another one, so a test can wrap it and change just the methods it needs.
#![allow(dead_code, unused_imports)]

use imt_core::analysis::{Anomaly, Comparison, FieldStats, RequestSpan, WaterfallRow};
use imt_core::api::{
    ChartData, ChartRequest, CompareRequest, CompletionRequest, ContextRequest, CorrelateRequest, ExportRequest, ExportResult,
    ExtractPreview, ExtractPreviewRequest, Facet, FacetRequest, FieldStatsRequest, Finding, MomentRequest, MomentSummary, ParsePreview,
    ParsePreviewRequest, QueryRequest, RequestsRequest, SourceContext, Timeline, TimelineRequest, TraceRequest, TraceSummary,
    WorkspaceState,
};
use imt_core::batch::{EventBatch, EventId, SourceId};
use imt_core::exceptions::ExceptionGroup;
use imt_core::patterns::Pattern;
use imt_core::ports::*;
use imt_core::profile::Profile;
use imt_core::ql::{Completion, QlError, QlResult, QueryControl};
use std::ops::ControlFlow;
use std::path::PathBuf;

/// Forwards every port method to the wrapped store, except `problems()` when `problems` is set: a store whose
/// import problems a test decides (the real stores report them only for files that really fail to read).
pub struct Delegating<'a, S> {
    pub inner: &'a S,
    pub problems: Option<StoreResult<Vec<DiagnosticRow>>>,
    /// Makes `remove_source` fail with this error instead of removing anything.
    pub remove_source_error: Option<StoreError>,
}

impl<'a, S> Delegating<'a, S> {
    pub fn new(inner: &'a S) -> Self {
        Delegating {
            inner,
            problems: None,
            remove_source_error: None,
        }
    }

    pub fn with_problems(inner: &'a S, problems: StoreResult<Vec<DiagnosticRow>>) -> Self {
        Delegating {
            inner,
            problems: Some(problems),
            remove_source_error: None,
        }
    }
}

impl<S: Ingest> Ingest for Delegating<'_, S> {
    fn open(&self, paths: &[PathBuf]) -> StoreResult<ImportSession> {
        self.inner.open(paths)
    }
    fn reload(&self) -> StoreResult<ImportSession> {
        self.inner.reload()
    }
    fn refresh(&self, changed: &[PathBuf]) -> StoreResult<ImportSession> {
        self.inner.refresh(changed)
    }
    fn set_source_encoding(&self, source: SourceId, encoding: Option<&str>) -> StoreResult<ImportSession> {
        self.inner.set_source_encoding(source, encoding)
    }
    fn remove_source(&self, id: SourceId) -> StoreResult<()> {
        match &self.remove_source_error {
            Some(e) => Err(e.clone()),
            None => self.inner.remove_source(id),
        }
    }
    fn clear(&self) -> StoreResult<()> {
        self.inner.clear()
    }
    fn restore(&self, state: &WorkspaceState) -> StoreResult<ImportSession> {
        self.inner.restore(state)
    }
    fn append_tail(&self, changed: &[PathBuf]) -> StoreResult<TailAppend> {
        self.inner.append_tail(changed)
    }
}

impl<S: Query> Query for Delegating<'_, S> {
    fn summary(&self) -> StoreResult<SummaryReply> {
        self.inner.summary()
    }
    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        self.inner.page(req)
    }
    fn events(&self, req: &EventsRequest) -> StoreResult<Vec<EventView>> {
        self.inner.events(req)
    }
    fn timeline(&self, req: &TimelineRequest) -> StoreResult<Timeline> {
        self.inner.timeline(req)
    }
    fn facet(&self, req: &FacetRequest) -> StoreResult<Facet> {
        self.inner.facet(req)
    }
    fn scan(&self, filter: &Filter, visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
        self.inner.scan(filter, visit)
    }
    fn chart(&self, req: &ChartRequest) -> StoreResult<ChartData> {
        self.inner.chart(req)
    }
    fn field_stats(&self, req: &FieldStatsRequest) -> StoreResult<FieldStats> {
        self.inner.field_stats(req)
    }
    fn source_context(&self, req: &ContextRequest) -> StoreResult<SourceContext> {
        self.inner.source_context(req)
    }
    fn run_query(&self, req: &QueryRequest) -> StoreResult<QlResult> {
        self.inner.run_query(req)
    }
    fn run_query_with(&self, req: &QueryRequest, control: &QueryControl) -> StoreResult<QlResult> {
        self.inner.run_query_with(req, control)
    }
    fn check_query(&self, text: &str) -> StoreResult<Option<QlError>> {
        self.inner.check_query(text)
    }
    fn complete_query(&self, req: &CompletionRequest) -> StoreResult<Vec<Completion>> {
        self.inner.complete_query(req)
    }
    fn export(&self, req: &ExportRequest) -> StoreResult<ExportResult> {
        self.inner.export(req)
    }
}

impl<S: Positions> Positions for Delegating<'_, S> {
    fn ids_at(&self, from: u64, len: usize) -> StoreResult<Vec<EventId>> {
        self.inner.ids_at(from, len)
    }
    fn position_of(&self, id: EventId) -> StoreResult<Option<u64>> {
        self.inner.position_of(id)
    }
    fn positioned(&self) -> StoreResult<u64> {
        self.inner.positioned()
    }
}

impl<S: Analysis> Analysis for Delegating<'_, S> {
    fn patterns(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Pattern>>> {
        self.inner.patterns(req)
    }
    fn exception_groups(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<ExceptionGroup>>> {
        self.inner.exception_groups(req)
    }
    fn anomalies(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Anomaly>>> {
        self.inner.anomalies(req)
    }
    fn compare(&self, req: &CompareRequest) -> StoreResult<Analyzed<Comparison>> {
        self.inner.compare(req)
    }
    fn correlate(&self, req: &CorrelateRequest) -> StoreResult<Analyzed<Vec<EventId>>> {
        self.inner.correlate(req)
    }
    fn traces(&self, req: &TraceRequest) -> StoreResult<Analyzed<(Vec<TraceSummary>, Vec<WaterfallRow>)>> {
        self.inner.traces(req)
    }
    fn requests(&self, req: &RequestsRequest) -> StoreResult<Analyzed<Vec<RequestSpan>>> {
        self.inner.requests(req)
    }
    fn moment(&self, req: &MomentRequest) -> StoreResult<Analyzed<MomentSummary>> {
        self.inner.moment(req)
    }
    fn overview(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Finding>>> {
        self.inner.overview(req)
    }
    fn runs(&self, req: &imt_core::spans::RunsRequest) -> StoreResult<imt_core::spans::RunsReply> {
        self.inner.runs(req)
    }
}

impl<S: Profiles> Profiles for Delegating<'_, S> {
    fn active_profile(&self) -> StoreResult<Profile> {
        self.inner.active_profile()
    }
    fn validate_profile(&self, profile: &Profile) -> StoreResult<()> {
        self.inner.validate_profile(profile)
    }
    fn set_profile(&self, profile: Profile) -> StoreResult<ImportSession> {
        self.inner.set_profile(profile)
    }
    fn parsers(&self) -> StoreResult<Vec<ParserUse>> {
        self.inner.parsers()
    }
    fn set_parsers(&self, setup: &imt_core::parser::ParserSetup) -> StoreResult<Vec<SourceId>> {
        self.inner.set_parsers(setup)
    }
    fn reread_parsers(&self) -> StoreResult<ImportSession> {
        self.inner.reread_parsers()
    }
    fn classify_parser(&self, parser: &ParserDef) -> StoreResult<ParserChange> {
        self.inner.classify_parser(parser)
    }
    fn assign_parser(&self, source: SourceId, parser: Option<&str>) -> StoreResult<ImportSession> {
        self.inner.assign_parser(source, parser)
    }
    fn preview_parse(&self, req: &ParsePreviewRequest) -> StoreResult<ParsePreview> {
        self.inner.preview_parse(req)
    }
    fn preview_extractor(&self, req: &ExtractPreviewRequest) -> StoreResult<ExtractPreview> {
        self.inner.preview_extractor(req)
    }
    fn propose_span_rule(&self, req: &imt_core::spans::infer::SpanRuleRequest) -> StoreResult<imt_core::spans::infer::SpanProposal> {
        self.inner.propose_span_rule(req)
    }
    fn preview_spans(&self, req: &imt_core::spans::SpanPreviewRequest) -> StoreResult<imt_core::spans::SpanPreview> {
        self.inner.preview_spans(req)
    }
    fn evaluate_span_rule(
        &self,
        req: &imt_core::spans::infer::SpanRuleCheck,
    ) -> StoreResult<Analyzed<imt_core::spans::infer::SpanEvaluation>> {
        self.inner.evaluate_span_rule(req)
    }
}

impl<S: Diagnostics> Diagnostics for Delegating<'_, S> {
    fn problems(&self) -> StoreResult<Vec<DiagnosticRow>> {
        match &self.problems {
            Some(rows) => rows.clone(),
            None => self.inner.problems(),
        }
    }
    fn evaluate_fix(&self, fix: &QuickFix) -> StoreResult<FixEvaluation> {
        self.inner.evaluate_fix(fix)
    }
}

impl<S: Search> Search for Delegating<'_, S> {
    fn search(&self, req: &SearchRequest) -> StoreResult<SearchReply> {
        self.inner.search(req)
    }
    fn index_status(&self) -> StoreResult<IndexStatus> {
        self.inner.index_status()
    }
    fn partial_matches(&self, filter: &Filter) -> StoreResult<PartialMatches> {
        self.inner.partial_matches(filter)
    }
}

impl<S: Projects> Projects for Delegating<'_, S> {
    fn store_dir(&self) -> PathBuf {
        self.inner.store_dir()
    }
    fn save_project(&self, manifest: &imt_core::package::ProjectManifest) -> StoreResult<imt_core::package::ProjectManifest> {
        self.inner.save_project(manifest)
    }
    fn export_project(&self, manifest: &imt_core::package::ProjectManifest, out: &std::path::Path) -> StoreResult<u64> {
        self.inner.export_project(manifest, out)
    }
    fn close(&self) {
        self.inner.close()
    }
}

impl<S: Inspect> Inspect for Delegating<'_, S> {
    fn store_overview(&self) -> StoreResult<StoreOverview> {
        self.inner.store_overview()
    }
    fn store_segments(&self, req: &SegmentsRequest) -> StoreResult<SegmentsPage> {
        self.inner.store_segments(req)
    }
    fn store_fields(&self) -> StoreResult<StoreFields> {
        self.inner.store_fields()
    }
    fn store_index(&self) -> StoreResult<StoreIndex> {
        self.inner.store_index()
    }
    fn store_memory(&self) -> StoreResult<StoreMemory> {
        self.inner.store_memory()
    }
}

impl<S: Maintain> Maintain for Delegating<'_, S> {
    fn maintain(&self, action: MaintenanceAction) -> StoreResult<MaintenanceReport> {
        self.inner.maintain(action)
    }
}

impl<S: Console> Console for Delegating<'_, S> {
    fn sql_schema(&self) -> StoreResult<SqlSchema> {
        self.inner.sql_schema()
    }
    fn sql_run(&self, req: &SqlRequest) -> StoreResult<SqlResult> {
        self.inner.sql_run(req)
    }
    fn sql_page(&self, req: &SqlPageRequest) -> StoreResult<SqlPage> {
        self.inner.sql_page(req)
    }
    fn sql_ids(&self, req: &SqlIdsRequest) -> StoreResult<SqlIds> {
        self.inner.sql_ids(req)
    }
    fn sql_export(&self, req: &SqlExportRequest) -> StoreResult<SqlExportResult> {
        self.inner.sql_export(req)
    }
}

impl<S: Redaction> Redaction for Delegating<'_, S> {
    fn export_redacted_project(
        &self,
        req: &imt_core::redact::copy::RedactedExportRequest,
    ) -> StoreResult<imt_core::redact::copy::RedactedExport> {
        self.inner.export_redacted_project(req)
    }
    fn redaction_summary(&self, req: &imt_core::redact::RedactionSummaryRequest) -> StoreResult<imt_core::redact::RedactionSummary> {
        self.inner.redaction_summary(req)
    }
}

impl<S: Skew> Skew for Delegating<'_, S> {
    fn clock_offsets(&self) -> StoreResult<imt_core::skew::ClockOffsets> {
        self.inner.clock_offsets()
    }
    fn set_clock_offsets(&self, offsets: &imt_core::skew::ClockOffsets) -> StoreResult<()> {
        self.inner.set_clock_offsets(offsets)
    }
    fn set_source_offset(&self, req: &imt_core::skew::SetOffsetRequest) -> StoreResult<imt_core::skew::OffsetChange> {
        self.inner.set_source_offset(req)
    }
    fn suggest_offsets(&self, req: &imt_core::skew::SuggestRequest) -> StoreResult<imt_core::skew::SuggestResult> {
        self.inner.suggest_offsets(req)
    }
}

impl<S: Tail> Tail for Delegating<'_, S> {
    fn tail_files(&self) -> StoreResult<Vec<TailFile>> {
        self.inner.tail_files()
    }
    fn open_at_end(&self, paths: &[PathBuf]) -> StoreResult<ImportSession> {
        self.inner.open_at_end(paths)
    }
}
