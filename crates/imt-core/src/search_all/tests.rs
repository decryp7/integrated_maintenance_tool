use super::*;
use crate::api::{Facet, FacetValue};
use crate::workspace::{Workspace, WorkspaceStore};
use crate::Profile;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const NEVER: &(dyn Fn() -> bool + Sync) = &|| false;

fn req(text: &str) -> SearchAllRequest {
    SearchAllRequest {
        text: text.into(),
        limit: 0,
    }
}

fn marked(parts: &[SnippetPart]) -> Vec<String> {
    parts.iter().filter(|p| p.mark).map(|p| p.text.clone()).collect()
}

fn joined(parts: &[SnippetPart]) -> String {
    parts.iter().map(|p| p.text.as_str()).collect()
}

/// A workspace over one invented log file `app.log` in a fresh folder.
fn store_with(name: &str, lines: &str) -> (WorkspaceStore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("imt-search-all-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("payments-service.log"), lines).unwrap();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    (WorkspaceStore::new(ws), dir)
}

fn lines(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!(
            "2026-10-14 09:00:{:02}.000 +00:00 [INF] worker-{} handled job in {} ms\n",
            i % 60,
            i % 3,
            10 + i
        ));
    }
    s.push_str("2026-10-14 09:01:00.000 +00:00 [ERR] Upstream TimeoutException while charging the card\n");
    s
}

fn doc(id: &str, scope: DocScope, title: &str, text: &str) -> SearchDoc {
    SearchDoc {
        id: id.into(),
        scope,
        title: title.into(),
        event: None,
        text: text.into(),
    }
}

// ---- the request ----

