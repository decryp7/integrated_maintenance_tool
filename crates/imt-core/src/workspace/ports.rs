//! The in-memory workspace behind the store ports: the reference implementation the DuckDB store is checked
//! against (differential tests) and the store of the engine's own tests (code written over the ports, such
//! as the folder survey, dashboard tiles and the AI tools, is tested here without a database). The app uses
//! the DuckDB store (ADR 0014 6c); this adapter is not wired into it.
//!
//! Operations the workspace has no equivalent for answer [`StoreError::Unsupported`] (positions, full-text
//! search, projects, quick fixes, span rule inference).

use super::Workspace;
use crate::analysis::{Anomaly, Comparison, FieldStats, RequestSpan, WaterfallRow};
use crate::api::{
    ChartData, ChartRequest, CompareRequest, CompletionRequest, ContextRequest, CorrelateRequest, ExportRequest, ExportResult,
    ExtractPreview, ExtractPreviewRequest, Facet, FacetRequest, FieldStatsRequest, Finding, MomentRequest, MomentSummary, ParsePreview,
    ParsePreviewRequest, QueryRequest, RecordView, RequestsRequest, SourceContext, Timeline, TimelineRequest, TraceRequest, TraceSummary,
    WorkspaceState, WorkspaceSummary,
};
use crate::batch::{EventBatch, EventId, SourceId, TsState};
use crate::exceptions::ExceptionGroup;
use crate::patterns::Pattern;
use crate::ports::{
    Analysis, Analyzed, Coverage, DiagnosticRow, Diagnostics, EventView, EventsRequest, Filter, FixEvaluation, ImportReport, ImportSession,
    IndexStatus, Ingest, PageQuery, PageReply, PartialMatches, Positions, Profiles, Projects, Query, QuickFix, Search, SearchReply,
    SearchRequest, StoreError, StoreResult, SummaryReply,
};
use crate::ql::{Completion, QlError, QlResult, QueryControl, RunError};
use crate::store::LogStore;
use crate::Profile;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// A [`Workspace`] behind the store ports (locks recover from poisoning).
pub struct WorkspaceStore(RwLock<Workspace>);

fn invalid(e: anyhow::Error) -> StoreError {
    StoreError::Invalid(format!("{e:#}"))
}

fn unsupported(operation: &'static str) -> StoreError {
    StoreError::unsupported(operation, "the DuckDB store (the in-memory workspace has none)")
}

impl WorkspaceStore {
    pub fn new(ws: Workspace) -> WorkspaceStore {
        WorkspaceStore(RwLock::new(ws))
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Workspace> {
        self.0.read().unwrap_or_else(|p| p.into_inner())
    }

    pub fn write(&self) -> RwLockWriteGuard<'_, Workspace> {
        self.0.write().unwrap_or_else(|p| p.into_inner())
    }

    /// What an import did, as the store reports it.
    fn report(&self, elapsed_ms: u64) -> ImportReport {
        let s = self.read().summary();
        ImportReport {
            events: s.total,
            sources: s.sources,
            unrecognized: s.unrecognized,
            bytes: 0,
            elapsed_ms,
        }
    }

    /// A finished import session (the workspace imports while it is called).
    fn done(&self, work: impl FnOnce(&mut Workspace) -> anyhow::Result<()>) -> StoreResult<ImportSession> {
        let started = std::time::Instant::now();
        work(&mut self.write()).map_err(invalid)?;
        let report = self.report(started.elapsed().as_millis() as u64);
        let progress = self.read().progress();
        ImportSession::spawn(progress, move |_| Ok(report))
    }

    fn coverage(&self, query: &crate::query::Query) -> Coverage {
        match self.read().analysis_coverage(&QueryRequest { query: query.clone() }) {
            Ok(c) => Coverage {
                matched: c.total as u64,
                analysed: c.analysed as u64,
                limits: Vec::new(),
            },
            Err(_) => Coverage::default(),
        }
    }

    fn analysed<T>(&self, query: &crate::query::Query, r: anyhow::Result<T>) -> StoreResult<Analyzed<T>> {
        let result = r.map_err(invalid)?;
        Ok(Analyzed {
            result,
            coverage: self.coverage(query),
        })
    }
}

