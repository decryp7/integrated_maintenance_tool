//! Helpers that read a store only through the `Query` port: the annotators' `PortView` and the clock-skew
//! suggestion `suggest_over`. They run here over a small invented store whose `scan`/`page` serve fixed
//! events, so limits, caps, ordering and failure paths are triggered deterministically.

use imt_core::api::{
    ChartData, ChartRequest, CompletionRequest, ContextRequest, ExportRequest, ExportResult, Facet, FacetRequest, FieldStatsRequest,
    QueryRequest, SourceContext, SourceInfo, Timeline, TimelineRequest,
};
use imt_core::batch::{
    ColumnType, EventBatch, EventBatchBuilder, EventId, EventSchema, EventTime, FieldValue, LevelCode, NewEvent, SourceId,
};
use imt_core::plugin::annotate::{AnnotationView, PortView, ViewFilter, MAX_VIEW_INSTANCES, MAX_VIEW_RECORDS};
use imt_core::ports::{EventView, EventsRequest, Filter, PageQuery, PageReply, Query, StoreError, StoreResult, SummaryReply};
use imt_core::ql::{Completion, QlError, QlResult, QueryControl};
use imt_core::skew::{suggest_over, SuggestRequest, MAX_SCAN};
use imt_core::{Profile, Workspace};
use serde_json::{json, Map};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;

/// One invented event: (ts, level code, source, message, correlation id).
type Ev = (Option<i64>, u8, u32, &'static str, Option<&'static str>);

fn source_info(id: u32, name: &str, offset_ms: i64) -> SourceInfo {
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/serilog-clef");
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[samples]).unwrap();
    let mut info = ws.summary().sources[0].clone();
    info.id = id;
    info.name = name.into();
    info.offset_ms = offset_ms;
    info
}

fn batch_of(events: &[Ev], first_id: u64) -> EventBatch {
    let schema = Arc::new(EventSchema::new([("CorrelationId", ColumnType::Utf8)]).unwrap());
    let mut b = EventBatchBuilder::new(schema);
    for (ts, level, source, message, corr) in events {
        let ev = NewEvent {
            time: EventTime::parsed(*ts),
            level: LevelCode::new(*level).unwrap(),
            source: SourceId(*source),
            line: 1,
            byte_offset: 0,
            message,
            template: None,
            exception: None,
            markers: 0,
            raw: "",
        };
        let fields: Vec<(&str, FieldValue)> = corr.iter().map(|c| ("CorrelationId", FieldValue::Text(c))).collect();
        b.push(&ev, fields).unwrap();
    }
    b.finish().assign_ids(EventId(first_id)).unwrap()
}

/// A store serving fixed batches (two events per batch) and recording how it was scanned.
struct Fake {
    events: Vec<Ev>,
    sources: Vec<SourceInfo>,
    group_by: Vec<String>,
    fail_scan: bool,
    fail_page: bool,
    chunk: usize,
}

impl Fake {
    fn new(events: Vec<Ev>) -> Fake {
        Fake {
            events,
            sources: vec![source_info(0, "a.log", 0), source_info(1, "b.log", 0)],
            group_by: vec!["@source".into()],
            fail_scan: false,
            fail_page: false,
            chunk: 2,
        }
    }
}

fn unsupported<T>() -> StoreResult<T> {
    Err(StoreError::unsupported("the fake store", "none"))
}

