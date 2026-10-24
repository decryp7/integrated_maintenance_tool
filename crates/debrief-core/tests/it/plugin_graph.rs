//! Plugin system v2: manifests and dependency resolution, cascade on
//! quarantine/disable, services and extension points, the event bus wired
//! into the workspace, the watchdog, plugin marker kinds, `mark()` and
//! annotators.

use crate::common::quiet;
use anyhow::{anyhow, Result};
use debrief_core::api::Severity;
use debrief_core::events::{PluginStateChanged, ProfileChanged, RecordsAppended, WorkspaceCleared};
use debrief_core::guard::{Budgets, QUARANTINE_AFTER};
use debrief_core::importers::{ImportCtx, Imported, Importer};
use debrief_core::model::Record;
use debrief_core::plugin::annotate::{
    Annotation, AnnotationView, AnnotationsRequest, Annotator, RecordCursor, ViewFilter, MAX_ANNOTATIONS,
};
use debrief_core::plugin::markers::{MarkerRole, PluginMarkerKind};
use debrief_core::plugin::{ActivationCtx, Manifest, PluginStatus, RecordTransform, Service, TransformCtx, Verdict};
use debrief_core::query::Query;
use debrief_core::{Plugin, PluginRegistry, Profile, Workspace};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- helpers

type RegisterFn = Box<dyn Fn(&mut PluginRegistry) -> Result<()> + Send + Sync>;
type ActivateFn = Box<dyn Fn(&ActivationCtx) -> Result<()> + Send + Sync>;

/// A configurable test plugin.
#[derive(Default)]
struct P {
    id: &'static str,
    version: &'static str,
    requires: Vec<(&'static str, &'static str, bool)>,
    register: Option<RegisterFn>,
    activate: Option<ActivateFn>,
    log: Option<Arc<Mutex<Vec<String>>>>,
}

impl P {
    fn new(id: &'static str, version: &'static str) -> P {
        P {
            id,
            version,
            ..P::default()
        }
    }
    fn needs(mut self, id: &'static str, req: &'static str) -> P {
        self.requires.push((id, req, false));
        self
    }
    fn wants(mut self, id: &'static str, req: &'static str) -> P {
        self.requires.push((id, req, true));
        self
    }
    fn on_register(mut self, f: impl Fn(&mut PluginRegistry) -> Result<()> + Send + Sync + 'static) -> P {
        self.register = Some(Box::new(f));
        self
    }
    fn on_activate(mut self, f: impl Fn(&ActivationCtx) -> Result<()> + Send + Sync + 'static) -> P {
        self.activate = Some(Box::new(f));
        self
    }
    fn logging(mut self, log: &Arc<Mutex<Vec<String>>>) -> P {
        self.log = Some(log.clone());
        self
    }
}

impl Plugin for P {
    fn id(&self) -> &'static str {
        self.id
    }
    fn name(&self) -> &'static str {
        self.id
    }
    fn version(&self) -> &'static str {
        self.version
    }
    fn manifest(&self) -> Manifest {
        self.requires
            .iter()
            .fold(Manifest::new(self.id, self.version), |m, (id, req, optional)| {
                if *optional {
                    m.optionally(*id, req)
                } else {
                    m.requires(*id, req)
                }
            })
    }
    fn register(&self, r: &mut PluginRegistry) -> Result<()> {
        match &self.register {
            Some(f) => f(r),
            None => Ok(()),
        }
    }
    fn activate(&self, ctx: &ActivationCtx) -> Result<()> {
        if let Some(log) = &self.log {
            log.lock().unwrap().push(format!("activate {}", ctx.plugin_id()));
        }
        match &self.activate {
            Some(f) => f(ctx),
            None => Ok(()),
        }
    }
    fn deactivate(&self) {
        if let Some(log) = &self.log {
            log.lock().unwrap().push(format!("deactivate {}", self.id));
        }
    }
}

fn info(r: &PluginRegistry, id: &str) -> debrief_core::plugin::PluginInfo {
    r.plugins().into_iter().find(|p| p.id == id).unwrap()
}

fn status(r: &PluginRegistry, id: &str) -> (PluginStatus, Option<String>) {
    let i = info(r, id);
    (i.status, i.inactive_reason)
}

fn wait_until(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("debrief-graph-{tag}-{}", std::process::id()));
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

// ------------------------------------------------------- resolution

#[test]
fn missing_incompatible_and_optional_dependencies() {
    let mut r = PluginRegistry::empty();
    r.install(&P::new("net.decryptology.perfmon", "0.9.0")).unwrap();
    r.install(&P::new("ai", "1.0.0").needs("net.decryptology.perfmon", "^1.0")).unwrap();
    r.install(&P::new("lonely", "1.0.0").needs("nowhere", ">=2")).unwrap();
    r.install(
        &P::new("flexible", "1.0.0")
            .wants("nowhere", "^1")
            .wants("net.decryptology.perfmon", "^1"),
    )
    .unwrap();
    r.resolve();
    assert_eq!(
        status(&r, "ai"),
        (
            PluginStatus::Inactive,
            Some("needs net.decryptology.perfmon ^1.0 (installed 0.9.0)".into())
        )
    );
    assert_eq!(status(&r, "lonely").1.unwrap(), "needs nowhere >=2 (not installed)");
    let flexible = info(&r, "flexible");
    assert_eq!(flexible.status, PluginStatus::Active);
    assert_eq!(
        flexible.warnings,
        vec![
            "optional: needs nowhere ^1 (not installed)".to_string(),
            "optional: needs net.decryptology.perfmon ^1 (installed 0.9.0)".to_string()
        ]
    );
    let ai = info(&r, "ai");
    assert_eq!(ai.requires[0].req, "^1.0");
    assert!(!ai.requires[0].optional);
    assert_eq!(info(&r, "net.decryptology.perfmon").dependents, vec!["ai"]);
}

#[test]
fn cycles_are_rejected_with_the_cycle_named() {
    let mut r = PluginRegistry::empty();
    r.install(&P::new("a", "1.0.0").needs("b", "*")).unwrap();
    r.install(&P::new("b", "1.0.0").needs("a", "*")).unwrap();
    r.install(&P::new("c", "1.0.0").needs("a", "*")).unwrap();
    r.resolve();
    assert_eq!(status(&r, "a").1.unwrap(), "dependency cycle: a → b → a");
    assert_eq!(status(&r, "b").1.unwrap(), "dependency cycle: b → a → b");
    assert!(status(&r, "c")
        .1
        .unwrap()
        .starts_with("needs a which is inactive (dependency cycle"));
}

