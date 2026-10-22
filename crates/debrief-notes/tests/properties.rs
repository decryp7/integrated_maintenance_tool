//! Property-based tests of the notes record read back from projects and sessions (untrusted: a project may come
//! from anyone): reading never panics, what it keeps passes every check and round-trips without a warning, and
//! the AI context block stays within its byte budget.
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p debrief-notes --test properties`.

use debrief_notes::context::{self, Focus};
use debrief_notes::{NoteRecord, NoteSet};
use proptest::prelude::*;
use proptest::strategy::BoxedStrategy;
use serde_json::{json, Value};

fn small_text() -> BoxedStrategy<Value> {
    prop_oneof![
        Just(json!("")),
        Just(json!(" ")),
        ".{0,30}".prop_map(Value::from),
        Just(json!("x".repeat(70_000)))
    ]
    .boxed()
}

fn number() -> BoxedStrategy<Value> {
    prop_oneof![
        any::<i64>().prop_map(Value::from),
        any::<u32>().prop_map(Value::from),
        Just(json!(-1)),
        Just(json!(1.5)),
        Just(json!("7"))
    ]
    .boxed()
}

fn anchor() -> BoxedStrategy<Value> {
    prop_oneof![
        Just(json!({"kind": "project"})),
        (number(), prop::option::of(number()), small_text(), small_text())
            .prop_map(|(id, ts, source, excerpt)| json!({"kind": "event", "eventId": id, "ts": ts, "source": source, "excerpt": excerpt})),
        number().prop_map(|at| json!({"kind": "instant", "at": at})),
        (number(), number()).prop_map(|(from, to)| json!({"kind": "range", "from": from, "to": to})),
        small_text().prop_map(|path| json!({"kind": "source", "path": path})),
        (small_text(), number(), prop::option::of(number())).prop_map(|(name, from, to)| json!({"kind": "marker", "name": name, "from": from, "to": to})),
        (small_text(), prop::option::of(small_text()), number(), prop::option::of(number()))
            .prop_map(|(chart, series, at, value)| json!({"kind": "chartPoint", "chart": chart, "series": series, "at": at, "value": value})),
        (small_text(), small_text(), number(), number(), prop::collection::vec((number(), number(), number(), number()), 0..3)).prop_map(
            |(key, name, width, height, regions)| json!({"kind": "image", "image": {"key": key, "name": name, "width": width, "height": height},
                "regions": regions.into_iter().map(|(x, y, w, h)| json!({"x": x, "y": y, "w": w, "h": h})).collect::<Vec<_>>()})
        ),
        small_text().prop_map(|k| json!({"kind": k})),
    ]
    .boxed()
}

fn note() -> BoxedStrategy<Value> {
    prop_oneof![
        4 => (small_text(), anchor(), small_text(), prop::collection::vec(small_text(), 0..4), prop::sample::select(vec!["info", "warning", "critical", "bogus"]), number(), number())
            .prop_map(|(id, anchor, text, tags, severity, c, u)| json!({"id": id, "anchor": anchor, "text": text, "tags": tags,
                "severity": severity, "author": "a", "createdMs": c, "updatedMs": u})),
        1 => Just(json!(42)),
        1 => Just(json!(null)),
        1 => Just(json!({"id": "n"})),
    ]
    .boxed()
}

fn record() -> BoxedStrategy<Value> {
    prop_oneof![
        5 => (prop_oneof![Just(json!(1)), Just(json!(0)), Just(json!(2)), Just(json!("1"))], prop::collection::vec(note(), 0..8))
            .prop_map(|(v, notes)| json!({"version": v, "notes": notes})),
        1 => Just(json!(null)),
        1 => Just(json!([])),
        1 => ".{0,20}".prop_map(Value::from),
        1 => Just(json!({"notes": []})),
    ]
    .boxed()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn reading_any_record_never_panics(v in record()) {
        let (set, warning) = NoteRecord::read(Some(&v));
        // Every kept note passes its checks, and ids are unique.
        for n in set.notes() {
            prop_assert!(n.check().is_ok(), "kept a note that fails its checks: {n:?}");
            prop_assert_eq!(set.notes().iter().filter(|m| m.id == n.id).count(), 1);
        }
        // What was kept is written and read back exactly, without a warning.
        let back = serde_json::to_value(NoteRecord::of(&set)).unwrap();
        prop_assert_eq!(NoteRecord::read(Some(&back)), (set.clone(), None));
        if warning.is_none() && v.get("version") == Some(&json!(1)) {
            let given = v.get("notes").and_then(Value::as_array).map_or(0, Vec::len);
            prop_assert_eq!(set.notes().len(), given, "notes were dropped without a warning");
        }
    }

    #[test]
    fn the_context_block_respects_its_budget(v in record(), budget in 0usize..4096, events in prop::collection::vec(any::<u64>(), 0..3)) {
        let (set, _) = NoteRecord::read(Some(&v));
        let focus = Focus { events, range: None, image: None };
        if let Some(block) = context::render(set.notes(), &focus, budget) {
            // The heading and the "left out" line are always there; past them, the budget holds.
            let bound = budget.clamp(200, context::MAX_CONTEXT_BYTES);
            prop_assert!(block.len() <= bound, "{} > {bound}", block.len());
        }
        for n in set.notes() {
            let _ = context::describe(&n.anchor);
        }
    }
}

#[test]
fn an_empty_set_is_no_record() {
    let back = serde_json::to_value(NoteRecord::of(&NoteSet::default())).unwrap();
    assert_eq!(NoteRecord::read(Some(&back)), (NoteSet::default(), None));
}
