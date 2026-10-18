//! A plugin process seen through the host adapter: every capability round trip, crashes, hangs, OOM and
//! panics becoming quarantine, permission enforcement, and the workspace using a remote plugin.

use imt_core::export::ExportRow;
use imt_core::guard::Budgets;
use imt_core::importers::ImportCtx;
use imt_core::model::{Level, Record};
use imt_core::plugin::annotate::{AnnotationView, ViewFilter, ViewRecord};
use imt_core::plugin::markers::MarkIndex;
use imt_core::plugin::{PluginStatus, TransformCtx, Verdict};
use imt_core::profile::{CompiledProfile, Profile};
use imt_core::query::Query;
use imt_core::{Plugin, PluginRegistry};
use imt_plugin_host::{PluginSpec, RemotePlugin};
use imt_plugin_sdk::{ByteService, Permission, PermissionRequest};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const ID: &str = "test.fake";

fn spec(args: &[&str]) -> PluginSpec {
    let mut s = PluginSpec::new(env!("CARGO_BIN_EXE_imt-plugin-host-fake"), "the fake plugin");
    s.args = args.iter().map(|a| a.to_string()).collect();
    s.supervisor.backoff_base = Duration::from_millis(5);
    s.supervisor.max_crashes = 50;
    s
}

fn fake(args: &[&str]) -> Arc<RemotePlugin> {
    RemotePlugin::start(spec(args)).unwrap()
}

fn installed(p: &Arc<RemotePlugin>) -> PluginRegistry {
    let mut r = PluginRegistry::empty();
    r.install_shared(p.clone()).unwrap();
    r.resolve();
    r
}

fn import(r: &PluginRegistry, file: &str, content: &str) -> anyhow::Result<imt_core::importers::Imported> {
    let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
    let ctx = ImportCtx {
        profile: &profile,
        path: Path::new(file),
        file_date: None,
    };
    let i = r.hosted_importers().find(|i| i.id() == "fake").unwrap();
    i.import(&ctx, content)
}

fn pid(p: &RemotePlugin) -> Option<u32> {
    p.process_status().pid
}

#[test]
fn importers_round_trip_with_typed_fields_and_tail() {
    let p = fake(&[]);
    assert_eq!((p.id(), p.name(), p.version()), (ID, "Fake plugin", "1.2.3"));
    let m = p.remote_manifest();
    assert_eq!(m.capabilities.importers.len(), 2);
    assert!(!m.capabilities.importers[0].multiline);
    assert_eq!(p.manifest().provides[0].id, "fake.echo");
    assert!(p.manifest().requires.iter().any(|d| d.id == "test.optional" && d.optional));
    let r = installed(&p);
    let info = &r.plugins()[0];
    assert_eq!(info.status, PluginStatus::Active);
    for c in [
        "importer:fake",
        "transform:fake.upper",
        "exporter:fake.lines",
        "marker:fake-mark",
        "annotator:fake.errors",
        "service:fake.echo",
        "settings",
    ] {
        assert!(info.contributes.contains(&c.to_string()), "{c} in {:?}", info.contributes);
    }
    let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
    let ctx = ImportCtx {
        profile: &profile,
        path: Path::new("/var/log/x.fake"),
        file_date: chrono::NaiveDate::from_ymd_opt(2024, 1, 2),
    };
    let i = r.hosted_importers().next().unwrap();
    assert_eq!(i.sniff(&ctx, "FAKE"), 80);
    assert!(!i.multiline());
    let out = i.import(&ctx, "ERR|boom|n=5 s=x\nnot a record\nINF|fine\n").unwrap();
    assert_eq!(out.skipped, 1);
    assert_eq!(out.records.len(), 2);
    assert_eq!(out.records[0].level, Level::Error);
    assert_eq!(out.records[0].fields["n"], serde_json::json!(5));
    assert_eq!(out.records[0].fields["s"], serde_json::json!("x"));
    assert_eq!(out.records[1].line, 3);
    // The binary importer.
    let b = r.hosted_binary_importers().next().unwrap();
    assert_eq!(b.id(), "fakebin");
    assert_eq!(b.sniff(&ctx, b"FBxx"), 100);
    assert_eq!(b.import(&ctx, b"FB123").unwrap().records[0].message, "5 bytes");
    assert!(i.import_tail(&ctx, "INF|a\n", "ERROR\n").unwrap().is_err());
    let tail = i.import_tail(&ctx, "INF|a\n", "WRN|b\n").unwrap().unwrap();
    assert_eq!(tail.records[0].message, "b");
    assert!(i.import_tail(&ctx, "INF|a\n", "NOTAIL\n").is_none());
    // A typed error of the plugin's code is not a crash.
    let e = i.import(&ctx, "ERROR\n").err().unwrap().to_string();
    assert!(e.contains("this file is not valid"), "{e}");
    assert_eq!(r.health().get(ID).failures, 0);
    // A big file crosses in several batches.
    let big: String = (0..150_000).map(|n| format!("INF|line {n}|k={n}\n")).collect();
    let out = i.import(&ctx, &big).unwrap();
    assert_eq!(out.records.len(), 150_000);
    assert_eq!(out.records[149_999].fields["k"], serde_json::json!(149_999));
}

