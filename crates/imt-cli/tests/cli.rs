//! The CLI end to end over the real DuckDB store in process (the worker only moves the same store into another
//! process): import, query, analyses, the report, exit codes, JSON shapes, bounded output and failures.

use imt_cli::args::{self, AnalyzeKind, Command, ReportFormat, Source, TableFormat};
use imt_cli::host::{SharedStore, StoreHost};
use imt_cli::{run, Exit};
use imt_core::ports::StoreResult;
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::DuckStore;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct InProcess;

impl StoreHost for InProcess {
    fn create(&self, dir: &Path, profile: &Profile) -> StoreResult<SharedStore> {
        Ok(Arc::new(DuckStore::create(dir, profile.clone(), PluginRegistry::new())?))
    }
    fn open(&self, dir: &Path, profile: &Profile) -> StoreResult<SharedStore> {
        Ok(Arc::new(DuckStore::open(dir, profile.clone(), PluginRegistry::new())?))
    }
}

/// Serilog-style text: a quiet hour, then a burst of timeouts with an exception.
fn write_log(dir: &Path) -> PathBuf {
    let mut text = String::new();
    for i in 0..200 {
        let (m, s) = (i / 60, i % 60);
        text.push_str(&format!(
            "2026-10-14 09:{m:02}:{s:02}.000 +00:00 [INF] Dispatched {i} outbox messages\n"
        ));
    }
    for i in 0..40 {
        text.push_str(&format!(
            "2026-10-14 10:00:{i:02}.000 +00:00 [ERR] Request {i} failed: timeout expired, max pool size reached\nSystem.TimeoutException: timeout expired\n   at Db.Open() in Db.cs:line 12\n"
        ));
    }
    let path = dir.join("app.log");
    std::fs::write(&path, text).unwrap();
    path
}

struct Run {
    exit: Exit,
    out: String,
    err: String,
}

