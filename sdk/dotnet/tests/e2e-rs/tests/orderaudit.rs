//! The published C# OrderAudit sample, started by imt-plugin-host's adapter like IMT does (private socket,
//! token on stdin, empty environment), imports the generated sample data into an imt-core Workspace: records,
//! fields, the enrich transform, marker kinds (rules and code-driven marks), order spans and the exporter.

use imt_core::model::{Level, Record};
use imt_core::plugin::annotate::AnnotationsRequest;
use imt_core::plugin::{PluginStatus, TransformCtx, Verdict};
use imt_core::profile::Profile;
use imt_core::{Plugin, PluginRegistry, Workspace};
use imt_plugin_host::{PluginSpec, RemotePlugin};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const ID: &str = "com.example.orderaudit";

fn sdk_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn rid() -> String {
    let os = match std::env::consts::OS {
        "macos" => "osx",
        "windows" => "win",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    format!("{os}-{arch}")
}

/// The plugin folder the SDK's publish target lays out: plugin.json + bin/<rid>/<executable>.
fn plugin_exe() -> PathBuf {
    let dir = std::env::var_os("IMT_DOTNET_PLUGINS")
        .map(PathBuf::from)
        .unwrap_or_else(|| sdk_dir().join("artifacts/plugins"))
        .join(ID);
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("plugin.json")).unwrap_or_else(|e| {
        panic!(
            "{}: {e}. Publish the sample first: dotnet publish sdk/dotnet/samples/OrderAudit/src -c Release -r {}",
            dir.join("plugin.json").display(),
            rid()
        )
    }))
    .unwrap();
    assert_eq!(manifest["format"], 1);
    assert_eq!(manifest["contract"]["package"], "imt.plugin.v1");
    assert_eq!(manifest["manifest"]["id"], ID);
    let exe = manifest["executable"].as_str().unwrap();
    let file = if cfg!(windows) { format!("{exe}.exe") } else { exe.to_string() };
    let path = dir.join("bin").join(rid()).join(file);
    assert!(path.is_file(), "{} is missing", path.display());
    path
}

fn expected() -> Value {
    serde_json::from_str(&std::fs::read_to_string(sdk_dir().join("samples/OrderAudit/data/expected.json")).unwrap()).unwrap()
}

fn n(v: &Value, key: &str) -> usize {
    v[key].as_u64().unwrap() as usize
}

fn start() -> Arc<RemotePlugin> {
    RemotePlugin::start(PluginSpec::new(plugin_exe(), "the OrderAudit sample")).expect("the C# plugin starts and shakes hands")
}

fn profile_with_enrich() -> Profile {
    let mut p = serde_json::to_value(Profile::default_profile()).unwrap();
    p["pipeline"] = json!([{ "step": "plugin", "id": "orderaudit.enrich" }]);
    serde_json::from_value(p).unwrap()
}

fn installed(p: &Arc<RemotePlugin>) -> PluginRegistry {
    let mut r = PluginRegistry::empty();
    r.install_shared(p.clone()).unwrap();
    r.resolve();
    r
}

