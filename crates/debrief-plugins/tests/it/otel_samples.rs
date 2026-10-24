//! End to end against the generated OpenTelemetry samples in `/samples/otel`
//! (Collector `file` exporter output: OTLP/JSON logs and traces, one export per
//! line), opened by a workspace with this plugin, plus snapshots.

use crate::common;

use debrief_core::model::{Level, Record};
use debrief_core::{Profile, Workspace};
use debrief_plugin_api::importer::{ImportCtx, TextImporter};
use debrief_plugin_api::text::decode;
use debrief_plugin_api::time::TimestampParser;
use debrief_plugin_otel::OtlpImporter;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// 2026-10-14 09:00 and 12:00 UTC, the scenario window of `samples/generate.py`.
const START: i64 = 1_791_968_400_000;
const END: i64 = 1_791_979_200_000;
const INCIDENT: i64 = START + 5_400_000;

const LOGS: &str = "otel/collector/logs-20261014.jsonl";
const TRACES: &str = "otel/collector/traces-20261014.jsonl";

fn workspace() -> Workspace {
    let registry = common::registry(&[common::OTEL]);
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
    ws.open(&[samples().join("otel")]).unwrap();
    ws
}

fn text<'a>(r: &'a Record, field: &str) -> &'a str {
    r.fields.get(field).and_then(Value::as_str).unwrap_or("")
}

fn of<'a>(ws: &'a Workspace, suffix: &str) -> Vec<&'a Record> {
    let s = ws.summary();
    let src = s.sources.iter().find(|x| x.path.replace('\\', "/").ends_with(suffix)).unwrap();
    assert_eq!((src.importer.as_str(), src.skipped), ("otlp", 0), "{suffix}");
    let (from, to) = (src.from.unwrap(), src.to.unwrap());
    assert!(START <= from && to <= END, "{suffix}: {from}..{to}");
    let id = src.id;
    ws.records().iter().filter(|r| r.source == id).collect()
}

#[test]
fn traces_link_gateway_order_service_and_payments() {
    let ws = workspace();
    let spans = of(&ws, TRACES);
    let services: HashSet<&str> = spans.iter().map(|r| text(r, "service.name")).collect();
    assert_eq!(services, HashSet::from(["gateway", "order-service", "payments-adapter"]));
    // Every trace has its six spans; failed captures carry an exception event.
    let traces: HashSet<&str> = spans.iter().map(|r| text(r, "TraceId")).collect();
    assert!(traces.len() > 300, "{}", traces.len());
    let errors: Vec<&&Record> = spans.iter().filter(|r| r.level == Level::Error).collect();
    assert!(!errors.is_empty());
    assert!(errors.iter().all(|r| r.ts.unwrap() >= INCIDENT), "errors only during the incident");
    assert!(spans
        .iter()
        .any(|r| r.exception.as_deref().is_some_and(|x| x.contains("HttpRequestException"))));
}

#[test]
fn logs_carry_the_trace_ids_of_their_spans() {
    let ws = workspace();
    let logs = of(&ws, LOGS);
    let spans = of(&ws, TRACES);
    let trace_ids: HashSet<&str> = spans.iter().map(|r| text(r, "TraceId")).collect();
    assert!(logs.iter().all(|r| trace_ids.contains(text(r, "TraceId"))));
    assert!(logs
        .iter()
        .any(|r| r.level == Level::Error && r.message.contains("context deadline exceeded")));
}

fn snapshot_of(rel: &str) -> Value {
    let bytes = std::fs::read(samples().join(rel)).unwrap();
    let content = decode(&bytes).unwrap();
    let ts = TimestampParser::default();
    let mut ctx = ImportCtx::new(Path::new(rel), &ts);
    ctx.file_date = None;
    let head: String = content.chars().take(64 * 1024).collect();
    assert!(OtlpImporter.sniff(&ctx, &head) > 0, "{rel}");
    let out = OtlpImporter.import(&ctx, &content).unwrap();
    let stamps: Vec<i64> = out.records.iter().filter_map(|r| r.ts).collect();
    let mut fields: Vec<&String> = out.records.iter().flat_map(|r| r.fields.keys()).collect();
    fields.sort();
    fields.dedup();
    serde_json::json!({
        "sample": rel,
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
fn otlp_snapshots() {
    insta::assert_yaml_snapshot!("otlp-logs", snapshot_of(LOGS));
    insta::assert_yaml_snapshot!("otlp-traces", snapshot_of(TRACES));
}
