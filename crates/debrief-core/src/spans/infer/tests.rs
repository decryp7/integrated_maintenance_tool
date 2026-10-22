use super::*;
use crate::model::Record;
use serde_json::json;

const T0: i64 = 1_791_968_400_000;

fn rec(id: u32, minute: i64, msg: &str) -> Record {
    let mut r = Record::new(id + 1);
    r.id = id;
    r.ts = Some(T0 + minute * 60_000);
    r.message = msg.into();
    r
}

fn with(mut r: Record, k: &str, v: serde_json::Value) -> Record {
    r.fields.insert(k.into(), v);
    r
}

fn pairs(examples: &[(Record, Record)]) -> Vec<(&dyn RowRef, &dyn RowRef)> {
    examples.iter().map(|(a, b)| (a as &dyn RowRef, b as &dyn RowRef)).collect()
}

/// The proposal's best candidate evaluated over `events` (one lane).
fn evaluate(c: &SpanCandidate, events: &[Record]) -> SpanEvaluation {
    let profile = Profile::default_profile().with_override(&c.profile_override).unwrap();
    let compiled = CompiledProfile::compile(profile).unwrap();
    let mut ev = SpanEvaluator::new(&compiled, &c.kind).unwrap();
    for r in events {
        ev.add(r, "app.log", "all");
    }
    ev.finish()
}

/// Opens and closes of keys `A`..`E`: A and B overlap, C is opened twice before it closes, D
/// closes without an open and E stays open; plus noise.
fn lifecycle(open: impl Fn(&str) -> String, close: impl Fn(&str) -> String) -> Vec<Record> {
    let lines = [
        (0, open("A1")),
        (1, "heartbeat ok".to_string()),
        (2, open("B2")),
        (3, close("A1")),
        (4, close("B2")),
        (5, open("C3")),
        (6, open("C3")),
        (7, close("C3")),
        (8, close("D4")),
        (9, open("E5")),
        (10, "heartbeat ok".to_string()),
    ];
    lines.iter().enumerate().map(|(i, (m, l))| rec(i as u32, *m, l)).collect()
}

fn check_lifecycle(e: &SpanEvaluation) {
    // A, B and C: C opened twice before it closes is one run (consecutive starts are one start, as in
    // every [[span]] rule).
    assert_eq!(e.spans, 3, "{e:#?}");
    assert_eq!(e.unmatched_starts, 0, "{e:#?}");
    assert_eq!(e.unmatched_ends, 1, "{e:#?}"); // D
    assert_eq!(e.open, 1, "{e:#?}"); // E
    assert_eq!(e.distinct_keys, 4, "{e:#?}");
    assert_eq!(e.max_concurrent, 2, "{e:#?}"); // A and B
    assert_eq!(e.longest_ms, Some(3 * 60_000));
    assert_eq!(e.shortest_ms, Some(2 * 60_000));
    assert_eq!(e.events, 11);
    assert!(
        e.samples
            .iter()
            .any(|s| s.key == "A1" && s.status == RunStatus::Closed && s.start_id == Some(0) && s.stop_id == Some(3)),
        "{:#?}",
        e.samples
    );
}

#[test]
fn verb_before_the_key() {
    // "open XXXXXX" / "close XXXXXX"
    let ex = [(rec(0, 0, "open XXXXXX"), rec(1, 1, "close XXXXXX"))];
    let cands = propose(&pairs(&ex), &SpanHints::default(), &Profile::default_profile()).unwrap();
    let c = &cands[0];
    assert_eq!(c.example_keys, vec!["XXXXXX"]);
    assert!(
        matches!(&c.key, KeySource::Pattern { regex, .. } if regex.contains("(?:open|close)")),
        "{c:#?}"
    );
    assert!(c.description.contains("\"open\" starts, \"close\" ends"), "{}", c.description);
    check_lifecycle(&evaluate(c, &lifecycle(|k| format!("open {k}"), |k| format!("close {k}"))));
}

#[test]
fn verb_after_the_key() {
    // "XXXXX started" / "XXXXX ended"
    let ex = [(rec(0, 0, "XXXXX started"), rec(1, 1, "XXXXX ended"))];
    let cands = propose(&pairs(&ex), &SpanHints::default(), &Profile::default_profile()).unwrap();
    assert_eq!(cands[0].example_keys, vec!["XXXXX"]);
    check_lifecycle(&evaluate(
        &cands[0],
        &lifecycle(|k| format!("{k} started"), |k| format!("{k} ended")),
    ));
}

