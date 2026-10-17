//! The plugin API end to end: installing plugins, transforms referenced from
//! profiles, hooks, exporters and API-version checks.

use anyhow::Result;
use imt_core::api::{ExportRequest, WorkspaceSummary};
use imt_core::export::{ExportRow, Exporter};
use imt_core::model::Record;
use imt_core::plugin::{RecordTransform, TransformCtx, Verdict, WorkspaceHook};
use imt_core::query::Query;
use imt_core::{LogStore, Plugin, PluginRegistry, Profile, Workspace, API_VERSION};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// Drops debug events and tags the rest with the team owning the source.
struct Normaliser;
impl RecordTransform for Normaliser {
    fn id(&self) -> &str {
        "acme.normaliser"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        if r.level == imt_core::model::Level::Debug {
            return Verdict::Drop;
        }
        let team = if ctx.source_name.contains("legacy") { "billing" } else { "orders" };
        r.fields.insert("Team".into(), team.into());
        Verdict::Keep
    }
}

#[derive(Default)]
struct Recorder {
    loaded: AtomicUsize,
    cleared: AtomicUsize,
    profiles: Mutex<Vec<String>>,
}
impl WorkspaceHook for Recorder {
    fn on_loaded(&self, summary: &WorkspaceSummary) {
        self.loaded.fetch_add(summary.sources.len(), Ordering::SeqCst);
    }
    fn on_cleared(&self) {
        self.cleared.fetch_add(1, Ordering::SeqCst);
    }
    fn on_profile_changed(&self, profile: &Profile) {
        self.profiles.lock().unwrap().push(profile.name.clone());
    }
}

/// Plain text, one message per line.
struct Lines;
impl Exporter for Lines {
    fn id(&self) -> &'static str {
        "lines"
    }
    fn name(&self) -> &'static str {
        "Messages only"
    }
    fn extension(&self) -> &'static str {
        "txt"
    }
    fn export(&self, rows: &mut dyn Iterator<Item = ExportRow<'_>>, out: &mut dyn Write) -> Result<usize> {
        let mut n = 0;
        for row in rows {
            writeln!(out, "{} | {}", row.group, row.record.message)?;
            n += 1;
        }
        Ok(n)
    }
}

struct Acme {
    hook: Arc<Recorder>,
}
impl Plugin for Acme {
    fn id(&self) -> &'static str {
        "com.acme.tools"
    }
    fn name(&self) -> &'static str {
        "ACME tools"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn register(&self, r: &mut PluginRegistry) -> Result<()> {
        r.add_transform(Arc::new(Normaliser))?;
        r.add_hook(self.hook.clone());
        r.add_exporter(Arc::new(Lines))?;
        Ok(())
    }
}

fn profile() -> Profile {
    Profile::from_toml(
        "name = 'With plugin'\n[[pipeline]]\nstep = 'plugin'\nid = 'acme.normaliser'\n[[pipeline]]\nstep = 'drop'\nwhen = [{ field = 'Team', op = 'eq', value = 'billing' }, { field = '@level', op = 'eq', value = 'Information' }]",
    )
    .unwrap()
}

