//! End to end against the generated web samples in `/samples/web`: the IIS
//! logs of WEB01/WEB02, the nginx edge's access log and the HAR capture of a
//! slow checkout, opened by a workspace with this plugin, plus snapshots.

use imt_core::api::SourceInfo;
use imt_core::importers::{decode, ImportCtx, Importer};
use imt_core::model::{Level, Record};
use imt_core::profile::CompiledProfile;
use imt_core::{PluginRegistry, Profile, Workspace};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// 2026-10-14 09:00 and 12:00 UTC, the scenario window of `samples/generate.py`.
const START: i64 = 1_791_968_400_000;
const END: i64 = 1_791_979_200_000;
/// WEB02's OrderService is down from 10:41:24 to 10:42:10.
const CRASH: i64 = START + 6_084_050;
const RESTART: i64 = START + 6_130_412;

fn workspace() -> Workspace {
    let mut registry = PluginRegistry::new();
    registry.install(&imt_plugin_web::WebPlugin).unwrap();
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
    ws.open(&[samples().join("web")]).unwrap();
    ws
}

fn source<'a>(sources: &'a [SourceInfo], suffix: &str) -> &'a SourceInfo {
    sources
        .iter()
        .find(|s| s.path.replace('\\', "/").ends_with(suffix))
        .unwrap_or_else(|| panic!("{suffix} not opened: {:?}", sources.iter().map(|s| &s.name).collect::<Vec<_>>()))
}

fn records<'a>(ws: &'a Workspace, src: &SourceInfo) -> Vec<&'a Record> {
    ws.records().iter().filter(|r| r.source == src.id).collect()
}

fn num(r: &Record, field: &str) -> i64 {
    r.fields.get(field).and_then(Value::as_i64).unwrap_or(-1)
}

#[test]
fn every_web_sample_is_read_by_its_importer() {
    let ws = workspace();
    let s = ws.summary();
    for (suffix, importer) in [
        ("WEB01/W3SVC1/u_ex261014.log", "w3c"),
        ("WEB02/W3SVC1/u_ex261014.log", "w3c"),
        ("edge01/access.log", "access-log"),
        ("har/checkout-slow.har", "har"),
    ] {
        let src = source(&s.sources, suffix);
        assert_eq!(src.importer, importer, "{suffix}");
        assert_eq!(src.skipped, 0, "{suffix}");
        assert!(src.records > 0, "{suffix}");
        let (from, to) = (src.from.unwrap(), src.to.unwrap());
        assert!(START <= from && to <= END, "{suffix}: {from}..{to}");
    }
    assert_eq!(s.sources.len(), 4, "nothing else in /samples/web");
}

#[test]
fn iis_on_web02_answers_502_while_the_app_is_down() {
    let ws = workspace();
    let s = ws.summary();
    let web02 = records(&ws, source(&s.sources, "WEB02/W3SVC1/u_ex261014.log"));
    let down: Vec<&&Record> = web02.iter().filter(|r| (CRASH..RESTART).contains(&r.ts.unwrap())).collect();
    assert!(!down.is_empty());
    assert!(down
        .iter()
        .all(|r| num(r, "sc-status") == 502 && num(r, "sc-substatus") == 3 && r.level == Level::Error));
    // The restart opened a second header block; its rows parse like the first.
    assert!(web02.iter().any(|r| r.ts.unwrap() > RESTART && num(r, "sc-status") < 300));
    // Slow captures before the crash, fast ones on WEB01.
    let slowest = |rows: &[&Record]| rows.iter().map(|r| num(r, "time-taken")).max().unwrap();
    let web01 = records(&ws, source(&s.sources, "WEB01/W3SVC1/u_ex261014.log"));
    assert!(slowest(&web02) > 5_000, "{}", slowest(&web02));
    assert!(slowest(&web02) > slowest(&web01));
}

#[test]
fn nginx_edge_sees_gateway_timeouts_during_the_incident() {
    let ws = workspace();
    let s = ws.summary();
    let edge = records(&ws, source(&s.sources, "edge01/access.log"));
    let timeouts: Vec<&&Record> = edge.iter().filter(|r| num(r, "Status") == 504).collect();
    assert!(!timeouts.is_empty());
    let incident = START + 5_400_000; // 10:30
    assert!(timeouts.iter().all(|r| r.ts.unwrap() >= incident), "504s only from 10:30");
    assert!(timeouts
        .iter()
        .all(|r| r.fields.get("RequestTimeMs").and_then(Value::as_f64).unwrap() >= 29_000.0));
}

#[test]
fn har_capture_shows_the_hanging_payment_capture() {
    let ws = workspace();
    let s = ws.summary();
    let har = records(&ws, source(&s.sources, "checkout-slow.har"));
    assert_eq!(har.len(), 10);
    let slow = har
        .iter()
        .max_by(|a, b| a.fields["TimeMs"].as_f64().partial_cmp(&b.fields["TimeMs"].as_f64()).unwrap())
        .unwrap();
    assert_eq!(num(slow, "Status"), 504);
    assert!(slow.fields["Url"].as_str().unwrap().ends_with("/api/orders/88412/capture"));
    assert!(slow.fields["TimeMs"].as_f64().unwrap() > 30_000.0);
    assert!(
        har.iter().any(|r| num(r, "Status") == 0 && r.level == Level::Error),
        "the blocked beacon"
    );
}

/// A compact projection of an import: counts, range and the first records.
fn snapshot_of(rel: &str, importer: &dyn Importer) -> Value {
    let bytes = std::fs::read(samples().join(rel)).unwrap();
    let content = decode(&bytes).unwrap();
    let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
    let ctx = ImportCtx {
        profile: &profile,
        path: Path::new(rel),
        file_date: None,
    };
    let head: String = content.chars().take(64 * 1024).collect();
    assert!(importer.sniff(&ctx, &head) > 0, "{rel} is not recognised by {}", importer.id());
    let out = importer.import(&ctx, &content).unwrap();
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
    insta::assert_yaml_snapshot!(
        "iis-web02",
        snapshot_of("web/iis/WEB02/W3SVC1/u_ex261014.log", &imt_plugin_web::W3cImporter)
    );
    insta::assert_yaml_snapshot!(
        "nginx-edge01",
        snapshot_of("web/nginx/edge01/access.log", &imt_plugin_web::AccessLogImporter)
    );
    insta::assert_yaml_snapshot!(
        "har-checkout",
        snapshot_of("web/har/checkout-slow.har", &imt_plugin_web::HarImporter)
    );
}