/// The store's summary of a workspace's.
pub fn summary_reply(s: WorkspaceSummary) -> SummaryReply {
    SummaryReply {
        untimed: s.untimed,
        total: s.total,
        from: s.from,
        to: s.to,
        levels: s.levels,
        sources: s.sources,
        unrecognized: s.unrecognized,
        fields: s.fields,
        default_group_by: s.default_group_by,
        marker_kinds: s.marker_kinds.into_iter().map(|m| m.kind).collect(),
        profile_name: s.profile_name,
    }
}

/// A grid row of the workspace as the store's event view.
pub fn event_view(r: &RecordView<'_>) -> EventView {
    EventView {
        id: EventId(u64::from(r.id)),
        ts: r.ts,
        ts_state: if r.ts.is_some() { TsState::Parsed } else { TsState::Untimed },
        level: r.level,
        message: r.message.to_string(),
        template: r.template.as_ref().map(|t| t.to_string()),
        exception: r.exception.as_ref().map(|x| x.to_string()),
        fields: r.fields.clone().into_owned(),
        source: SourceId(r.source),
        source_name: r.source_name.to_string(),
        line: r.line,
        byte_offset: 0,
        group: r.group.clone(),
        markers: r.markers.iter().map(|m| m.to_string()).collect(),
        search_match: None,
    }
}

impl Ingest for WorkspaceStore {
    fn open(&self, paths: &[PathBuf]) -> StoreResult<ImportSession> {
        self.done(|ws| ws.open(paths))
    }
    fn reload(&self) -> StoreResult<ImportSession> {
        self.done(|ws| ws.reload())
    }
    fn refresh(&self, changed: &[PathBuf]) -> StoreResult<ImportSession> {
        self.done(|ws| ws.refresh_files(changed).map(|_| ()))
    }
    fn remove_source(&self, id: SourceId) -> StoreResult<()> {
        let mut ws = self.write();
        if !ws.sources().iter().any(|s| s.id == id.0) {
            return Err(StoreError::NotFound(format!("source {}", id.0)));
        }
        ws.remove_source(id.0);
        Ok(())
    }
    fn clear(&self) -> StoreResult<()> {
        self.write().clear();
        Ok(())
    }
    fn restore(&self, state: &WorkspaceState) -> StoreResult<ImportSession> {
        self.done(|ws| {
            let profile = ws.profile().clone();
            ws.restore(profile, state).map(|_| ())
        })
    }
    fn append_tail(&self, changed: &[PathBuf]) -> StoreResult<usize> {
        self.write().refresh_tail(changed).map(|r| r.appended).map_err(invalid)
    }
}

impl Query for WorkspaceStore {
    fn summary(&self) -> StoreResult<SummaryReply> {
        Ok(summary_reply(self.read().summary()))
    }
    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        let ws = self.read();
        let page = ws
            .page(&crate::api::PageRequest {
                query: req.query.clone(),
                offset: req.offset,
                limit: req.rows(),
                descending: req.descending,
                anchor: req.anchor.map(|a| u32::try_from(a.0).unwrap_or(u32::MAX)),
            })
            .map_err(invalid)?;
        Ok(PageReply {
            total: page.total,
            offset: page.offset,
            rows: {
                let mut rows: Vec<EventView> = page.rows.iter().map(event_view).collect();
                crate::ports::mark_search_matches(&mut rows, &req.query);
                rows
            },
            anchor_index: page.anchor_index,
        })
    }
    fn events(&self, req: &EventsRequest) -> StoreResult<Vec<EventView>> {
        let ws = self.read();
        let cq = ws.compile(&Filter {
            group_by: req.group_by.clone(),
            ..Filter::default()
        });
        let cq = cq.map_err(invalid)?;
        Ok(req
            .ids
            .iter()
            .filter_map(|id| {
                let r = ws.records.get(usize::try_from(id.0).ok()?).filter(|r| u64::from(r.id) == id.0)?;
                let source_name = ws.source_name(r.source);
                Some(EventView {
                    id: *id,
                    ts: r.ts,
                    ts_state: if r.ts.is_some() { TsState::Parsed } else { TsState::Untimed },
                    level: r.level,
                    message: r.message.clone(),
                    template: r.template.clone(),
                    exception: r.exception.clone(),
                    fields: r.fields.clone(),
                    source: SourceId(r.source),
                    source_name: source_name.to_string(),
                    line: r.line,
                    byte_offset: 0,
                    group: cq.group_key(r, source_name),
                    markers: Vec::new(),
                    search_match: None,
                })
            })
            .collect())
    }
    fn timeline(&self, req: &TimelineRequest) -> StoreResult<Timeline> {
        self.read().timeline(req).map_err(invalid)
    }
    fn facet(&self, req: &FacetRequest) -> StoreResult<Facet> {
        self.read().facet(req).map_err(invalid)
    }
    fn scan(&self, _filter: &Filter, _visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
        Err(unsupported("scan"))
    }
    fn chart(&self, req: &ChartRequest) -> StoreResult<ChartData> {
        self.read().chart(req).map_err(invalid)
    }
    fn field_stats(&self, req: &FieldStatsRequest) -> StoreResult<FieldStats> {
        self.read().field_stats(req).map_err(invalid)
    }
    fn source_context(&self, req: &ContextRequest) -> StoreResult<SourceContext> {
        self.read().source_context(req).map_err(invalid)
    }
    fn run_query_with(&self, req: &QueryRequest, control: &QueryControl) -> StoreResult<QlResult> {
        LogStore::run_query_with(&*self.read(), req, control).map_err(|e| match e {
            RunError::Invalid(q) => StoreError::Query(q),
            RunError::Cancelled => StoreError::Cancelled,
        })
    }
    fn check_query(&self, text: &str) -> StoreResult<Option<QlError>> {
        Ok(self.read().check_query(text))
    }
    fn complete_query(&self, req: &CompletionRequest) -> StoreResult<Vec<Completion>> {
        Ok(self.read().complete_query(req))
    }
    fn export(&self, req: &ExportRequest) -> StoreResult<ExportResult> {
        self.read().export(req).map_err(invalid)
    }
}