#[test]
fn plugin_contributions_flow_through_the_workspace() {
    let hook = Arc::new(Recorder::default());
    let mut registry = PluginRegistry::new();
    registry.install(&Acme { hook: hook.clone() }).unwrap();
    assert_eq!(
        registry.plugins()[0].contributes,
        vec!["transform:acme.normaliser", "hook", "exporter:lines"]
    );

    // A profile using the transform only compiles where the plugin is installed.
    assert!(Workspace::new(profile()).is_err());
    let mut ws = Workspace::with_registry(profile(), registry).unwrap();
    ws.open(&[samples().join("log4net"), samples().join("serilog-text")]).unwrap();
    assert_eq!(hook.loaded.load(Ordering::SeqCst), 2);

    let s = ws.summary();
    assert!(s.records_without_debug());
    assert!(s.dropped > 0);
    assert_eq!(s.pipeline.len(), 2);
    assert!(s.pipeline[0].input > s.pipeline[0].output, "debug lines dropped by the plugin");
    assert!(s.pipeline[1].input > s.pipeline[1].output, "billing info dropped by the drop step");
    assert_eq!(s.pipeline[1].input, s.pipeline[0].output);
    let billing = s.sources.iter().find(|x| x.name.contains("billing")).unwrap();
    assert_eq!(billing.dropped, billing.dropped.max(1));
    assert!(ws.records().iter().all(|r| r.fields.contains_key("Team")));
    assert!(s.exporters.iter().any(|e| e.id == "lines" && e.extension == "txt"));
    assert!(s.plugins.iter().any(|p| p.id == "com.acme.tools"));

    // Exporting with the plugin's exporter.
    let out = std::env::temp_dir().join(format!("imt-export-{}.txt", std::process::id()));
    let res = ws
        .export(&ExportRequest {
            query: Query::default(),
            format: "lines".into(),
            path: out.to_string_lossy().into_owned(),
            descending: true,
        })
        .unwrap();
    assert_eq!(res.events, ws.summary().total);
    let text = std::fs::read_to_string(&out).unwrap();
    assert_eq!(text.lines().count(), res.events);
    std::fs::remove_file(&out).unwrap();

    // Hooks see profile changes and clears; validation knows the plugin.
    let store: &mut dyn LogStore = &mut ws;
    store.validate_profile(profile()).unwrap();
    assert!(store
        .validate_profile(Profile::from_toml("[[pipeline]]\nstep='plugin'\nid='nope'").unwrap())
        .is_err());
    store.set_profile(profile()).unwrap();
    store.clear();
    assert_eq!(hook.profiles.lock().unwrap().as_slice(), ["With plugin"]);
    assert_eq!(hook.cleared.load(Ordering::SeqCst), 1);
}

trait NoDebug {
    fn records_without_debug(&self) -> bool;
}
impl NoDebug for WorkspaceSummary {
    fn records_without_debug(&self) -> bool {
        self.levels[&imt_core::model::Level::Debug] == 0
    }
}

#[test]
fn built_in_exporters_and_errors() {
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[samples().join("csv")]).unwrap();
    let dir = std::env::temp_dir();
    for (format, ext) in [("csv", "csv"), ("clef", "clef")] {
        let path = dir.join(format!("imt-export-{}.{ext}", std::process::id()));
        let q = Query {
            text: "WEB02".into(),
            ..Default::default()
        };
        let res = ws
            .export(&ExportRequest {
                query: q,
                format: format.into(),
                path: path.to_string_lossy().into_owned(),
                descending: false,
            })
            .unwrap();
        assert!(res.events > 0);
        assert_eq!(res.format, format);
        std::fs::remove_file(path).unwrap();
    }
    let bad_format = ExportRequest {
        query: Query::default(),
        format: "pdf".into(),
        path: "x".into(),
        descending: false,
    };
    assert!(ws.export(&bad_format).unwrap_err().to_string().contains("unknown export format"));
    let bad_path = ExportRequest {
        query: Query::default(),
        format: "csv".into(),
        path: "/no/such/dir/out.csv".into(),
        descending: false,
    };
    assert!(ws.export(&bad_path).unwrap_err().to_string().contains("cannot create"));
    let bad_query = ExportRequest {
        query: Query {
            text: "(".into(),
            regex: true,
            ..Default::default()
        },
        format: "csv".into(),
        path: "x".into(),
        descending: false,
    };
    assert!(ws.export(&bad_query).is_err());
    let summary = ws.summary();
    assert_eq!(summary.exporters.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["csv", "clef"]);
    assert!(summary.plugins.is_empty());
    assert!(ws.plugins().is_empty());
}

