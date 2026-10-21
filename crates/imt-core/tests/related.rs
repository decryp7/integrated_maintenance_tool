//! "Find related logs" on the generated incident in /samples: screenshot facts (what a model reads off an error
//! dialog, a frozen window, a console) against real events, through both adapters of the port (the in-memory
//! store and the store's `Query` port), which must agree.

use imt_core::api::{
    ChartData, ChartRequest, ContextRequest, ExportRequest, ExportResult, Facet, FacetRequest, FieldStatsRequest, PageRequest,
};
use imt_core::api::{CompletionRequest, QueryRequest, SourceContext, Timeline, TimelineRequest};
use imt_core::batch::{EventBatch, EventId, SourceId, TsState};
use imt_core::ports::{EventView, Filter, PageQuery, PageReply, Query, StoreError, StoreResult, SummaryReply};
use imt_core::ql::{Completion, QlError, QlResult, QueryControl};
use imt_core::related::*;
use imt_core::{LogStore, Profile, Workspace};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::OnceLock;

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

fn store() -> &'static (dyn LogStore + 'static) {
    static STORE: OnceLock<Box<dyn LogStore>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            let text = std::fs::read_to_string(samples().join("profiles/orderservice.imtprofile.toml")).unwrap();
            let mut ws: Box<dyn LogStore> = Box::new(Workspace::new(Profile::from_toml(&text).unwrap()).unwrap());
            ws.open(&[
                samples().join("serilog-clef/web01/orders-20261014.clef"),
                samples().join("serilog-text/worker01/worker-20261014.txt"),
            ])
            .unwrap();
            ws
        })
        .as_ref()
}

fn ms(h: u32, m: u32, s: u32) -> i64 {
    chrono::NaiveDate::from_ymd_opt(2026, 10, 14)
        .and_then(|d| d.and_hms_opt(h, m, s))
        .map(|t| t.and_utc().timestamp_millis())
        .unwrap()
}

/// The store's `Query` port over the in-memory store (only what the adapter uses).
struct Port(&'static dyn LogStore);

fn unsupported<T>() -> StoreResult<T> {
    Err(StoreError::unsupported("this test port", "none"))
}

impl Query for Port {
    fn summary(&self) -> StoreResult<SummaryReply> {
        let s = self.0.summary();
        Ok(SummaryReply {
            total: s.total,
            untimed: 0,
            from: None,
            to: None,
            levels: Default::default(),
            sources: s.sources,
            unrecognized: vec![],
            fields: vec![],
            default_group_by: vec![],
            marker_kinds: vec![],
            profile_name: String::new(),
        })
    }
    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        let p = self
            .0
            .page(&PageRequest {
                query: req.query.clone(),
                offset: req.offset,
                limit: req.rows(),
                descending: req.descending,
                anchor: None,
            })
            .map_err(|e| StoreError::Invalid(e.to_string()))?;
        Ok(PageReply {
            total: p.total,
            offset: p.offset,
            anchor_index: None,
            rows: p
                .rows
                .into_iter()
                .map(|r| EventView {
                    id: EventId(u64::from(r.id)),
                    ts: r.ts,
                    ts_state: TsState::Parsed,
                    level: r.level,
                    message: r.message.into_owned(),
                    template: r.template.map(|t| t.into_owned()),
                    exception: r.exception.map(|t| t.into_owned()),
                    fields: r.fields.into_owned(),
                    source: SourceId(r.source),
                    source_name: r.source_name.to_string(),
                    line: r.line,
                    byte_offset: 0,
                    group: r.group,
                    markers: vec![],
                    search_match: None,
                })
                .collect(),
        })
    }
    fn events(&self, _: &imt_core::ports::EventsRequest) -> StoreResult<Vec<EventView>> {
        unsupported()
    }
    fn timeline(&self, _: &TimelineRequest) -> StoreResult<Timeline> {
        unsupported()
    }
    fn facet(&self, _: &FacetRequest) -> StoreResult<Facet> {
        unsupported()
    }
    fn scan(&self, _: &Filter, _: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
        unsupported()
    }
    fn chart(&self, _: &ChartRequest) -> StoreResult<ChartData> {
        unsupported()
    }
    fn field_stats(&self, _: &FieldStatsRequest) -> StoreResult<imt_core::analysis::FieldStats> {
        unsupported()
    }
    fn source_context(&self, _: &ContextRequest) -> StoreResult<SourceContext> {
        unsupported()
    }
    fn run_query_with(&self, _: &QueryRequest, _: &QueryControl) -> StoreResult<QlResult> {
        unsupported()
    }
    fn check_query(&self, _: &str) -> StoreResult<Option<QlError>> {
        unsupported()
    }
    fn complete_query(&self, _: &CompletionRequest) -> StoreResult<Vec<Completion>> {
        unsupported()
    }
    fn export(&self, _: &ExportRequest) -> StoreResult<ExportResult> {
        unsupported()
    }
}

