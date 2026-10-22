//! End-to-end: the generated files in /samples, opened by a workspace with this
//! plugin and the other text-format plugins installed (so a competing importer
//! that outranked ours would show up here).

mod common;

use debrief_core::api::SourceInfo;
use debrief_core::model::{Level, Record};
use debrief_core::{Profile, Workspace};
use std::path::{Path, PathBuf};

const OURS: &[&str] = &["log4j-xml", "sql-errorlog", "docker-json", "cri"];

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// 2026-10-14 09:00 and 12:00 UTC, the scenario window of `samples/generate.py`.
const START: i64 = 1_791_968_400_000;
const END: i64 = 1_791_979_200_000;

fn workspace() -> Workspace {
    let registry = common::registry(&[common::FORMATS, common::WEB, common::SYSLOG, common::OTEL]);
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
    // Profiler captures are not logs (and are opened by another plugin).
    let paths: Vec<PathBuf> = std::fs::read_dir(samples())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir() && !p.ends_with("profiling"))
        .collect();
    ws.open(&paths).unwrap();
    ws
}

fn source<'a>(sources: &'a [SourceInfo], suffix: &str) -> &'a SourceInfo {
    sources
        .iter()
        .find(|s| s.path.replace('\\', "/").ends_with(suffix))
        .unwrap_or_else(|| panic!("{suffix} not opened: {:?}", sources.iter().map(|s| &s.path).collect::<Vec<_>>()))
}

fn records<'a>(ws: &'a Workspace, src: &SourceInfo) -> Vec<&'a Record> {
    ws.records().iter().filter(|r| r.source == src.id).collect()
}

#[test]
fn every_new_sample_is_read_by_its_importer_and_nothing_else_is() {
    let ws = workspace();
    let s = ws.summary();
    let expect = [
        ("log4j-xml/billing-api/billing-api.xml", "log4j-xml"),
        ("log4j-xml/scheduler/scheduler-20261014.nlog.xml", "log4j-xml"),
        ("real-libraries/log4net-log4j.xml", "log4j-xml"),
        ("sqlserver/SQL01/ERRORLOG", "sql-errorlog"),
        ("sqlserver/sqlrpt01/errorlog", "sql-errorlog"),
        ("-json.log", "docker-json"),
        ("consumer/0.log", "cri"),
        ("consumer/1.log", "cri"),
    ];
    for (suffix, importer) in expect {
        let src = source(&s.sources, suffix);
        assert_eq!(src.importer, importer, "{suffix}");
        assert_eq!(src.skipped, 0, "{suffix}");
        assert!(src.records > 0, "{suffix}");
        let (from, to) = (src.from.unwrap(), src.to.unwrap());
        assert!(START <= from && to <= END, "{suffix}: {from}..{to}");
    }
    // The other samples are never claimed by these importers.
    for src in &s.sources {
        let ours = expect.iter().any(|(suffix, _)| src.path.replace('\\', "/").ends_with(suffix));
        assert_eq!(OURS.contains(&src.importer.as_str()), ours, "{} read by {}", src.path, src.importer);
    }
}

#[test]
fn log4j_and_nlog_events_have_typed_fields() {
    let ws = workspace();
    let s = ws.summary();
    let billing = records(&ws, source(&s.sources, "billing-api.xml"));
    let first = billing[0];
    assert_eq!(first.ts, Some(START));
    assert_eq!(first.message, "Billing API starting (v2.8.1)");
    assert_eq!(first.fields["MachineName"], "BILL01");
    assert_eq!(first.fields["Application"], "Billing.Api.exe");
    assert!(billing.iter().any(|r| r.message.contains("<posted> & acknowledged")));
    let errors: Vec<_> = billing.iter().filter(|r| r.level == Level::Error).collect();
    assert!(!errors.is_empty());
    assert!(errors.iter().all(|r| r
        .exception
        .as_deref()
        .is_some_and(|e| e.starts_with("System.ServiceModel.CommunicationException"))));

    let sched = records(&ws, source(&s.sources, "scheduler-20261014.nlog.xml"));
    assert_eq!(sched.len(), 90);
    assert_eq!(sched[0].fields["SequenceNumber"], 1);
    assert_eq!(sched[0].fields["MachineName"], "SCHED01");
    assert!(sched.iter().any(|r| r.level == Level::Warning) && sched.iter().any(|r| r.level == Level::Debug));
}

