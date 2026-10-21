//! Ranking and orchestration over a fake source (the samples end-to-end are in `tests/related.rs`).

use super::*;
use crate::api::{FileKind, SourceInfo};

pub(super) fn source(id: u32, name: &str, offset_ms: i64, from: Option<i64>, to: Option<i64>) -> SourceInfo {
    SourceInfo {
        id,
        path: format!("/logs/{name}"),
        name: name.into(),
        group: name.into(),
        importer: "text".into(),
        records: 0,
        skipped: 0,
        split_events: 0,
        dropped: 0,
        step_stats: vec![],
        error: None,
        from,
        to,
        offset_ms,
        file_kind: FileKind::Text,
        timestamp_issue: None,
    }
}

fn event(id: u64, ts: i64, level: Level, message: &str, source: u32) -> RelatedEvent {
    RelatedEvent {
        id,
        ts: Some(ts),
        level,
        message: message.into(),
        exception: None,
        source,
        source_name: format!("s{source}"),
        group: String::new(),
        fields: Map::new(),
    }
}

/// A source that applies time ranges, source and level filters and quoted text terms, and records the filters.
struct Fake {
    sources: Vec<SourceInfo>,
    events: Vec<RelatedEvent>,
    asked: std::cell::RefCell<Vec<Filter>>,
}

impl RelatedSource for Fake {
    fn sources(&self) -> StoreResult<Vec<SourceInfo>> {
        Ok(self.sources.clone())
    }

    fn events(&self, f: &Filter, limit: usize) -> StoreResult<Vec<RelatedEvent>> {
        self.asked.borrow_mut().push(f.clone());
        let phrase = f.ql.trim_matches('"').replace("\\\"", "\"").to_lowercase();
        Ok(self
            .events
            .iter()
            .filter(|e| f.from.is_none_or(|from| e.ts.is_some_and(|t| t >= from)))
            .filter(|e| f.to.is_none_or(|to| e.ts.is_some_and(|t| t <= to)))
            .filter(|e| f.sources.is_empty() || f.sources.contains(&e.source))
            .filter(|e| f.levels.is_empty() || f.levels.contains(&e.level))
            .filter(|e| phrase.is_empty() || e.message.to_lowercase().contains(&phrase))
            .take(limit)
            .cloned()
            .collect())
    }
}

const T: i64 = 1_760_000_000_000;

fn fake() -> Fake {
    let mut worker = event(4, T + 900, Level::Information, "Dispatched 16 outbox messages", 2);
    worker.fields.insert("Application".into(), Value::String("OrderWorker".into()));
    worker.group = "OrderWorker · 2208".into();
    Fake {
        sources: vec![
            source(1, "web01.clef", 0, Some(T - 3_600_000), Some(T + 3_600_000)),
            source(2, "worker01.txt", 0, Some(T - 3_600_000), Some(T + 3_600_000)),
        ],
        events: vec![
            event(1, T - 30_000, Level::Information, "Request finished in 12 ms", 1),
            event(
                2,
                T + 1_400,
                Level::Error,
                "Failed to capture payment for order 91698: HttpRequestException",
                1,
            ),
            event(
                3,
                T - 1_800_000,
                Level::Error,
                "Failed to capture payment for order 91534: HttpRequestException",
                1,
            ),
            worker,
            event(5, T + 500, Level::Information, "Heartbeat", 1),
        ],
        asked: Default::default(),
    }
}

fn at(text: &str) -> TimeEvidence {
    TimeEvidence {
        shown: vec![ShownTime {
            text: text.into(),
            has_date: true,
        }],
        ..TimeEvidence::default()
    }
}

fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