struct Future;
impl Plugin for Future {
    fn id(&self) -> &'static str {
        "future"
    }
    fn name(&self) -> &'static str {
        "Future"
    }
    fn version(&self) -> &'static str {
        "9.0.0"
    }
    fn api_version(&self) -> u32 {
        API_VERSION + 1
    }
    fn register(&self, _: &mut PluginRegistry) -> Result<()> {
        Ok(())
    }
}

struct Failing;
impl Plugin for Failing {
    fn id(&self) -> &'static str {
        "failing"
    }
    fn name(&self) -> &'static str {
        "Failing"
    }
    fn version(&self) -> &'static str {
        "0.1.0"
    }
    fn register(&self, r: &mut PluginRegistry) -> Result<()> {
        r.add_exporter(Arc::new(imt_core::export::CsvExporter))?;
        Ok(())
    }
}

#[test]
fn registry_rejects_conflicts_and_wrong_versions() {
    let mut r = PluginRegistry::default();
    assert!(r.install(&Future).unwrap_err().to_string().contains("plugin API"));
    let err = r.install(&Failing).unwrap_err().to_string();
    assert!(err.contains("failing") && err.contains("already registered"), "{err}");
    let hook = Arc::new(Recorder::default());
    r.install(&Acme { hook: hook.clone() }).unwrap();
    assert!(r.install(&Acme { hook }).unwrap_err().to_string().contains("already installed"));
    assert!(r.add_transform(Arc::new(Normaliser)).is_err());
    assert!(r.exporter("lines").is_some() && r.exporter("nope").is_none());

    let mut empty = PluginRegistry::empty();
    empty.add_exporter(Arc::new(Lines)).unwrap();
    let ws = Workspace::with_registry(Profile::default_profile(), empty).unwrap();
    assert_eq!(ws.summary().exporters.len(), 1);
}

/// A plugin that only adds an importer and a hook relying on the default methods.
struct Quiet;
impl WorkspaceHook for Quiet {}
struct Tsv;
impl imt_core::importers::Importer for Tsv {
    fn id(&self) -> &'static str {
        "tsv-lines"
    }
    fn sniff(&self, _: &imt_core::importers::ImportCtx, head: &str) -> u8 {
        if head.starts_with("TSV") {
            100
        } else {
            0
        }
    }
    fn import(&self, _: &imt_core::importers::ImportCtx, content: &str) -> Result<imt_core::importers::Imported> {
        let mut out = imt_core::importers::Imported::default();
        for (i, line) in content.lines().skip(1).enumerate() {
            let mut r = Record::new(i as u32 + 2);
            r.message = line.replace('\t', " ");
            out.records.push(r);
        }
        Ok(out)
    }
}
struct TsvPlugin;
impl Plugin for TsvPlugin {
    fn id(&self) -> &'static str {
        "tsv"
    }
    fn name(&self) -> &'static str {
        "TSV"
    }
    fn version(&self) -> &'static str {
        "0.0.1"
    }
    fn register(&self, r: &mut PluginRegistry) -> Result<()> {
        r.add_importer(Box::new(Tsv)).add_hook(Arc::new(Quiet));
        Ok(())
    }
}