impl Query for Fake {
    fn summary(&self) -> StoreResult<SummaryReply> {
        let timed: Vec<i64> = self.events.iter().filter_map(|e| e.0).collect();
        Ok(SummaryReply {
            total: self.events.len(),
            untimed: self.events.iter().filter(|e| e.0.is_none()).count(),
            from: timed.iter().min().copied(),
            to: timed.iter().max().copied(),
            levels: Default::default(),
            sources: self.sources.clone(),
            unrecognized: vec![],
            fields: vec![],
            default_group_by: self.group_by.clone(),
            marker_kinds: vec![],
            profile_name: String::new(),
        })
    }
    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        if self.fail_page {
            return Err(StoreError::Invalid("page failed".into()));
        }
        let mut all: Vec<(u64, &Ev)> = self.events.iter().enumerate().map(|(i, e)| (i as u64, e)).collect();
        all.sort_by_key(|(_, e)| e.0.unwrap_or(i64::MAX));
        if req.descending {
            all.reverse();
        }
        let rows = all
            .iter()
            .skip(req.offset)
            .take(req.rows().min(2)) // short pages force the view to page on
            .map(|(id, e)| EventView {
                id: EventId(*id),
                ts: e.0,
                ts_state: imt_core::batch::TsState::Parsed,
                level: imt_core::model::Level::Information,
                message: e.3.to_string(),
                template: None,
                exception: None,
                fields: Map::new(),
                source: SourceId(e.2),
                source_name: String::new(),
                line: 1,
                byte_offset: 0,
                group: String::new(),
                markers: vec![],
                search_match: None,
            })
            .collect();
        Ok(PageReply {
            total: all.len(),
            offset: req.offset,
            rows,
            anchor_index: None,
        })
    }
    fn events(&self, _: &EventsRequest) -> StoreResult<Vec<EventView>> {
        unsupported()
    }
    fn timeline(&self, _: &TimelineRequest) -> StoreResult<Timeline> {
        unsupported()
    }
    fn facet(&self, _: &FacetRequest) -> StoreResult<Facet> {
        unsupported()
    }
    fn scan(&self, filter: &Filter, visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
        if self.fail_scan {
            return Err(StoreError::Invalid("scan failed".into()));
        }
        let mut sorted: Vec<Ev> = self
            .events
            .iter()
            .filter(|e| match e.0 {
                Some(t) => filter.from.is_none_or(|f| t >= f) && filter.to.is_none_or(|u| t <= u),
                None => filter.from.is_none() && filter.to.is_none(),
            })
            .copied()
            .collect();
        sorted.sort_by_key(|e| e.0.unwrap_or(i64::MAX));
        for (i, chunk) in sorted.chunks(self.chunk).enumerate() {
            if visit(&batch_of(chunk, (i * 2) as u64)).is_break() {
                break;
            }
        }
        Ok(())
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

fn five() -> Fake {
    Fake::new(vec![
        (Some(300), 2, 1, "third", None),
        (Some(100), 2, 0, "first", None),
        (None, 2, 0, "untimed", None),
        (Some(200), 2, 1, "second", None),
        (Some(400), 2, 0, "fourth", None),
    ])
}

fn messages(v: &[imt_core::plugin::annotate::ViewRecord]) -> Vec<String> {
    v.iter().map(|r| r.record.message.clone()).collect()
}

#[test]
fn a_view_reads_sources_instances_and_the_time_range_from_the_summary() {
    let store = five();
    let view = PortView::new(&store).unwrap();
    assert_eq!(view.time_range(), Some((100, 400)));
    let mut instances = view.instances();
    instances.sort();
    assert_eq!(instances, vec!["a.log".to_string(), "b.log".to_string()]);
    let all = view.query(&Filter::default(), 100).unwrap();
    assert_eq!(
        messages(&all),
        ["first", "second", "third", "fourth", "untimed"],
        "time order, untimed last"
    );
    assert_eq!(all[0].source, "a.log");
    assert_eq!(all[1].source, "b.log");
    assert_eq!(all[1].instance, "b.log");
    // A source the summary does not know has an empty name.
    let empty = Fake::new(vec![(Some(1), 2, 9, "orphan", None)]);
    assert_eq!(PortView::new(&empty).unwrap().query(&Filter::default(), 5).unwrap()[0].source, "");
}

#[test]
fn a_view_without_times_has_no_range() {
    let store = Fake::new(vec![(None, 2, 0, "only", None)]);
    assert_eq!(PortView::new(&store).unwrap().time_range(), None);
}

#[test]
fn queries_are_limited_and_bounded() {
    let store = five();
    let view = PortView::new(&store).unwrap();
    assert!(view.query(&Filter::default(), 0).unwrap().is_empty());
    assert_eq!(messages(&view.query(&Filter::default(), 3).unwrap()), ["first", "second", "third"]);
    let ranged = Filter {
        from: Some(150),
        to: Some(350),
        ..Filter::default()
    };
    assert_eq!(messages(&view.query(&ranged, 10).unwrap()), ["second", "third"]);
    // The scan cap stops a call early; what was found until then stays.
    let capped = PortView::new(&store).unwrap().with_max_scan(3);
    assert_eq!(
        messages(&capped.query(&Filter::default(), 10).unwrap()),
        ["first", "second", "third"]
    );
    assert_eq!(capped.instances().len(), 2);
    // A cap above the hard maximum is clamped, not honoured.
    assert_eq!(view_len(PortView::new(&store).unwrap().with_max_scan(usize::MAX)), 5);
}

fn view_len(v: PortView<'_, Fake>) -> usize {
    v.query(&Filter::default(), MAX_VIEW_RECORDS + 1).unwrap().len()
}

#[test]
fn select_filters_by_condition_and_time_in_either_direction() {
    let store = five();
    let view = PortView::new(&store).unwrap();
    let when = |op: &str, value: &str| -> ViewFilter {
        serde_json::from_value(json!({"when": [{"field": "@message", "op": op, "value": value}]})).unwrap()
    };
    assert_eq!(messages(&view.select(&when("contains", "ir"), 10).unwrap()), ["first", "third"]);
    assert!(view.select(&ViewFilter::default(), 0).unwrap().is_empty());
    let bounded = ViewFilter {
        from: Some(150),
        to: Some(350),
        ..ViewFilter::default()
    };
    assert_eq!(messages(&view.select(&bounded, 10).unwrap()), ["second", "third"]);
    assert_eq!(messages(&view.select(&bounded, 1).unwrap()), ["second"], "the limit stops the scan");
    // An invalid condition is the caller's error, not a panic.
    assert!(view.select(&when("no-such-operator", "x"), 10).is_err());

    // Newest first pages from the end (untimed events first), across page boundaries.
    let newest = ViewFilter {
        newest_first: true,
        ..ViewFilter::default()
    };
    assert_eq!(
        messages(&view.select(&newest, 10).unwrap()),
        ["untimed", "fourth", "third", "second", "first"]
    );
    assert_eq!(messages(&view.select(&newest, 3).unwrap()), ["untimed", "fourth", "third"]);
    let filtered = ViewFilter {
        newest_first: true,
        ..when("contains", "ir")
    };
    assert_eq!(messages(&view.select(&filtered, 10).unwrap()), ["third", "first"]);
    // The scan cap also bounds the newest-first pages.
    let capped = PortView::new(&store).unwrap().with_max_scan(2);
    assert_eq!(messages(&capped.select(&newest, 10).unwrap()), ["untimed", "fourth"]);
}

#[test]
fn newest_first_on_an_empty_store_is_empty() {
    let store = Fake::new(vec![]);
    let view = PortView::new(&store).unwrap();
    let newest = ViewFilter {
        newest_first: true,
        ..ViewFilter::default()
    };
    assert!(view.select(&newest, 5).unwrap().is_empty());
    assert_eq!(view.time_range(), None);
}

#[test]
fn store_failures_reach_the_annotator_as_errors_or_an_empty_list() {
    let mut store = five();
    store.fail_scan = true;
    let view = PortView::new(&store).unwrap();
    let err = view.query(&Filter::default(), 5).unwrap_err().to_string();
    assert!(err.contains("scan failed"), "{err}");
    assert!(view.select(&ViewFilter::default(), 5).is_err());
    assert!(view.instances().is_empty(), "instances degrade to an empty list and log a warning");

    let mut store = five();
    store.fail_page = true;
    let view = PortView::new(&store).unwrap();
    let newest = ViewFilter {
        newest_first: true,
        ..ViewFilter::default()
    };
    let err = view.select(&newest, 5).unwrap_err().to_string();
    assert!(err.contains("page failed"), "{err}");
}

#[test]
fn a_store_that_cannot_summarize_cannot_make_a_view() {
    // The summary error is passed on; checked through the same port with a failing summary.
    struct Failing(Fake);
    impl Query for Failing {
        fn summary(&self) -> StoreResult<SummaryReply> {
            Err(StoreError::Invalid("no summary".into()))
        }
        fn page(&self, r: &PageQuery) -> StoreResult<PageReply> {
            self.0.page(r)
        }
        fn events(&self, r: &EventsRequest) -> StoreResult<Vec<EventView>> {
            self.0.events(r)
        }
        fn timeline(&self, r: &TimelineRequest) -> StoreResult<Timeline> {
            self.0.timeline(r)
        }
        fn facet(&self, r: &FacetRequest) -> StoreResult<Facet> {
            self.0.facet(r)
        }
        fn scan(&self, f: &Filter, v: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
            self.0.scan(f, v)
        }
        fn chart(&self, r: &ChartRequest) -> StoreResult<ChartData> {
            self.0.chart(r)
        }
        fn field_stats(&self, r: &FieldStatsRequest) -> StoreResult<imt_core::analysis::FieldStats> {
            self.0.field_stats(r)
        }
        fn source_context(&self, r: &ContextRequest) -> StoreResult<SourceContext> {
            self.0.source_context(r)
        }
        fn run_query_with(&self, r: &QueryRequest, c: &QueryControl) -> StoreResult<QlResult> {
            self.0.run_query_with(r, c)
        }
        fn check_query(&self, t: &str) -> StoreResult<Option<QlError>> {
            self.0.check_query(t)
        }
        fn complete_query(&self, r: &CompletionRequest) -> StoreResult<Vec<Completion>> {
            self.0.complete_query(r)
        }
        fn export(&self, r: &ExportRequest) -> StoreResult<ExportResult> {
            self.0.export(r)
        }
    }
    let failing = Failing(five());
    assert!(PortView::new(&failing).is_err());
    assert!(suggest_over(&failing, &SuggestRequest::default()).is_err());
}

#[test]
fn instances_are_listed_once_each_and_capped() {
    let events: Vec<Ev> = (0..6).map(|i| (Some(i), 2, (i % 3) as u32, "e", None)).collect();
    let mut store = Fake::new(events);
    store.sources.push(source_info(2, "c.log", 0));
    let view = PortView::new(&store).unwrap();
    let mut keys = view.instances();
    keys.sort();
    assert_eq!(keys, ["a.log", "b.log", "c.log"], "one key per instance, listed once");

    // More distinct instances than the cap: the list stops at the cap.
    let n = MAX_VIEW_INSTANCES + 5;
    let events: Vec<Ev> = (0..n)
        .map(|i| (Some(i as i64), 2, 0, "e", Some(&*Box::leak(format!("id{i}").into_boxed_str()))))
        .collect();
    let mut store = Fake::new(events);
    store.group_by = vec!["CorrelationId".into()];
    store.chunk = 4096;
    assert_eq!(PortView::new(&store).unwrap().instances().len(), MAX_VIEW_INSTANCES);
}

// -- suggest_over -----------------------------------------------------------------------------------------------

/// Source 0 sees request i at i*1000; source 1's clock runs 2.5 s behind.
fn skewed(offset_ms_of_b: i64) -> Fake {
    let ids = ["c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7"];
    let mut events: Vec<Ev> = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let t = 1_000_000 + i as i64 * 1000;
        events.push((Some(t), 2, 0, "A", Some(id)));
        // What is stored for source 1 already includes its current offset.
        events.push((Some(t - 2_500 + offset_ms_of_b), 2, 1, "B", Some(id)));
        // A later repeat of the id in the same source never moves the earliest time.
        events.push((Some(t + 50_000), 2, 1, "B again", Some(id)));
    }
    // Events without an id or without a time are skipped.
    events.push((Some(5), 2, 0, "no id", None));
    events.push((None, 2, 1, "no time", Some("c0")));
    let mut store = Fake::new(events);
    store.sources[1] = source_info(1, "b.log", offset_ms_of_b);
    store
}

