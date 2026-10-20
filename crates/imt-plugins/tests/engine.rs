//! Every kind of contribution of the built-in plugins through the engine, over the real IPC: importers (text and
//! binary) in workspaces, configuration parsers in the diff, marker kinds, annotators (with their view calls back
//! into the host), settings, tools (with path arguments checked against the caller's grants) and profile
//! importers (the profiling plugin as the profile worker).

mod common;

use imt_core::config::{diff, ChangeKind, CompareOptions, ConfigParsers, Limits};
use imt_core::plugin::tools::{NoTasks, PathAccess, PathKind, PathPolicy, ToolCtx, ToolError};
use imt_profile::{Capture, CaptureImporter};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn web_importers_open_their_fixtures_and_claim_only_web_samples() {
    let mut ws = common::workspace(&[common::WEB]);
    ws.open(&[common::fixture("imt-plugin-web", "")]).unwrap();
    let s = ws.summary();
    let mut ids: Vec<&str> = s.sources.iter().map(|x| x.importer.as_str()).collect();
    ids.sort();
    assert_eq!(ids, vec!["access-log", "har", "w3c"]);
    assert!(ws.records().iter().any(|r| r.fields.contains_key("cs-uri-stem")));
    assert!(ws.records().iter().any(|r| r.fields.contains_key("ClientIp")));
    assert!(ws.records().iter().any(|r| r.fields.contains_key("Url")));

    let mut ws = common::workspace(&[common::WEB]);
    let _ = ws.open(&[common::samples()]);
    let summary = ws.summary();
    let mut claimed: Vec<&str> = Vec::new();
    for src in &summary.sources {
        let in_web = src.path.replace('\\', "/").contains("/samples/web/");
        let ours = ["w3c", "access-log", "har"].contains(&src.importer.as_str());
        assert_eq!(ours, in_web, "{} claimed by {}", src.path, src.importer);
        if ours {
            claimed.push(src.importer.as_str());
        }
    }
    claimed.sort();
    claimed.dedup();
    assert_eq!(claimed, vec!["access-log", "har", "w3c"]);
}

#[test]
fn syslog_and_journald_fixtures_open_through_a_workspace() {
    let mut ws = common::workspace(&[common::SYSLOG]);
    let f = |n: &str| common::fixture("imt-plugin-syslog", n);
    ws.open(&[f("rfc5424.log"), f("auth.log"), f("journal.json")]).unwrap();
    let s = ws.summary();
    let importer = |name: &str| s.sources.iter().find(|x| x.name.contains(name)).map(|x| x.importer.clone());
    assert_eq!(importer("rfc5424").as_deref(), Some("syslog"));
    assert_eq!(importer("auth").as_deref(), Some("syslog"));
    assert_eq!(importer("journal").as_deref(), Some("journald"));
    assert!(s.plugins.iter().any(|p| p.id == common::SYSLOG));
    assert!(ws
        .records()
        .iter()
        .any(|r| r.fields.get("App") == Some(&json!("sshd")) && r.fields.get("ProcId") == Some(&json!(4242))));
    assert!(ws.records().iter().any(|r| r.message == "Started OrderService."));
}

#[test]
fn otlp_fixtures_open_through_a_workspace() {
    let mut ws = common::workspace(&[common::OTEL]);
    let f = |n: &str| common::fixture("imt-plugin-otel", n);
    ws.open(&[f("logs.jsonl"), f("traces.json")]).unwrap();
    let s = ws.summary();
    assert_eq!(s.sources.len(), 2);
    assert!(s.sources.iter().all(|x| x.importer == "otlp"));
    assert_eq!(ws.records().len(), 9);
}

#[test]
fn registry_exports_diff_case_insensitively_through_the_plugin_parser() {
    let registry = common::registry(&[common::REGISTRY]);
    let parsers = ConfigParsers::with_registry(&registry);
    assert!(parsers.formats().iter().any(|f| f.id == "registry"));
    let left = "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_LOCAL_MACHINE\\SOFTWARE\\Contoso]\r\n\"Timeout\"=dword:00001388\r\n\"InstallDate\"=dword:00000001\r\n";
    let right = left
        .replace("HKEY_LOCAL_MACHINE\\SOFTWARE\\Contoso", "hkey_local_machine\\Software\\CONTOSO")
        .replace("00001388", "00001770")
        .replace("00000001", "00000002");
    let l = parsers.parse("left.reg", left, &Limits::default()).unwrap();
    let r = parsers.parse("right.reg", &right, &Limits::default()).unwrap();
    assert_eq!(l.format, "registry");
    let d = diff(&l, &r, &CompareOptions::default()).unwrap();
    let changed: Vec<&str> = d
        .entries
        .iter()
        .filter(|e| e.kind == ChangeKind::Changed)
        .map(|e| e.display.as_str())
        .collect();
    assert_eq!(changed, ["hkey_local_machine/Software/CONTOSO/Timeout"]);
}