#[test]
fn importer_plugins_and_default_hooks_through_the_store_trait() {
    let dir = std::env::temp_dir().join(format!("imt-tsv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.data"), "TSV\nx\ty\nlong message here\n").unwrap();
    let mut registry = PluginRegistry::new();
    registry.install(&TsvPlugin).unwrap();
    assert_eq!(registry.plugins()[0].contributes, vec!["importer:tsv-lines", "hook"]);
    let profile = Profile::from_toml(
        "[[pipeline]]\nstep='truncate'\nmax=100\n[[pipeline]]\nstep='lookup'\nfield='@message'\ntable={ 'x y' = { Kind = 'pair' } }",
    )
    .unwrap();
    let mut store: Box<dyn LogStore> = Box::new(Workspace::with_registry(profile.clone(), registry).unwrap());
    store.open(std::slice::from_ref(&dir)).unwrap();
    store.set_profile(profile).unwrap();
    store.clear();
    store.open(std::slice::from_ref(&dir)).unwrap();
    let s = store.summary();
    assert_eq!(s.sources[0].importer, "tsv-lines");
    assert_eq!(s.total, 2);
    let out = dir.join("out.csv");
    let res = store
        .export(&ExportRequest {
            query: Query::default(),
            format: "csv".into(),
            path: out.to_string_lossy().into_owned(),
            descending: false,
        })
        .unwrap();
    assert_eq!(res.events, 2);
    let csv = std::fs::read_to_string(&out).unwrap();
    assert!(csv.contains("\"{\"\"Kind\"\":\"\"pair\"\"}\""), "{csv}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn lookup_file_defaults_to_the_first_column() {
    let file = std::env::temp_dir().join(format!("imt-lookup-first-{}.csv", std::process::id()));
    std::fs::write(&file, "Machine,Owner\nWEB01,team-a\n").unwrap();
    let toml = format!("[[pipeline]]\nstep='lookup'\nfield='MachineName'\nfile='{}'", file.display());
    let mut ws = Workspace::new(Profile::from_toml(&toml).unwrap()).unwrap();
    ws.open(&[samples().join("serilog-clef/web01")]).unwrap();
    assert!(ws.records().iter().all(|r| r.fields["Owner"] == "team-a"));
    std::fs::remove_file(file).unwrap();
}

/// A made-up binary format: "BIN1" then one u8 count of fixed events.
struct Bin;
impl imt_core::importers::BinaryImporter for Bin {
    fn id(&self) -> &'static str {
        "bin1"
    }
    fn sniff(&self, _: &imt_core::importers::ImportCtx, head: &[u8]) -> u8 {
        if head.starts_with(b"BIN1") {
            100
        } else {
            0
        }
    }
    fn import(&self, _: &imt_core::importers::ImportCtx, bytes: &[u8]) -> Result<imt_core::importers::Imported> {
        let n = *bytes.get(4).ok_or_else(|| anyhow::anyhow!("truncated"))?;
        let mut out = imt_core::importers::Imported::default();
        for i in 0..n {
            let mut r = Record::new(u32::from(i) + 1);
            r.message = format!("binary event {i}");
            out.records.push(r);
        }
        Ok(out)
    }
}
struct BinPlugin;
impl Plugin for BinPlugin {
    fn id(&self) -> &'static str {
        "bin"
    }
    fn name(&self) -> &'static str {
        "Binary"
    }
    fn version(&self) -> &'static str {
        "0.0.1"
    }
    fn register(&self, r: &mut PluginRegistry) -> Result<()> {
        r.add_binary_importer(Box::new(Bin));
        Ok(())
    }
}