/// Both adapters give the same answer; returns it.
fn find(req: &RelatedRequest) -> RelatedResult {
    let a = find_related(&LogStoreSource(store()), req).unwrap();
    let port = Port(store());
    let b = find_related(&QuerySource(&port), req).unwrap();
    assert_eq!(a, b, "the ports adapter and the in-memory adapter disagree");
    a
}

fn shown(text: &str, has_date: bool) -> TimeEvidence {
    TimeEvidence {
        shown: vec![ShownTime {
            text: text.into(),
            has_date,
        }],
        ..TimeEvidence::default()
    }
}

fn payment_dialog() -> Clues {
    Clues {
        window_title: Some("OrderService - Error".into()),
        application: Some("OrderService".into()),
        error_texts: vec!["Failed to capture payment for order 91698: request timed out".into()],
        exception_names: vec!["System.Net.Http.HttpRequestException".into()],
        machine_names: vec!["WEB01".into()],
        ..Clues::default()
    }
}

#[test]
fn an_error_dialog_links_to_its_event_first() {
    let r = find(&RelatedRequest {
        clues: payment_dialog(),
        time: shown("2026-10-14 10:41:15", true),
        assume_timezone: "UTC".into(),
        prefer_errors: true,
        ..RelatedRequest::default()
    });
    let top = &r.links[0];
    assert!(top.event.message.contains("91698"), "{:?}", top.event.message);
    assert_eq!(top.event.ts.map(|t| (t - ms(10, 41, 14)) / 1000), Some(0));
    let labels: Vec<String> = top.reasons.iter().map(Reason::label).collect();
    assert!(labels[0].starts_with("time −0.6 s"), "{labels:?}");
    assert!(
        labels.iter().any(|l| l == "text 'System.Net.Http.HttpRequestException'"),
        "{labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.starts_with("text 'Failed to capture payment")),
        "{labels:?}"
    );
    assert!(labels.iter().any(|l| l == "text 'WEB01'"), "{labels:?}");
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    assert!(r.links.len() <= DEFAULT_LIMIT && r.links.len() > 1);
}

#[test]
fn a_frozen_window_at_a_time_of_day_finds_the_worker() {
    // "OrderWorker (Not Responding)" with the taskbar clock; the date comes from the file name.
    let r = find(&RelatedRequest {
        clues: Clues {
            window_title: Some("OrderWorker (Not Responding)".into()),
            ..Clues::default()
        },
        time: TimeEvidence {
            file_name: Some("Screenshot 2026-10-14 090500.png".into()),
            ..shown("09:00", false)
        },
        assume_timezone: "UTC".into(),
        ..RelatedRequest::default()
    });
    let t = r.time.unwrap();
    assert_eq!(
        (t.at_ms, t.precision_ms, t.source),
        (ms(9, 0, 0), 60_000, TimeSource::ScreenTimeOfDay)
    );
    let top = &r.links[0];
    assert!(top.event.source_name.starts_with("worker"), "{:?}", top.event);
    assert!(top
        .reasons
        .iter()
        .any(|x| matches!(x, Reason::Title { target: TitleTarget::Field { field, .. } | TitleTarget::Lane { lane: field }, .. } if field.contains("Application") || field.contains("OrderWorker"))));
}

#[test]
fn without_a_readable_time_the_text_decides_and_says_so() {
    let r = find(&RelatedRequest {
        clues: Clues {
            exception_names: vec!["HttpRequestException".into()],
            ids: vec!["91534".into()],
            ..Clues::default()
        },
        assume_timezone: "UTC".into(),
        ..RelatedRequest::default()
    });
    assert_eq!(r.notes, [RelatedNote::NoTime]);
    assert!(r.links[0].event.message.contains("91534"), "{:?}", r.links[0].event.message);
}

#[test]
fn a_screen_clock_two_hours_off_is_reported_with_the_source_to_correct() {
    let r = find(&RelatedRequest {
        clues: payment_dialog(),
        time: shown("2026-10-14 12:41:14", true),
        assume_timezone: "UTC".into(),
        ..RelatedRequest::default()
    });
    assert!(r.links[0].event.message.contains("91698"));
    let n = r.notes.iter().find_map(|n| match n {
        RelatedNote::ClockDisagreement { delta_ms, source_name, .. } => Some((*delta_ms, source_name.clone())),
        _ => None,
    });
    let (delta, source) = n.expect("clock disagreement");
    assert!((delta + 2 * 3_600_000).abs() < 2_000, "{delta}");
    assert!(source.contains("orders"), "{source}");
    // The same screenshot read in a +02:00 profile is on time: no disagreement.
    let local = find(&RelatedRequest {
        clues: payment_dialog(),
        time: shown("2026-10-14 12:41:14", true),
        assume_timezone: "+02:00".into(),
        ..RelatedRequest::default()
    });
    assert!(
        !local.notes.iter().any(|n| matches!(n, RelatedNote::ClockDisagreement { .. })),
        "{:?}",
        local.notes
    );
    assert!(local.links[0].reasons.iter().any(|x| matches!(x, Reason::Time { .. })));
}
