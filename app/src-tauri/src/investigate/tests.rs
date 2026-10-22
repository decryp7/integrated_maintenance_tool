//! The investigation commands through the real IPC path (`tauri::test`): known issues from knowledge files, the
//! incident summary with its notes, the AI commands without a model, and playbooks (library, runs, validation).

use crate::test_support::{main_window, samples, test_app_with_commands, window};
use crate::tools::tests::ipc;
use serde_json::{json, Value};

fn err(r: Result<Value, Value>) -> String {
    r.unwrap_err().to_string()
}

const KNOWLEDGE: &str = "```known-issue\ntitle: Payment timeouts\npattern: (?i)request timed out\nfix: Raise the gateway timeout.\n```\n";

#[test]
fn known_issues_come_from_the_enabled_knowledge_files_and_the_incident_report_cites_them() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    ipc(&main, "open_paths", json!({ "paths": [samples().join("serilog-clef").to_string_lossy()] })).unwrap();
    // No knowledge: nothing matches and nothing is wrong.
    let none = ipc(&main, "known_issues_match", json!({})).unwrap();
    assert_eq!((none["matches"].as_array().unwrap().len(), none["checked"].as_u64()), (0, Some(0)));
    // A project knowledge file declares the issue.
    main_window(&app).ai.knowledge.change(|s| s.add("Payments", KNOWLEDGE).map(|_| ())).unwrap();
    let r = ipc(&main, "known_issues_match", json!({})).unwrap();
    assert_eq!(r["matches"][0]["issue"]["title"], "Payment timeouts");
    assert_eq!(r["matches"][0]["origin"], "rule");
    assert!(r["matches"][0]["count"].as_u64().unwrap() > 100);
    // A disabled file is not read.
    let id = main_window(&app).ai.knowledge.snapshot().files()[0].id.clone();
    main_window(&app).ai.knowledge.change(|s| s.update(&id, &imt_ai::knowledge::KnowledgeChange { enabled: Some(false), ..Default::default() }).map(|_| ())).unwrap();
    assert_eq!(ipc(&main, "known_issues_match", json!({})).unwrap()["checked"], 0);
    main_window(&app).ai.knowledge.change(|s| s.update(&id, &imt_ai::knowledge::KnowledgeChange { enabled: Some(true), ..Default::default() }).map(|_| ())).unwrap();

    let report = ipc(&main, "incident_report_build", json!({"request": {"title": "Checkout outage", "top": 3}})).unwrap();
    assert_eq!(report["title"], "Checkout outage");
    assert!(report["id"].as_str().unwrap().starts_with("incident-"));
    let titles: Vec<&str> = report["sections"].as_array().unwrap().iter().map(|s| s["title"].as_str().unwrap()).collect();
    assert!(titles.contains(&"Known issues") && titles.contains(&"Top exceptions"), "{titles:?}");
    // It is an ordinary report: the report pipeline accepts it.
    imt_core::report::Report::parse(report).unwrap();

    assert!(err(ipc(&main, "incident_report_build", json!({"request": {"from": 5, "to": 1}}))).contains("before its start"));
    assert!(err(ipc(&main, "incident_report_build", json!({"request": {"title": "x".repeat(201)}}))).contains("longer than 200"));
}

#[test]
fn the_ai_commands_say_so_without_a_model_and_refuse_a_bad_report() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    let e = err(ipc(&main, "known_issues_suggest", json!({})));
    assert!(e.to_lowercase().contains("not available") || e.to_lowercase().contains("model") || e.to_lowercase().contains("ai"), "{e}");
    let e = err(ipc(&main, "incident_narrative", json!({"report": {"nope": 1}})));
    assert!(e.contains("not a valid report"), "{e}");
}

fn playbook() -> Value {
    json!({"id": "mine", "title": "Mine", "steps": [
        {"id": "a", "title": "Errors", "action": {"type": "query", "ql": "level >= error"}},
        {"id": "b", "title": "No fatal", "action": {"type": "check", "ql": "level = fatal", "expect": {"kind": "noMatch"}}}
    ]})
}

#[test]
fn playbooks_validate_run_and_record() {
    let app = test_app_with_commands(None);
    let main = window(&app, "main");
    ipc(&main, "open_paths", json!({ "paths": [samples().join("serilog-clef").to_string_lossy()] })).unwrap();
    let lib = ipc(&main, "playbook_library", json!({})).unwrap();
    assert!(lib["templates"].as_array().unwrap().len() >= 4);
    // Save, list, remove (the library is a folder of files in the configuration folder).
    let saved = ipc(&main, "playbook_save", json!({"playbook": playbook()})).unwrap();
    assert_eq!(saved["version"], 1);
    let lib = ipc(&main, "playbook_library", json!({})).unwrap();
    assert!(lib["playbooks"].as_array().unwrap().iter().any(|p| p["id"] == "mine"));
    assert_eq!(ipc(&main, "playbook_remove", json!({"id": "mine"})).unwrap(), true);
    assert_eq!(ipc(&main, "playbook_remove", json!({"id": "mine"})).unwrap(), false);
    assert!(err(ipc(&main, "playbook_remove", json!({"id": "../x"}))).contains("not a playbook id"));
    let mut bad = playbook();
    bad["id"] = json!("Bad Id");
    assert!(err(ipc(&main, "playbook_save", json!({"playbook": bad}))).contains("lowercase"));

    // A run: start, run steps, record.
    let run = ipc(&main, "playbook_start", json!({"playbook": playbook(), "runId": "run-1"})).unwrap();
    assert_eq!(run["playbook"]["id"], "mine");
    let step = run["playbook"]["steps"][1].clone();
    let result = ipc(&main, "playbook_run_step", json!({"step": step})).unwrap();
    assert_eq!(result["status"], "fail");
    let run = ipc(&main, "playbook_record", json!({"run": run, "stepId": "b", "result": result, "note": "fatal at 09:41"})).unwrap();
    assert_eq!(run["steps"][0]["note"], "fatal at 09:41");
    assert!(err(ipc(&main, "playbook_record", json!({"run": run, "stepId": "zzz", "result": result, "note": ""}))).contains("no step"));
    assert!(err(ipc(&main, "playbook_run_step", json!({"step": {"id": "x"}}))).contains("not a valid step"));
    assert!(err(ipc(&main, "playbook_start", json!({"playbook": playbook(), "runId": " "}))).contains("needs an id"));

    // The project's saved runs are checked on the way back in.
    let ok = ipc(&main, "playbook_restore", json!({"record": {"version": 1, "runs": [run]}})).unwrap();
    assert_eq!(ok["record"]["runs"].as_array().unwrap().len(), 1);
    let newer = ipc(&main, "playbook_restore", json!({"record": {"version": 9, "runs": []}})).unwrap();
    assert!(newer["warnings"][0].as_str().unwrap().contains("newer version"));
    assert_eq!(ipc(&main, "playbook_restore", json!({"record": null})).unwrap()["record"]["runs"].as_array().unwrap().len(), 0);
}
