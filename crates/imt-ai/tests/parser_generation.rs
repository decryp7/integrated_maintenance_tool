//! Parser generation (ADR 0032) with a scripted model: the loop converges, gives up when a round brings no
//! improvement, stops after the last round, and never returns a rule the engine did not verify. Every log line here
//! is invented.

use imt_ai::assistant::*;
use imt_ai::llm::ScriptedProvider;
use imt_ai::AiError;
use imt_core::parser::verify::FileFacts;
use imt_core::parser::Parser;
use imt_core::workspace::WorkspaceStore;
use imt_core::{Profile, Workspace};
use serde_json::json;

fn store() -> &'static dyn imt_core::ports::Store {
    Box::leak(Box::new(WorkspaceStore::new(Workspace::new(Profile::default_profile()).unwrap())))
}

fn assistant(replies: &[String]) -> Assistant<ScriptedProvider> {
    let r: Vec<&str> = replies.iter().map(String::as_str).collect();
    Assistant::new(ScriptedProvider::text(&r), AssistantConfig::default())
}

const LEVELS: [&str; 6] = ["INFO", "WARN", "ERROR", "DEBUG", "TRACE", "FATAL"];
/// 2026-10-14 10:00:00 UTC.
const T0: i64 = 1_791_972_000_000;

/// An invented job log with Japanese timestamps: `2026年10月14日 10時00分03秒 [INFO] job=1 seq=3 WorkingSet 603 MB`.
fn jobs(n: usize) -> String {
    (0..n)
        .map(|i| {
            format!(
                "2026年10月14日 10時{:02}分{:02}秒 [{}] job={} seq={i} WorkingSet {} MB\n",
                i / 60,
                i % 60,
                LEVELS[i % 6],
                i % 3,
                600 + i % 50
            )
        })
        .collect()
}

const TS: &str = r"\d{4}年\d{1,2}月\d{1,2}日 \d{1,2}時\d{1,2}分\d{1,2}秒";

fn reply(regex: &str, identity: &[&str], metrics: serde_json::Value) -> String {
    json!({"name": "Jobs", "format": "text", "regex": regex, "timestamp_formats": [], "identity": identity, "metrics": metrics, "rationale": "job runner"}).to_string()
}

/// A regex reading the levels in `levels` only.
fn regex_for(levels: &[&str]) -> String {
    format!(
        r"^(?P<timestamp>{TS}) \[(?P<level>{})\] job=(?P<Job>\d+) seq=(?P<Seq>\d+) (?P<message>.*)$",
        levels.join("|")
    )
}

fn facts() -> FileFacts {
    FileFacts {
        // A few minutes after the last line (300 lines, one a second, wall clock +09:00 against UTC).
        modified_ms: Some(T0 - 9 * 3_600_000 + 6 * 60_000),
        machine_offsets: vec![],
    }
}

fn job<'a>(content: &'a str) -> ParserJob<'a> {
    ParserJob {
        file_name: "jobs.log",
        content,
        facts: facts(),
    }
}

#[test]
fn converges_once_the_engine_explains_the_failing_lines() {
    let content = jobs(300);
    // Round 1: the timestamp group stops at the date; round 2: the whole timestamp.
    let date_only = r"^(?P<timestamp>\d{4}年\d{1,2}月\d{1,2}日) \S+ \[(?P<level>[A-Z]+)\] (?P<message>.*)$";
    let a = assistant(&[reply(date_only, &[], json!([])), reply(&regex_for(&LEVELS), &[], json!([]))]);
    let p = a.generate_parser(store(), &job(&content)).unwrap();
    assert_eq!(p.status, ParserStatus::Done);
    assert_eq!(p.rounds.len(), 2);
    assert!(!p.rounds[0].timestamps_accepted && p.rounds[1].timestamps_accepted);
    let v = p.verdict.as_ref().unwrap();
    assert!(v.done() && v.score >= 0.99, "{}", v.feedback());
    // The feedback named exact lines, the rule and the fragment that failed.
    let feedback = a.provider().requests()[1].messages.last().unwrap().content.clone();
    assert!(feedback.contains("line 1 [Timestamp] `2026年10月14日`"), "{feedback}");
    assert!(feedback.lines().count() < 20, "never more than needed: {feedback}");
    // The engine decided the zone from the modification time: +09:00.
    let parser = Parser::from_toml(&p.toml).unwrap();
    assert_eq!(parser.format.assume_timezone, "+09:00");
    assert_eq!(parser.format.timezone_source, "modification-time");
    assert_eq!(p.preview[0].ts, Some(T0 - 9 * 3_600_000));
}