#[test]
fn suggestions_come_from_ids_shared_between_sources() {
    let store = skewed(0);
    let r = suggest_over(&store, &SuggestRequest::default()).unwrap();
    assert_eq!(r.fields, ["CorrelationId", "TraceId", "RequestId"]);
    let s = r
        .suggestions
        .iter()
        .find(|s| s.source == 1)
        .expect("a suggestion for the lagging source");
    assert_eq!((s.suggested_ms, s.current_ms), (2_500, 0));
    assert!(r.insufficient.is_empty());
}

#[test]
fn suggestions_are_relative_to_the_original_times_not_the_shown_ones() {
    // Source 1 was already shifted by +1 s: the stored times include it and are corrected for it.
    let store = skewed(1_000);
    let r = suggest_over(&store, &SuggestRequest::default()).unwrap();
    let s = r.suggestions.iter().find(|s| s.source == 1).unwrap();
    assert_eq!(
        (s.suggested_ms, s.current_ms),
        (2_500, 1_000),
        "the same absolute offset, whatever is applied now"
    );
}

#[test]
fn only_the_named_fields_are_read_and_blank_names_ignored() {
    let store = skewed(0);
    let req = SuggestRequest {
        fields: vec!["  ".into(), "NoSuchField".into()],
        ..SuggestRequest::default()
    };
    let r = suggest_over(&store, &req).unwrap();
    assert_eq!(r.fields, ["NoSuchField"]);
    assert!(r.suggestions.is_empty());
}