#[test]
fn the_matching_error_at_the_time_of_the_screenshot_ranks_first_with_its_reasons() {
    let f = fake();
    let req = RelatedRequest {
        clues: Clues {
            exception_names: vec!["System.Net.Http.HttpRequestException".into()],
            ids: vec!["91698".into()],
            ..Clues::default()
        },
        time: at(&iso(T)),
        assume_timezone: "UTC".into(),
        prefer_errors: true,
        ..RelatedRequest::default()
    };
    let r = find_related(&f, &req).unwrap();
    assert_eq!(r.time.as_ref().unwrap().at_ms, T);
    assert_eq!(r.links[0].event.id, 2);
    let labels: Vec<String> = r.links[0].reasons.iter().map(Reason::label).collect();
    assert_eq!(
        labels,
        ["time +1.4 s", "text 'System.Net.Http.HttpRequestException'", "text '91698'"]
    );
    // The other payment failure matches the exception but is 30 minutes away: lower.
    let pos3 = r.links.iter().position(|l| l.event.id == 3).unwrap();
    assert!(pos3 > 0 && r.links[pos3].score < r.links[0].score);
    // An event only near in time is linked (within the precision); one 30 s away without a clue is not.
    assert!(r.links.iter().any(|l| l.event.id == 5));
    assert!(!r.links.iter().any(|l| l.event.id == 1));
    assert!(r.notes.is_empty(), "{:?}", r.notes);
    assert_eq!(r.examined, 5);
    // The engine searched the window and each strong clue as a phrase.
    let asked = f.asked.borrow();
    assert!(asked[0].from.is_some() && asked[0].ql.is_empty());
    assert_eq!(asked[1].ql, "\"HttpRequestException\"");
    assert_eq!(asked[2].ql, "\"91698\"");
}

#[test]
fn a_title_matches_lanes_fields_and_sources() {
    let f = fake();
    let req = RelatedRequest {
        clues: Clues {
            window_title: Some("OrderWorker - Not Responding".into()),
            ..Clues::default()
        },
        time: at(&iso(T)),
        assume_timezone: "UTC".into(),
        ..RelatedRequest::default()
    };
    let r = find_related(&f, &req).unwrap();
    let w = r.links.iter().find(|l| l.event.id == 4).unwrap();
    assert!(
        matches!(&w.reasons[1], Reason::Title { text, target: TitleTarget::Lane { lane } } if text == "orderworker" && lane.starts_with("OrderWorker"))
    );
    assert_eq!(r.links[0].event.id, 4, "the title match beats an event only near in time");
    // A source named like the title is matched too.
    let mut f2 = fake();
    f2.sources[1].name = "orderworker-20261014.txt".into();
    f2.events[3].group = String::new();
    let r = find_related(&f2, &req).unwrap();
    assert!(
        matches!(&r.links[0].reasons[1], Reason::Title { target: TitleTarget::Source { name }, .. } if name.starts_with("orderworker"))
    );
    f2.sources[1].name = "w.txt".into();
    let r = find_related(&f2, &req).unwrap();
    assert!(matches!(&r.links[0].reasons[1], Reason::Title { target: TitleTarget::Field { field, .. }, .. } if field == "Application"));
}

#[test]
fn without_a_time_text_and_title_decide_and_the_note_says_so() {
    let f = fake();
    let req = RelatedRequest {
        clues: Clues {
            error_texts: vec!["Failed to capture payment for order 91534".into()],
            ..Clues::default()
        },
        assume_timezone: "UTC".into(),
        ..RelatedRequest::default()
    };
    let r = find_related(&f, &req).unwrap();
    assert_eq!(r.time, None);
    assert_eq!(r.links[0].event.id, 3);
    assert_eq!(r.notes, [RelatedNote::NoTime]);
    // A wording close to the event's (most words) is a partial match.
    let close = RelatedRequest {
        clues: Clues {
            error_texts: vec!["failed capturing payment for order 91698".into()],
            ..Clues::default()
        },
        time: at(&iso(T)),
        ..req.clone()
    };
    let r = find_related(&f, &close).unwrap();
    assert!(r.links[0]
        .reasons
        .iter()
        .any(|x| matches!(x, Reason::Text { partial: true, .. }) && x.label().starts_with("text close to")));
    // Only a title and no time: the errors of the sources it names.
    let mut f2 = fake();
    f2.sources[0].name = "orderservice.clef".into();
    let title = RelatedRequest {
        clues: Clues {
            application: Some("OrderService".into()),
            ..Clues::default()
        },
        ..req
    };
    let _ = find_related(&f2, &title).unwrap();
    let asked = f2.asked.borrow();
    assert!(asked.iter().any(|a| a.sources == [1] && a.levels.contains(&Level::Error)));
}