#[test]
fn transforms_run_in_batches_with_verdicts_and_marks() {
    let p = fake(&[]);
    let r = installed(&p);
    let t = r.hosted_transforms().into_iter().next().unwrap();
    assert!(t.batched());
    let marks = MarkIndex::of_kinds(ID, &["fake-mark".to_string()]);
    let ctx = TransformCtx::with_marks("a.fake", &marks);
    let mut records: Vec<Record> = ["hello", "please drop", "mark me", "x"]
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mut r = Record::new(i as u32);
            r.message = m.to_string();
            r.id = 100 + i as u32;
            r.source = 7;
            r
        })
        .collect();
    let verdicts = t.apply_batch(&mut records, &ctx);
    assert_eq!(verdicts, [Verdict::Keep, Verdict::Drop, Verdict::Keep, Verdict::Keep]);
    assert_eq!(records[0].message, "HELLO");
    assert_eq!(records[0].fields["upper"], serde_json::json!(true));
    assert_eq!((records[0].id, records[0].source), (100, 7), "IMT's ids stay");
    assert_eq!(records[2].markers, 1, "marked with its own kind");
    assert_eq!(records[0].markers, 0);
    assert!(
        r.health().notes(ID).iter().any(|n| n.contains("not-mine")),
        "{:?}",
        r.health().notes(ID)
    );
    // One at a time works too.
    let mut one = Record::new(1);
    one.message = "solo".into();
    assert_eq!(t.apply(&mut one, &ctx), Verdict::Keep);
    assert_eq!(one.message, "SOLO");
    // Small batches: many round trips on one stream.
    let p2 = fake(&[]);
    p2.set_batch_rows(3);
    let r2 = installed(&p2);
    let t2 = r2.hosted_transforms().into_iter().next().unwrap();
    let mut many: Vec<Record> = (0..10)
        .map(|i| {
            let mut r = Record::new(i);
            r.message = if i % 2 == 0 { format!("drop {i}") } else { format!("keep {i}") };
            r
        })
        .collect();
    let v = t2.apply_batch(&mut many, &TransformCtx::new("s"));
    assert_eq!(v.iter().filter(|v| **v == Verdict::Drop).count(), 5);
    assert_eq!(many[9].message, "KEEP 9");
    assert!(t2.apply_batch(&mut [], &TransformCtx::new("s")).is_empty());
}

#[test]
fn the_workspace_imports_and_runs_the_pipeline_through_the_process() {
    let p = fake(&[]);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.fake");
    std::fs::write(&file, "INF|keep me\nINF|drop me\nERR|mark this\n").unwrap();
    let mut profile = Profile::default_profile();
    profile.pipeline = serde_json::from_value(serde_json::json!([{ "step": "plugin", "id": "fake.upper" }])).unwrap();
    let mut ws = imt_core::Workspace::with_registry(Profile::default_profile(), installed(&p)).unwrap();
    ws.set_profile(profile).unwrap();
    ws.open(&[file]).unwrap();
    let s = ws.summary();
    assert_eq!(s.total, 2, "{s:?}");
    let page = ws.page(&serde_json::from_value(serde_json::json!({})).unwrap()).unwrap();
    let messages: Vec<String> = page.rows.iter().map(|r| r.message.to_string()).collect();
    assert!(messages.contains(&"KEEP ME".to_string()), "{messages:?}");
}