#[test]
fn plugin_settings_are_applied_in_the_plugin_process() {
    let registry = common::registry(&[common::WINDOWS]);
    let s = registry.plugin_settings();
    let windows = s.iter().find(|s| s.plugin.id == common::WINDOWS).unwrap();
    let section = windows.configurable.settings_section();
    assert_eq!(section.id, common::WINDOWS);
    let valid = imt_core::settings::validate(&section, &json!({"relogTimeout": 600_000})).unwrap();
    windows.apply(&valid).unwrap();
}

/// Grants every path under one folder (the shell's grants, for these tests).
struct Under(PathBuf);

impl PathPolicy for Under {
    fn check(&self, path: &str, _: PathKind, _: PathAccess) -> Result<PathBuf, String> {
        let p = Path::new(path);
        if p.starts_with(&self.0) {
            Ok(p.to_path_buf())
        } else {
            Err(format!("\"{path}\" was not chosen"))
        }
    }
}

#[test]
fn tools_run_in_the_plugin_process_and_path_arguments_need_a_grant() {
    let registry = common::registry(&[common::TOOLS]);
    let tools = registry.tools();
    let listed = tools.list();
    assert!(listed.iter().any(|t| t.info.id == "perfmon.templates"), "{listed:?}");
    let presets = tools
        .invoke("perfmon.templates", "presets", json!({}), &ToolCtx::new(None, Arc::new(NoTasks)))
        .unwrap();
    assert!(presets.is_array() || presets.is_object(), "{presets}");
    // `generate` writes the Data Collector Set XML to a path the user must have chosen.
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("set.xml");
    let info = listed.iter().find(|t| t.info.id == "perfmon.templates").unwrap();
    let generate = info.info.ops.iter().find(|o| o.name == "generate").unwrap();
    assert!(generate.args.has_paths(), "generate's xmlPath is a typed path argument");
    let args = json!({"template": {"name": "IMT-e2e"}, "xmlPath": target.to_string_lossy()});
    let refused = tools.invoke(
        "perfmon.templates",
        "generate",
        args.clone(),
        &ToolCtx::new(None, Arc::new(NoTasks)),
    );
    assert!(matches!(refused, Err(ToolError::PathRefused { .. })), "{refused:?}");
    let granted = ToolCtx::new(None, Arc::new(NoTasks)).with_paths(Arc::new(Under(dir.path().to_path_buf())));
    match tools.invoke("perfmon.templates", "generate", args, &granted) {
        Ok(_) => assert!(target.exists(), "the plugin wrote the granted file"),
        // A template the schema refuses is an argument error, never a refused path.
        Err(e) => assert!(!matches!(e, ToolError::PathRefused { .. }), "{e}"),
    }
}

#[test]
fn profiles_are_parsed_in_the_profiling_process() {
    let registry = common::registry(&[common::PROFILING]);
    let importers = registry.contributions::<dyn CaptureImporter>(imt_profile::IMPORTERS_POINT);
    assert!(!importers.is_empty());
    let folded = importers
        .iter()
        .find(|i| i.call(|i| i.id().to_string()).ok().as_deref() == Some("folded"))
        .unwrap();
    let capture = folded
        .call(|i| i.import("x.folded", b"main;work 3\nmain;idle 2\n", &imt_profile::Limits::default()))
        .unwrap()
        .unwrap();
    match capture {
        Capture::Profile(p) => assert_eq!(p.samples.len(), 2),
        other => panic!("{other:?}"),
    }
    let sniff = folded.call(|i| i.sniff("x.folded", b"main;work 3\n")).unwrap();
    assert!(sniff > 0);
    let bad = folded
        .call(|i| i.import("x.folded", b"\xff\xfe not folded", &imt_profile::Limits::default()))
        .unwrap();
    assert!(bad.is_err());
}
