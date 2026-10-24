//! Crashing plugins must never take Debrief down: panics become errors attributed
//! to the plugin, repeat offenders are quarantined, and disabled plugins are
//! not registered at all.

use crate::common::quiet;
use anyhow::Result;
use debrief_core::api::{ExportRequest, WorkspaceSummary};
use debrief_core::export::{EventSink, ExportRow, Exporter};
use debrief_core::importers::{BinaryImporter, ImportCtx, Imported, Importer};
use debrief_core::model::Record;
use debrief_core::plugin::{Collector, PluginStatus, RecordTransform, TransformCtx, Verdict, WorkspaceHook};
use debrief_core::query::Query;
use debrief_core::{LogStore, Plugin, PluginRegistry, Profile, Workspace};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("debrief-robust-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct BadImporter;
impl Importer for BadImporter {
    fn id(&self) -> &'static str {
        "bad"
    }
    fn sniff(&self, _: &ImportCtx, head: &str) -> u8 {
        if head.starts_with("SNIFF-PANIC") {
            panic!("sniff exploded");
        }
        if head.starts_with("BAD") {
            100
        } else {
            0
        }
    }
    fn import(&self, _: &ImportCtx, _: &str) -> Result<Imported> {
        panic!("import exploded")
    }
}
struct BadBinary;
impl BinaryImporter for BadBinary {
    fn id(&self) -> &'static str {
        "bad-bin"
    }
    fn sniff(&self, _: &ImportCtx, head: &[u8]) -> u8 {
        if head.starts_with(b"\x00BIN") {
            100
        } else {
            0
        }
    }
    fn import(&self, _: &ImportCtx, _: &[u8]) -> Result<Imported> {
        panic!("binary exploded")
    }
}
struct BadTransform(AtomicUsize);
impl RecordTransform for BadTransform {
    fn id(&self) -> &str {
        "bad.transform"
    }
    fn apply(&self, r: &mut Record, _: &TransformCtx) -> Verdict {
        self.0.fetch_add(1, Ordering::SeqCst);
        if r.message.contains("2") {
            panic!("transform exploded");
        }
        r.fields.insert("Touched".into(), true.into());
        Verdict::Keep
    }
}
struct BadExporter;
impl Exporter for BadExporter {
    fn id(&self) -> &'static str {
        "bad-export"
    }
    fn name(&self) -> &'static str {
        "Bad"
    }
    fn extension(&self) -> &'static str {
        "txt"
    }
    fn export(&self, _: &mut dyn Iterator<Item = ExportRow<'_>>, _: &mut dyn std::io::Write) -> Result<usize> {
        panic!("export exploded")
    }
}
struct BadHook;
impl WorkspaceHook for BadHook {
    fn on_loaded(&self, _: &WorkspaceSummary) {
        panic!("hook exploded")
    }
    fn on_cleared(&self) {
        panic!("hook exploded")
    }
    fn on_profile_changed(&self, _: &Profile) {
        panic!("hook exploded")
    }
}
struct BadCollector;
impl Collector for BadCollector {
    fn id(&self) -> &str {
        "bad-collector"
    }
    fn name(&self) -> &str {
        "Bad"
    }
    fn run(&self, _: &serde_json::Value, _: &dyn EventSink, _: &AtomicBool) -> Result<()> {
        panic!("collector exploded")
    }
}

struct Chaos(Arc<BadTransform>);
impl Plugin for Chaos {
    fn id(&self) -> &'static str {
        "test.chaos"
    }
    fn name(&self) -> &'static str {
        "Chaos"
    }
    fn version(&self) -> &'static str {
        "6.6.6"
    }
    fn register(&self, r: &mut PluginRegistry) -> Result<()> {
        r.add_importer(Box::new(BadImporter))
            .add_binary_importer(Box::new(BadBinary))
            .add_hook(Arc::new(BadHook));
        r.add_transform(self.0.clone())?;
        r.add_exporter(Arc::new(BadExporter))?;
        r.add_collector(Arc::new(BadCollector))?;
        Ok(())
    }
}

struct PanicsOnRegister;
impl Plugin for PanicsOnRegister {
    fn id(&self) -> &'static str {
        "test.register-panic"
    }
    fn name(&self) -> &'static str {
        "x"
    }
    fn version(&self) -> &'static str {
        "1"
    }
    fn register(&self, _: &mut PluginRegistry) -> Result<()> {
        panic!("register exploded")
    }
}

fn chaos_quarantined(s: &WorkspaceSummary) -> bool {
    s.plugins
        .iter()
        .any(|p| p.id == "test.chaos" && p.status == PluginStatus::Quarantined)
}

