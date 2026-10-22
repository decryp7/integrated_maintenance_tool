//! The AI parts of known issues and the incident summary, with a scripted model over the generated incident:
//! suggestions are engine-verified (quote in the notes, valid query, matches events) and the narrative's
//! citations are checked against what the report and the tools returned.

mod common;
use common::*;
use debrief_ai::assistant::*;
use debrief_ai::llm::ScriptedProvider;
use debrief_ai::AiError;
use debrief_core::known_issues::{KnowledgeText, Origin};
use serde_json::json;

fn assistant(replies: &[&str]) -> Assistant<ScriptedProvider> {
    Assistant::new(ScriptedProvider::text(replies), AssistantConfig::default())
}

const NOTES: &str = "# Payments\n\nWhen the gateway says the request timed out, raise the gateway\n timeout to 30 seconds and check the provider status page.\n\nDisk alerts: free space on D:.\n";

fn notes() -> Vec<KnowledgeText<'static>> {
    vec![KnowledgeText {
        name: "payments.md",
        text: NOTES,
    }]
}

#[test]
fn suggestions_are_kept_only_when_quoted_valid_and_matching() {
    let reply = json!({"issues": [
        {"title": "Gateway timeouts", "fix": "raise the gateway timeout to 30 seconds and check the provider status page.", "ql": "message contains \"timed out\""},
        {"title": "Invented fix", "fix": "Reboot the moon", "ql": "level >= error"},
        {"title": "Bad query", "fix": "free space on D:", "ql": "level >="},
        {"title": "Matches nothing", "fix": "free space on D:", "ql": "message contains \"zzz-never\""},
        {"title": "", "fix": "free space on D:", "ql": "level >= error"}
    ]})
    .to_string();
    let a = assistant(&[&reply]);
    let found = a
        .suggest_known_issues(store(), &notes(), "- Failed to capture payment: request timed out (169)")
        .unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    let m = &found[0];
    assert_eq!(m.origin, Origin::Ai);
    assert_eq!(m.issue.file, "payments.md");
    assert_eq!(m.issue.id, "ai-gateway-timeouts");
    assert!(m.count > 0 && !m.sample_ids.is_empty());
    // The model saw the problems and the notes, as data.
    let sent = a.provider().requests()[0].clone();
    assert!(sent.messages[0].content.contains("request timed out") && sent.messages[0].content.contains("Disk alerts"));
    assert!(sent.json_schema.is_some());
}

#[test]
fn suggestions_are_bounded_and_need_notes() {
    let many: Vec<_> = (0..12)
        .map(|i| json!({"title": format!("Issue {i}"), "fix": "free space on D:", "ql": "level >= error"}))
        .collect();
    let a = assistant(&[&json!({ "issues": many }).to_string()]);
    assert_eq!(a.suggest_known_issues(store(), &notes(), "x").unwrap().len(), MAX_SUGGESTIONS);
    let none = [KnowledgeText {
        name: "empty.md",
        text: "  ",
    }];
    let e = assistant(&[]).suggest_known_issues(store(), &none, "x").unwrap_err();
    assert!(matches!(e, AiError::Invalid(m) if m.contains("no knowledge notes")));
    // An answer that is not the requested JSON is an error after one repair round, never trusted.
    let e = assistant(&["not json", "still not"])
        .suggest_known_issues(store(), &notes(), "x")
        .unwrap_err();
    assert!(matches!(e, AiError::Malformed(_)));
    // "No note fits" is an empty list.
    assert!(assistant(&[r#"{"issues": []}"#])
        .suggest_known_issues(store(), &notes(), "x")
        .unwrap()
        .is_empty());
}

#[test]
fn the_incident_narrative_cites_report_events_as_verified() {
    let id = first_id("level >= error");
    let other = id + 1_000_000;
    let text = format!("Payments failed (event {id}). A guess about event {other}.");
    let a = assistant(&[&text]);
    let n = a.incident_narrative(store(), "## Summary\nEvents: many\n", &[id]).unwrap();
    assert_eq!(n.text, text);
    let verified: Vec<(u32, bool)> = n
        .citations
        .iter()
        .filter_map(|c| match c {
            Citation::Record { id, verified } => Some((*id, *verified)),
            _ => None,
        })
        .collect();
    assert_eq!(verified, vec![(id, true), (other, false)]);
    let sent = a.provider().requests()[0].clone();
    assert!(sent.messages[0].content.contains("Events: many") && sent.messages[0].content.contains("at most 150 words"));
}
