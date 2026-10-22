//! Starter questions for an empty chat, built from the health-check findings.

use super::time::format_hm;
use crate::access::StoreRef;
use debrief_core::api::{FindingAction, QueryRequest};
use debrief_core::query::Query;

const GENERIC: &[&str] = &[
    "What are the main problems in these logs?",
    "Which instance has the most errors?",
    "Summarise what happened before the crash",
    "Which messages are new or unusual?",
    "What should I look at first?",
];

/// Three to five questions, most relevant first.
pub fn suggested_prompts<'s>(store: impl Into<StoreRef<'s>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Ok(findings) = store.into().with(|s| s.overview(&QueryRequest { query: Query::default() })) {
        for f in findings.result.iter().take(4) {
            let q = match &f.action {
                Some(FindingAction::Zoom { from, .. }) => format!("What happened around {} ({})?", format_hm(*from), f.title),
                Some(FindingAction::Exception { .. }) => format!("Why is \"{}\" happening and what should I do?", f.title),
                _ => format!("Explain this finding: {}", f.title),
            };
            if !out.contains(&q) {
                out.push(q);
            }
        }
    }
    out.truncate(3);
    for g in GENERIC {
        if out.len() >= 4 {
            break;
        }
        out.push((*g).to_string());
    }
    out.truncate(5);
    out
}
