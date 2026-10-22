//! Knowledge files through the AI commands (ADR 0029): both scopes managed over IPC, the project record saved and
//! restored like the chats, import and export only of paths the user chose, and the enabled files reaching the
//! model's system prompt in every feature (a use case and a chat turn, with a scripted model).

use super::*;

fn grant(app: &tauri::App<MockRuntime>, path: &Path, access: crate::paths::Access) {
    crate::paths::grant(app.handle(), "main", path, access, crate::paths::GrantSource::Dialog).unwrap();
}

fn create(w: &tauri::WebviewWindow<MockRuntime>, scope: &str, name: &str, text: &str) -> String {
    let info = ipc(w, "ai_knowledge_create", json!({"scope": scope, "name": name, "text": text})).unwrap();
    assert_eq!((info["scope"].as_str(), info["enabled"].as_bool()), (Some(scope), Some(true)));
    info["id"].as_str().unwrap().to_string()
}

#[test]
fn knowledge_files_are_managed_in_both_scopes() {
    let f = Fixture::new("knowledge-manage");
    let app = f.app(f.core());
    let main = window(&app, "main");
    let empty = ipc(&main, "ai_knowledge_list", json!({})).unwrap();
    assert_eq!(
        (
            empty["project"].as_array().unwrap().len(),
            empty["global"].as_array().unwrap().len()
        ),
        (0, 0)
    );
    assert_eq!(
        (empty["maxFileBytes"].as_u64(), empty["maxFiles"].as_u64()),
        (Some(65536), Some(32))
    );
    let g = create(&main, "global", "Glossary", "ACME-GW is the payment gateway.");
    let p = create(&main, "project", "Incident", "WEB01 was patched on Monday.");
    let list = ipc(&main, "ai_knowledge_list", json!({})).unwrap();
    assert_eq!(list["global"][0]["name"], "Glossary");
    assert_eq!(list["project"][0]["bytes"], 28);
    assert!(list["project"][0].get("text").is_none(), "lists carry no text");
    // The global file is on disk in Debrief's configuration folder.
    let dir = f.tmp.0.join("ai/knowledge");
    assert_eq!(
        std::fs::read_to_string(dir.join(format!("{g}.md"))).unwrap(),
        "ACME-GW is the payment gateway."
    );
    // Read, rename, disable, edit.
    let doc = ipc(&main, "ai_knowledge_get", json!({"scope": "project", "id": p})).unwrap();
    assert_eq!(doc["text"], "WEB01 was patched on Monday.");
    let changed = ipc(
        &main,
        "ai_knowledge_update",
        json!({"scope": "global", "id": g, "change": {"name": "Names", "enabled": false, "text": "ACME-GW: gateway"}}),
    )
    .unwrap();
    assert_eq!(
        (changed["name"].as_str(), changed["enabled"].as_bool()),
        (Some("Names"), Some(false))
    );
    // Typed refusals: unknown files, bad ids, names and sizes, unknown fields and scopes.
    let bad = |cmd: &str, args: Value| code(ipc(&main, cmd, args).unwrap_err());
    assert_eq!(bad("ai_knowledge_get", json!({"scope": "global", "id": "k9"})), "notFound");
    assert_eq!(bad("ai_knowledge_remove", json!({"scope": "global", "id": "../index"})), "invalid");
    assert_eq!(bad("ai_knowledge_create", json!({"scope": "project", "name": " "})), "invalid");
    let huge = "x".repeat(debrief_ai::knowledge::MAX_FILE_BYTES + 1);
    assert_eq!(
        bad("ai_knowledge_create", json!({"scope": "project", "name": "Big", "text": huge})),
        "invalid"
    );
    assert!(ipc(
        &main,
        "ai_knowledge_update",
        json!({"scope": "project", "id": p, "change": {"evil": 1}})
    )
    .is_err());
    assert!(ipc(&main, "ai_knowledge_get", json!({"scope": "team", "id": p})).is_err());
    // Remove.
    assert_eq!(
        ipc(&main, "ai_knowledge_remove", json!({"scope": "global", "id": g})).unwrap(),
        true
    );
    assert!(!dir.join(format!("{g}.md")).exists());
    assert_eq!(bad("ai_knowledge_remove", json!({"scope": "global", "id": g})), "notFound");
    // The project's files belong to its window.
    let other = window(&app, "w-2");
    assert!(ipc(&other, "ai_knowledge_list", json!({})).unwrap()["project"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn project_knowledge_round_trips_through_the_project_record() {
    let f = Fixture::new("knowledge-record");
    let app = f.app(f.core());
    let main = window(&app, "main");
    create(&main, "project", "Incident", "WEB01 was patched on Monday.");
    let record = ipc(&main, "ai_knowledge_project_record", json!({})).unwrap();
    assert_eq!(record["version"], 1);
    let other = window(&app, "w-2");
    let r = ipc(&other, "ai_knowledge_project_restore", json!({"record": record})).unwrap();
    assert!(r["warning"].is_null());
    assert_eq!(r["files"][0]["name"], "Incident");
    assert_eq!(ipc(&other, "ai_knowledge_project_record", json!({})).unwrap(), record);
    // An older project (none) clears it; damaged and newer records give none with a reason.
    assert!(ipc(&other, "ai_knowledge_project_restore", json!({})).unwrap()["files"]
        .as_array()
        .unwrap()
        .is_empty());
    let damaged = ipc(&other, "ai_knowledge_project_restore", json!({"record": {"version": "x"}})).unwrap();
    assert!(damaged["warning"].as_str().unwrap().contains("damaged"));
    let newer = ipc(&other, "ai_knowledge_project_restore", json!({"record": {"version": 9}})).unwrap();
    assert!(newer["warning"].as_str().unwrap().contains("newer"));
}

#[test]
fn import_and_export_only_use_paths_the_user_chose() {
    let f = Fixture::new("knowledge-files");
    let app = f.app(f.core());
    let main = window(&app, "main");
    let src = f.tmp.0.join("Runbook.md");
    std::fs::write(&src, "\u{feff}# Runbook\r\nRestart the gateway.").unwrap();
    let path = src.to_string_lossy().to_string();
    let args = json!({"scope": "global", "path": path});
    assert_eq!(code(ipc(&main, "ai_knowledge_import", args.clone()).unwrap_err()), "notGranted");
    grant(&app, &src, crate::paths::Access::Read);
    let info = ipc(&main, "ai_knowledge_import", args.clone()).unwrap();
    assert_eq!(info["name"], "Runbook");
    let id = info["id"].as_str().unwrap();
    let doc = ipc(&main, "ai_knowledge_get", json!({"scope": "global", "id": id})).unwrap();
    assert_eq!(doc["text"], "# Runbook\nRestart the gateway.", "BOM and CRLF are normalised");
    // Too large or not text.
    let big = f.tmp.0.join("big.md");
    std::fs::write(&big, "x".repeat(debrief_ai::knowledge::MAX_FILE_BYTES + 1)).unwrap();
    grant(&app, &big, crate::paths::Access::Read);
    let e = ipc(
        &main,
        "ai_knowledge_import",
        json!({"scope": "project", "path": big.to_string_lossy()}),
    )
    .unwrap_err();
    assert_eq!(code(e.clone()), "limit");
    assert!(e["message"].as_str().unwrap().contains("too large"));
    let bin = f.tmp.0.join("bin.md");
    std::fs::write(&bin, [0xff, 0xfe, 0xfd]).unwrap();
    grant(&app, &bin, crate::paths::Access::Read);
    assert_eq!(
        code(
            ipc(
                &main,
                "ai_knowledge_import",
                json!({"scope": "project", "path": bin.to_string_lossy()})
            )
            .unwrap_err()
        ),
        "invalid"
    );
    // Export, with the overwrite guard.
    let out = f.tmp.0.join("shared.md");
    let export = json!({"scope": "global", "id": id, "path": out.to_string_lossy()});
    assert_eq!(code(ipc(&main, "ai_knowledge_export", export.clone()).unwrap_err()), "notGranted");
    grant(&app, &out, crate::paths::Access::Write);
    ipc(&main, "ai_knowledge_export", export.clone()).unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "# Runbook\nRestart the gateway.");
    assert_eq!(code(ipc(&main, "ai_knowledge_export", export.clone()).unwrap_err()), "fileExists");
    let mut again = export;
    again["overwrite"] = json!(true);
    ipc(&main, "ai_knowledge_export", again).unwrap();
}

#[test]
fn every_ai_feature_reads_the_enabled_knowledge_project_first() {
    let f = Fixture::new("knowledge-prompts");
    let core = f.core();
    let model = Arc::new(ScriptedProvider::new(vec![
        text(r#"{"ql": "level >= error", "explanation": "All errors and worse."}"#),
        text("ACME-GW is the payment gateway; it failed at 10:41."),
    ]));
    core.set_fixed_provider(model.clone());
    let app = f.app(core);
    let main = window(&app, "main");
    open_samples(&main);
    create(&main, "global", "Environments", "PROD runs in eu-west.");
    create(&main, "project", "Glossary", "ACME-GW is the payment gateway.");
    let off = create(&main, "project", "Draft", "UNFINISHED-NOTE");
    ipc(
        &main,
        "ai_knowledge_update",
        json!({"scope": "project", "id": off, "change": {"enabled": false}}),
    )
    .unwrap();
    // A use case.
    let id = start(&main, json!({"op": "query", "question": "show me the errors"}));
    assert_eq!(ipc(&main, "jobs_wait", json!({"id": id})).unwrap()["state"], "done");
    // A chat turn.
    let sid = ipc(&main, "ai_chat_new", json!({})).unwrap()["id"].as_str().unwrap().to_string();
    ipc(&main, "ai_chat_send", json!({"sessionId": sid, "text": "What is ACME-GW?"})).unwrap();
    assert_eq!(kinds(&finish_turn(&main, &sid)).last().unwrap(), "done");
    let sent = model.requests();
    assert_eq!(sent.len(), 2);
    for r in &sent {
        let s = &r.system;
        let (rules, knowledge) = s.split_once(debrief_ai::knowledge::BLOCK_OPEN).expect("the knowledge block");
        assert!(!rules.trim().is_empty(), "Debrief's rules come first");
        let (p, g) = (knowledge.find("ACME-GW").unwrap(), knowledge.find("eu-west").unwrap());
        assert!(p < g, "project before global");
        assert!(!s.contains("UNFINISHED-NOTE"), "disabled files are skipped");
    }
}