#[test]
fn key_in_the_middle_of_tab_views() {
    // "View XXXX1 loaded" / "View XXXX1 closed": several views open at once.
    let ex = [
        (rec(0, 0, "View XXXX1 loaded"), rec(1, 1, "View XXXX1 closed")),
        (rec(2, 0, "View XXXX2 loaded"), rec(3, 2, "View XXXX2 closed")),
    ];
    let hints = SpanHints {
        name: Some("view".into()),
        key_field: None,
    };
    let cands = propose(&pairs(&ex), &hints, &Profile::default_profile()).unwrap();
    let c = &cands[0];
    assert_eq!(c.kind, "view");
    assert_eq!(c.example_keys, vec!["XXXX1", "XXXX2"]);
    // "View" is shared too, but it does not vary between the examples: not a key.
    assert!(cands.iter().all(|c| c.example_keys != vec!["View", "View"]), "{cands:#?}");
    check_lifecycle(&evaluate(
        c,
        &lifecycle(|k| format!("View {k} loaded"), |k| format!("View {k} closed")),
    ));
}

#[test]
fn bracketed_ids_with_alternatives() {
    // "[Tab:42] open dashboard" / "[Tab:42] close dashboard": the id, and "dashboard" as an alternative.
    let ex = [(rec(0, 0, "[Tab:42] open dashboard"), rec(1, 1, "[Tab:42] close dashboard"))];
    let cands = propose(&pairs(&ex), &SpanHints::default(), &Profile::default_profile()).unwrap();
    assert_eq!(cands[0].example_keys, vec!["42"], "{cands:#?}");
    assert!(cands.len() > 1, "alternatives are offered");
    assert!(cands.iter().any(|c| c.example_keys == vec!["dashboard"]));
    let e = evaluate(
        &cands[0],
        &lifecycle(|k| format!("[Tab:{k}] open dashboard"), |k| format!("[Tab:{k}] close dashboard")),
    );
    check_lifecycle(&e);
}

#[test]
fn json_fields_pair_and_tell_starts_from_ends() {
    let start = |id: u32, m: i64, key: &str| with(with(rec(id, m, "request"), "Phase", json!("begin")), "RequestId", json!(key));
    let end = |id: u32, m: i64, key: &str| with(with(rec(id, m, "request"), "Phase", json!("end")), "RequestId", json!(key));
    let ex = [(start(0, 0, "r-1"), end(1, 1, "r-1")), (start(2, 0, "r-2"), end(3, 1, "r-2"))];
    let cands = propose(&pairs(&ex), &SpanHints::default(), &Profile::default_profile()).unwrap();
    let c = &cands[0];
    assert_eq!(c.key, KeySource::Field { name: "RequestId".into() });
    assert_eq!((c.start.field.as_str(), c.stop.field.as_str()), ("Phase", "Phase"));
    let events = vec![start(0, 0, "a"), start(1, 1, "b"), end(2, 2, "a"), end(3, 3, "b"), end(4, 4, "x")];
    let e = evaluate(c, &events);
    assert_eq!((e.spans, e.unmatched_ends, e.max_concurrent), (2, 1, 2), "{e:#?}");
    // A hint picks the field.
    let hinted = propose(
        &pairs(&ex),
        &SpanHints {
            name: None,
            key_field: Some("RequestId".into()),
        },
        &Profile::default_profile(),
    )
    .unwrap();
    assert_eq!(hinted.len(), 1);
}

#[test]
fn proposals_keep_the_profile_and_refuse_examples_that_cannot_work() {
    let ex = [(rec(0, 0, "open K1"), rec(1, 1, "close K1"))];
    let profile = Profile::default_profile();
    let c = &propose(&pairs(&ex), &SpanHints::default(), &profile).unwrap()[0];
    let merged = profile.with_override(&c.profile_override).unwrap();
    // The profile's own markers and spans stay; the rule's are added.
    assert_eq!(merged.marker.len(), profile.marker.len() + 2);
    assert_eq!(merged.span.len(), profile.span.len() + 1);
    assert!(merged.span.iter().any(|s| s.kind == "span" && s.per == "field:SpanKey"));
    // A second proposal does not clash with the first.
    let again = &propose(&pairs(&ex), &SpanHints::default(), &merged).unwrap()[0];
    assert_eq!(again.kind, "span2");
    assert!(propose(&[], &SpanHints::default(), &profile).is_err());
    let same = [(rec(0, 0, "tick"), rec(1, 1, "tick"))];
    let e = propose(&pairs(&same), &SpanHints::default(), &profile).unwrap_err().to_string();
    assert!(e.contains("do not differ"), "{e}");
    let unshared = [(rec(0, 0, "open A1"), rec(1, 1, "close B2"))];
    assert!(propose(&pairs(&unshared), &SpanHints::default(), &profile).is_err());
    // An AI-proposed override is evaluated the same way; a missing span is an error.
    let compiled = CompiledProfile::compile(merged).unwrap();
    assert!(SpanEvaluator::new(&compiled, "nope").is_err());
}