#[test]
fn bad_manifests_and_ids_are_rejected_at_install() {
    struct Mismatch;
    impl Plugin for Mismatch {
        fn id(&self) -> &'static str {
            "x"
        }
        fn name(&self) -> &'static str {
            "x"
        }
        fn version(&self) -> &'static str {
            "1"
        }
        fn manifest(&self) -> Manifest {
            Manifest::new("y", "1")
        }
        fn register(&self, _: &mut PluginRegistry) -> Result<()> {
            Ok(())
        }
    }
    let mut r = PluginRegistry::empty();
    assert!(r.install(&Mismatch).unwrap_err().to_string().contains("different id \"y\""));
    let e = r.install(&P::new("v", "one.two")).unwrap_err().to_string();
    assert!(e.contains("invalid manifest") && e.contains("one.two"), "{e}");
    let e = r.install(&P::new("w", "1").needs("z", "~~")).unwrap_err().to_string();
    assert!(e.contains("version requirement \"~~\""), "{e}");
    assert!(r.plugins().is_empty());
}

#[test]
fn activation_runs_in_dependency_order_and_deactivation_in_reverse() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut r = PluginRegistry::empty();
    // Installed dependents first: order comes from the graph, not install order.
    r.install_shared(Arc::new(P::new("top", "1.0.0").needs("mid", "^1").logging(&log)))
        .unwrap();
    r.install_shared(Arc::new(P::new("mid", "1.0.0").needs("base", "^1").logging(&log)))
        .unwrap();
    r.install_shared(Arc::new(P::new("base", "1.0.0").logging(&log))).unwrap();
    assert!(log.lock().unwrap().is_empty(), "nothing activates before resolve");
    r.resolve();
    r.resolve(); // idempotent
    assert_eq!(*log.lock().unwrap(), ["activate base", "activate mid", "activate top"]);
    log.lock().unwrap().clear();
    r.shutdown();
    assert_eq!(*log.lock().unwrap(), ["deactivate top", "deactivate mid", "deactivate base"]);
}

#[test]
fn a_failed_activation_leaves_the_plugin_and_its_dependents_inactive_until_reset() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let a = attempts.clone();
    let mut r = PluginRegistry::empty();
    r.install_shared(Arc::new(P::new("flaky", "1.0.0").on_activate(move |_| {
        if a.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(anyhow!("no license"))
        } else {
            Ok(())
        }
    })))
    .unwrap();
    r.install_shared(Arc::new(P::new("user", "1.0.0").needs("flaky", "*"))).unwrap();
    r.resolve();
    assert_eq!(status(&r, "flaky").1.unwrap(), "activation failed: no license");
    assert_eq!(status(&r, "user").0, PluginStatus::Inactive);
    r.reset_plugin("flaky");
    assert_eq!(status(&r, "flaky").0, PluginStatus::Active);
    assert_eq!(status(&r, "user").0, PluginStatus::Active);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[test]
fn quarantine_and_disable_cascade_to_dependents_and_reset_reactivates_them() {
    quiet(|| {
        let calls = Arc::new(AtomicUsize::new(0));
        let log = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::<(String, PluginStatus)>::new()));
        let mut r = PluginRegistry::empty();
        r.install_shared(Arc::new(P::new("provider", "1.0.0").logging(&log).on_register(|r| {
            r.add_transform(Arc::new(Exploding("exploding")))?;
            Ok(())
        })))
        .unwrap();
        let c = calls.clone();
        r.install_shared(Arc::new(
            P::new("consumer", "1.0.0")
                .needs("provider", "^1")
                .logging(&log)
                .on_register(move |r| {
                    r.add_transform(Arc::new(Counting("counting", c.clone())))?;
                    Ok(())
                }),
        ))
        .unwrap();
        r.install_shared(Arc::new(P::new("leaf", "1.0.0").needs("consumer", "*").logging(&log)))
            .unwrap();
        let e = events.clone();
        r.events()
            .subscribe::<PluginStateChanged>("test", move |c| e.lock().unwrap().push((c.id.clone(), c.status)))
            .unwrap();
        let profile =
            Profile::from_toml("[[pipeline]]\nstep = 'plugin'\nid = 'exploding'\n[[pipeline]]\nstep = 'plugin'\nid = 'counting'").unwrap();
        let compiled = r.compile_profile(profile).unwrap();
        r.resolve();
        log.lock().unwrap().clear();

        // Both run while healthy.
        compiled.ingest(vec![Record::new(0)], "f");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // The provider's transform crashes until it is quarantined (the
        // pipeline runs step by step, so the dependent step is already refused).
        let records: Vec<Record> = (0..QUARANTINE_AFTER)
            .map(|i| {
                let mut r = Record::new(i);
                r.message = "boom".into();
                r
            })
            .collect();
        compiled.ingest(records, "f");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(status(&r, "provider").0, PluginStatus::Quarantined);
        assert_eq!(
            status(&r, "consumer"),
            (PluginStatus::Inactive, Some("needs provider which is quarantined".into()))
        );
        assert_eq!(
            status(&r, "leaf").1.unwrap(),
            "needs consumer which is inactive (needs provider which is quarantined)"
        );
        // The dependents' contributions are refused now (the record passes unchanged).
        compiled.ingest(vec![Record::new(9)], "f");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Dependents first; a quarantined plugin's own hook is not called (it crashed).
        assert_eq!(*log.lock().unwrap(), ["deactivate leaf", "deactivate consumer"]);
        wait_until("state events", || events.lock().unwrap().len() >= 3);
        assert!(events.lock().unwrap().contains(&("consumer".to_string(), PluginStatus::Inactive)));

        // Reset: the chain comes back, in dependency order.
        log.lock().unwrap().clear();
        r.reset_plugin("provider");
        assert_eq!(status(&r, "leaf").0, PluginStatus::Active);
        assert_eq!(*log.lock().unwrap(), ["activate provider", "activate consumer", "activate leaf"]);
        compiled.ingest(vec![Record::new(10)], "f");
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // Disabling at run time cascades the same way; enabling restores it.
        assert!(!r.set_enabled("provider", false).unwrap());
        assert_eq!(status(&r, "provider").0, PluginStatus::Disabled);
        assert_eq!(status(&r, "consumer").1.unwrap(), "needs provider which is disabled");
        assert!(!r.set_enabled("provider", true).unwrap());
        assert_eq!(status(&r, "consumer").0, PluginStatus::Active);
        assert!(r.set_enabled("nope", true).is_err());
    });
}