#[test]
fn the_sample_imports_into_a_workspace_with_fields_markers_and_spans() {
    let exp = expected();
    let p = start();
    assert_eq!((p.id(), p.name(), p.version()), (ID, "Order audit", "1.0.0"));
    let m = p.remote_manifest();
    assert!(m.permissions.is_empty());
    assert!(m.requires.iter().any(|d| d.id == "net.decryptology.otel" && d.optional));
    let registry = installed(&p);
    let info = registry.plugins().into_iter().find(|i| i.id == ID).unwrap();
    assert_eq!(info.status, PluginStatus::Active, "{:?}", info.inactive_reason);
    for c in [
        "importer:orderaudit.log",
        "transform:orderaudit.enrich",
        "exporter:orderaudit.summary",
        "annotator:orderaudit.orders",
        "settings",
    ] {
        assert!(info.contributes.iter().any(|x| x == c), "{c} in {:?}", info.contributes);
    }

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("orders-audit.log");
    std::fs::copy(sdk_dir().join("samples/OrderAudit/data/orders-audit.log"), &file).unwrap();
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
    ws.set_profile(profile_with_enrich()).unwrap();
    ws.open(&[file]).unwrap();

    let s = ws.summary();
    let src = &s.sources[0];
    assert_eq!(src.importer, "orderaudit.log", "the C# importer wins the sniff: {:?}", s.sources);
    assert_eq!(src.error, None);
    assert_eq!(src.skipped, n(&exp, "skipped"));
    let warnings = &s.plugins.iter().find(|p| p.id == ID).unwrap().warnings;
    assert!(warnings.iter().all(|w| w.starts_with("optional:")), "no failed calls: {warnings:?}");
    assert_eq!(s.dropped, n(&exp, "heartbeats"), "heartbeats dropped by the C# transform");
    assert_eq!(s.total, n(&exp, "records") - n(&exp, "heartbeats"));
    assert_eq!(s.levels.get(&Level::Error).copied().unwrap_or(0), n(&exp, "failed"));
    for k in ["order-failed", "payment-retry", "slow-order"] {
        let full = format!("{ID}/{k}");
        assert!(
            s.marker_kinds.iter().any(|mk| mk.kind == full && mk.owner == ID),
            "{full} in {:?}",
            s.marker_kinds.iter().map(|m| &m.kind).collect::<Vec<_>>()
        );
    }

    let page = ws.page(&serde_json::from_value(json!({ "limit": 5000 })).unwrap()).unwrap();
    assert_eq!(page.total, s.total);
    let rows = &page.rows;
    let with = |marker: &str| {
        rows.iter()
            .filter(|r| r.markers.iter().any(|m| *m == format!("{ID}/{marker}")))
            .count()
    };
    assert_eq!(
        with("order-failed"),
        n(&exp, "failed"),
        "rule-based marker from the C# marker kinds"
    );
    assert_eq!(with("payment-retry"), n(&exp, "paymentRetries"));
    assert_eq!(with("slow-order"), n(&exp, "slow"), "code-driven mark from the C# transform");

    let first = rows
        .iter()
        .find(|r| r.fields.get("OrderId") == Some(&json!("ORD-10001")) && r.fields.get("Event") == Some(&json!("OrderStarted")))
        .unwrap();
    assert_eq!(first.ts, Some(1_792_224_008_031), "2026-10-17T08:00:08.031Z");
    assert_eq!(first.level, Level::Information);
    assert_eq!(first.template.as_deref(), Some("OrderStarted"));
    let keys: Vec<&str> = first.fields.keys().map(String::as_str).collect();
    assert_eq!(
        &keys[..6],
        ["Component", "Event", "OrderId", "Customer", "Amount", "Channel"],
        "field order survives Arrow"
    );
    assert!(
        first.fields["AmountValue"].is_f64(),
        "typed by the transform: {:?}",
        first.fields["AmountValue"]
    );
    assert!(first.fields["Currency"].is_string());
    let ends: Vec<_> = rows
        .iter()
        .filter(|r| {
            matches!(
                r.fields.get("Event").and_then(Value::as_str),
                Some("OrderCompleted" | "OrderFailed")
            )
        })
        .collect();
    assert_eq!(ends.len(), n(&exp, "completed") + n(&exp, "failed"));
    assert!(ends.iter().all(|r| r.fields["DurationMs"].as_i64().unwrap() > 0));
    let failed = rows.iter().find(|r| r.exception.is_some()).unwrap();
    assert!(failed.exception.as_deref().unwrap().starts_with("Shop.Orders.OrderException:"));
    assert_eq!(rows.iter().filter(|r| r.exception.is_some()).count(), n(&exp, "withException"));

    let spans = ws.annotations(&AnnotationsRequest::default());
    assert_eq!(spans.len(), n(&exp, "orders"), "one span per order from the C# annotator");
    assert!(spans.iter().all(|a| a.owner == ID && a.kind == format!("{ID}/order")));
    assert_eq!(
        spans.iter().filter(|a| a.severity == imt_core::api::Severity::Critical).count(),
        n(&exp, "failed")
    );
    assert_eq!(spans.iter().filter(|a| a.label.ends_with("(open)")).count(), n(&exp, "open"));
    let one = spans
        .iter()
        .find(|a| a.on.as_ref().is_some_and(|o| o.value == "ORD-10002"))
        .unwrap();
    assert_eq!(one.on.as_ref().unwrap().field, "OrderId");
    assert!(one.to > one.from);
    assert_eq!(one.evidence_query.from, Some(one.from));

    // The exporter: one CSV line per order, streamed back from the process.
    let exporter = ws
        .summary()
        .exporters
        .into_iter()
        .find(|e| e.id == "orderaudit.summary")
        .expect("the exporter is listed");
    assert_eq!(exporter.extension, "csv");
    let out = dir.path().join("summary.csv");
    let req = serde_json::from_value(json!({ "format": "orderaudit.summary", "path": out.to_string_lossy() })).unwrap();
    ws.export(&req).unwrap();
    let csv = std::fs::read_to_string(&out).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        "OrderId,Customer,Amount,Currency,Status,Started,DurationMs,PaymentRetries,Source"
    );
    assert_eq!(lines.len() - 1, n(&exp, "orders"));
    assert_eq!(lines.iter().filter(|l| l.contains(",failed,")).count(), n(&exp, "failed"));
}

