//! What redaction would change in the events of a query (`redact::summary`): counted over a store's `scan`, with
//! samples, a scan budget, and failures reported rather than hidden.

mod common;

use common::{Ev, Fake};
use imt_core::ports::{Filter, StoreError};
use imt_core::redact::view::{summary, summary_within, MAX_SAMPLES};
use imt_core::redact::RedactionSummaryRequest;
use imt_core::Profile;

fn cfg(toml: &str) -> imt_core::redact::RedactCfg {
    Profile::from_toml(&format!("name = 'p'\n[redact]\n{toml}")).unwrap().redact
}

fn store() -> Fake {
    let events: Vec<Ev> = vec![
        (Some(100), 2, 0, "mail ann@example.org about order 1", None),
        (Some(200), 2, 0, "nothing sensitive here", None),
        (Some(300), 2, 0, "ping bob@example.org and cy@example.org", None),
        (Some(400), 2, 0, "all quiet", None),
        (None, 2, 0, "untimed dan@example.org", None),
    ];
    Fake::new(events)
}

fn request(samples: usize, query: Filter) -> RedactionSummaryRequest {
    serde_json::from_value(serde_json::json!({"samples": samples, "query": query})).unwrap()
}

#[test]
fn it_counts_changed_events_hits_and_samples() {
    let s = summary(&store(), &cfg("detectors = ['email']"), &request(2, Filter::default())).unwrap();
    assert_eq!((s.scanned, s.changed, s.truncated), (5, 3, false));
    assert_eq!(
        s.hits.iter().map(|h| (h.rule.as_str(), h.count)).collect::<Vec<_>>(),
        [("email", 4)]
    );
    assert_eq!(s.samples.len(), 2, "no more samples than asked");
    assert_eq!(s.samples[0].before, "mail ann@example.org about order 1");
    assert_eq!(s.samples[0].after, "mail <email> about order 1");
    // Asking for none, or for more than the maximum, is clamped.
    let none = summary(&store(), &cfg("detectors = ['email']"), &request(0, Filter::default())).unwrap();
    assert!(none.samples.is_empty() && none.changed == 3);
    let many = summary(
        &store(),
        &cfg("detectors = ['email']"),
        &request(MAX_SAMPLES + 50, Filter::default()),
    )
    .unwrap();
    assert_eq!(many.samples.len(), 3);
}

#[test]
fn only_the_events_of_the_query_are_scanned() {
    let query = Filter {
        from: Some(150),
        to: Some(350),
        ..Filter::default()
    };
    let s = summary(&store(), &cfg("detectors = ['email']"), &request(5, query)).unwrap();
    assert_eq!((s.scanned, s.changed), (2, 1));
}

#[test]
fn a_scan_budget_truncates_and_says_so() {
    let s = summary_within(&store(), &cfg("detectors = ['email']"), &request(5, Filter::default()), 2).unwrap();
    assert_eq!((s.scanned, s.changed, s.truncated), (2, 1, true));
    // A budget as large as the data is not a truncation.
    let exact = summary_within(&store(), &cfg("detectors = ['email']"), &request(5, Filter::default()), 5).unwrap();
    assert!(!exact.truncated && exact.scanned == 5);
    // The budget cannot exceed the engine's maximum.
    let big = summary_within(&store(), &cfg("detectors = ['email']"), &request(5, Filter::default()), usize::MAX).unwrap();
    assert_eq!(big.scanned, 5);
}

#[test]
fn bad_rules_and_store_failures_are_errors_with_a_message() {
    let bad = summary(
        &store(),
        &cfg("[[redact.rule]]\nname = 'x'\nregex = '('"),
        &request(1, Filter::default()),
    )
    .unwrap_err();
    assert!(matches!(bad, StoreError::Invalid(_)), "{bad:?}");
    let mut failing = store();
    failing.fail_scan = true;
    let err = summary(&failing, &cfg("detectors = ['email']"), &request(1, Filter::default())).unwrap_err();
    assert!(err.to_string().contains("scan failed"), "{err}");
}