#[test]
fn plugins_disabled_at_install_need_a_restart_and_block_dependents() {
    let mut r = PluginRegistry::empty();
    r.disable(["base"]);
    r.install(&P::new("base", "1.0.0")).unwrap();
    r.install(&P::new("dep", "1.0.0").needs("base", "*")).unwrap();
    assert_eq!(status(&r, "dep").1.unwrap(), "needs base which is disabled");
    assert!(r.set_enabled("base", true).unwrap(), "restart needed");
    assert!(!r.set_enabled("base", false).unwrap());
}

#[test]
fn a_plugin_that_fails_to_register_is_listed_inactive() {
    let mut r = PluginRegistry::empty();
    let e = r
        .install(&P::new("broken", "1.0.0").on_register(|_| Err(anyhow!("missing file"))))
        .unwrap_err();
    assert!(e.to_string().contains("missing file"));
    r.install(&P::new("dep", "1.0.0").needs("broken", "*")).unwrap();
    assert_eq!(status(&r, "broken").1.unwrap(), "registration failed: missing file");
    assert!(status(&r, "dep")
        .1
        .unwrap()
        .contains("needs broken which is inactive (registration failed"));
    assert!(r
        .install(&P::new("broken", "1.0.0"))
        .unwrap_err()
        .to_string()
        .contains("already installed"));
}

struct Exploding(&'static str);
impl RecordTransform for Exploding {
    fn id(&self) -> &str {
        self.0
    }
    fn apply(&self, r: &mut Record, _: &TransformCtx) -> Verdict {
        if r.message == "boom" {
            panic!("transform exploded")
        }
        Verdict::Keep
    }
}

struct Counting(&'static str, Arc<AtomicUsize>);
impl RecordTransform for Counting {
    fn id(&self) -> &str {
        self.0
    }
    fn apply(&self, _: &mut Record, _: &TransformCtx) -> Verdict {
        self.1.fetch_add(1, Ordering::SeqCst);
        Verdict::Keep
    }
}

// --------------------------------------------------------- services

trait Clock: Send + Sync {
    fn now(&self) -> i64;
}
struct Fixed(i64);
impl Clock for Fixed {
    fn now(&self) -> i64 {
        if self.0 < 0 {
            panic!("clock exploded");
        }
        self.0
    }
}

#[test]
fn services_are_looked_up_by_type_and_version_in_activate() {
    let got: Arc<Mutex<Option<Service<dyn Clock>>>> = Arc::default();
    let errors = Arc::new(Mutex::new(Vec::new()));
    let mut r = PluginRegistry::empty();
    r.install_shared(Arc::new(P::new("clock", "1.0.0").on_register(|r| {
        r.provide::<dyn Clock>("test.clock", "1.4.0", Arc::new(Fixed(42)))?;
        Ok(())
    })))
    .unwrap();
    let (g, e) = (got.clone(), errors.clone());
    r.install_shared(Arc::new(P::new("user", "1.0.0").needs("clock", "^1").on_activate(move |ctx| {
        assert!(ctx.is_available("clock") && !ctx.is_available("nope"));
        assert!(ctx.contributions::<dyn Clock>("none").is_empty());
        let _ = ctx.events();
        e.lock()
            .unwrap()
            .push(ctx.service::<dyn Clock>("test.clock", "^2").unwrap_err().to_string());
        *g.lock().unwrap() = Some(ctx.service::<dyn Clock>("test.clock", "^1.2")?);
        Ok(())
    })))
    .unwrap();
    r.resolve();
    let clock = got.lock().unwrap().clone().unwrap();
    assert_eq!(clock.call(|c| c.now()).unwrap(), 42);
    assert_eq!((clock.provider(), clock.version().to_string().as_str()), ("clock", "1.4.0"));
    assert_eq!(
        errors.lock().unwrap()[0],
        "service \"test.clock\" ^2 is not available: plugin \"clock\" provides version 1.4.0"
    );
    let provides = info(&r, "clock").provides;
    assert_eq!((provides[0].id.as_str(), provides[0].version.as_str()), ("test.clock", "1.4.0"));
    assert!(info(&r, "clock").contributes.contains(&"service:test.clock".to_string()));
    assert!(r
        .service::<dyn Clock>("test.clock", "nonsense")
        .unwrap_err()
        .to_string()
        .contains("not valid"));
}

#[test]
fn a_manifest_declared_service_must_be_provided_in_that_version() {
    struct Declared;
    impl Plugin for Declared {
        fn id(&self) -> &'static str {
            "declared"
        }
        fn name(&self) -> &'static str {
            "d"
        }
        fn version(&self) -> &'static str {
            "1.0.0"
        }
        fn manifest(&self) -> Manifest {
            Manifest::new("declared", "1.0.0").provides("test.clock", "2.0.0")
        }
        fn register(&self, r: &mut PluginRegistry) -> Result<()> {
            r.provide::<dyn Clock>("test.clock", "1.0.0", Arc::new(Fixed(1)))?;
            Ok(())
        }
    }
    let mut r = PluginRegistry::empty();
    let e = r.install(&Declared).unwrap_err().to_string();
    assert!(e.contains("provided as 1.0.0 but the manifest declares 2.0.0"), "{e}");
    let e = r
        .provide::<dyn Clock>("host.clock", "x", Arc::new(Fixed(1)))
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("semver"));
    r.provide::<dyn Clock>("host.clock", "1.0.0", Arc::new(Fixed(7))).unwrap();
    assert_eq!(r.service::<dyn Clock>("host.clock", "*").unwrap().call(|c| c.now()).unwrap(), 7);
}

#[test]
fn a_panicking_provider_becomes_an_error_and_is_quarantined() {
    quiet(|| {
        let mut r = PluginRegistry::empty();
        r.install(&P::new("bad", "1.0.0").on_register(|r| {
            r.provide::<dyn Clock>("bad.clock", "1.0.0", Arc::new(Fixed(-1)))?;
            Ok(())
        }))
        .unwrap();
        r.install(&P::new("dep", "1.0.0").needs("bad", "*")).unwrap();
        r.resolve();
        let s = r.service::<dyn Clock>("bad.clock", "^1").unwrap();
        for _ in 0..QUARANTINE_AFTER {
            assert!(s.call(|c| c.now()).is_err());
        }
        assert_eq!(
            s.call(|c| c.now()).unwrap_err().to_string(),
            "service \"bad.clock\": provider \"bad\" is quarantined"
        );
        assert_eq!(status(&r, "bad").0, PluginStatus::Quarantined);
        assert_eq!(status(&r, "dep").0, PluginStatus::Inactive);
        let e = r.service::<dyn Clock>("bad.clock", "^1").unwrap_err().to_string();
        assert!(e.contains("is not available") && e.contains("repeated crashes"), "{e}");
        let health = r.health().get("bad");
        assert!(health.last_error.unwrap().contains("clock exploded"));
    });
}

