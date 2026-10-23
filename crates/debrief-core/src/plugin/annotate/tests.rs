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

/// A view of `n` records per read.
struct Records(usize);
impl AnnotationView for Records {
    fn select(&self, _: &ViewFilter, _: usize) -> Result<Vec<ViewRecord>> {
        Ok((0..self.0)
            .map(|i| ViewRecord {
                record: crate::model::Record::new(i as u32),
                source: "s".into(),
                instance: "i".into(),
            })
            .collect())
    }
    fn query(&self, _: &Query, limit: usize) -> Result<Vec<ViewRecord>> {
        self.select(&ViewFilter::default(), limit)
    }
    fn instances(&self) -> Vec<String> {
        Vec::new()
    }
    fn time_range(&self) -> Option<(i64, i64)> {
        None
    }
}

/// An annotator with its own id that reads the view `reads` times and marks `at`.
struct Reading {
    id: String,
    reads: usize,
    at: i64,
}
impl Annotator for Reading {
    fn id(&self) -> &str {
        &self.id
    }
    fn annotate(&self, view: &dyn AnnotationView, _: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        for _ in 0..self.reads {
            view.select(&ViewFilter::default(), 10)?;
        }
        Ok(vec![Annotation {
            kind: "k".into(),
            from: self.at,
            to: self.at,
            instance: None,
            on: None,
            label: self.id.clone(),
            severity: Severity::Info,
            evidence_query: Query::default(),
        }])
    }
}

fn reading(n: usize) -> Vec<Registered> {
    (0..n)
        .map(|i| Registered {
            owner: format!("o{i}"),
            guard: None,
            annotator: Arc::new(Reading {
                id: format!("a{i:02}"),
                reads: 3,
                at: 100 - i as i64,
            }),
        })
        .collect()
}

#[test]
fn annotators_in_parallel_store_what_the_sequential_run_stores_and_count_what_they_read() {
    let annotators = reading(9);
    let store = Store::default();
    let last = Mutex::new(None);
    let record = |p: AnnotateProgress| *last.lock().unwrap() = Some(p);
    let never = || false;
    let ctl = RunControl {
        cancelled: &never,
        progress: &record,
    };
    let notes = store.run_with(&annotators, &Records(5), None, &|_| true, &ctl);
    assert!(notes.is_empty());
    let all = store.query(&AnnotationsRequest::default(), &|_| true);
    assert_eq!(all.len(), 9);
    // Same as running them one by one.
    let sequential = Store::default();
    for a in &annotators {
        sequential.run(std::slice::from_ref(a), &Records(5), None, &|_| true);
    }
    let want = sequential.query(&AnnotationsRequest::default(), &|_| true);
    assert_eq!(
        all.iter().map(|a| (&a.annotator, a.from)).collect::<Vec<_>>(),
        want.iter().map(|a| (&a.annotator, a.from)).collect::<Vec<_>>()
    );
    let p = last.lock().unwrap().unwrap();
    assert_eq!((p.annotators_done, p.annotators_total), (9, 9));
    assert_eq!(p.events_read, 9 * 3 * 5, "n of m events: every record the annotators were given");
}

#[test]
fn a_cancelled_run_stops_the_annotators_and_keeps_the_previous_sets() {
    let annotators = reading(4);
    let store = Store::default();
    store.run(&annotators, &Records(1), None, &|_| true);
    let before = store.query(&AnnotationsRequest::default(), &|_| true);
    assert_eq!(before.len(), 4);
    let reads = AtomicUsize::new(0);
    let cancel_after_two = || reads.load(Ordering::Relaxed) >= 2;
    let count = |p: AnnotateProgress| reads.store(usize::try_from(p.events_read).unwrap(), Ordering::Relaxed);
    let ctl = RunControl {
        cancelled: &cancel_after_two,
        progress: &count,
    };
    let more = reading(4);
    let notes = store.run_with(&more, &Records(1), None, &|_| true, &ctl);
    assert!(notes.is_empty(), "a cancel is not a failure of the plugins: {notes:?}");
    let after = store.query(&AnnotationsRequest::default(), &|_| true);
    assert_eq!(
        after.iter().map(|a| (&a.annotator, a.from)).collect::<Vec<_>>(),
        before.iter().map(|a| (&a.annotator, a.from)).collect::<Vec<_>>()
    );
}