#[test]
fn sql_errorlogs_fold_errors_and_use_the_utc_adjustment() {
    let ws = workspace();
    let s = ws.summary();
    // UTF-16 LE, local time UTC+2: the first line says 11:00, which is 09:00 UTC.
    let sql01 = records(&ws, source(&s.sources, "SQL01/ERRORLOG"));
    assert_eq!(sql01[0].ts, Some(START));
    assert_eq!(sql01[0].message.lines().count(), 4);
    let logins: Vec<_> = sql01
        .iter()
        .filter(|r| r.fields.get("ErrorNumber").and_then(|v| v.as_u64()) == Some(18456))
        .collect();
    assert!(!logins.is_empty());
    assert!(logins
        .iter()
        .all(|r| r.message.starts_with("Login failed") && r.level == Level::Warning && r.fields["Source"] == "Logon"));
    let deadlocks = sql01
        .iter()
        .filter(|r| r.fields.get("ErrorNumber").and_then(|v| v.as_u64()) == Some(1205))
        .count();
    assert!(deadlocks > 3, "{deadlocks}");
    let full = sql01
        .iter()
        .find(|r| r.message.contains("transaction log for database 'Orders' is full"))
        .unwrap();
    assert_eq!((full.level, full.fields["Spid"].as_u64()), (Level::Error, Some(88)));
    assert!(
        !sql01.iter().any(|r| r.message.starts_with("Error: ")),
        "every error line is folded"
    );

    // UTF-8 from Linux; an error at the end of a burst stays on its own.
    let rpt = records(&ws, source(&s.sources, "sqlrpt01/errorlog"));
    assert_eq!(rpt.iter().filter(|r| r.level == Level::Fatal).count(), 1);
    assert!(rpt.iter().any(|r| r.message == "Error: 17053, Severity: 16, State: 1."));
}

#[test]
fn container_logs_join_partials_and_carry_metadata() {
    let ws = workspace();
    let s = ws.summary();
    let docker = records(&ws, source(&s.sources, "-json.log"));
    let id = docker[0].fields["ContainerId"].as_str().unwrap();
    assert_eq!(id.len(), 64);
    assert_eq!(docker[0].fields["tag"], "payments-adapter");
    let dump = docker.iter().find(|r| r.message.contains("gateway response")).unwrap();
    assert!(dump.message.len() > 16384 && !dump.message.ends_with('\n'));
    assert_eq!(dump.level, Level::Debug);
    assert!(docker.iter().any(|r| r.level == Level::Error) && docker.iter().any(|r| r.level == Level::Warning));
    let gc = docker.iter().find(|r| r.message == "GC forced").unwrap();
    assert_eq!((gc.level, gc.fields["Stream"].as_str()), (Level::Warning, Some("stderr")));

    let before = records(&ws, source(&s.sources, "consumer/0.log"));
    let after = records(&ws, source(&s.sources, "consumer/1.log"));
    for (recs, restart) in [(&before, 0), (&after, 1)] {
        let f = &recs[0].fields;
        assert_eq!(
            (f["Namespace"].as_str(), f["Pod"].as_str()),
            (Some("shop"), Some("order-events-7c9d8b5f4-x2k4p"))
        );
        assert_eq!((f["Container"].as_str(), f["Restart"].as_u64()), (Some("consumer"), Some(restart)));
    }
    let snapshot = before.iter().find(|r| r.message.contains("batch snapshot")).unwrap();
    assert!(snapshot.message.len() > 16384 && serde_json::from_str::<serde_json::Value>(&snapshot.message).is_ok());
    let panic = before.iter().find(|r| r.message.starts_with("panic:")).unwrap();
    assert_eq!(panic.level, Level::Fatal);
    assert!(before.iter().any(|r| r.message.starts_with("W1014") && r.level == Level::Warning));
    assert!(before.iter().any(|r| r.level == Level::Error));
}