// ------------------------------------------------- extension points

trait Resolver: Send + Sync {
    fn resolve(&self, addr: u64) -> String;
}
struct Named(&'static str);
impl Resolver for Named {
    fn resolve(&self, addr: u64) -> String {
        format!("{}@{addr:x}", self.0)
    }
}

#[test]
fn extension_points_collect_contributions_in_any_order() {
    let mut r = PluginRegistry::empty();
    // Contributions before the point is declared.
    r.install(&P::new("pdb", "1.0.0").on_register(|r| {
        r.contribute::<dyn Resolver>("profiler.symbols", Arc::new(Named("pdb")));
        Ok(())
    }))
    .unwrap();
    r.install(&P::new("profiler", "1.0.0").on_register(|r| {
        r.declare_extension_point::<dyn Resolver>("profiler.symbols")?;
        Ok(())
    }))
    .unwrap();
    r.install(&P::new("elf", "1.0.0").on_register(|r| {
        r.contribute::<dyn Resolver>("profiler.symbols", Arc::new(Named("elf")));
        r.contribute::<dyn Resolver>("nobody.declares.this", Arc::new(Named("x")));
        Ok(())
    }))
    .unwrap();
    r.install(&P::new("twice", "1.0.0").on_register(|r| {
        r.declare_extension_point::<dyn Resolver>("profiler.symbols")?;
        Ok(())
    }))
    .map(|_| ())
    .unwrap_err();
    r.resolve();
    let all = r.contributions::<dyn Resolver>("profiler.symbols");
    let names: Vec<String> = all.iter().map(|c| c.call(|x| x.resolve(16)).unwrap()).collect();
    assert_eq!(names, ["pdb@10", "elf@10"]);
    assert_eq!(all[1].provider(), "elf");
    // Undeclared: a warning and an inactive contribution, never a crash.
    let elf = info(&r, "elf");
    assert_eq!(elf.status, PluginStatus::Active);
    assert!(elf.warnings[0].contains("\"nobody.declares.this\" is inactive: no installed plugin declares it"));
    assert!(r.contributions::<dyn Resolver>("nobody.declares.this").is_empty());
    assert!(info(&r, "profiler")
        .contributes
        .contains(&"extension-point:profiler.symbols".to_string()));
    // A disabled contributor drops out.
    r.set_enabled("pdb", false).unwrap();
    assert_eq!(r.contributions::<dyn Resolver>("profiler.symbols").len(), 1);
}

// --------------------------------------------------------- events

#[test]
fn the_workspace_publishes_its_events() {
    let d = Dir::new("events");
    std::fs::write(d.0.join("a.log"), "2026-10-14 10:00:01 INFO one\n2026-10-14 10:00:02 INFO two\n").unwrap();
    let ws_registry = PluginRegistry::new();
    let bus = ws_registry.events();
    let appended = Arc::new(Mutex::new(Vec::new()));
    let (a, profiles, cleared) = (appended.clone(), Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicUsize::new(0)));
    bus.subscribe::<RecordsAppended>("t", move |e| a.lock().unwrap().push(e.count))
        .unwrap();
    let p = profiles.clone();
    bus.subscribe::<ProfileChanged>("t", move |e| p.lock().unwrap().push(e.name.clone()))
        .unwrap();
    let c = cleared.clone();
    bus.subscribe::<WorkspaceCleared>("t", move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    let mut ws = Workspace::with_registry(Profile::default_profile(), ws_registry).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    wait_until("the import event", || appended.lock().unwrap().as_slice() == [2]);
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(d.0.join("a.log")).unwrap();
    f.write_all(b"2026-10-14 10:00:03 INFO three\n").unwrap();
    drop(f);
    ws.refresh_tail(&[d.0.join("a.log")]).unwrap();
    wait_until("the append event", || appended.lock().unwrap().as_slice() == [2, 1]);
    ws.set_profile(Profile::default_profile()).unwrap();
    wait_until("the profile event", || profiles.lock().unwrap().len() == 1);
    ws.clear();
    wait_until("the clear event", || cleared.load(Ordering::SeqCst) == 1);
    assert!(Arc::ptr_eq(&ws.events(), &bus));
    assert!(ws.registry().events().metrics().iter().all(|m| m.failures == 0));
}

// --------------------------------------------------------- watchdog

struct Sleepy(Duration);
impl Importer for Sleepy {
    fn id(&self) -> &'static str {
        "sleepy"
    }
    fn sniff(&self, _: &ImportCtx, head: &str) -> u8 {
        if head.starts_with("SLEEPY") {
            100
        } else {
            0
        }
    }
    fn import(&self, _: &ImportCtx, _: &str) -> Result<Imported> {
        std::thread::sleep(self.0);
        let mut r = Record::new(1);
        r.message = "late".into();
        Ok(Imported {
            records: vec![r],
            skipped: 0,
            ..Default::default()
        })
    }
}

#[test]
fn the_watchdog_abandons_a_sleeping_import_and_fails_fast_afterwards() {
    let d = Dir::new("sleepy");
    std::fs::write(d.0.join("slow.txt"), "SLEEPY\n").unwrap();
    let mut r = PluginRegistry::new();
    r.install(&P::new("t.sleepy", "1.0.0").on_register(|r| {
        // Far longer than the 100 ms budget, so "abandoned, not waited for" holds even on a loaded machine.
        r.add_importer(Box::new(Sleepy(Duration::from_millis(3000))));
        Ok(())
    }))
    .unwrap();
    r.set_budgets(Budgets {
        import: Some(Duration::from_millis(100)),
        ..Budgets::default()
    });
    let health = r.health();
    let mut ws = Workspace::with_registry(Profile::default_profile(), r).unwrap();
    let started = Instant::now();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(2500),
        "the import was abandoned: {:?}",
        started.elapsed()
    );
    let s = ws.summary();
    let reason = &s.unrecognized[0].reason;
    assert!(reason.contains("did not finish within 100 ms; the call was abandoned"), "{reason}");
    let info = s.plugins.iter().find(|p| p.id == "t.sleepy").unwrap();
    assert!(info.hung && info.failures == 1, "{info:?}");
    // While the abandoned call is still running, calls fail fast.
    let ctx_profile = debrief_core::profile::CompiledProfile::compile(Profile::default_profile()).unwrap();
    let guard = debrief_core::guard::Guard::new("t.sleepy", health.clone());
    let e = guard.call(debrief_core::guard::CallKind::Service, "x", || Ok(())).unwrap_err();
    assert!(e.to_string().contains("is not responding"), "{e}");
    drop(ctx_profile);
    // Once it finishes (result discarded), the plugin is callable again.
    wait_until("the abandoned call to end", || !health.get("t.sleepy").hung);
    guard.call(debrief_core::guard::CallKind::Service, "x", || Ok(())).unwrap();
}