#[test]
fn a_scan_that_fails_is_an_error() {
    let mut store = skewed(0);
    store.fail_scan = true;
    assert!(suggest_over(&store, &SuggestRequest::default()).is_err());
}

#[test]
fn the_scan_stops_at_the_cap() {
    // A store streaming more than MAX_SCAN untimed events: the read stops and the result is still produced.
    struct Endless(Fake);
    impl Query for Endless {
        fn summary(&self) -> StoreResult<SummaryReply> {
            self.0.summary()
        }
        fn page(&self, r: &PageQuery) -> StoreResult<PageReply> {
            self.0.page(r)
        }
        fn events(&self, r: &EventsRequest) -> StoreResult<Vec<EventView>> {
            self.0.events(r)
        }
        fn timeline(&self, r: &TimelineRequest) -> StoreResult<Timeline> {
            self.0.timeline(r)
        }
        fn facet(&self, r: &FacetRequest) -> StoreResult<Facet> {
            self.0.facet(r)
        }
        fn scan(&self, _: &Filter, visit: &mut dyn FnMut(&EventBatch) -> ControlFlow<()>) -> StoreResult<()> {
            let rows: Vec<Ev> = (0..1000).map(|_| (None, 2, 0, "x", None)).collect();
            let batch = batch_of(&rows, 0);
            let mut sent = 0usize;
            while sent <= MAX_SCAN + 1000 {
                if visit(&batch).is_break() {
                    return Ok(());
                }
                sent += 1000;
            }
            panic!("the scan was not stopped at MAX_SCAN");
        }
        fn chart(&self, r: &ChartRequest) -> StoreResult<ChartData> {
            self.0.chart(r)
        }
        fn field_stats(&self, r: &FieldStatsRequest) -> StoreResult<imt_core::analysis::FieldStats> {
            self.0.field_stats(r)
        }
        fn source_context(&self, r: &ContextRequest) -> StoreResult<SourceContext> {
            self.0.source_context(r)
        }
        fn run_query_with(&self, r: &QueryRequest, c: &QueryControl) -> StoreResult<QlResult> {
            self.0.run_query_with(r, c)
        }
        fn check_query(&self, t: &str) -> StoreResult<Option<QlError>> {
            self.0.check_query(t)
        }
        fn complete_query(&self, r: &CompletionRequest) -> StoreResult<Vec<Completion>> {
            self.0.complete_query(r)
        }
        fn export(&self, r: &ExportRequest) -> StoreResult<ExportResult> {
            self.0.export(r)
        }
    }
    let r = suggest_over(&Endless(five()), &SuggestRequest::default()).unwrap();
    assert!(r.suggestions.is_empty());
}