fn exec(argv: &[&str]) -> Run {
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let command = args::parse(&argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let exit = run::run(command, &InProcess, &mut out, &mut err);
    Run {
        exit,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

fn json(r: &Run) -> Value {
    serde_json::from_str(r.out.trim()).unwrap_or_else(|e| panic!("not one JSON object: {e}\n{}", r.out))
}

#[test]
fn imports_and_queries_into_every_format() {
    let dir = tempfile::tempdir().unwrap();
    let log = write_log(dir.path());
    let log = log.to_str().unwrap();

    let r = exec(&["import", log, "--json"]);
    assert_eq!(r.exit, Exit::Ok, "{}", r.err);
    let v = json(&r);
    assert_eq!((v["ok"].as_bool(), v["events"].as_u64()), (Some(true), Some(240)));
    assert_eq!(v["sources"][0]["records"], 240);

    let r = exec(&[
        "query",
        "level = \"Error\" | stats count() as n by level",
        "--open",
        log,
        "--format",
        "csv",
    ]);
    assert_eq!(r.exit, Exit::Ok, "{}", r.err);
    assert!(r.out.lines().count() >= 2 && r.out.contains("40"), "{}", r.out);

    let r = exec(&["query", "level = \"Error\"", "--open", log, "--json", "--limit", "5"]);
    let v = json(&r);
    assert_eq!(v["total"], 40);
    assert_eq!(v["rows"].as_array().unwrap().len(), 5);
    assert_eq!(v["truncated"], true);
    assert!(v["rows"][0]["message"].as_str().unwrap().contains("timeout expired"));
    assert!(r.err.contains("5 of 40"), "{}", r.err);

    let r = exec(&["query", "level = \"Error\"", "--open", log, "--limit", "3"]);
    assert!(r.out.lines().next().unwrap().starts_with("id"), "{}", r.out);
    assert_eq!(r.out.lines().count(), 4);
}

#[test]
fn a_kept_store_is_reused_without_the_files() {
    let dir = tempfile::tempdir().unwrap();
    let log = write_log(dir.path());
    let store = dir.path().join("store");
    let r = exec(&["import", log.to_str().unwrap(), "--store", store.to_str().unwrap()]);
    assert_eq!(r.exit, Exit::Ok, "{}", r.err);
    assert!(r.out.contains("imported 240 events"), "{}", r.out);
    std::fs::remove_file(&log).unwrap();
    let r = exec(&[
        "query",
        "level >= verbose | stats count() as n",
        "--store",
        store.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(r.exit, Exit::Ok, "{}", r.err);
    assert_eq!(json(&r)["rows"][0].as_object().unwrap().values().next().unwrap(), 240);
}

#[test]
fn analyses_report_findings_and_set_the_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let log = write_log(dir.path());
    let log = log.to_str().unwrap();

    let r = exec(&["analyze", "patterns", "--open", log, "--json"]);
    assert_eq!(r.exit, Exit::Ok, "{}", r.err);
    let v = json(&r);
    assert_eq!(v["kind"], "patterns");
    assert!(v["total"].as_u64().unwrap() >= 2);
    assert!(v["items"][0]["count"].as_u64().unwrap() >= 40);
    assert!(v["matched"].as_u64().unwrap() > 0);

    let r = exec(&["analyze", "exceptions", "--open", log]);
    assert!(r.out.contains("TimeoutException"), "{}", r.out);
    let r = exec(&["analyze", "exceptions", "--open", log, "--fail-on-findings"]);
    assert_eq!(r.exit, Exit::Findings);
    let r = exec(&[
        "analyze",
        "exceptions",
        "--open",
        log,
        "--filter",
        "level = \"Verbose\"",
        "--fail-on-findings",
    ]);
    assert_eq!(r.exit, Exit::Ok, "{}", r.err);
    assert!(r.out.contains("no exceptions"));

    let r = exec(&["analyze", "patterns", "--open", log, "--limit", "1", "--json"]);
    let v = json(&r);
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    assert_eq!(v["truncated"], true);

    let r = exec(&["analyze", "anomalies", "--open", log, "--json"]);
    assert_eq!(r.exit, Exit::Ok, "{}", r.err);
    assert_eq!(json(&r)["kind"], "anomalies");
}

#[test]
fn known_issues_match_from_knowledge_files() {
    let dir = tempfile::tempdir().unwrap();
    let log = write_log(dir.path());
    let know = dir.path().join("notes.md");
    std::fs::write(
        &know,
        "# Notes\n\n```known-issue\ntitle: Pool exhausted\npattern: (?i)timeout expired.*max pool size\nfix: Raise Max Pool Size.\n```\n\n```known-issue\ntitle: Never\npattern: zzz-not-there\nfix: n/a\n```\n",
    )
    .unwrap();
    let r = exec(&[
        "analyze",
        "issues",
        "--open",
        log.to_str().unwrap(),
        "--knowledge",
        know.to_str().unwrap(),
        "--fail-on-findings",
        "--json",
    ]);
    assert_eq!(r.exit, Exit::Findings, "{}", r.err);
    let v = json(&r);
    assert_eq!(v["total"], 1);
    assert_eq!(v["items"][0]["count"], 40);
    assert_eq!(v["items"][0]["issue"]["title"], "Pool exhausted");

    let missing = exec(&[
        "analyze",
        "issues",
        "--open",
        log.to_str().unwrap(),
        "--knowledge",
        dir.path().join("none.md").to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(missing.exit, Exit::Failure);
    assert_eq!(json(&missing)["ok"], false);
}

#[test]
fn the_report_is_written_as_html_and_never_overwritten_silently() {
    let dir = tempfile::tempdir().unwrap();
    let log = write_log(dir.path());
    let log = log.to_str().unwrap();
    let out = dir.path().join("incident.html");
    let r = exec(&["report", "--open", log, "--out", out.to_str().unwrap(), "--title", "Pool incident"]);
    assert_eq!(r.exit, Exit::Ok, "{}", r.err);
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(
        html.contains("<html") && html.contains("Pool incident"),
        "{}",
        &html[..html.len().min(300)]
    );

    let again = exec(&["report", "--open", log, "--out", out.to_str().unwrap(), "--json"]);
    assert_eq!(again.exit, Exit::Failure);
    assert!(json(&again)["error"].as_str().unwrap().contains("already exists"));
    let replaced = exec(&["report", "--open", log, "--out", out.to_str().unwrap(), "--overwrite", "--json"]);
    assert_eq!(replaced.exit, Exit::Ok, "{}", replaced.err);
    assert_eq!(json(&replaced)["ok"], true);

    let pdf = dir.path().join("incident.pdf");
    let r = exec(&["report", "--open", log, "--out", pdf.to_str().unwrap()]);
    assert_eq!(r.exit, Exit::Ok, "{}", r.err);
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF"));
    // No temporary file is left behind.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn failures_are_messages_with_a_failure_exit() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.log");
    let r = exec(&["import", missing.to_str().unwrap(), "--json"]);
    assert_eq!(r.exit, Exit::Failure);
    assert!(json(&r)["error"].as_str().unwrap().contains("does not exist"));
    assert!(r.err.contains("does not exist"));

    // A bad query names what is wrong.
    let log = write_log(dir.path());
    let r = exec(&["query", "level = = 3", "--open", log.to_str().unwrap()]);
    assert_eq!(r.exit, Exit::Failure);
    assert!(r.err.contains("query"), "{}", r.err);

    // Nothing to read.
    let r = exec(&["query", "level >= verbose | stats count() as n"]);
    assert_eq!(r.exit, Exit::Failure);
    assert!(r.err.contains("nothing to read"));

    // A file no importer reads still imports as text or is reported; a folder of nothing is not a crash.
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let r = exec(&["import", empty.to_str().unwrap(), "--json"]);
    assert_eq!(r.exit, Exit::Ok, "{}", r.err);
    assert_eq!(json(&r)["events"], 0);
}

#[test]
fn help_and_version_need_no_store() {
    let mut out = Vec::new();
    assert_eq!(run::run_static(&Command::Help, &mut out), Some(Exit::Ok));
    assert!(String::from_utf8(out).unwrap().contains("usage:"));
    let mut out = Vec::new();
    assert_eq!(run::run_static(&Command::Version, &mut out), Some(Exit::Ok));
    assert!(String::from_utf8(out).unwrap().starts_with("imt "));
    let q = Command::Query {
        src: Source::default(),
        ql: String::new(),
        format: TableFormat::Table,
        limit: 1,
    };
    assert_eq!(run::run_static(&q, &mut Vec::new()), None);
    let _ = (AnalyzeKind::Issues, ReportFormat::Html);
}