#[test]
fn crashing_plugins_are_contained_and_quarantined() {
    quiet(|| {
        let d = Dir::new("chaos");
        std::fs::write(
            d.0.join("ok.log"),
            "2026-10-14 10:00:01 INFO line 1\n2026-10-14 10:00:02 INFO line 2\n",
        )
        .unwrap();
        std::fs::write(d.0.join("bad.txt"), "BAD format\n").unwrap();
        std::fs::write(d.0.join("sniff.txt"), "SNIFF-PANIC\n2026-10-14 10:00:03 INFO still text\n").unwrap();
        std::fs::write(d.0.join("bin.dat"), b"\x00BIN\x01\x02").unwrap();

        let transform = Arc::new(BadTransform(AtomicUsize::new(0)));
        let mut registry = PluginRegistry::new();
        registry.install(&Chaos(transform.clone())).unwrap();
        let err = registry.install(&PanicsOnRegister).unwrap_err().to_string();
        assert!(err.contains("register exploded"), "{err}");

        let profile = Profile::from_toml("[[pipeline]]\nstep = 'plugin'\nid = 'bad.transform'").unwrap();
        let mut ws = Workspace::with_registry(profile, registry).unwrap();
        ws.open(std::slice::from_ref(&d.0)).unwrap();

        let s = ws.summary();
        // The plain log still imported in full despite the crashing transform.
        let ok = s.sources.iter().find(|x| x.name == "ok.log").unwrap();
        assert_eq!(ok.records, 2);
        // (Whether the transform touched records first depends on parallel import order.)
        // A crashing sniff only removes that importer from the race.
        assert!(s.sources.iter().any(|x| x.name == "sniff.txt" && x.importer == "text"));
        let reason = |n: &str| {
            s.unrecognized
                .iter()
                .find(|u| u.name == n)
                .map(|u| u.reason.clone())
                .unwrap_or_default()
        };
        // Files import in parallel (order and timing vary with the machine's cores): the plugin may crash on bad.txt
        // (the reason names it), be quarantined between choosing its importer and importing (the reason says so), or be
        // quarantined before bad.txt is even sniffed, when its importer is skipped and nothing else recognises the file.
        let bad = reason("bad.txt");
        assert!(
            (bad.contains("test.chaos") && (bad.contains("import exploded") || bad.contains("disabled after repeated crashes")))
                || ((bad == "no importer recognises this format" || bad.starts_with("no enabled parser reads this file"))
                    && chaos_quarantined(&s)),
            "{bad}"
        );
        // Likewise, if the plugin was quarantined before bin.dat's turn, its crashing binary importer is skipped and
        // the file is simply an unrecognised binary file.
        assert!(
            reason("bin.dat").contains("binary exploded") || reason("bin.dat").contains("binary file"),
            "{}",
            reason("bin.dat")
        );

        // Sniff + import + binary + transform + hook crashes: well past the threshold.
        let chaos = s.plugins.iter().find(|p| p.id == "test.chaos").unwrap();
        assert_eq!(chaos.status, PluginStatus::Quarantined);
        assert!(chaos.failures >= 3 && chaos.last_error.is_some());
        // The health check points at the plugins panel.
        let findings = ws.overview(&debrief_core::api::QueryRequest { query: Query::default() }).unwrap();
        let disabled = findings.iter().find(|f| f.title == "Plugin disabled after crashes").unwrap();
        assert_eq!(disabled.severity, debrief_core::api::Severity::Warning);
        assert!(disabled.detail.starts_with("Chaos: plugin \"test.chaos\""), "{}", disabled.detail);
        assert_eq!(
            disabled.action,
            Some(debrief_core::api::FindingAction::OpenPanel { panel: "plugins".into() })
        );

        // Quarantined: not called any more; exporting and collecting fail cleanly.
        let calls = transform.0.load(Ordering::SeqCst);
        ws.reload().unwrap();
        assert_eq!(transform.0.load(Ordering::SeqCst), calls);
        let out = d.0.join("out.txt").to_string_lossy().into_owned();
        let e = ws
            .export(&ExportRequest {
                query: Query::default(),
                format: "bad-export".into(),
                path: out,
                descending: false,
                redact: false,
                overwrite: false,
            })
            .unwrap_err();
        assert!(e.to_string().contains("disabled after repeated crashes"), "{e}");

        // Reset lifts the quarantine; the next crash is reported, not fatal.
        let store: &dyn LogStore = &ws;
        store.reset_plugin("test.chaos");
        assert_eq!(
            ws.summary().plugins.iter().find(|p| p.id == "test.chaos").unwrap().status,
            PluginStatus::Active
        );
        let out = d.0.join("out2.txt").to_string_lossy().into_owned();
        let e = ws
            .export(&ExportRequest {
                query: Query::default(),
                format: "bad-export".into(),
                path: out,
                descending: false,
                redact: false,
                overwrite: false,
            })
            .unwrap_err();
        assert!(e.to_string().contains("export exploded"), "{e}");
        ws.clear();
    });
}

#[test]
fn collectors_and_hooks_crash_safely() {
    quiet(|| {
        let mut registry = PluginRegistry::new();
        registry.install(&Chaos(Arc::new(BadTransform(AtomicUsize::new(0))))).unwrap();
        let c = registry.collector("bad-collector").unwrap().clone();
        assert_eq!((c.id(), c.name()), ("bad-collector", "Bad"));
        let scratch = tempfile::Builder::new().prefix("debrief-robust-sink-").tempdir().unwrap();
        let sink = debrief_core::export::ClefFileSink::new(scratch.path().to_path_buf(), "x", 1024, 1).unwrap();
        let e = c.run(&serde_json::Value::Null, &sink, &AtomicBool::new(false)).unwrap_err();
        assert!(e.to_string().contains("collector exploded"));
        let mut ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
        ws.set_profile(Profile::default_profile()).unwrap();
        ws.clear();
        assert!(ws.summary().plugins[0].failures >= 3);
    });
}

#[test]
fn disabled_plugins_are_not_registered() {
    let mut registry = PluginRegistry::new();
    registry.disable(["test.chaos"]);
    registry.install(&Chaos(Arc::new(BadTransform(AtomicUsize::new(0))))).unwrap();
    let info = &registry.plugins()[0];
    assert_eq!((info.status, info.contributes.len()), (PluginStatus::Disabled, 0));
    assert!(registry.collector("bad-collector").is_none());
    // A profile that needs the disabled plugin's transform does not validate.
    let profile = Profile::from_toml("[[pipeline]]\nstep = 'plugin'\nid = 'bad.transform'").unwrap();
    assert!(Workspace::with_registry(profile, registry).is_err());
}
