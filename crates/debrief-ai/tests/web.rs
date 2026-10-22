//! What the web helpers know about the loaded workspace: names the logs mark as private.

mod common;
use common::*;
use debrief_ai::web::{compose_query, private_terms_in, QueryIntent};

#[test]
fn names_known_from_the_logs_come_from_host_and_user_fields_and_file_names() {
    let terms = private_terms_in(store());
    assert!(terms.iter().any(|t| t == "WEB01"), "{terms:?}");
    assert!(terms.windows(2).all(|w| w[0] < w[1]), "sorted and unique: {terms:?}");
    // Plain words of a path are not private; identifier-like ones are.
    assert!(!terms.iter().any(|t| t == "logs" || t == "samples"), "{terms:?}");
    let q = compose_query(
        &QueryIntent {
            free_text: Some("connection refused on WEB01".into()),
            ..Default::default()
        },
        &|_| None,
        &terms,
    )
    .unwrap();
    assert!(!q.query.contains("WEB01"), "{q:?}");
    assert!(q.removed.iter().any(|r| r.contains("names from your logs")), "{q:?}");
}
