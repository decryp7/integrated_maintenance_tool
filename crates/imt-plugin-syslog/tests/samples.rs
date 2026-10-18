//! End to end against the generated Linux samples in `/samples/linux`: the
//! Docker host's `/var/log/syslog` (RFC 3164 / BSD lines), RFC 5424 lines of a
//! Kubernetes node and a `journalctl -o json` export, plus snapshots.
//!
//! BSD lines carry no year: the importer takes it from the file's date. A
//! checkout's file date is arbitrary, so the imports here pass the scenario's
//! date (2026-10-14) like the workspace would for a file written that day.

use chrono::NaiveDate;
use imt_core::importers::{decode, ImportCtx, Imported, Importer};
use imt_core::model::{Level, Record};
use imt_core::profile::CompiledProfile;
use imt_core::{PluginRegistry, Profile, Workspace};
use imt_plugin_syslog::{JournaldImporter, SyslogImporter};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// 2026-10-14 09:00 and 12:00 UTC, the scenario window of `samples/generate.py`.
const START: i64 = 1_791_968_400_000;
const END: i64 = 1_791_979_200_000;
/// The order-events consumer crashes with WEB02 at 10:41:24.
const CRASH: i64 = START + 6_084_050;

const BSD: &str = "linux/docker01/syslog";
const RFC5424: &str = "linux/k8s-node-2/rsyslog-rfc5424.log";
const JOURNAL: &str = "linux/sqlrpt01/journal.json";

fn import(rel: &str, importer: &dyn Importer) -> Imported {
    let bytes = std::fs::read(samples().join(rel)).unwrap();
    let content = decode(&bytes).unwrap();
    let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
    let ctx = ImportCtx {
        profile: &profile,
        path: Path::new(rel),
        file_date: NaiveDate::from_ymd_opt(2026, 10, 14),
    };
    let head: String = content.chars().take(64 * 1024).collect();
    assert!(importer.sniff(&ctx, &head) > 0, "{rel} is not recognised by {}", importer.id());
    importer.import(&ctx, &content).unwrap()
}

fn text<'a>(r: &'a Record, field: &str) -> &'a str {
    r.fields.get(field).and_then(Value::as_str).unwrap_or("")
}

#[test]
fn the_workspace_picks_the_syslog_and_journald_importers() {
    let mut registry = PluginRegistry::new();
    registry.install(&imt_plugin_syslog::SyslogPlugin).unwrap();
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
    ws.open(&[samples().join("linux")]).unwrap();
    let s = ws.summary();
    for (suffix, importer) in [(BSD, "syslog"), (RFC5424, "syslog"), (JOURNAL, "journald")] {
        let src = s
            .sources
            .iter()
            .find(|x| x.path.replace('\\', "/").ends_with(suffix))
            .unwrap_or_else(|| panic!("{suffix} not opened"));
        assert_eq!((src.importer.as_str(), src.skipped), (importer, 0), "{suffix}");
        assert!(src.records > 0 && src.from.is_some(), "{suffix}");
    }
    assert_eq!(s.sources.len(), 3);
}

#[test]
fn every_linux_sample_is_inside_the_scenario_window() {
    for (rel, importer) in [
        (BSD, &SyslogImporter as &dyn Importer),
        (RFC5424, &SyslogImporter),
        (JOURNAL, &JournaldImporter),
    ] {
        let out = import(rel, importer);
        assert_eq!(out.skipped, 0, "{rel}");
        assert!(!out.records.is_empty(), "{rel}");
        for r in &out.records {
            let ts = r.ts.unwrap_or_else(|| panic!("{rel} line {} has no time", r.line));
            assert!((START..=END).contains(&ts), "{rel} line {}: {ts}", r.line);
        }
    }
}

#[test]
fn bsd_syslog_has_hosts_apps_and_the_conntrack_warnings() {
    let out = import(BSD, &SyslogImporter);
    assert!(out.records.iter().all(|r| text(r, "Host") == "docker01"));
    let apps: std::collections::BTreeSet<&str> = out.records.iter().map(|r| text(r, "App")).collect();
    for app in ["CRON", "dockerd", "kernel", "sshd", "systemd"] {
        assert!(apps.contains(app), "{apps:?}");
    }
    let conntrack: Vec<&Record> = out
        .records
        .iter()
        .filter(|r| r.message.contains("nf_conntrack: table full"))
        .collect();
    assert!(!conntrack.is_empty() && conntrack.iter().all(|r| r.ts.unwrap() < CRASH));
}

#[test]
fn rfc5424_has_structured_data_and_the_container_crash() {
    let out = import(RFC5424, &SyslogImporter);
    let crash = out.records.iter().find(|r| text(r, "MsgId") == "CRASH").unwrap();
    assert_eq!(crash.level, Level::Error);
    assert_eq!(text(crash, "sd.k8s@32473.pod"), "order-events-7c9d8b5f4-x2k4p");
    assert!((crash.ts.unwrap() - CRASH).abs() < 1_000);
    assert!(out.records.iter().any(|r| text(r, "MsgId") == "PROBE" && r.level == Level::Warning));
}

#[test]
fn journal_shows_the_disk_errors_behind_sql_error_823() {
    let out = import(JOURNAL, &JournaldImporter);
    let io: Vec<&Record> = out.records.iter().filter(|r| r.message.contains("I/O error, dev sdb")).collect();
    assert_eq!(io.len(), 6);
    assert!(io.iter().all(|r| r.level == Level::Error));
    let sql = out.records.iter().find(|r| r.message.starts_with("Error: 823")).unwrap();
    assert!(sql.ts.unwrap() > io[0].ts.unwrap());
}

/// A compact projection of an import: counts, range and the first records.
fn snapshot_of(rel: &str, importer: &dyn Importer) -> Value {
    let out = import(rel, importer);
    let stamps: Vec<i64> = out.records.iter().filter_map(|r| r.ts).collect();
    let mut fields: Vec<&String> = out.records.iter().flat_map(|r| r.fields.keys()).collect();
    fields.sort();
    fields.dedup();
    serde_json::json!({
        "sample": rel,
        "importer": importer.id(),
        "records": out.records.len(),
        "skipped": out.skipped,
        "first_ts": stamps.iter().min(),
        "last_ts": stamps.iter().max(),
        "fields": fields,
        "first": out.records.iter().take(3).map(|r| serde_json::json!({
            "ts": r.ts, "level": r.level.as_str(), "message": r.message, "fields": r.fields,
        })).collect::<Vec<_>>(),
    })
}

#[test]
fn text_format_snapshots() {
    insta::assert_yaml_snapshot!("syslog-bsd-docker01", snapshot_of(BSD, &SyslogImporter));
    insta::assert_yaml_snapshot!("syslog-rfc5424-k8s-node-2", snapshot_of(RFC5424, &SyslogImporter));
    insta::assert_yaml_snapshot!("journald-sqlrpt01", snapshot_of(JOURNAL, &JournaldImporter));
}