#[test]
fn a_disagreeing_clock_and_time_outside_the_data_are_reported() {
    let f = fake();
    let req = RelatedRequest {
        clues: Clues {
            ids: vec!["91698".into()],
            ..Clues::default()
        },
        time: at(&iso(T + 2 * 3_600_000)),
        assume_timezone: "UTC".into(),
        ..RelatedRequest::default()
    };
    let r = find_related(&f, &req).unwrap();
    assert_eq!(r.links[0].event.id, 2, "the text still finds it");
    assert!(r.notes.contains(&RelatedNote::ClockDisagreement {
        delta_ms: 1_400 - 2 * 3_600_000,
        source_id: 1,
        source_name: "s1".into(),
    }));
    let far = RelatedRequest {
        time: at(&iso(T + 30 * 86_400_000)),
        ..req
    };
    assert!(find_related(&f, &far)
        .unwrap()
        .notes
        .iter()
        .any(|n| matches!(n, RelatedNote::OutsideData { .. })));
}

#[test]
fn the_screenshot_machine_clock_offset_is_applied() {
    let mut f = fake();
    f.sources[0].name = "WEB01-orders.clef".into();
    f.sources[0].offset_ms = -2_000;
    let req = RelatedRequest {
        clues: Clues {
            machine_names: vec!["web01".into()],
            ..Clues::default()
        },
        time: at(&iso(T)),
        assume_timezone: "UTC".into(),
        ..RelatedRequest::default()
    };
    let r = find_related(&f, &req).unwrap();
    assert_eq!(r.time.unwrap().at_ms, T - 2_000);
    assert!(r.notes.contains(&RelatedNote::OffsetApplied {
        source_name: "WEB01-orders.clef".into(),
        offset_ms: -2_000
    }));
}

#[test]
fn nothing_to_search_and_bounds() {
    let f = fake();
    let r = find_related(&f, &RelatedRequest::default()).unwrap();
    assert_eq!(r.notes, [RelatedNote::NothingToSearch]);
    assert!(r.links.is_empty() && f.asked.borrow().is_empty());
    let huge = Clues {
        ids: (0..100).map(|i| format!("id-{i}")).collect(),
        error_texts: vec!["x".repeat(5000), "\u{7}".into()],
        window_title: Some(" ".into()),
        ..Clues::default()
    }
    .bounded();
    assert_eq!(huge.ids.len(), MAX_CLUES);
    assert_eq!(huge.error_texts.len(), 1);
    assert_eq!(huge.error_texts[0].chars().count(), MAX_CLUE_CHARS);
    assert_eq!(huge.window_title, None);
    // At most MAX_SEARCHES clue searches, and the limit is honoured.
    let req = RelatedRequest {
        clues: huge,
        limit: 1,
        time: at(&iso(T)),
        assume_timezone: "UTC".into(),
        ..RelatedRequest::default()
    };
    let r = find_related(&f, &req).unwrap();
    assert!(r.links.len() <= 1);
    assert!(f.asked.borrow().len() <= 1 + MAX_SEARCHES);
    assert_eq!(ql_phrase(r#"a "b" \c"#), r#""a \"b\" \\c""#);
}

#[test]
fn ranking_is_deterministic() {
    let f = fake();
    let req = RelatedRequest {
        clues: Clues {
            exception_names: vec!["HttpRequestException".into()],
            ..Clues::default()
        },
        assume_timezone: "UTC".into(),
        ..RelatedRequest::default()
    };
    let a = find_related(&f, &req).unwrap();
    let b = find_related(&f, &req).unwrap();
    assert_eq!(a, b);
    // Equal scores without a time: the lower id first.
    assert_eq!(a.links.iter().map(|l| l.event.id).collect::<Vec<_>>(), [2, 3]);
}