struct View(Vec<ViewRecord>);

impl AnnotationView for View {
    fn select(&self, filter: &ViewFilter, limit: usize) -> anyhow::Result<Vec<ViewRecord>> {
        let conds: Vec<_> = filter
            .when
            .iter()
            .map(imt_core::condition::Condition::compile)
            .collect::<anyhow::Result<_>>()?;
        Ok(self
            .0
            .iter()
            .filter(|v| conds.iter().all(|c| c.eval(&v.record, &v.source)))
            .take(limit)
            .cloned()
            .collect())
    }
    fn query(&self, _: &Query, _: usize) -> anyhow::Result<Vec<ViewRecord>> {
        Ok(Vec::new())
    }
    fn instances(&self) -> Vec<String> {
        vec!["i1".into()]
    }
    fn time_range(&self) -> Option<(i64, i64)> {
        Some((1, 2))
    }
}

#[test]
fn exporters_annotators_markers_services_and_events() {
    let p = fake(&[]);
    let r = installed(&p);
    // Exporter: the plugin's output streams back.
    let e = r.exporter("fake.lines").unwrap().clone();
    assert_eq!((e.name(), e.extension()), ("Fake lines", "txt"));
    let recs: Vec<Record> = (0..70_000)
        .map(|i| {
            let mut r = Record::new(i);
            r.message = format!("m{i}");
            r
        })
        .collect();
    let mut rows = recs.iter().map(|r| ExportRow {
        record: std::borrow::Cow::Borrowed(r),
        source_name: "a.log",
        group: "g".into(),
    });
    let mut out = Vec::new();
    assert_eq!(e.export(&mut rows, &mut out).unwrap(), 70_000);
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("a.log|g|m0\n") && text.ends_with("a.log|g|m69999\n"));
    let mut empty = std::iter::empty();
    let mut out = Vec::new();
    assert_eq!(e.export(&mut empty, &mut out).unwrap(), 0);
    // Annotator: only the records it declared interest in are sent.
    let view = View(
        [(Level::Error, "bad"), (Level::Information, "fine")]
            .into_iter()
            .map(|(l, m)| {
                let mut r = Record::new(1);
                r.level = l;
                r.ts = Some(5);
                r.message = m.into();
                ViewRecord {
                    record: r,
                    source: "a.log".into(),
                    instance: "i1".into(),
                }
            })
            .collect(),
    );
    r.run_annotators(&view, None);
    let list = r.annotations(&Default::default());
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(
        (list[0].label.as_str(), list[0].kind.as_str(), list[0].owner.as_str()),
        ("bad", "test.fake/fake-mark", ID)
    );
    // Marker kinds are the plugin's.
    let compiled = r.compile_profile(Profile::default_profile()).unwrap();
    assert!(compiled.markers.iter().any(|m| m.kind == "test.fake/fake-mark"));
    // Services: opaque bytes; a panic in the service is a crash.
    let svc = r.service::<dyn ByteService>("fake.echo", "^1").unwrap();
    assert_eq!(svc.try_call(|s| s.call("echo", b"hi")).unwrap(), b"hi");
    assert!(svc
        .try_call(|s| s.call("nope", b""))
        .err()
        .unwrap()
        .to_string()
        .contains("no method"));
    // Events reach the plugin.
    r.events().publish(imt_core::events::RecordsAppended {
        count: 3,
        sources: vec![1],
    });
    r.events().publish(imt_core::events::ProfileChanged { name: "p".into() });
    r.events().publish(imt_core::events::WorkspaceCleared);
    let started = Instant::now();
    loop {
        let n = svc.try_call(|s| s.call("events", b"")).unwrap();
        if n == b"1103" {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "events not delivered: {:?}",
            String::from_utf8_lossy(&n)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    r.shutdown();
}

#[test]
fn settings_are_described_validated_and_applied_remotely() {
    let p = fake(&[]);
    let r = installed(&p);
    let s = &r.plugin_settings()[0];
    let section = s.configurable.settings_section();
    assert_eq!((section.id.as_str(), section.fields[0].key.as_str()), (ID, "prefix"));
    let valid = imt_core::settings::validate(&section, &serde_json::json!({"prefix": "P:"})).unwrap();
    s.configurable.apply_settings(&valid).unwrap();
    let out = import(&r, "a.fake", "INF|x\n").unwrap();
    assert_eq!(out.records[0].message, "P:x");
    let bad = imt_core::settings::validate(&section, &serde_json::json!({"prefix": "explode"})).unwrap();
    assert!(s.configurable.apply_settings(&bad).err().unwrap().to_string().contains("refused"));
    assert_eq!(r.health().get(ID).failures, 0);
    // A crash while applying counts against the plugin like any call.
    let crash = imt_core::settings::validate(&section, &serde_json::json!({"prefix": "abort"})).unwrap();
    assert!(s.configurable.apply_settings(&crash).err().unwrap().to_string().contains("crashed"));
    assert_eq!(r.health().get(ID).failures, 1);
}

#[test]
fn a_crashing_process_is_restarted_then_quarantined() {
    let p = fake(&[]);
    let r = installed(&p);
    let before = pid(&p);
    let e = import(&r, "a.fake", "CRASH\n").err().unwrap().to_string();
    assert!(e.contains("crashed") && e.contains(ID), "{e}");
    assert_eq!(r.health().get(ID).failures, 1);
    // The next call starts a fresh process.
    assert_eq!(import(&r, "a.fake", "INF|ok\n").unwrap().records.len(), 1);
    assert_ne!(pid(&p), before);
    assert!(p.process_status().restarts >= 1);
    // Panics inside the process count like in-process panics (and restart it too).
    let e = import(&r, "a.fake", "PANIC\n").err().unwrap().to_string();
    assert!(e.contains("the importer panicked"), "{e}");
    let e = import(&r, "a.fake", "OOM\n").err().unwrap().to_string();
    assert!(e.contains("crashed"), "{e}");
    // Three failures: quarantined like an in-process plugin.
    let info = r.plugins().into_iter().next().unwrap();
    assert_eq!(info.status, PluginStatus::Quarantined, "{info:?}");
    let e = import(&r, "a.fake", "INF|x\n").err().unwrap().to_string();
    assert!(e.contains("disabled after repeated crashes"), "{e}");
    // Resetting lifts it.
    r.reset_plugin(ID);
    p.reset();
    assert_eq!(import(&r, "a.fake", "INF|x\n").unwrap().records.len(), 1);
}

#[test]
fn a_hung_call_is_stopped_at_its_deadline_and_the_process_replaced() {
    let p = fake(&[]);
    let r = installed(&p);
    r.set_budgets(Budgets {
        import: Some(Duration::from_millis(800)),
        transform: Some(Duration::from_millis(800)),
        ..Budgets::default()
    });
    let before = pid(&p);
    let started = Instant::now();
    let e = import(&r, "a.fake", "HANG\n").err().unwrap().to_string();
    assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    assert!(e.contains("did not finish within") || e.contains("not respond"), "{e}");
    assert!(r.health().get(ID).failures >= 1);
    // The stuck process was killed; a new one answers.
    let started = Instant::now();
    while r.health().get(ID).hung && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(import(&r, "a.fake", "INF|x\n").unwrap().records.len(), 1);
    assert_ne!(pid(&p), before);
    // A hanging transform keeps the records unchanged.
    let t = r.hosted_transforms().into_iter().next().unwrap();
    let mut recs = vec![Record::new(1)];
    recs[0].message = "hang".into();
    assert_eq!(t.apply_batch(&mut recs, &TransformCtx::new("s")), [Verdict::Keep]);
    assert_eq!(recs[0].message, "hang");
}

#[test]
fn a_transform_crash_keeps_records_and_counts() {
    let p = fake(&[]);
    let r = installed(&p);
    let t = r.hosted_transforms().into_iter().next().unwrap();
    for (msg, what) in [("abort", "crashed"), ("panic", "the transform panicked")] {
        let mut recs = vec![Record::new(1), Record::new(2)];
        recs[0].message = msg.into();
        recs[1].message = "fine".into();
        let v = t.apply_batch(&mut recs, &TransformCtx::new("s"));
        assert_eq!(v, [Verdict::Keep, Verdict::Keep]);
        assert_eq!(recs[1].message, "fine", "unchanged");
        let last = r.health().get(ID).last_error.unwrap_or_default();
        assert!(last.contains(what), "{last}");
    }
    assert_eq!(r.health().get(ID).failures, 2);
    // A sniff that panics is "no" and counts.
    let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
    let ctx = ImportCtx {
        profile: &profile,
        path: Path::new("a.log"),
        file_date: None,
    };
    assert_eq!(r.hosted_importers().next().unwrap().sniff(&ctx, "SNIFFPANIC"), 0);
    assert_eq!(r.health().get(ID).failures, 3);
}

#[test]
fn permissions_must_be_granted_and_paths_are_only_sent_when_they_are() {
    // No permission requested: the plugin sees the file name only.
    let p = fake(&[]);
    let r = installed(&p);
    let out = import(&r, "/secret/dir/app.fake", "PATH\n").unwrap();
    assert_eq!(out.records[0].message, "app.fake");
    // Requested but not granted: the plugin is inactive with the reason, and its process stopped.
    let p = fake(&["--want-files", "--want-network"]);
    assert_eq!(p.missing_permissions().len(), 2);
    let mut reg = PluginRegistry::empty();
    let e = reg.install_shared(p.clone()).err().unwrap().to_string();
    assert!(
        e.contains("needs permission") && e.contains("Read the files you open") && e.contains("example.com"),
        "{e}"
    );
    assert_eq!(p.process_status().pid, None);
    // Granted: the full path is sent.
    let mut s = spec(&["--want-files"]);
    s.grants = vec![PermissionRequest {
        permission: Permission::ReadOpenedFiles,
        reason: String::new(),
    }];
    let p = RemotePlugin::start(s).unwrap();
    assert!(p.missing_permissions().is_empty());
    let r = installed(&p);
    let out = import(&r, "/secret/dir/app.fake", "PATH\n").unwrap();
    assert_eq!(out.records[0].message, "/secret/dir/app.fake");
}

#[test]
fn processes_that_cannot_start_are_errors() {
    let e = RemotePlugin::start(spec(&["--fail-start"])).err().unwrap().to_string();
    assert!(e.contains("exited"), "{e}");
    let mut s = spec(&[]);
    s.expected_id = Some("other.id".into());
    let e = RemotePlugin::start(s).err().unwrap().to_string();
    assert!(e.contains("expected plugin \"other.id\""), "{e}");
    let mut s = spec(&["--sleep-start=3000"]);
    s.connect_window = Duration::from_millis(200);
    assert!(RemotePlugin::start(s).is_err());
    let e = RemotePlugin::start(PluginSpec::new("/no/such/plugin", "nothing"))
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("cannot start nothing"), "{e}");
}

#[test]
fn a_quarantined_process_is_not_restarted_and_counts_in_the_registry() {
    let mut s = spec(&[]);
    s.supervisor.max_crashes = 2;
    let p = RemotePlugin::start(s).unwrap();
    let r = installed(&p);
    let _ = import(&r, "a.fake", "CRASH\n");
    let _ = import(&r, "a.fake", "CRASH\n");
    assert_eq!(p.process_status().state, imt_ipc::supervisor::ChildState::Quarantined);
    let e = import(&r, "a.fake", "INF|x\n").err().unwrap().to_string();
    assert!(e.contains("failed 2 times") || e.contains("disabled after repeated crashes"), "{e}");
    p.shutdown();
}

#[test]
fn the_plugin_process_has_a_child_log_with_its_version() {
    let p = fake(&[]);
    assert!(pid(&p).is_some());
    let log = imt_ipc::child_log::find("plugin-the-fake-plugin").expect("a log for the plugin process");
    let s = log.status();
    assert!(s.starts >= 1, "{s:?}");
    assert_eq!(s.version.as_deref(), Some("1.2.3"));
    assert_eq!(s.kind, imt_ipc::child_log::ChildKind::Plugin);
}
