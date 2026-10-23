use super::*;

struct Empty;
impl AnnotationView for Empty {
    fn select(&self, _: &ViewFilter, _: usize) -> Result<Vec<ViewRecord>> {
        Ok(Vec::new())
    }
    fn query(&self, _: &Query, _: usize) -> Result<Vec<ViewRecord>> {
        Ok(Vec::new())
    }
    fn instances(&self) -> Vec<String> {
        Vec::new()
    }
    fn time_range(&self) -> Option<(i64, i64)> {
        None
    }
}

struct Fixed(Vec<(i64, i64, Option<&'static str>)>);
impl Annotator for Fixed {
    fn id(&self) -> &str {
        "fixed"
    }
    fn annotate(&self, view: &dyn AnnotationView, _: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        assert!(view.instances().is_empty() && view.time_range().is_none());
        assert!(view.query(&Query::default(), 1)?.is_empty() && view.select(&ViewFilter::default(), 1)?.is_empty());
        Ok(self
            .0
            .iter()
            .map(|(from, to, instance)| Annotation {
                kind: "k".into(),
                from: *from,
                to: *to,
                instance: instance.map(String::from),
                on: None,
                label: "l".into(),
                severity: Severity::Info,
                evidence_query: Query::default(),
            })
            .collect())
    }
}

#[test]
fn the_store_replaces_filters_and_hides() {
    let store = Store::default();
    let host = |a: Fixed| Registered {
        owner: "host".into(),
        guard: None,
        annotator: Arc::new(a),
    };
    let notes = store.run(
        &[host(Fixed(vec![(10, 20, None), (30, 30, Some("A")), (50, 40, Some("B"))]))],
        &Empty,
        None,
        &|_| false, // unguarded (host) annotators run regardless
    );
    assert!(notes.is_empty());
    let all = store.query(&AnnotationsRequest::default(), &|_| true);
    assert_eq!(all.len(), 3);
    assert_eq!((all[2].from, all[2].to), (40, 50), "normalised");
    assert_eq!(all[0].kind, "host/k");
    let q = |from, to, instance: Option<&str>| {
        store
            .query(
                &AnnotationsRequest {
                    from,
                    to,
                    instance: instance.map(String::from),
                },
                &|_| true,
            )
            .len()
    };
    assert_eq!(q(Some(21), None, None), 2);
    assert_eq!(q(None, Some(29), None), 1);
    assert_eq!(q(None, None, Some("A")), 2, "A plus the all-instance one");
    assert!(store.query(&AnnotationsRequest::default(), &|_| false).is_empty());
    // A re-run replaces the set.
    store.run(&[host(Fixed(vec![(1, 1, None)]))], &Empty, None, &|_| true);
    assert_eq!(store.query(&AnnotationsRequest::default(), &|_| true).len(), 1);
    store.clear();
    assert!(store.query(&AnnotationsRequest::default(), &|_| true).is_empty());
    // A guarded annotator of an unavailable plugin is skipped.
    let guarded = Registered {
        owner: "p".into(),
        guard: Some(Guard::new("p", Arc::new(crate::guard::Health::default()))),
        annotator: Arc::new(Fixed(vec![(1, 1, None)])),
    };
    store.run(std::slice::from_ref(&guarded), &Empty, None, &|_| false);
    assert!(store.query(&AnnotationsRequest::default(), &|_| true).is_empty());
    store.run(&[guarded], &Empty, Some(RecordCursor { id: 0, ts: None }), &|_| true);
    assert_eq!(store.query(&AnnotationsRequest::default(), &|_| true)[0].kind, "p/k");
}