#[test]
fn the_watchdog_marks_a_sleeping_service_hung_on_the_calling_thread() {
    struct Slow;
    impl Clock for Slow {
        fn now(&self) -> i64 {
            std::thread::sleep(Duration::from_millis(400));
            1
        }
    }
    let mut r = PluginRegistry::empty();
    r.install(&P::new("t.slow", "1.0.0").on_register(|r| {
        r.provide::<dyn Clock>("slow", "1.0.0", Arc::new(Slow))?;
        Ok(())
    }))
    .unwrap();
    r.set_budgets(Budgets {
        service: Some(Duration::from_millis(50)),
        ..Budgets::default()
    });
    let s = r.service::<dyn Clock>("slow", "*").unwrap();
    let h = r.health();
    let watcher = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !h.get("t.slow").hung && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        h.get("t.slow").hung
    });
    // The call itself cannot be pre-empted: it completes, late.
    assert_eq!(s.call(|c| c.now()).unwrap(), 1);
    assert!(watcher.join().unwrap(), "marked hung while running");
    let info = info(&r, "t.slow");
    assert!(!info.hung && info.failures == 1);
    assert!(info.last_error.unwrap().contains("(service) did not finish within 50 ms"));
    // Detached: the caller stops waiting.
    let started = Instant::now();
    let e = s.call_detached(|c| c.now()).unwrap_err().to_string();
    assert!(e.contains("abandoned") && started.elapsed() < Duration::from_millis(350), "{e}");
}

// ---------------------------------------------- markers and mark()

const MARKER_PLUGIN: &str = "acme.win";

fn marker_plugin(transform: bool) -> P {
    P::new(MARKER_PLUGIN, "1.0.0").on_register(move |r| {
        r.add_marker_kinds(vec![
            PluginMarkerKind::new("crash", "Crash")
                .when("@message", "contains", "crash")
                .role(MarkerRole::Crash)
                .color("#F00")
                .badge("C", "circle")
                .describe("an app crashed"),
            PluginMarkerKind::new("tagged", "Tagged by code").icon("flag"),
        ])?;
        if transform {
            r.add_transform(Arc::new(Marking))?;
        }
        Ok(())
    })
}

struct Marking;
impl RecordTransform for Marking {
    fn id(&self) -> &str {
        "acme.marking"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        if r.message.contains("tag me") {
            assert!(ctx.mark("tagged"));
            assert!(ctx.mark("acme.win/tagged"));
            assert!(!ctx.mark("error"), "a built-in kind is not this plugin's");
            assert!(!ctx.mark("other.plugin/crash"));
        }
        Verdict::Keep
    }
}

fn kinds(c: &debrief_core::profile::CompiledProfile) -> Vec<(String, String)> {
    c.markers.iter().map(|m| (m.kind.clone(), m.owner.clone())).collect()
}

#[test]
fn profile_rules_come_first_then_plugin_kinds_then_builtins() {
    let mut r = PluginRegistry::new();
    r.install(&marker_plugin(false)).unwrap();
    let toml = format!(
        "{}\n[[marker]]\nkind = 'mine'\nwhen = [{{ field = '@message', op = 'contains', value = 'crash' }}]\n",
        debrief_core::DEFAULT_PROFILE_TOML
    );
    let c = r.compile_profile(Profile::from_toml(&toml).unwrap()).unwrap();
    let k = kinds(&c);
    assert_eq!(k[0], ("mine".into(), "profile".into()));
    assert_eq!(k[1], ("acme.win/crash".into(), MARKER_PLUGIN.into()));
    assert_eq!(k[2], ("acme.win/tagged".into(), MARKER_PLUGIN.into()));
    assert_eq!(k[3].1, "builtin");
    assert_eq!(c.markers[1].role, Some(MarkerRole::Crash));
    assert!(c.lifecycle_mask & 0b10 != 0, "crash kinds are lifecycle kinds");
    // A record matching both: the profile rule is primary, the plugin kind is also set.
    let mut rec = Record::new(1);
    rec.message = "app crash".into();
    rec.level = debrief_core::model::Level::Error;
    let mask = c.compute_markers(&rec, "f");
    assert_eq!(mask.trailing_zeros(), 0);
    assert!(mask & 0b10 != 0);

    // The default rule of a plugin kind is listed for read-only display; others have none.
    let rule = r.marker_rule(MARKER_PLUGIN, "acme.win/crash");
    assert_eq!(
        (rule.len(), rule[0].field.as_str(), rule[0].op.as_str()),
        (1, "@message", "contains")
    );
    assert!(
        r.marker_rule(MARKER_PLUGIN, "acme.win/tagged").is_empty(),
        "marked only programmatically"
    );
    assert!(r.marker_rule("profile", "mine").is_empty());
    assert!(r.marker_rule("other.plugin", "acme.win/crash").is_empty(), "owner must match");
    assert!(r.marker_rule(MARKER_PLUGIN, "acme.winx/crash").is_empty());
    let ws = debrief_core::Workspace::with_registry(Profile::from_toml(&toml).unwrap(), r).unwrap();
    let summary = ws.summary();
    let crash = summary.marker_kinds.iter().find(|k| k.kind == "acme.win/crash").unwrap();
    assert_eq!(crash.when.len(), 1);
    assert!(summary
        .marker_kinds
        .iter()
        .filter(|k| k.owner != MARKER_PLUGIN)
        .all(|k| k.when.is_empty()));
    let json = serde_json::to_value(crash).unwrap();
    assert_eq!(json["when"][0]["value"], "crash");
    assert_eq!(json["role"], "crash");
}

