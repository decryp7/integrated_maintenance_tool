use super::*;
use crate::fake::{ok, FakeRunner};
use imt_core::plugin::tools::Availability;
use imt_core::plugin::PluginStatus;
use imt_core::PluginRegistry;
use imt_plugin_api::clef::EventSink;
use imt_plugin_api::tools::{NoTasks, Platform, ToolCtx};
use serde_json::json;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// A monitor that idles until stopped.
struct IdleMonitor;

impl Collector for IdleMonitor {
    fn id(&self) -> &str {
        "process-monitor"
    }
    fn name(&self) -> &str {
        "idle"
    }
    fn run(&self, _: &serde_json::Value, _: &dyn EventSink, stop: &AtomicBool) -> Result<()> {
        while !stop.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

fn fake_plugin(runner: Arc<FakeRunner>, windows: bool) -> ToolsPlugin {
    ToolsPlugin::with_env(ToolsEnv {
        runner,
        monitor: Arc::new(IdleMonitor),
        event_log: Arc::new(crate::eventlog_tool::NoEventLog),
        windows,
    })
}

/// The plugin's tools and settings installed in an engine registry in this process, as the host's proxies
/// install them (tests of the tools through the engine's tool host: validation, availability, quarantine).
struct InProcess(ToolsPlugin);

impl imt_core::Plugin for InProcess {
    fn id(&self) -> &str {
        PLUGIN_ID
    }
    fn name(&self) -> &str {
        "System tools"
    }
    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
    fn register(&self, r: &mut PluginRegistry) -> Result<()> {
        for tool in &self.0.shared.tools {
            r.add_tool(tool.clone())?;
        }
        Ok(())
    }
    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        Some(Arc::new(settings::ToolsSettings(self.0.shared.knobs.clone())))
    }
}

fn registry(plugin: &ToolsPlugin) -> PluginRegistry {
    let mut r = PluginRegistry::empty();
    r.install_shared(Arc::new(InProcess(plugin.clone()))).unwrap();
    r.resolve();
    r
}

/// The user chose every path (path arguments are checked by the host; these tests are about the tools).
struct AnyPath;

impl imt_plugin_api::tools::PathPolicy for AnyPath {
    fn check(
        &self,
        path: &str,
        _: imt_plugin_api::tools::PathKind,
        _: imt_plugin_api::tools::PathAccess,
    ) -> Result<std::path::PathBuf, String> {
        Ok(path.into())
    }
}

fn ctx() -> ToolCtx {
    ToolCtx::new(None, Arc::new(NoTasks)).with_paths(Arc::new(AnyPath))
}

#[test]
fn registers_its_tools_service_and_settings() {
    let plugin = fake_plugin(Arc::new(FakeRunner::new(|_, _| ok(""))), true);
    let r = registry(&plugin);
    let info = &r.plugins()[0];
    assert_eq!((info.id.as_str(), info.status), (PLUGIN_ID, PluginStatus::Active));
    assert_eq!(
        info.contributes,
        [
            "tool:perfmon.templates",
            "tool:perfmon.sets",
            "tool:process-monitor",
            "tool:event-log-follower",
            "tool:diagnostics",
            "tool:elevation",
            "settings"
        ]
    );
    assert!(info.provides.is_empty() && info.requires.is_empty());
    let mut c = Contributions::default();
    plugin.register(&mut c).unwrap();
    let m = c.manifest(&plugin);
    m.check().unwrap();
    assert_eq!(m.contributions(), info.contributes);
    assert!(m.asks_for(Permission::RunPrograms) && m.asks_for(Permission::WriteFolders));
    assert_eq!(plugin.name(), "System tools");
    assert_eq!(plugin.version(), env!("CARGO_PKG_VERSION"));
    let section = r.plugin_settings()[0].configurable.settings_section();
    assert_eq!(section.id, PLUGIN_ID);
}

#[test]
fn platform_gating_and_dispatch_through_the_host() {
    let fake = Arc::new(FakeRunner::new(|_, _| ok("Status: Stopped\n")));
    let r = registry(&fake_plugin(fake.clone(), false));
    let mac = r.tools().on_platform(Platform::Macos);
    let listed = mac.list();
    for windows_only in [ids::PERFMON_SETS, ids::EVENT_LOG] {
        let t = listed.iter().find(|t| t.info.id == windows_only).unwrap();
        assert!(matches!(&t.availability, Availability::Unsupported { .. }), "{windows_only}");
    }
    let sets = listed.iter().find(|t| t.info.id == ids::PERFMON_SETS).unwrap();
    assert!(
        matches!(&sets.availability, Availability::Unsupported { reason } if reason.contains("only available on Windows (this is macOS)"))
    );
    let others: Vec<_> = listed
        .iter()
        .filter(|t| t.info.id != ids::PERFMON_SETS && t.info.id != ids::EVENT_LOG)
        .collect();
    assert!(others.iter().all(|t| t.availability == Availability::Available), "{others:?}");
    assert!(mac.invoke(ids::PERFMON_SETS, "outputs", json!({ "name": "IMT" }), &ctx()).is_err());
    let presets = mac.invoke(ids::PERFMON_TEMPLATES, "presets", json!({}), &ctx()).unwrap();
    assert!(presets.as_array().unwrap().len() >= 4);
    let bad = mac
        .invoke(ids::PERFMON_SETS, "control", json!({ "name": "IMT", "action": "explode" }), &ctx())
        .unwrap_err();
    assert!(bad.to_string().contains("Windows"), "platform first: {bad}");
    let win = r.tools().on_platform(Platform::Windows);
    let bad = win
        .invoke(ids::PERFMON_SETS, "control", json!({ "name": "IMT", "action": "explode" }), &ctx())
        .unwrap_err();
    assert_eq!(
        bad.to_string(),
        "perfmon.sets control: args.action \"explode\" is not one of: start, stop, delete, query"
    );
    assert!(fake.calls().is_empty());
}

#[test]
fn clones_share_state_across_registries_and_settings_reach_logman() {
    let fake = Arc::new(FakeRunner::new(|_, _| ok("Status: Running\n")));
    let plugin = fake_plugin(fake.clone(), true);
    let (a, b) = (registry(&plugin), registry(&plugin));
    let dir = std::env::temp_dir().join(format!("imt-tools-shared-{}", std::process::id()));
    let started = a
        .tools()
        .invoke(
            ids::PROCESS_MONITOR,
            "start",
            json!({ "config": { "targets": [{ "pid": 1 }] }, "dir": dir }),
            &ctx(),
        )
        .unwrap();
    assert_eq!(started, json!(dir.to_string_lossy()));
    // The other window sees the same run.
    assert_eq!(
        b.tools().invoke(ids::PROCESS_MONITOR, "status", json!({}), &ctx()).unwrap()["running"],
        true
    );
    assert_eq!(b.tools().invoke(ids::PROCESS_MONITOR, "stop", json!({}), &ctx()).unwrap(), started);
    let _ = std::fs::remove_dir_all(dir);

    let settings = a.plugin_settings()[0].configurable.clone();
    let values = imt_plugin_api::settings::validate(&settings.settings_section(), &json!({ "logmanTimeout": 9_000 })).unwrap();
    settings.apply_settings(&values).unwrap();
    b.tools()
        .on_platform(Platform::Windows)
        .invoke(ids::PERFMON_SETS, "control", json!({ "name": "IMT", "action": "query" }), &ctx())
        .unwrap();
    assert_eq!(fake.timeouts().last(), Some(&Duration::from_secs(9)));
}

#[test]
fn a_quarantined_plugin_lists_its_tools_as_unavailable_with_the_reason() {
    let r = registry(&fake_plugin(Arc::new(FakeRunner::new(|_, _| ok(""))), true));
    for _ in 0..imt_core::guard::QUARANTINE_AFTER {
        r.health().fail(PLUGIN_ID, "boom");
    }
    let listed = r.tools().on_platform(Platform::Windows).list();
    assert!(listed
        .iter()
        .all(|t| matches!(&t.availability, Availability::PluginUnavailable { reason } if reason.contains("repeated crashes"))));
    let e = r.tools().invoke(ids::ELEVATION, "info", json!({}), &ctx()).unwrap_err();
    assert!(e.to_string().contains("repeated crashes"), "{e}");
}

#[test]
fn disabled_or_safe_mode_means_no_tools() {
    let mut r = PluginRegistry::empty();
    r.disable([PLUGIN_ID]);
    r.install_shared(Arc::new(InProcess(ToolsPlugin::default()))).unwrap();
    assert!(r.tools().is_empty());
    assert!(PluginRegistry::empty().tools().is_empty());
}