#[test]
fn evaluation_is_linear_and_skips_unrelated_events_cheaply() {
    // 10M events with DEBRIEF_BENCH_SPANS=1 (run in release); 500k otherwise.
    let n: u32 = if std::env::var("DEBRIEF_BENCH_SPANS").is_ok_and(|v| v == "1") {
        10_000_000
    } else {
        500_000
    };
    let ex = [(rec(0, 0, "open K1"), rec(1, 1, "close K1"))];
    let c = &propose(&pairs(&ex), &SpanHints::default(), &Profile::default_profile()).unwrap()[0];
    let profile = Profile::default_profile().with_override(&c.profile_override).unwrap();
    let compiled = CompiledProfile::compile(profile).unwrap();
    let mut ev = SpanEvaluator::new(&compiled, &c.kind).unwrap();
    let started = std::time::Instant::now();
    let mut r = rec(0, 0, "");
    for i in 0..n {
        r.id = i;
        r.ts = Some(T0 + i64::from(i));
        r.message = match i % 1000 {
            0 => format!("open K{}", i / 1000 % 50),
            500 => format!("close K{}", i / 1000 % 50),
            _ => "processing item".into(),
        };
        ev.add(&r, "app.log", "all");
    }
    let e = ev.finish();
    let secs = started.elapsed().as_secs_f64();
    eprintln!("{n} events evaluated in {secs:.2} s: {} spans", e.spans);
    // Every run is counted: kept (up to the per-lane limit of a span rule) or dropped.
    assert_eq!(e.spans + e.dropped, (n / 1000) as usize, "{e:?}");
    assert!(secs < 120.0, "{secs} s");
}

const JOB_PROFILE: &str = r#"
[[marker]]
kind = "job-start"
when = [{ field = "@message", op = "contains", value = "job begin" }]
[[marker]]
kind = "job-stop"
when = [{ field = "@message", op = "contains", value = "job end" }]
[[span]]
kind = "job"
start = "job-start"
stop = "job-stop"
per = "field:Job"
restart_on_start = true
[[span]]
kind = "ghost"
start = "never-defined"
stop = "job-stop"
"#;

#[test]
fn an_evaluator_pairs_per_key_counts_unmatched_ends_and_open_runs_and_refuses_unknown_markers() {
    let profile = Profile::default_profile().with_override(JOB_PROFILE).unwrap();
    let compiled = CompiledProfile::compile(profile).unwrap();
    // A span naming a marker kind the profile does not define cannot be evaluated, and the error says which.
    let e = SpanEvaluator::new(&compiled, "ghost")
        .err()
        .expect("unknown marker kinds are refused")
        .to_string();
    assert!(e.contains("never-defined"), "{e}");

    let mut ev = SpanEvaluator::new(&compiled, "job").unwrap();
    assert!(!ev.gates().is_empty(), "start and stop each have a gate");
    let job = |id: u32, minute: i64, msg: &str, key: &str| with(rec(id, minute, msg), "Job", json!(key));
    let events = [
        job(0, 0, "job begin", "a"),
        job(1, 1, "job begin", "a"), // a second start before a stop is the same run
        job(2, 2, "job end", "a"),
        job(3, 3, "job end", "b"), // a stop without a start
        job(4, 4, "job begin", "c"),
        job(5, 5, "unrelated chatter", "c"),
    ];
    for r in &events {
        ev.add(r, "app.log", "WEB01");
    }
    let mut untimed = job(6, 6, "job begin", "d");
    untimed.ts = None;
    ev.add(&untimed, "app.log", "WEB01");
    let e = ev.finish();
    assert_eq!(e.events, 7, "events without a time are looked at but never paired");
    assert_eq!((e.spans, e.unmatched_starts, e.unmatched_ends, e.open), (1, 0, 1, 1), "{e:#?}");
    assert_eq!(
        (e.longest_ms, e.shortest_ms),
        (Some(120_000), Some(120_000)),
        "from the first start to the stop"
    );
    assert_eq!(e.distinct_keys, 2, "a and c: {e:#?}");
}