#[test]
fn requests_are_bounded_and_texts_without_words_search_nothing() {
    let (store, dir) = store_with("limits", &lines(80));
    let run_with = |text: &str, limit: usize| {
        run(
            &store,
            &Corpus::default(),
            &SearchAllRequest { text: text.into(), limit },
            &Control::new(BUDGET, NEVER),
        )
    };
    assert_eq!(run_with("  ", 0), Ok(None));
    assert_eq!(run_with("-- ::", 0), Ok(None), "punctuation has no words");
    let default = run_with("worker", 0).unwrap().unwrap();
    assert_eq!(default.events.items.len(), DEFAULT_PER_KIND);
    assert_eq!(default.events.total, 80, "the total counts every match, not only those shown");
    let max = run_with("worker", 10_000).unwrap().unwrap();
    assert_eq!(max.events.items.len(), MAX_PER_KIND);
    assert_eq!(run_with("worker", 3).unwrap().unwrap().events.items.len(), 3);
    let long = "a".repeat(MAX_TEXT_CHARS + 1);
    assert!(matches!(run_with(&long, 0), Err(SearchAllError::Invalid(m)) if m.contains("256")));
    assert!(
        run_with(&format!("  {}  ", "a".repeat(MAX_TEXT_CHARS)), 0).is_ok(),
        "surrounding spaces do not count"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn requests_are_camel_case_and_default_when_fields_are_missing() {
    let r: SearchAllRequest = serde_json::from_str(r#"{"text":"timeout"}"#).unwrap();
    assert_eq!(r, req("timeout"));
    let reply = SearchAllReply {
        fields: KindHits {
            total: 1,
            items: vec![],
            outcome: KindOutcome::Failed { message: "busy".into() },
        },
        events: KindHits {
            outcome: KindOutcome::TimedOut,
            ..KindHits::default()
        },
        ..SearchAllReply::default()
    };
    let json = serde_json::to_value(&reply).unwrap();
    assert_eq!(json["fields"]["outcome"], serde_json::json!({"kind": "failed", "message": "busy"}));
    assert_eq!(json["events"]["outcome"], serde_json::json!({"kind": "timedOut"}));
    assert_eq!(json["notes"]["outcome"], serde_json::json!({"kind": "complete"}));
    assert_eq!(serde_json::to_value(DocScope::Global).unwrap(), "global");
}

// ---- every kind, against the in-memory store ----

#[test]
fn finds_events_sources_notes_and_knowledge_with_the_words_marked() {
    let (store, dir) = store_with("kinds", &lines(5));
    let corpus = Corpus {
        notes: vec![
            SearchDoc {
                event: Some(crate::batch::EventId(5)),
                ..doc(
                    "n1",
                    DocScope::Project,
                    "Card declined",
                    "The timeout happens after the card is charged.",
                )
            },
            doc("n2", DocScope::Project, "Unrelated", "nothing to see"),
        ],
        knowledge: vec![
            doc("k1", DocScope::Global, "Glossary", "Timeouts: see the payments runbook."),
            doc("k2", DocScope::Project, "Timeout policy", "Retry three times."),
        ],
    };
    let reply = run(&store, &corpus, &req("timeout"), &Control::new(BUDGET, NEVER))
        .unwrap()
        .unwrap();

    assert_eq!(reply.events.total, 1);
    let ev = &reply.events.items[0];
    assert_eq!(ev.level, crate::model::Level::Error);
    assert_eq!(ev.source, "payments-service.log");
    assert_eq!(marked(&ev.snippet), vec!["Timeout"], "the word is marked as written, ignoring case");
    assert_eq!(reply.events.outcome, KindOutcome::Complete);

    assert_eq!(reply.notes.total, 1);
    assert_eq!(reply.notes.items[0].id, "n1");
    assert_eq!(reply.notes.items[0].event, Some(crate::batch::EventId(5)));
    assert_eq!(reply.knowledge.total, 2, "the prefix finds `Timeouts`, and a title match counts");
    assert_eq!(reply.knowledge.items[0].scope, DocScope::Global);
    assert_eq!(marked(&reply.knowledge.items[0].snippet), vec!["Timeout"]);

    let by_path = run(&store, &corpus, &req("payments serv"), &Control::new(BUDGET, NEVER))
        .unwrap()
        .unwrap();
    assert_eq!(by_path.sources.total, 1);
    assert_eq!(by_path.sources.items[0].records, 6);
    assert!(by_path.sources.items[0].path.ends_with("payments-service.log"));
    assert_eq!(marked(&by_path.sources.items[0].snippet), vec!["payments", "serv"]);
    assert_eq!(
        by_path.knowledge.total, 0,
        "every word must be there: the glossary names payments only"
    );
    let split = run(&store, &corpus, &req("policy retry"), &Control::new(BUDGET, NEVER))
        .unwrap()
        .unwrap();
    assert_eq!(split.knowledge.total, 1, "the words may be split between the title and the text");
    std::fs::remove_dir_all(dir).unwrap();
}

// ---- a fake store for fields, failures and the budget ----

#[derive(Default)]
struct Fake {
    fail_page: Option<StoreError>,
    fail_summary: Option<StoreError>,
    /// Facet requests answered before the next one fails.
    fail_facet_after: Option<usize>,
    /// How long each facet request takes.
    facet_delay: Duration,
    facets: AtomicUsize,
    /// Set when the page was read (the test's "newer search" arrives then).
    on_page: Option<&'static AtomicBool>,
    fields: Vec<FieldInfo>,
    marker_kinds: Vec<String>,
    exceptions: Vec<crate::exceptions::ExceptionGroup>,
    fail_exceptions: Option<StoreError>,
}

fn group(type_name: &str, message: &str, top: Option<&str>, count: usize) -> crate::exceptions::ExceptionGroup {
    crate::exceptions::ExceptionGroup {
        fingerprint: format!("fp-{type_name}"),
        type_name: type_name.into(),
        message: message.into(),
        top_frame: top.map(Into::into),
        count,
        instances: vec![],
        first_ts: None,
        last_ts: None,
        sample_ids: vec![7, 9],
    }
}

fn field(name: &str, distinct: usize, numeric: bool) -> FieldInfo {
    FieldInfo {
        name: name.into(),
        count: 10,
        distinct,
        numeric,
        derived: false,
    }
}

impl Fake {
    fn with_fields() -> Fake {
        Fake {
            fields: vec![
                field("CardType", 2, false),
                field("RequestId", 100_000, false),
                field("ElapsedMs", 50, true),
                field("Region", 3, false),
            ],
            marker_kinds: vec!["error".into(), "card-declined".into()],
            ..Fake::default()
        }
    }
}

impl SearchSource for Fake {
    fn summary(&self) -> StoreResult<SummaryReply> {
        if let Some(e) = &self.fail_summary {
            return Err(e.clone());
        }
        Ok(SummaryReply {
            total: 10,
            untimed: 0,
            from: None,
            to: None,
            levels: Default::default(),
            sources: vec![],
            unrecognized: vec![],
            fields: self.fields.clone(),
            default_group_by: vec![],
            marker_kinds: self.marker_kinds.clone(),
            profile_name: "test".into(),
        })
    }

    fn exception_groups(&self) -> StoreResult<Vec<crate::exceptions::ExceptionGroup>> {
        match &self.fail_exceptions {
            Some(e) => Err(e.clone()),
            None => Ok(self.exceptions.clone()),
        }
    }

    fn page(&self, req: &PageQuery) -> StoreResult<PageReply> {
        assert_eq!(req.query.text_mode, TextMode::Words, "events are found by words, from the index");
        if let Some(flag) = self.on_page {
            flag.store(true, Ordering::SeqCst);
        }
        match &self.fail_page {
            Some(e) => Err(e.clone()),
            None => Ok(PageReply {
                total: 0,
                offset: 0,
                rows: vec![],
                anchor_index: None,
            }),
        }
    }

    fn facet(&self, req: &FacetRequest) -> StoreResult<Facet> {
        let n = self.facets.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.facet_delay);
        if self.fail_facet_after.is_some_and(|after| n >= after) {
            return Err(StoreError::Busy("an import is running".into()));
        }
        let values: &[&str] = match req.field.as_str() {
            "CardType" => &["Visa", "Card Debit"],
            "Region" => &["eu-west", "card-us", "ap-south"],
            other => panic!("{other} has too many values (or is numeric) to be searched by value"),
        };
        Ok(Facet {
            field: req.field.clone(),
            values: values
                .iter()
                .map(|v| FacetValue {
                    value: (*v).into(),
                    count: 4,
                })
                .collect(),
            distinct: values.len(),
            total: 10,
        })
    }
}

#[test]
fn finds_field_names_values_of_fields_with_few_values_and_marker_kinds() {
    let fake = Fake::with_fields();
    let reply = run(&fake, &Corpus::default(), &req("card"), &Control::new(BUDGET, NEVER))
        .unwrap()
        .unwrap();
    let found: Vec<(String, Option<String>)> = reply.fields.items.iter().map(|f| (f.field.clone(), f.value.clone())).collect();
    assert_eq!(
        found,
        vec![
            ("CardType".to_string(), None),
            ("CardType".to_string(), Some("Card Debit".to_string())),
            ("Region".to_string(), Some("card-us".to_string())),
        ]
    );
    assert_eq!(reply.fields.items[1].count, 4);
    assert_eq!(marked(&reply.fields.items[0].snippet), vec!["Card"]);
    assert_eq!(fake.facets.load(Ordering::SeqCst), 2, "ids and numbers are not searched by value");
    assert_eq!(
        reply.markers.items.iter().map(|m| m.kind.as_str()).collect::<Vec<_>>(),
        vec!["card-declined"]
    );
    assert_eq!(reply.fields.outcome, KindOutcome::Complete);
}

#[test]
fn field_values_are_searched_in_at_most_a_bounded_number_of_fields() {
    let fake = Fake {
        fields: (0..MAX_VALUE_FIELDS + 10).map(|_| field("Region", 3, false)).collect(),
        ..Fake::default()
    };
    let reply = run(
        &fake,
        &Corpus::default(),
        &SearchAllRequest {
            text: "card".into(),
            limit: 2,
        },
        &Control::new(BUDGET, NEVER),
    )
    .unwrap()
    .unwrap();
    assert_eq!(fake.facets.load(Ordering::SeqCst), MAX_VALUE_FIELDS);
    assert_eq!(reply.fields.total, MAX_VALUE_FIELDS as u64);
    assert_eq!(reply.fields.items.len(), 2, "the limit applies to every kind");
}

#[test]
fn a_store_failure_fails_only_its_kinds() {
    let fake = Fake {
        fail_page: Some(StoreError::Engine("disk I/O error".into())),
        ..Fake::with_fields()
    };
    let corpus = Corpus {
        notes: vec![doc("n1", DocScope::Project, "Card", "card")],
        ..Corpus::default()
    };
    let reply = run(&fake, &corpus, &req("card"), &Control::new(BUDGET, NEVER)).unwrap().unwrap();
    assert_eq!(
        reply.events.outcome,
        KindOutcome::Failed {
            message: "store: disk I/O error".into()
        }
    );
    assert_eq!(reply.fields.total, 3);
    assert_eq!(reply.notes.total, 1);

    let fake = Fake {
        fail_summary: Some(StoreError::Busy("an import is running".into())),
        ..Fake::with_fields()
    };
    let reply = run(&fake, &corpus, &req("card"), &Control::new(BUDGET, NEVER)).unwrap().unwrap();
    let busy = KindOutcome::Failed {
        message: "busy: an import is running".into(),
    };
    assert_eq!(
        (&reply.sources.outcome, &reply.fields.outcome, &reply.markers.outcome),
        (&busy, &busy, &busy)
    );
    assert_eq!(reply.events.outcome, KindOutcome::Complete);
    assert_eq!(reply.notes.total, 1, "notes do not need the store");
    assert_eq!(reply.exceptions.outcome, KindOutcome::Complete);

    let fake = Fake {
        fail_facet_after: Some(1),
        ..Fake::with_fields()
    };
    let reply = run(&fake, &corpus, &req("card"), &Control::new(BUDGET, NEVER)).unwrap().unwrap();
    assert!(matches!(reply.fields.outcome, KindOutcome::Failed { .. }));
    assert_eq!(reply.fields.total, 2, "the name and the values found before the failure stay");
}

#[test]
fn finds_exception_groups_by_type_message_or_top_frame() {
    let fake = Fake {
        exceptions: vec![
            group(
                "System.TimeoutException",
                "The operation has timed out",
                Some("Payments.Gateway.Charge"),
                12,
            ),
            group(
                "System.InvalidOperationException",
                "Sequence contains no elements",
                Some("Payments.Card.Validate"),
                3,
            ),
        ],
        ..Fake::with_fields()
    };
    let found = |text: &str| {
        run(&fake, &Corpus::default(), &req(text), &Control::new(BUDGET, NEVER))
            .unwrap()
            .unwrap()
            .exceptions
    };
    let by_type = found("timeoutexc");
    assert_eq!(by_type.total, 1);
    let hit = &by_type.items[0];
    assert_eq!(
        (hit.count, hit.sample, hit.type_name.as_str()),
        (12, Some(crate::batch::EventId(7)), "System.TimeoutException")
    );
    assert_eq!(joined(&hit.snippet), "System.TimeoutException: The operation has timed out");
    assert_eq!(marked(&hit.snippet), vec!["TimeoutExc"]);
    assert_eq!(found("sequence elements").total, 1, "words may be in the message");
    assert_eq!(found("card validate").total, 1, "or the top frame");
    assert_eq!(found("payments").total, 2);
    assert_eq!(found("nothing here").total, 0);

    let broken = Fake {
        fail_exceptions: Some(StoreError::Busy("an import is running".into())),
        ..Fake::with_fields()
    };
    let reply = run(&broken, &Corpus::default(), &req("card"), &Control::new(BUDGET, NEVER))
        .unwrap()
        .unwrap();
    assert!(matches!(reply.exceptions.outcome, KindOutcome::Failed { .. }));
    assert_eq!(reply.fields.total, 3, "the other kinds are still searched");
}

// ---- the budget and cancellation ----

#[test]
fn a_spent_budget_skips_what_is_left_and_says_so() {
    let fake = Fake::with_fields();
    let reply = run(&fake, &Corpus::default(), &req("card"), &Control::new(Duration::ZERO, NEVER))
        .unwrap()
        .unwrap();
    for outcome in [
        &reply.events.outcome,
        &reply.sources.outcome,
        &reply.fields.outcome,
        &reply.markers.outcome,
        &reply.exceptions.outcome,
        &reply.notes.outcome,
        &reply.knowledge.outcome,
    ] {
        assert_eq!(outcome, &KindOutcome::TimedOut);
    }
    assert_eq!(fake.facets.load(Ordering::SeqCst), 0);

    // The budget runs out during the first facet request: the second field's values are not searched.
    let slow = Fake {
        facet_delay: Duration::from_millis(60),
        ..Fake::with_fields()
    };
    let reply = run(
        &slow,
        &Corpus::default(),
        &req("card"),
        &Control::new(Duration::from_millis(40), NEVER),
    )
    .unwrap()
    .unwrap();
    assert_eq!(slow.facets.load(Ordering::SeqCst), 1);
    assert_eq!(reply.fields.outcome, KindOutcome::TimedOut);
    assert_eq!(reply.fields.total, 2, "what was found before the budget ran out stays");
    assert_eq!(reply.events.outcome, KindOutcome::Complete);
}

#[test]
fn a_newer_search_cancels_this_one() {
    let already = || true;
    assert_eq!(
        run(
            &Fake::with_fields(),
            &Corpus::default(),
            &req("card"),
            &Control::new(BUDGET, &already)
        ),
        Err(SearchAllError::Cancelled)
    );
    // Replaced while the events were read: the rest is not searched.
    static REPLACED: AtomicBool = AtomicBool::new(false);
    let fake = Fake {
        on_page: Some(&REPLACED),
        ..Fake::with_fields()
    };
    let replaced = || REPLACED.load(Ordering::SeqCst);
    assert_eq!(
        run(&fake, &Corpus::default(), &req("card"), &Control::new(BUDGET, &replaced)),
        Err(SearchAllError::Cancelled)
    );
    assert_eq!(fake.facets.load(Ordering::SeqCst), 0);
    assert_eq!(SearchAllError::Cancelled.to_string(), "cancelled");
}

#[test]
fn every_query_store_is_a_search_source() {
    let (store, dir) = store_with("port", &lines(3));
    let dynamic: &dyn crate::ports::Store = &store;
    let reply = run(dynamic, &Corpus::default(), &req("worker"), &Control::new(BUDGET, NEVER))
        .unwrap()
        .unwrap();
    assert_eq!(reply.events.total, 3);
    assert_eq!(reply.sources.total, 0);
    assert_eq!(SearchSource::summary(dynamic).unwrap().total, 4);
    std::fs::remove_dir_all(dir).unwrap();
}

// ---- snippets ----

#[test]
fn snippets_are_a_bounded_window_around_the_first_match() {
    let q = WordQuery::parse("needle ");
    let n = needles(&q);
    let text = format!("{} needle {}", "a ".repeat(400), "b ".repeat(400));
    let parts = snippet(&text, &n);
    let shown = joined(&parts);
    assert!(shown.starts_with('…') && shown.ends_with('…'));
    assert_eq!(shown.chars().count(), SNIPPET_CHARS + 2);
    assert_eq!(marked(&parts), vec!["needle"]);
    // Near the end: a full window that ends with the text.
    let tail = format!("{}needle", "x ".repeat(300));
    let shown = joined(&snippet(&tail, &n));
    assert!(shown.starts_with('…') && shown.ends_with("needle"));
    assert_eq!(shown.chars().count(), SNIPPET_CHARS + 1);
    // No match: the start of the text; short texts are whole.
    assert_eq!(joined(&snippet("short text", &n)), "short text");
    assert_eq!(snippet("", &n), vec![]);
}

#[test]
fn snippets_fold_whitespace_ignore_case_merge_overlaps_and_read_a_bounded_prefix() {
    let q = WordQuery::parse("\"time out\" timeout");
    let n = needles(&q);
    let parts = snippet("A\tTIME\n\nOUT, then a Timeout.", &n);
    assert_eq!(joined(&parts), "A TIME OUT, then a Timeout.");
    assert_eq!(marked(&parts), vec!["TIME OUT", "Timeout"]);
    // Overlapping needles mark one run.
    let n = needles(&WordQuery::parse("abc bcd "));
    assert_eq!(marked(&snippet("xabcdx", &n)), vec!["abcd"]);
    // Non-ASCII text keeps its characters.
    let n = needles(&WordQuery::parse("größe "));
    assert_eq!(marked(&snippet("Die GRÖSSE und die Größe", &n)), vec!["Größe"]);
    // A match past the scanned prefix is not looked for.
    let huge = format!("{}needle", "a".repeat(MAX_SCAN_CHARS + 10));
    let n = needles(&WordQuery::parse("needle "));
    assert!(!contains_any(&huge, &n));
    assert!(contains_any("a NEEDLE here", &n));
    assert!(!contains_any("anything", &[String::new()]), "an empty needle matches nothing");
    // A phrase that is also a word is one needle.
    assert_eq!(needles(&WordQuery::parse("\"disk\" disk ")), vec!["disk".to_string()]);
}
