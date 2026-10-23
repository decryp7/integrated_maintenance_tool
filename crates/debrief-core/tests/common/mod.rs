//! A small invented store behind the `Query` port (fixed events served by `scan` and `page`), shared by the
//! integration tests of code that reads a store only through its ports.
#![allow(dead_code)]

pub mod delegate;

use debrief_core::api::{
    ChartData, ChartRequest, CompletionRequest, ContextRequest, ExportRequest, ExportResult, Facet, FacetRequest, FieldStatsRequest,
    QueryRequest, SourceContext, SourceInfo, Timeline, TimelineRequest,
};
use debrief_core::batch::{
    ColumnType, EventBatch, EventBatchBuilder, EventId, EventSchema, EventTime, FieldValue, LevelCode, NewEvent, SourceId,
};
use debrief_core::ports::{EventView, EventsRequest, Filter, PageQuery, PageReply, Query, StoreError, StoreResult, SummaryReply};
use debrief_core::ql::{Completion, QlError, QlResult, QueryControl};
use debrief_core::{Profile, Workspace};
use serde_json::Map;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;

/// One invented event: (ts, level code, source, message, correlation id).
pub type Ev = (Option<i64>, u8, u32, &'static str, Option<&'static str>);

pub fn source_info(id: u32, name: &str, offset_ms: i64) -> SourceInfo {
    let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/serilog-clef");
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[samples]).unwrap();
    let mut info = ws.summary().sources[0].clone();
    info.id = id;
    info.name = name.into();
    info.offset_ms = offset_ms;
    info
}

pub fn batch_of(events: &[Ev], first_id: u64) -> EventBatch {
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
pub struct Fake {
    pub events: Vec<Ev>,
    pub sources: Vec<SourceInfo>,
    pub group_by: Vec<String>,
    pub fail_scan: bool,
    pub fail_page: bool,
    pub chunk: usize,
}

impl Fake {
    pub fn new(events: Vec<Ev>) -> Fake {
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

pub fn unsupported<T>() -> StoreResult<T> {
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
                ts_state: debrief_core::batch::TsState::Parsed,
                level: debrief_core::model::Level::Information,
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
    fn field_stats(&self, _: &FieldStatsRequest) -> StoreResult<debrief_core::analysis::FieldStats> {
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
