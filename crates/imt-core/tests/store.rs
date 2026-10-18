//! The `LogStore` seam: the in-memory `Workspace` behind `&dyn LogStore`, and
//! the defaults a store without live tail or plugins inherits from the trait.

use anyhow::Result;
use imt_core::analysis::{Anomaly, Comparison, FieldStats, RequestSpan, WaterfallRow};
use imt_core::api::{
    ChartData, ChartRequest, CompareRequest, CompletionRequest, ContextRequest, CorrelateRequest, ExportRequest, ExportResult,
    ExtractPreview, ExtractPreviewRequest, Facet, FacetRequest, FieldStatsRequest, Finding, MomentRequest, MomentSummary, Page,
    PageRequest, ParsePreview, ParsePreviewRequest, QueryRequest, RequestsRequest, SourceContext, Timeline, TimelineRequest, TraceRequest,
    TraceSummary, WorkspaceState, WorkspaceSummary,
};
use imt_core::exceptions::ExceptionGroup;
use imt_core::patterns::Pattern;
use imt_core::ql::{Completion, QlError, QlResult};
use imt_core::workspace::{ImportProgress, OpenOptions, Retention, TailOptions};
use imt_core::{LogStore, Profile, Workspace};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A temporary folder removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!("imt-store-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn folder() -> TempDir {
    let d = TempDir::new();
    let text: String = (1..=6)
        .map(|i| format!(r#"{{"@t":"2026-10-14T10:00:0{i}Z","@l":"Information","@mt":"step {i}","MachineName":"WEB01"}}"#) + "\n")
        .collect();
    std::fs::write(d.0.join("app.clef"), text).unwrap();
    d
}

#[test]
fn workspace_answers_context_progress_and_tail_options_through_the_trait() {
    let d = folder();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    let store: &mut dyn LogStore = &mut ws;
    store.open(std::slice::from_ref(&d.0)).unwrap();
    assert_eq!(store.summary().total, 6);

    let ctx = store
        .source_context(&ContextRequest {
            record_id: 2,
            before: 1,
            after: 1,
        })
        .unwrap();
    assert_eq!(ctx.source_name, "app.clef");
    assert_eq!(ctx.line, 3);
    assert_eq!(ctx.lines.len(), 3);

    let progress = store.progress().snapshot();
    assert!(!progress.running);

    // Retention through the trait evicts the oldest records.
    store.set_tail_options(TailOptions {
        retention: Retention {
            max_records: Some(1),
            ..Retention::default()
        },
        ..TailOptions::default()
    });
    assert_eq!(store.summary().total, 1);
    assert!(store.plugin_settings().is_empty());
}

#[test]
fn workspace_exposes_its_event_bus_and_shuts_its_plugins_down() {
    use imt_core::events::SnapshotTaken;
    let ws = Workspace::new(Profile::default_profile()).unwrap();
    let store: &dyn LogStore = &ws;
    let bus = store.event_bus().expect("a workspace has an event bus");
    let seen = Arc::new(AtomicUsize::new(0));
    let s = seen.clone();
    bus.subscribe::<SnapshotTaken>("host-test", move |_| {
        s.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    bus.publish(SnapshotTaken { id: "x".into() });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while seen.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    assert!(store
        .event_bus()
        .unwrap()
        .metrics()
        .iter()
        .any(|m| m.plugin == "host-test" && m.delivered == 1));
    // Idempotent, and the store stays usable for queries.
    store.shutdown_plugins();
    store.shutdown_plugins();
    assert_eq!(store.summary().total, 0);
    // A workspace without tool plugins has an empty tool host.
    assert!(store.tool_host().expect("a workspace has a tool host").is_empty());
}

/// A store implementing only the required methods (delegating to a workspace),
/// to observe the trait's defaults.
struct Minimal(Workspace);

impl LogStore for Minimal {
    fn open(&mut self, paths: &[PathBuf]) -> Result<()> {
        self.0.open(paths)
    }
    fn reload(&mut self) -> Result<()> {
        self.0.reload()
    }
    fn clear(&mut self) {
        self.0.clear()
    }
    fn remove_source(&mut self, id: u32) {
        self.0.remove_source(id)
    }
    fn set_profile(&mut self, profile: Profile) -> Result<()> {
        self.0.set_profile(profile)
    }
    fn profile(&self) -> &Profile {
        self.0.profile()
    }
    fn summary(&self) -> WorkspaceSummary {
        self.0.summary()
    }
    fn page(&self, req: &PageRequest) -> Result<Page<'_>> {
        self.0.page(req)
    }
    fn timeline(&self, req: &TimelineRequest) -> Result<Timeline> {
        self.0.timeline(req)
    }
    fn facet(&self, req: &FacetRequest) -> Result<Facet> {
        self.0.facet(req)
    }
    fn chart(&self, req: &ChartRequest) -> Result<ChartData> {
        self.0.chart(req)
    }
    fn preview_extractor(&self, req: &ExtractPreviewRequest) -> ExtractPreview {
        self.0.preview_extractor(req)
    }
    fn validate_profile(&self, profile: Profile) -> Result<()> {
        self.0.validate_profile(profile)
    }
    fn export(&self, req: &ExportRequest) -> Result<ExportResult> {
        self.0.export(req)
    }
    fn source_context(&self, req: &ContextRequest) -> Result<SourceContext> {
        self.0.source_context(req)
    }
    fn state(&self) -> WorkspaceState {
        self.0.state()
    }
    fn refresh_files(&mut self, changed: &[PathBuf]) -> Result<usize> {
        self.0.refresh_files(changed)
    }
    fn restore(&mut self, profile: Profile, state: &WorkspaceState) -> Result<Vec<String>> {
        self.0.restore(profile, state)
    }
    fn run_query(&self, req: &QueryRequest) -> std::result::Result<QlResult, QlError> {
        self.0.run_query(req)
    }
    fn check_query(&self, text: &str) -> Option<QlError> {
        self.0.check_query(text)
    }
    fn complete_query(&self, req: &CompletionRequest) -> Vec<Completion> {
        self.0.complete_query(req)
    }
    fn patterns(&self, req: &QueryRequest) -> Result<Vec<Pattern>> {
        self.0.patterns(req)
    }
    fn exception_groups(&self, req: &QueryRequest) -> Result<Vec<ExceptionGroup>> {
        self.0.exception_groups(req)
    }
    fn anomalies(&self, req: &QueryRequest) -> Result<Vec<Anomaly>> {
        self.0.anomalies(req)
    }
    fn compare(&self, req: &CompareRequest) -> Result<Comparison> {
        self.0.compare(req)
    }
    fn field_stats(&self, req: &FieldStatsRequest) -> Result<FieldStats> {
        self.0.field_stats(req)
    }
    fn correlate(&self, req: &CorrelateRequest) -> Vec<u32> {
        self.0.correlate(req)
    }
    fn traces(&self, req: &TraceRequest) -> Result<(Vec<TraceSummary>, Vec<WaterfallRow>)> {
        self.0.traces(req)
    }
    fn requests(&self, req: &RequestsRequest) -> Result<Vec<RequestSpan>> {
        self.0.requests(req)
    }
    fn moment(&self, req: &MomentRequest) -> Result<MomentSummary> {
        self.0.moment(req)
    }
    fn preview_parse(&self, req: &ParsePreviewRequest) -> ParsePreview {
        self.0.preview_parse(req)
    }
    fn reset_plugin(&self, id: &str) {
        self.0.reset_plugin(id)
    }
    fn progress(&self) -> Arc<ImportProgress> {
        self.0.progress()
    }
    fn overview(&self, req: &QueryRequest) -> Result<Vec<Finding>> {
        self.0.overview(req)
    }
}

#[test]
fn stores_without_live_tail_or_plugins_get_safe_defaults() {
    let d = folder();
    let mut store: Box<dyn LogStore> = Box::new(Minimal(Workspace::new(Profile::default_profile()).unwrap()));
    assert!(store.plugin_settings().is_empty());

    // `start_at_end` is ignored: the whole history is imported.
    store
        .open_with(std::slice::from_ref(&d.0), &OpenOptions { start_at_end: true })
        .unwrap();
    assert_eq!(store.summary().total, 6);

    // A tail refresh re-imports changed files and reports nothing finer.
    let file = d.0.join("app.clef");
    let mut text = std::fs::read_to_string(&file).unwrap();
    text.push_str(r#"{"@t":"2026-10-14T10:00:07Z","@mt":"step 7"}"#);
    text.push('\n');
    std::fs::write(&file, text).unwrap();
    let report = store.refresh_tail(&[file]).unwrap();
    assert_eq!((report.appended, report.sources_changed.len()), (0, 0));
    assert_eq!(store.summary().total, 7);

    assert_eq!(store.tail_status(), Default::default());
    assert!(store.annotations(&Default::default()).is_empty());
    assert!(store.event_bus().is_none());
    // Clock-skew correction: refused, and nothing to suggest.
    let req: imt_core::skew::SetOffsetRequest = serde_json::from_str(r#"{"kind": "manual", "source": 0, "offsetMs": 5}"#).unwrap();
    assert!(store
        .set_source_offset(&req)
        .unwrap_err()
        .to_string()
        .contains("cannot correct clock skew"));
    assert!(store.suggest_offsets(&Default::default()).suggestions.is_empty());
    assert!(store.tool_host().is_none());
    store.shutdown_plugins();
    // Tail options are accepted and ignored: nothing is evicted.
    store.set_tail_options(TailOptions {
        retention: Retention {
            max_records: Some(1),
            ..Retention::default()
        },
        ..TailOptions::default()
    });
    assert_eq!(store.summary().total, 7);
    // Nothing to mute: accepted and ignored.
    let target = imt_core::workspace::AlertTarget::Rule("r".into());
    store.mute_alerts(target.clone(), 0).unwrap();
    store.unmute_alerts(&target);
}

#[test]
fn stores_without_sharing_features_say_so() {
    use imt_core::workspace::{RedactionPreviewRequest, RedactionSummaryRequest};
    let profile = Profile::from_toml("[redact]\ndetectors = ['email']").unwrap();
    let mut store: Box<dyn LogStore> = Box::new(Minimal(Workspace::new(profile).unwrap()));
    assert!(store.set_view_redaction(true).unwrap_err().to_string().contains("no redacted view"));
    store.set_view_redaction(false).unwrap();
    // Previews only need the profile, so every store has them.
    let p = store.preview_redaction(&RedactionPreviewRequest {
        sample: "a@b.io".into(),
        toml: None,
    });
    assert_eq!(p.after, "<email>");
    let p = store.preview_redaction(&RedactionPreviewRequest {
        sample: "a@b.io".into(),
        toml: Some("[redact]\nmode = 'drop'\ndetectors = ['email']".into()),
    });
    assert_eq!(p.after, "");
    let p = store.preview_redaction(&RedactionPreviewRequest {
        sample: "s".into(),
        toml: Some("[".into()),
    });
    assert!(p.error.is_some());
    let q = RedactionSummaryRequest {
        query: Default::default(),
        samples: 1,
    };
    assert!(store.redaction_summary(&q).is_err());
    let tile: imt_core::dashboard::TileCfg = toml::from_str("id='a'\ntype='annotations'").unwrap();
    assert!(store
        .dashboard_tile(&imt_core::dashboard::TileRequest {
            tile,
            query: Default::default()
        })
        .is_err());
    let report = imt_core::report::Report::parse(serde_json::json!({"id": "r"})).unwrap();
    assert!(store.resolve_report(&report, false).is_err());
    let opts: imt_core::bundle::BundleOptions = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(store.estimate_bundle(&opts, "").is_err());
    assert!(store.write_bundle(&opts, "", std::path::Path::new("x.imtz")).is_err());
}

#[test]
fn stores_without_cancellation_run_to_completion_but_report_a_cancel() {
    use imt_core::ql::{QueryControl, RunError};
    let d = folder();
    let mut store: Box<dyn LogStore> = Box::new(Minimal(Workspace::new(Profile::default_profile()).unwrap()));
    store.open(std::slice::from_ref(&d.0)).unwrap();
    let req = QueryRequest {
        query: imt_core::query::Query {
            ql: "step".into(),
            ..Default::default()
        },
    };
    assert!(matches!(
        store.run_query_with(&req, &QueryControl::new()),
        Ok(QlResult::Events { .. })
    ));
    let cancelled = QueryControl::new();
    cancelled.cancel();
    assert_eq!(store.run_query_with(&req, &cancelled), Err(RunError::Cancelled));
    let bad = QueryRequest {
        query: imt_core::query::Query {
            ql: "level >".into(),
            ..Default::default()
        },
    };
    assert!(matches!(
        store.run_query_with(&bad, &QueryControl::new()),
        Err(RunError::Invalid(_))
    ));
}