#[test]
fn overrides_disable_or_restyle_a_kind_and_bad_styles_fail_fast() {
    let mut r = PluginRegistry::new();
    r.install(&marker_plugin(false)).unwrap();
    let base = debrief_core::DEFAULT_PROFILE_TOML;
    let c = r
        .compile_profile(
            Profile::from_toml(&format!(
                "{base}\n[[marker_override]]\nkind = 'acme.win/crash'\nenabled = false\n\
                 [[marker_override]]\nkind = 'acme.win/tagged'\ncolor = '#00f'\nbadge = 'T'\nshape = 'square'\nlabel = 'T!'\n\
                 [[marker_override]]\nkind = 'error'\ndescription = 'restyled builtin'\n\
                 [[marker_override]]\nkind = 'not.installed/x'\nenabled = false\n"
            ))
            .unwrap(),
        )
        .unwrap();
    assert!(!kinds(&c).iter().any(|(k, _)| k == "acme.win/crash"));
    let tagged = c.markers.iter().find(|m| m.kind == "acme.win/tagged").unwrap();
    assert_eq!(
        (
            tagged.color.as_deref(),
            tagged.badge.as_deref(),
            tagged.shape.as_deref(),
            tagged.label.as_str()
        ),
        (Some("#00f"), Some("T"), Some("square"), "T!")
    );
    assert_eq!(
        c.markers.iter().find(|m| m.kind == "error").unwrap().description.as_deref(),
        Some("restyled builtin")
    );
    let bad = Profile::from_toml(&format!("{base}\n[[marker_override]]\nkind = 'acme.win/tagged'\ncolor = 'red'\n")).unwrap();
    let e = r.compile_profile(bad).err().unwrap().to_string();
    assert!(e.contains("color \"red\"") && e.contains("marker_override"), "{e}");
    // Round trip: overrides survive saving the profile.
    let p = Profile::from_toml(&format!("{base}\n[[marker_override]]\nkind = 'k'\nenabled = false\n")).unwrap();
    assert!(toml::to_string(&p).unwrap().contains("marker_override"));
}

#[test]
fn invalid_or_duplicate_plugin_kinds_are_rejected() {
    let mut r = PluginRegistry::new();
    let e = r
        .install(&P::new("p1", "1").on_register(|r| {
            r.add_marker_kinds(vec![PluginMarkerKind::new("has/slash", "x")])?;
            Ok(())
        }))
        .unwrap_err();
    assert!(e.to_string().contains("use 1–64 characters"), "{e}");
    let e = r
        .install(&P::new("p2", "1").on_register(|r| {
            r.add_marker_kinds(vec![PluginMarkerKind::new("k", "x").icon("no-such-icon")])?;
            Ok(())
        }))
        .unwrap_err();
    assert!(e.to_string().contains("unknown icon"), "{e}");
    let e = r
        .install(&P::new("p3", "1").on_register(|r| {
            r.add_marker_kinds(vec![PluginMarkerKind::new("k", "x"), PluginMarkerKind::new("k", "y")])?;
            Ok(())
        }))
        .unwrap_err();
    assert!(e.to_string().contains("declared twice"), "{e}");
    let e = r
        .install(&P::new("p4", "1").on_register(|r| {
            r.add_marker_kinds(vec![PluginMarkerKind::new("k", "x").when("@message", "nope", "v")])?;
            Ok(())
        }))
        .unwrap_err();
    assert!(e.to_string().contains("when"), "{e}");
}

#[test]
fn too_many_kinds_drop_plugin_kinds_with_a_note() {
    let mut r = PluginRegistry::new();
    r.install(&P::new("many", "1").on_register(|r| {
        let kinds = (0..70).map(|i| PluginMarkerKind::new(&format!("k{i}"), "k")).collect();
        r.add_marker_kinds(kinds)?;
        Ok(())
    }))
    .unwrap();
    let c = r.compile_profile(Profile::default_profile()).unwrap();
    assert_eq!(c.markers.len(), 64);
    assert!(c.markers.iter().any(|m| m.kind == "warning"), "built-ins stay");
    let w = info(&r, "many").warnings;
    assert!(w.iter().any(|w| w.contains("at most 64 kinds fit")), "{w:?}");
}

#[test]
fn mark_is_restricted_to_the_plugins_own_kinds_and_quarantine_hides_them() {
    quiet(|| {
        let d = Dir::new("mark");
        std::fs::write(
            d.0.join("a.log"),
            "2026-10-14 10:00:01 INFO tag me\n2026-10-14 10:00:02 INFO app crash\n2026-10-14 10:00:03 INFO plain\n",
        )
        .unwrap();
        let mut r = PluginRegistry::new();
        r.install(&marker_plugin(true)).unwrap();
        let profile = Profile::from_toml(&format!(
            "{}\n[[pipeline]]\nstep = 'plugin'\nid = 'acme.marking'\n",
            debrief_core::DEFAULT_PROFILE_TOML
        ))
        .unwrap();
        let mut ws = Workspace::with_registry(profile, r).unwrap();
        ws.open(std::slice::from_ref(&d.0)).unwrap();
        let page = |ws: &Workspace| {
            ws.page(&debrief_core::api::PageRequest {
                query: Query::default(),
                offset: 0,
                limit: 10,
                descending: false,
                anchor: None,
            })
            .unwrap()
            .rows
            .iter()
            .map(|r| r.markers.iter().map(|m| m.to_string()).collect::<Vec<_>>())
            .collect::<Vec<_>>()
        };
        let rows = page(&ws);
        assert_eq!(rows[0], ["acme.win/tagged"]);
        assert_eq!(rows[1], ["acme.win/crash"]);
        assert!(rows[2].is_empty());
        let info = ws.summary().plugins.into_iter().find(|p| p.id == MARKER_PLUGIN).unwrap();
        assert!(
            info.warnings.iter().any(|w| w.contains("mark(\"error\") was refused")),
            "{:?}",
            info.warnings
        );
        assert!(info.warnings.iter().any(|w| w.contains("mark(\"other.plugin/crash\") was refused")));
        let summary_kind = ws.summary().marker_kinds.into_iter().find(|k| k.kind == "acme.win/crash").unwrap();
        assert_eq!(
            (summary_kind.owner.as_str(), summary_kind.role),
            (MARKER_PLUGIN, Some(MarkerRole::Crash))
        );

        // Quarantined: its kinds disappear from the summary and from records, with a note.
        let health = ws.registry().health();
        for _ in 0..QUARANTINE_AFTER {
            health.fail(MARKER_PLUGIN, "boom");
        }
        let s = ws.summary();
        assert!(!s.marker_kinds.iter().any(|k| k.owner == MARKER_PLUGIN));
        assert!(page(&ws).iter().all(|r| r.is_empty()));
        let tl = ws
            .timeline(&debrief_core::api::TimelineRequest {
                query: Query::default(),
                buckets: 10,
                always_show_lifecycle: true,
            })
            .unwrap();
        assert!(tl.lanes.iter().all(|l| l.markers.is_empty()));
        let info = s.plugins.into_iter().find(|p| p.id == MARKER_PLUGIN).unwrap();
        assert!(
            info.warnings.iter().any(|w| w.contains("2 marker kind(s) are hidden")),
            "{:?}",
            info.warnings
        );
        // Reset brings them back.
        ws.reset_plugin(MARKER_PLUGIN);
        assert_eq!(page(&ws)[1], ["acme.win/crash"]);
    });
}