#[test]
fn binary_importers_get_raw_bytes_first() {
    let dir = std::env::temp_dir().join(format!("imt-bin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.dat"), [b'B', b'I', b'N', b'1', 3, 0, 0, 0, 0xff, 0xfe]).unwrap();
    std::fs::write(dir.join("b.dat"), *b"BIN1").unwrap();
    std::fs::write(dir.join("c.log"), "2026-10-14 10:00:00 INFO text still works\n").unwrap();
    let mut registry = PluginRegistry::new();
    registry.install(&BinPlugin).unwrap();
    assert_eq!(registry.plugins()[0].contributes, ["importer:bin1"]);
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    let s = ws.summary();
    assert_eq!(s.sources.iter().map(|x| x.importer.as_str()).collect::<Vec<_>>(), ["bin1", "text"]);
    assert_eq!(s.total, 4);
    assert!(s.unrecognized.iter().any(|u| u.name == "b.dat" && u.reason == "truncated"));

    // Forcing a format selects that binary importer even without its magic.
    let mut registry = PluginRegistry::new();
    registry.install(&BinPlugin).unwrap();
    let forced = Profile::from_toml("[parse]\nformat = 'bin1'").unwrap();
    let mut ws = Workspace::with_registry(forced, registry).unwrap();
    std::fs::write(dir.join("d.dat"), [0u8, 0, 0, 0, 2]).unwrap();
    ws.open(&[dir.join("d.dat")]).unwrap();
    assert_eq!(ws.summary().total, 2);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Emits one heartbeat per loop until stopped.
struct Heartbeat;
impl imt_core::plugin::Collector for Heartbeat {
    fn id(&self) -> &str {
        "heartbeat"
    }
    fn name(&self) -> &str {
        "Heartbeat"
    }
    fn run(&self, config: &serde_json::Value, sink: &dyn imt_core::export::EventSink, stop: &std::sync::atomic::AtomicBool) -> Result<()> {
        let max = config["max"].as_u64().unwrap_or(3);
        let mut n = 0;
        while !stop.load(Ordering::SeqCst) && n < max {
            let mut r = Record::new(0);
            r.ts = Some(1_791_972_000_000 + n as i64 * 1000);
            r.message = format!("beat {n}");
            sink.write(&[r])?;
            n += 1;
        }
        Ok(())
    }
}

#[test]
fn collectors_write_to_sinks_that_imt_can_open() {
    let mut r = PluginRegistry::new();
    r.add_collector(Arc::new(Heartbeat)).unwrap();
    assert!(r.add_collector(Arc::new(Heartbeat)).is_err());
    assert_eq!(r.collectors().len(), 1);
    let c = r.collector("heartbeat").unwrap().clone();
    assert_eq!(c.name(), "Heartbeat");
    assert!(r.collector("nope").is_none());

    let dir = std::env::temp_dir().join(format!("imt-collector-{}", std::process::id()));
    let sink = imt_core::export::ClefFileSink::new(&dir, "heartbeat", 1 << 20, 2).unwrap();
    let stop = std::sync::atomic::AtomicBool::new(false);
    c.run(&serde_json::json!({ "max": 4 }), &sink, &stop).unwrap();
    stop.store(true, Ordering::SeqCst);
    c.run(&serde_json::Value::Null, &sink, &stop).unwrap();

    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[sink.current()]).unwrap();
    let s = ws.summary();
    assert_eq!((s.total, s.sources[0].importer.as_str()), (4, "json"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn contributions_registered_directly_are_built_ins() {
    // Registered outside `install`: not attributed to any plugin, not guarded.
    let dir = std::env::temp_dir().join(format!("imt-direct-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.data"), "TSV\nx\ty\n").unwrap();
    std::fs::write(dir.join("b.dat"), [b'B', b'I', b'N', b'1', 2]).unwrap();
    let hook = Arc::new(Recorder::default());
    let mut registry = PluginRegistry::new();
    registry
        .add_importer(Box::new(Tsv))
        .add_binary_importer(Box::new(Bin))
        .add_hook(hook.clone());
    registry.add_transform(Arc::new(Normaliser)).unwrap();
    assert!(registry.plugins().is_empty());
    let health = registry.health();

    let profile = Profile::from_toml("[[pipeline]]\nstep = 'plugin'\nid = 'acme.normaliser'").unwrap();
    let mut ws = Workspace::with_registry(profile, registry).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    let s = ws.summary();
    let mut importers: Vec<&str> = s.sources.iter().map(|x| x.importer.as_str()).collect();
    importers.sort();
    assert_eq!(importers, ["bin1", "tsv-lines"]);
    assert_eq!(s.total, 3);
    assert!(ws.records().iter().all(|r| r.fields["Team"] == "orders"));
    assert_eq!(hook.loaded.load(Ordering::SeqCst), 2);
    // Nothing crashed, and built-ins have no health record.
    assert_eq!(health.get("com.acme.tools"), Default::default());
    std::fs::remove_dir_all(dir).unwrap();
}