#[test]
fn gives_up_when_a_round_brings_no_improvement() {
    let content = jobs(300);
    let partial = reply(&regex_for(&["INFO", "WARN", "ERROR"]), &[], json!([]));
    let a = assistant(&[partial.clone(), partial.clone(), partial]);
    let p = a.generate_parser(store(), &job(&content)).unwrap();
    assert_eq!(p.status, ParserStatus::BestEffort);
    assert_eq!(p.rounds.len(), 2, "stopped after the round without improvement");
    assert_eq!(a.provider().requests().len(), 2);
    let v = p.verdict.unwrap();
    assert!(v.score < 0.99 && v.score > 0.4, "{}", v.score);
    assert!(
        v.failures.iter().any(|f| f.fragment.contains("[DEBUG]")),
        "the missed lines are shown: {:?}",
        v.failures
    );
}

#[test]
fn stops_after_the_last_round_while_still_improving() {
    let content = jobs(300);
    // Each round reads one more level, never all six.
    let replies: Vec<String> = (1..=6).map(|k| reply(&regex_for(&LEVELS[..k.min(5)]), &[], json!([]))).collect();
    let a = assistant(&replies);
    let p = a.generate_parser(store(), &job(&content)).unwrap();
    assert_eq!(p.rounds.len(), MAX_ROUNDS);
    assert_eq!(a.provider().requests().len(), MAX_ROUNDS);
    assert!(p.rounds.windows(2).all(|w| w[1].score > w[0].score), "{:?}", p.rounds);
    assert_eq!(p.status, ParserStatus::BestEffort);
}

#[test]
fn never_accepts_an_unverified_rule() {
    let content = jobs(300);
    let metrics = json!([
        {"name": "WorkingSet", "regex": r"WorkingSet (?P<value>\d+) MB", "unit": "MB"},
        {"name": "Handles", "regex": r"Handles (?P<value>\d+)", "unit": "count"},
        {"name": "Broken", "regex": r"(?P<value>[", "unit": "count"}
    ]);
    let a = assistant(&[reply(&regex_for(&LEVELS), &["Job", "Seq", "Nope"], metrics)]);
    let mut f = facts();
    f.machine_offsets = vec![540, 540];
    let p = a
        .generate_parser(
            store(),
            &ParserJob {
                file_name: "jobs.log",
                content: &content,
                facts: f,
            },
        )
        .unwrap();
    let parser = Parser::from_toml(&p.toml).unwrap();
    // Seq changes with every line, Nope is on no event: only Job is an identity field.
    assert_eq!(parser.format.identity, ["Job"]);
    // Only the metric with plausible values is extracted and charted.
    assert_eq!(parser.extract.len(), 1);
    assert!(parser.extract[0].fields.contains_key("WorkingSet"));
    assert_eq!(p.charts_toml.matches("[[chart]]").count(), 1);
    let v = p.verdict.as_ref().unwrap();
    assert!(v.identity.iter().any(|c| c.field == "Seq" && !c.accepted));
    // The final parser was verified again: what is saved is what the engine checked.
    assert!(v.done());
    assert_eq!(
        (parser.format.assume_timezone.as_str(), parser.format.timezone_source.as_str()),
        ("+09:00", "same-machine")
    );
    assert_eq!(parser.format.timezone_confidence, Some(0.8));
    assert!(
        p.preview[0].fields.contains_key("WorkingSet"),
        "the extractor runs: {:?}",
        p.preview[0].fields
    );
}

#[test]
fn no_usable_answer_is_an_error_and_the_model_sees_only_samples() {
    let content = jobs(5000);
    let a = assistant(&["not json".into(), reply("(", &[], json!([]))]);
    assert!(matches!(a.generate_parser(store(), &job(&content)), Err(AiError::Malformed(_))));
    let requests = a.provider().requests();
    assert_eq!(requests.len(), 2);
    let prompt = &requests[0].messages[0].content;
    assert!(prompt.contains("seq=0 ") && prompt.contains("seq=4999 "), "start and end");
    assert!(!prompt.contains("seq=1000 "), "not the whole file");
    assert!(prompt.len() < 12_000, "{}", prompt.len());
    assert!(requests[1].messages.last().unwrap().content.contains("not valid JSON"));
}
