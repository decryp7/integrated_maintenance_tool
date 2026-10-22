//! Annotators over the store's ports ([`PortView`]): they read events through the bounded view, and a failing
//! or panicking annotator is contained by its plugin's guard (its note is reported, the others still run, the
//! store keeps answering).

mod common;

use anyhow::Result;
use common::{store_with, TempDir};
use imt_core::api::Severity;
use imt_core::plugin::annotate::{Annotation, AnnotationView, AnnotationsRequest, Annotator, PortView, RecordCursor, ViewFilter};
use imt_core::plugin::Manifest;
use imt_core::ports::Query as _;
use imt_core::query::Query;
use imt_core::{Plugin, PluginRegistry, Profile};
use std::sync::Arc;

struct Spans;
impl Annotator for Spans {
    fn id(&self) -> &str {
        "test.spans"
    }
    fn annotate(&self, view: &dyn AnnotationView, _: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        let (from, to) = view.time_range().unwrap_or((0, 0));
        let errors = view.select(&ViewFilter::default(), 100)?;
        Ok(vec![Annotation {
            kind: "window".into(),
            from,
            to,
            instance: None,
            on: None,
            label: format!("{} events, {} instances", errors.len(), view.instances().len()),
            severity: Severity::Info,
            evidence_query: Query::default(),
        }])
    }
}

struct Panicky;
impl Annotator for Panicky {
    fn id(&self) -> &str {
        "test.panicky"
    }
    fn annotate(&self, _: &dyn AnnotationView, _: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        panic!("annotator exploded")
    }
}

struct Host(&'static str, fn(&mut PluginRegistry) -> Result<()>);
impl Plugin for Host {
    fn id(&self) -> &'static str {
        self.0
    }
    fn name(&self) -> &'static str {
        self.0
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn manifest(&self) -> Manifest {
        Manifest::new(self.0, "1.0.0")
    }
    fn register(&self, r: &mut PluginRegistry) -> Result<()> {
        (self.1)(r)
    }
}

#[test]
fn a_panicking_annotator_is_contained_and_the_others_still_run_over_the_store() {
    let logs = TempDir::new("annot-logs");
    std::fs::write(
        logs.0.join("a.log"),
        "2026-10-14 10:00:01 INFO one\n2026-10-14 10:00:02 ERROR two\n2026-10-14 10:00:03 INFO three\n",
    )
    .unwrap();
    let dir = TempDir::new("annot-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();

    let mut registry = PluginRegistry::new();
    registry
        .install(&Host("good", |r| r.add_annotator(Arc::new(Spans)).map(|_| ())))
        .unwrap();
    registry
        .install(&Host("chaos", |r| r.add_annotator(Arc::new(Panicky)).map(|_| ())))
        .unwrap();
    assert!(registry.has_annotators());

    // Quiet the expected panic message.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let view = PortView::new(&store).unwrap();
    registry.run_annotators(&view, None);
    registry.run_annotators(&view, None);
    let _ = std::panic::take_hook();
    std::panic::set_hook(prev);

    let all = registry.annotations(&AnnotationsRequest::default());
    assert_eq!(all.len(), 1, "only the healthy annotator produced annotations");
    assert_eq!(all[0].label, "3 events, 1 instances");
    let plugins = registry.plugins();
    let chaos = plugins.iter().find(|p| p.id == "chaos").unwrap();
    assert_eq!(chaos.failures, 2, "each panic is counted against its plugin");
    assert!(chaos.last_error.as_deref().unwrap().contains("annotator exploded"));
    let good = plugins.iter().find(|p| p.id == "good").unwrap();
    assert_eq!(good.failures, 0);
    // The store is unharmed.
    assert_eq!(store.summary().unwrap().total, 3);
}

#[test]
fn the_view_is_bounded_and_a_bad_condition_is_an_error() {
    let logs = TempDir::new("annot-bound-logs");
    let text: String = (0..50).map(|i| format!("2026-10-14 10:00:{i:02} INFO event {i}\n")).collect();
    std::fs::write(logs.0.join("a.log"), text).unwrap();
    let dir = TempDir::new("annot-bound-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    let view = PortView::new(&store).unwrap().with_max_scan(10);
    assert_eq!(
        view.select(&ViewFilter::default(), 1000).unwrap().len(),
        10,
        "at most max_scan events are read"
    );
    assert_eq!(view.select(&ViewFilter::default(), 3).unwrap().len(), 3);
    let newest = PortView::new(&store)
        .unwrap()
        .select(
            &ViewFilter {
                newest_first: true,
                ..ViewFilter::default()
            },
            2,
        )
        .unwrap();
    assert!(newest[0].record.ts >= newest[1].record.ts);
    let bad = ViewFilter {
        when: vec![imt_core::condition::ConditionCfg {
            field: "x".into(),
            op: "bogus".into(),
            value: None,
        }],
        ..ViewFilter::default()
    };
    assert!(view.select(&bad, 1).is_err());
}
