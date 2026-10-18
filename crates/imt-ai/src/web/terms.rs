//! "Names known from the logs": values the loaded workspace itself marks as private. Queries and browser
//! actions use them to keep workspace data from leaving the machine.

use imt_core::api::FacetRequest;
use imt_core::LogStore;

/// Names the logs themselves identify as private, in one short borrow of the store: values of host, user and
/// address fields, and identifier-like parts of file paths (those with digits, hyphens or underscores; plain
/// words are not private).
pub fn private_terms_in(store: &dyn LogStore) -> Vec<String> {
    const HOSTISH: &[&str] = &[
        "machinename",
        "host",
        "hostname",
        "machine",
        "computer",
        "computername",
        "server",
        "username",
        "user",
        "userid",
        "clientip",
        "instance",
        "instanceid",
    ];
    let mut terms: Vec<String> = Vec::new();
    let summary = store.summary();
    for f in summary.fields.iter().filter(|f| HOSTISH.contains(&f.name.to_lowercase().as_str())) {
        if let Ok(facet) = store.facet(&FacetRequest {
            query: Default::default(),
            field: f.name.clone(),
            limit: 50,
        }) {
            terms.extend(facet.values.into_iter().map(|v| v.value));
        }
    }
    for s in &summary.sources {
        for seg in s.path.split(['/', '\\']) {
            let stem = seg.split('.').next().unwrap_or(seg);
            for part in [seg, stem] {
                if part.len() >= 4 && part.chars().any(|c| c.is_ascii_digit() || c == '-' || c == '_') {
                    terms.push(part.to_string());
                }
            }
        }
    }
    terms.sort();
    terms.dedup();
    terms
}