// --------------------------------------------------------- annotators

struct Spans {
    runs: Arc<Mutex<Vec<Option<RecordCursor>>>>,
    count: usize,
}
impl Annotator for Spans {
    fn id(&self) -> &str {
        "test.spans"
    }
    fn annotate(&self, view: &dyn AnnotationView, since: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        self.runs.lock().unwrap().push(since);
        let errors = view.select(
            &ViewFilter {
                when: vec![debrief_core::condition::ConditionCfg {
                    field: "@level".into(),
                    op: "eq".into(),
                    value: Some(debrief_core::condition::CondValue::Text("Error".into())),
                }],
                ..ViewFilter::default()
            },
            100,
        )?;
        let newest = view.select(
            &ViewFilter {
                newest_first: true,
                from: Some(0),
                to: Some(i64::MAX),
                ..ViewFilter::default()
            },
            1,
        )?;
        assert_eq!(
            view.query(&Query::default(), 2)?.len(),
            2.min(view.query(&Query::default(), 1000)?.len())
        );
        assert!(!view.instances().is_empty());
        let (from, to) = view.time_range().unwrap();
        assert_eq!(newest[0].record.ts, Some(to));
        let mut out: Vec<Annotation> = errors
            .iter()
            .map(|r| Annotation {
                kind: "error-point".into(),
                from: r.record.ts.unwrap(),
                to: r.record.ts.unwrap(),
                instance: Some(r.instance.clone()),
                on: None,
                label: r.record.message.clone(),
                severity: Severity::Warning,
                evidence_query: Query::default(),
            })
            .collect();
        out.push(Annotation {
            kind: "acme/whole".into(),
            from: to,
            to: from, // reversed on purpose: stored normalised
            instance: None,
            on: None,
            label: "everything".into(),
            severity: Severity::Info,
            evidence_query: Query::default(),
        });
        out.extend((0..self.count).map(|i| Annotation {
            kind: "filler".into(),
            from: i as i64,
            to: i as i64,
            instance: None,
            on: None,
            label: String::new(),
            severity: Severity::Info,
            evidence_query: Query::default(),
        }));
        Ok(out)
    }
}

struct Panicky;
impl Annotator for Panicky {
    fn id(&self) -> &str {
        "test.panicky"
    }
    fn annotate(&self, _: &dyn AnnotationView, _: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        panic!("annotator exploded")
    }
}

struct Failing;
impl Annotator for Failing {
    fn id(&self) -> &str {
        "test.failing"
    }
    fn annotate(&self, view: &dyn AnnotationView, _: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        // An invalid condition is an error, not a crash.
        view.select(
            &ViewFilter {
                when: vec![debrief_core::condition::ConditionCfg {
                    field: "x".into(),
                    op: "bogus".into(),
                    value: None,
                }],
                ..ViewFilter::default()
            },
            1,
        )?;
        Ok(Vec::new())
    }
}

#[test]
fn annotators_run_after_import_and_appends_are_capped_and_contained() {
    quiet(|| {
        let d = Dir::new("annot");
        let file = d.0.join("a.log");
        std::fs::write(&file, "2026-10-14 10:00:01 INFO one\n2026-10-14 10:00:02 ERROR bad\n").unwrap();
        let runs = Arc::new(Mutex::new(Vec::new()));
        let mut r = PluginRegistry::new();
        let rr = runs.clone();
        r.install(&P::new("acme", "1.0.0").on_register(move |r| {
            r.add_annotator(Arc::new(Spans {
                runs: rr.clone(),
                count: MAX_ANNOTATIONS,
            }))?;
            r.add_annotator(Arc::new(Failing))?;
            Ok(())
        }))
        .unwrap();
        r.install(&P::new("chaos", "1.0.0").on_register(|r| {
            r.add_annotator(Arc::new(Panicky))?;
            Ok(())
        }))
        .unwrap();
        assert!(r
            .install(&P::new("dup", "1.0.0").on_register(|r| {
                r.add_annotator(Arc::new(Failing))?;
                Ok(())
            }))
            .unwrap_err()
            .to_string()
            .contains("already registered"));
        let mut ws = Workspace::with_registry(Profile::default_profile(), r).unwrap();
        ws.open(std::slice::from_ref(&d.0)).unwrap();
        assert_eq!(*runs.lock().unwrap(), [None]);
        let all = ws.annotations(&AnnotationsRequest::default());
        assert_eq!(all.len(), MAX_ANNOTATIONS, "capped per annotator");
        let point = all.iter().find(|a| a.kind == "acme/error-point").unwrap();
        assert_eq!((point.label.as_str(), point.from == point.to), ("bad", true));
        let whole = all.iter().find(|a| a.kind == "acme/whole").unwrap();
        assert!(whole.from < whole.to);
        let acme = ws.summary().plugins.into_iter().find(|p| p.id == "acme").unwrap();
        assert!(acme.warnings.iter().any(|w| w.contains("only the first 10000 are kept")));
        assert!(acme.warnings.iter().any(|w| w.contains("annotator \"test.failing\" failed")));
        let chaos = ws.summary().plugins.into_iter().find(|p| p.id == "chaos").unwrap();
        assert_eq!(chaos.failures, 1);
        assert!(chaos.last_error.unwrap().contains("annotator exploded"));
        // Instance filter keeps all-instance annotations and matching ones.
        let other = ws.annotations(&AnnotationsRequest {
            instance: Some("nope".into()),
            ..Default::default()
        });
        assert!(other.iter().all(|a| a.instance.is_none()));

        // Append: the annotators run again, told where the new records start.
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        f.write_all(b"2026-10-14 10:00:03 ERROR worse\n").unwrap();
        drop(f);
        ws.refresh_tail(std::slice::from_ref(&file)).unwrap();
        let since = runs.lock().unwrap()[1].unwrap();
        assert_eq!(since.id, 2);
        let errors: Vec<String> = ws
            .annotations(&AnnotationsRequest::default())
            .into_iter()
            .filter(|a| a.kind == "acme/error-point")
            .map(|a| a.label)
            .collect();
        assert_eq!(errors, ["bad", "worse"]);
        // Nothing changed: no run.
        ws.refresh_tail(&[]).unwrap();
        assert_eq!(runs.lock().unwrap().len(), 2);
        // A quarantined plugin's annotations are hidden.
        let h = ws.registry().health();
        for _ in 0..QUARANTINE_AFTER {
            h.fail("acme", "x");
        }
        assert!(ws.annotations(&AnnotationsRequest::default()).is_empty());
        ws.reset_plugin("acme");
        ws.remove_source(ws.summary().sources[0].id);
        assert_eq!(runs.lock().unwrap().len(), 3, "removing a source re-runs them");
    });
}