#[test]
fn records_round_trip_through_the_csharp_decoder_and_encoder() {
    let p = start();
    let r = installed(&p);
    let t = r.hosted_transforms().into_iter().find(|t| t.id() == "orderaudit.enrich").unwrap();
    let mut records: Vec<Record> = (0..300)
        .map(|i| {
            let mut r = Record::new(i);
            r.ts = if i % 7 == 0 { None } else { Some(1_700_000_000_000 + i as i64) };
            r.level = Level::ALL[i as usize % 6];
            r.message = format!("naïve ☃ line {i}\nsecond \u{0} line");
            r.template = (i % 2 == 0).then(|| "t {x}".into());
            r.exception = (i % 3 == 0).then(|| "System.Exception: boom\n   at A.B()".into());
            if i % 5 != 0 {
                r.fields.insert("s".into(), json!(format!("v{i}")));
                r.fields.insert("n".into(), json!(-(i as i64)));
                r.fields.insert("big".into(), json!(u64::MAX));
                r.fields.insert("f".into(), json!(1.5 + i as f64));
                r.fields.insert("b".into(), json!(i % 2 == 0));
                r.fields.insert("nested".into(), json!({"a": [1, 2.5, null, true, "x"]}));
                r.fields.insert("mixed".into(), if i % 2 == 0 { json!(1) } else { json!("one") });
            }
            if i % 4 == 0 {
                // A different field order than the first record's.
                let mut f = serde_json::Map::new();
                f.insert("z".into(), json!(null));
                for (k, v) in std::mem::take(&mut r.fields) {
                    f.insert(k, v);
                }
                r.fields = f.into_iter().collect();
            }
            r.id = 1000 + i;
            r.source = 3;
            r
        })
        .collect();
    // One record the transform changes, to prove the batch really went through the C# code.
    let mut probe = Record::new(9999);
    probe.fields.insert("Amount".into(), json!("12.50 EUR"));
    records.push(probe);
    let original = records.clone();
    let ctx = TransformCtx::new("roundtrip.log");
    let verdicts = t.apply_batch(&mut records, &ctx);
    assert!(verdicts.iter().all(|v| *v == Verdict::Keep));
    let probe = records.pop().unwrap();
    assert_eq!(
        (probe.fields["AmountValue"].as_f64(), probe.fields["Currency"].as_str()),
        (Some(12.5), Some("EUR")),
        "the C# transform ran"
    );
    for (a, b) in original.iter().zip(&records) {
        assert_eq!(
            (a.ts, a.level, &a.message, &a.template, &a.exception, a.line, a.id, a.source),
            (b.ts, b.level, &b.message, &b.template, &b.exception, b.line, b.id, b.source)
        );
        assert_eq!(
            serde_json::to_string(&a.fields).unwrap(),
            serde_json::to_string(&b.fields).unwrap(),
            "fields (types and order) of line {}",
            a.line
        );
    }
    assert_eq!(r.health().get(ID).failures, 0);
}

#[test]
fn settings_reach_the_csharp_plugin() {
    let p = start();
    let c = p.configurable().expect("settings section");
    let section = c.settings_section();
    assert_eq!(
        section.fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(),
        ["slowOrderMs", "dropHeartbeats", "defaultCurrency"]
    );
    let values = imt_core::settings::validate(&section, &json!({"slowOrderMs": 3_600_000, "dropHeartbeats": false})).unwrap();
    c.apply_settings(&values).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("orders-audit.log");
    std::fs::copy(sdk_dir().join("samples/OrderAudit/data/orders-audit.log"), &file).unwrap();
    let mut ws = Workspace::with_registry(Profile::default_profile(), installed(&p)).unwrap();
    ws.set_profile(profile_with_enrich()).unwrap();
    ws.open(&[file]).unwrap();
    let exp = expected();
    let s = ws.summary();
    assert_eq!(s.total, n(&exp, "records"), "heartbeats kept");
    let page = ws.page(&serde_json::from_value(json!({ "limit": 5000 })).unwrap()).unwrap();
    assert!(
        page.rows.iter().all(|r| !r.markers.iter().any(|m| m.ends_with("/slow-order"))),
        "nothing is slow with a one-hour threshold"
    );
}