impl Positions for WorkspaceStore {
    fn ids_at(&self, _from: u64, _len: usize) -> StoreResult<Vec<EventId>> {
        Err(unsupported("positions"))
    }
    fn position_of(&self, _id: EventId) -> StoreResult<Option<u64>> {
        Err(unsupported("positions"))
    }
    fn positioned(&self) -> StoreResult<u64> {
        Err(unsupported("positions"))
    }
}

impl Analysis for WorkspaceStore {
    fn patterns(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Pattern>>> {
        self.analysed(&req.query, self.read().patterns(req))
    }
    fn exception_groups(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<ExceptionGroup>>> {
        self.analysed(&req.query, self.read().exception_groups(req))
    }
    fn anomalies(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Anomaly>>> {
        self.analysed(&req.query, self.read().anomalies(req))
    }
    fn compare(&self, req: &CompareRequest) -> StoreResult<Analyzed<Comparison>> {
        let result = self.read().compare(req).map_err(invalid)?;
        Ok(Analyzed {
            result,
            coverage: self.coverage(&req.a).and(self.coverage(&req.b)),
        })
    }
    fn correlate(&self, req: &CorrelateRequest) -> StoreResult<Analyzed<Vec<EventId>>> {
        let ids: Vec<EventId> = self.read().correlate(req).into_iter().map(|i| EventId(u64::from(i))).collect();
        let n = ids.len() as u64;
        Ok(Analyzed {
            result: ids,
            coverage: Coverage {
                matched: n,
                analysed: n,
                limits: Vec::new(),
            },
        })
    }
    fn traces(&self, req: &TraceRequest) -> StoreResult<Analyzed<(Vec<TraceSummary>, Vec<WaterfallRow>)>> {
        self.analysed(&req.query, self.read().traces(req))
    }
    fn requests(&self, req: &RequestsRequest) -> StoreResult<Analyzed<Vec<RequestSpan>>> {
        self.analysed(&req.query, self.read().requests(req))
    }
    fn moment(&self, req: &MomentRequest) -> StoreResult<Analyzed<MomentSummary>> {
        self.analysed(&req.query, self.read().moment(req))
    }
    fn overview(&self, req: &QueryRequest) -> StoreResult<Analyzed<Vec<Finding>>> {
        self.analysed(&req.query, self.read().overview(req))
    }
    fn runs(&self, req: &crate::spans::RunsRequest) -> StoreResult<crate::spans::RunsReply> {
        Ok(self.read().runs(req))
    }
}

impl Profiles for WorkspaceStore {
    fn active_profile(&self) -> StoreResult<Profile> {
        Ok(self.read().profile().clone())
    }
    fn validate_profile(&self, profile: &Profile) -> StoreResult<()> {
        LogStore::validate_profile(&*self.read(), profile.clone()).map_err(invalid)
    }
    fn set_profile(&self, profile: Profile) -> StoreResult<ImportSession> {
        self.done(|ws| ws.set_profile(profile))
    }
    fn preview_parse(&self, req: &ParsePreviewRequest) -> StoreResult<ParsePreview> {
        Ok(LogStore::preview_parse(&*self.read(), req))
    }
    fn preview_extractor(&self, req: &ExtractPreviewRequest) -> StoreResult<ExtractPreview> {
        Ok(LogStore::preview_extractor(&*self.read(), req))
    }
    fn parsers(&self) -> StoreResult<Vec<crate::ports::ParserUse>> {
        let ws = self.read();
        let mut uses: Vec<crate::ports::ParserUse> = Vec::new();
        for s in ws.sources() {
            let Some(r) = ws.parser_of(s.id) else { continue };
            match uses.iter_mut().find(|u| u.parser == *r) {
                Some(u) => u.sources.push(SourceId(s.id)),
                None => uses.push(crate::ports::ParserUse {
                    parser: r.clone(),
                    sources: vec![SourceId(s.id)],
                }),
            }
        }
        for u in &mut uses {
            u.sources.sort_by_key(|s| s.0);
        }
        uses.sort_by(|a, b| (&a.parser.id, &a.parser.hash).cmp(&(&b.parser.id, &b.parser.hash)));
        Ok(uses)
    }
    fn set_parsers(&self, setup: &crate::parser::ParserSetup) -> StoreResult<Vec<SourceId>> {
        let mut ws = self.write();
        let mut setup = setup.clone();
        setup.overrides = ws.parsers().setup().overrides.clone();
        Ok(ws.set_parsers(&setup).map_err(invalid)?.into_iter().map(SourceId).collect())
    }
    fn reread_parsers(&self) -> StoreResult<ImportSession> {
        self.done(|ws| {
            let stale = ws.stale_sources();
            ws.reread(&stale)
        })
    }
    fn classify_parser(&self, parser: &crate::ports::ParserDef) -> StoreResult<crate::ports::ParserChange> {
        let ws = self.read();
        let new = crate::parser::Parser::from_toml(&parser.toml).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let r = new.reference();
        let sources: Vec<SourceId> = ws
            .sources()
            .iter()
            .filter(|s| ws.parser_of(s.id).is_some_and(|u| u.id == r.id && u.hash != r.hash))
            .map(|s| SourceId(s.id))
            .collect();
        Ok(crate::ports::ParserChange {
            class: if sources.is_empty() {
                crate::profile_change::ChangeClass::Presentation
            } else {
                crate::profile_change::ChangeClass::Parsing
            },
            sections: if sources.is_empty() { Vec::new() } else { vec!["format".into()] },
            sources,
        })
    }
    fn assign_parser(&self, source: SourceId, parser: Option<&str>) -> StoreResult<ImportSession> {
        let path = {
            let ws = self.read();
            let s = ws
                .sources()
                .iter()
                .find(|s| s.id == source.0)
                .ok_or_else(|| StoreError::NotFound(format!("source {source}")))?;
            if let Some(id) = parser.filter(|id| ws.parsers().parser(id).is_none()) {
                return Err(StoreError::NotFound(format!("parser \"{id}\" (it is not in the library)")));
            }
            s.path.clone()
        };
        self.done(move |ws| {
            let mut setup = ws.parsers().setup().clone();
            match parser {
                Some(id) => setup.overrides.insert(path.clone(), id.to_string()),
                None => setup.overrides.remove(&path),
            };
            ws.set_parsers(&setup)?;
            ws.reread(&[source.0])
        })
    }
    fn preview_spans(&self, req: &crate::spans::SpanPreviewRequest) -> StoreResult<crate::spans::SpanPreview> {
        Ok(self.read().preview_spans(req))
    }
    fn propose_span_rule(&self, _req: &crate::spans::infer::SpanRuleRequest) -> StoreResult<crate::spans::infer::SpanProposal> {
        Err(unsupported("span rule inference"))
    }
    fn evaluate_span_rule(&self, _req: &crate::spans::infer::SpanRuleCheck) -> StoreResult<Analyzed<crate::spans::infer::SpanEvaluation>> {
        Err(unsupported("span rule evaluation"))
    }
}

impl Diagnostics for WorkspaceStore {
    fn problems(&self) -> StoreResult<Vec<DiagnosticRow>> {
        Ok(Vec::new())
    }
    fn evaluate_fix(&self, _fix: &QuickFix) -> StoreResult<FixEvaluation> {
        Err(unsupported("quick-fix evaluation"))
    }
}

impl Search for WorkspaceStore {
    fn search(&self, req: &SearchRequest) -> StoreResult<SearchReply> {
        let ws = self.read();
        let words = crate::words::WordQuery::parse(&req.text);
        let limit = if req.limit == 0 { 1000 } else { req.limit.min(10_000) };
        let mut total = 0u64;
        let mut ids = Vec::new();
        for r in ws.records() {
            let texts: Vec<&str> = [Some(r.message.as_str()), r.exception.as_deref(), r.template.as_deref()]
                .into_iter()
                .flatten()
                .collect();
            if words.matches(&texts) {
                total += 1;
                if ids.len() < limit {
                    ids.push(EventId(u64::from(r.id)));
                }
            }
        }
        Ok(SearchReply { total, ids, exact: true })
    }
    fn partial_matches(&self, filter: &Filter) -> StoreResult<PartialMatches> {
        let ws = self.read();
        let mut sub = filter.clone();
        sub.text_mode = crate::query::TextMode::Substring;
        let mut words = filter.clone();
        words.text_mode = crate::query::TextMode::Words;
        let (sub, words) = (ws.compile(&sub).map_err(invalid)?, ws.compile(&words).map_err(invalid)?);
        let control = crate::ql::QueryControl::current();
        let mut added = 0;
        for r in ws.records() {
            if control.as_ref().is_some_and(|c| c.is_cancelled()) {
                return Err(StoreError::Cancelled);
            }
            let name = ws.source_name(r.source);
            if sub.matches(r, name) && !words.matches(r, name) {
                added += 1;
            }
        }
        Ok(PartialMatches { added, complete: true })
    }
    fn index_status(&self) -> StoreResult<IndexStatus> {
        let total = self.read().records().len() as u64;
        Ok(IndexStatus {
            indexed: 0,
            total,
            ready: false,
        })
    }
}

impl Projects for WorkspaceStore {
    fn store_dir(&self) -> PathBuf {
        PathBuf::new()
    }
    fn save_project(&self, manifest: &crate::package::ProjectManifest) -> StoreResult<crate::package::ProjectManifest> {
        let mut m = manifest.clone();
        m.roots = self.read().state().roots;
        Ok(m)
    }
    fn export_project(&self, _manifest: &crate::package::ProjectManifest, _out: &Path) -> StoreResult<u64> {
        Err(unsupported("projects"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
    }

    #[test]
    fn the_workspace_answers_through_the_ports() {
        let store = WorkspaceStore::new(Workspace::new(Profile::default_profile()).unwrap());
        let report = store.open(&[samples().join("serilog-clef")]).unwrap().wait().unwrap();
        assert!(report.events > 0);
        let s = Query::summary(&store).unwrap();
        assert_eq!(s.total, report.events);
        let page = store
            .page(&PageQuery {
                limit: 5,
                ..PageQuery::default()
            })
            .unwrap();
        assert_eq!(page.rows.len(), 5);
        let again = store
            .events(&EventsRequest {
                ids: vec![page.rows[2].id, EventId(u64::MAX)],
                group_by: vec![],
            })
            .unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].message, page.rows[2].message);
        assert!(
            store
                .patterns(&QueryRequest { query: Filter::default() })
                .unwrap()
                .coverage
                .analysed
                > 0
        );
        let first = page.rows[0].message.split_whitespace().next().unwrap_or_default().to_string();
        let hits = store
            .search(&SearchRequest {
                text: first.clone(),
                limit: 1,
            })
            .unwrap();
        assert!(hits.total >= 1 && hits.ids.len() == 1 && hits.exact, "{first}: {hits:?}");
        let words = Filter {
            text: first,
            text_mode: crate::query::TextMode::Words,
            ..Filter::default()
        };
        assert!(store.partial_matches(&words).unwrap().complete);
        let control = QueryControl::new();
        control.cancel();
        assert!(matches!(
            control.scope(|| store.partial_matches(&words)),
            Err(StoreError::Cancelled)
        ));
        assert!(matches!(store.remove_source(SourceId(999)), Err(StoreError::NotFound(_))));
        let bad = QueryRequest {
            query: Filter {
                ql: "where ===".into(),
                ..Filter::default()
            },
        };
        assert!(matches!(store.run_query(&bad), Err(StoreError::Query(_))));
        assert_eq!(store.save_project(&Default::default()).unwrap().roots.len(), 1);
        store.clear().unwrap();
        assert_eq!(Query::summary(&store).unwrap().total, 0);
    }
}