#[test]
fn a_store_without_plugins_has_no_annotations() {
    struct Bare;
    let _ = Bare;
    let ws = Workspace::new(Profile::default_profile()).unwrap();
    assert!(ws.annotations(&AnnotationsRequest::default()).is_empty());
    assert!(!ws.registry().has_annotators());
    let _ = Path::new("x");
}

// ------------------------------------------------- remaining edges

/// Uses the default `activate`/`deactivate` and a manifest-declared service.
struct Plain;
impl Plugin for Plain {
    fn id(&self) -> &'static str {
        "plain"
    }
    fn name(&self) -> &'static str {
        "Plain"
    }
    fn version(&self) -> &'static str {
        "2.0.0"
    }
    fn manifest(&self) -> Manifest {
        Manifest::new("plain", "2.0.0").provides("plain.clock", "1.0.0")
    }
    fn register(&self, r: &mut PluginRegistry) -> Result<()> {
        r.provide::<dyn Clock>("plain.clock", "1.0.0", Arc::new(Fixed(3)))?;
        Ok(())
    }
}

#[test]
fn default_hooks_declared_services_and_plain_errors() {
    let mut r = PluginRegistry::empty();
    r.install_shared(Arc::new(Plain)).unwrap();
    r.resolve();
    let s = r.service::<dyn Clock>("plain.clock", "^1").unwrap();
    assert_eq!(s.id(), "plain.clock");
    let e = s.try_call::<()>(|_| Err(anyhow!("no time"))).unwrap_err().to_string();
    assert_eq!(e, "service \"plain.clock\" (provider \"plain\"): no time");
    assert_eq!(info(&r, "plain").provides.len(), 1, "declared once, not twice");
    r.shutdown();
}

#[test]
fn a_provider_crashing_during_a_dependents_activation_is_handled_reentrantly() {
    quiet(|| {
        let mut r = PluginRegistry::empty();
        r.install_shared(Arc::new(P::new("crashy", "1.0.0").on_register(|r| {
            r.provide::<dyn Clock>("crashy.clock", "1.0.0", Arc::new(Fixed(-1)))?;
            Ok(())
        })))
        .unwrap();
        r.install_shared(Arc::new(P::new("eager", "1.0.0").needs("crashy", "*").on_activate(|ctx| {
            let clock = ctx.service::<dyn Clock>("crashy.clock", "*")?;
            for _ in 0..QUARANTINE_AFTER {
                let _ = clock.call(|c| c.now());
            }
            clock.call(|c| c.now()).map(|_| ())
        })))
        .unwrap();
        r.resolve();
        assert_eq!(status(&r, "crashy").0, PluginStatus::Quarantined);
        let (s, reason) = status(&r, "eager");
        assert_eq!(s, PluginStatus::Inactive);
        assert!(reason.unwrap().contains("quarantined"));
    });
}

#[test]
fn inactive_plugins_show_read_only_settings_and_views_respect_bounds() {
    use debrief_core::settings::{Configurable, SectionOwner, SettingsSection, ValidValues};
    struct S;
    impl Configurable for S {
        fn settings_section(&self) -> SettingsSection {
            SettingsSection::new("x", "X", SectionOwner::Core)
        }
        fn apply_settings(&self, _: &ValidValues) -> Result<()> {
            Ok(())
        }
    }
    struct WithSettings;
    impl Plugin for WithSettings {
        fn id(&self) -> &'static str {
            "with.settings"
        }
        fn name(&self) -> &'static str {
            "W"
        }
        fn version(&self) -> &'static str {
            "1"
        }
        fn manifest(&self) -> Manifest {
            Manifest::new("with.settings", "1").requires("absent", "*")
        }
        fn register(&self, _: &mut PluginRegistry) -> Result<()> {
            Ok(())
        }
        fn configurable(&self) -> Option<Arc<dyn Configurable>> {
            Some(Arc::new(S))
        }
    }
    let mut r = PluginRegistry::empty();
    r.install(&WithSettings).unwrap();
    let s = &r.plugin_settings()[0];
    assert!(s.read_only());
    assert!(s.note().unwrap().contains("inactive"));

    // Time bounds exclude records without a time.
    struct Bounded(Arc<Mutex<Vec<usize>>>);
    impl Annotator for Bounded {
        fn id(&self) -> &str {
            "bounded"
        }
        fn annotate(&self, view: &dyn AnnotationView, _: Option<RecordCursor>) -> Result<Vec<Annotation>> {
            let all = view.select(&ViewFilter::default(), 100)?.len();
            let bounded = view
                .select(
                    &ViewFilter {
                        from: Some(i64::MIN),
                        ..ViewFilter::default()
                    },
                    100,
                )?
                .len();
            self.0.lock().unwrap().extend([all, bounded]);
            Ok(Vec::new())
        }
    }
    let d = Dir::new("bounds");
    std::fs::write(d.0.join("a.log"), "2026-10-14 10:00:01 INFO timed\nno time here\n").unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut r = PluginRegistry::new();
    let s2 = seen.clone();
    r.install(&P::new("b", "1").on_register(move |r| {
        r.add_annotator(Arc::new(Bounded(s2.clone())))?;
        Ok(())
    }))
    .unwrap();
    let mut ws = Workspace::with_registry(Profile::default_profile(), r).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let seen = seen.lock().unwrap().clone();
    let total = ws.summary().total;
    assert_eq!(seen[0], total);
    assert!(seen[1] <= seen[0]);
    let store: &dyn debrief_core::LogStore = &ws;
    assert!(store.annotations(&AnnotationsRequest::default()).is_empty());
}
