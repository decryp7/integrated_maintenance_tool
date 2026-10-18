//! IMT tools plugin (`net.decryptology.tools`): the system tools the shell
//! used to call directly, contributed as [`imt_core::plugin::tools::Tool`]s so
//! they can be disabled, quarantined and left out in safe mode (ADR 0002).
//!
//! | Tool id | Platforms | Operations |
//! |---|---|---|
//! | [`ids::PERFMON_TEMPLATES`] | all | `presets`, `generate` (Data Collector Set XML and `logman` commands) |
//! | [`ids::PERFMON_SETS`] | Windows | `install`, `control` (`start`/`stop`/`delete`/`query`), `outputs` |
//! | [`ids::PROCESS_MONITOR`] | all | `start`, `stop`, `status` (rolling CLEF files, see `imt-monitor`) |
//! | [`ids::EVENT_LOG`] | Windows | `channels`, `validate`, `start`, `stop`, `pause`, `resume`, `status` (live event logs to rolling CLEF files, see `imt_plugin_windows::eventlog`) |
//! | [`ids::DIAGNOSTICS`] | all | `default-plan`, `collect` (an `.imtz` bundle, see `imt-collect`) |
//! | [`ids::ELEVATION`] | all | `info` (administrator rights, platform) |
//!
//! It also provides the [`imt_api_perf::CounterSource`] service from live
//! system sampling ([`counters::SystemCounters`]), and declares an optional
//! dependency on the Windows plugin, which imports the `.blg` files a
//! collector set writes.
//!
//! One [`ToolsPlugin`] value is shared by every window's registry (clones
//! share state), so a running process monitor is the same in every window.
//! External programs (`logman`, `wevtutil`, `id`, ...) run only through the
//! injected [`Runner`], and the monitor collector is injected too: tests use
//! fakes for both ([`ToolsEnv`]).

mod collect_tool;
pub mod counters;
mod elevation;
mod eventlog_tool;
#[cfg(test)]
pub(crate) mod fake;
mod monitor_tool;
pub mod path_counters;
pub mod perfmon_control;
pub mod perfmon_template;
mod perfmon_tools;
mod settings;

use anyhow::Result;
use imt_core::plugin::tools::Tool;
use imt_core::plugin::{Collector, Manifest};
use imt_core::settings::Configurable;
use imt_core::{Plugin, PluginRegistry};
use std::sync::Arc;

/// Runs external programs (with a timeout, never through a shell).
pub type Runner = Arc<dyn imt_collect::CommandRunner + Send + Sync>;

/// The plugin id.
pub const PLUGIN_ID: &str = "net.decryptology.tools";

/// Ids of the contributed tools (what `tool_invoke` names).
pub mod ids {
    pub const PERFMON_TEMPLATES: &str = "perfmon.templates";
    pub const PERFMON_SETS: &str = "perfmon.sets";
    pub const PROCESS_MONITOR: &str = "process-monitor";
    pub const EVENT_LOG: &str = "event-log-follower";
    pub const DIAGNOSTICS: &str = "diagnostics";
    pub const ELEVATION: &str = "elevation";
}

/// What the tools reach outside the process: injectable for tests.
pub struct ToolsEnv {
    pub runner: Runner,
    /// The process monitor (`imt_monitor::ProcessMonitor` in the app).
    pub monitor: Arc<dyn Collector>,
    /// The Windows Event Log (`imt_plugin_windows::eventlog::system()` in the app).
    pub event_log: Arc<dyn imt_plugin_windows::eventlog::EventLog>,
    /// Whether Performance Monitor (`logman`) can be driven here.
    pub windows: bool,
}

impl ToolsEnv {
    /// Real processes, the real monitor, the current platform.
    pub fn system() -> ToolsEnv {
        ToolsEnv {
            runner: Arc::new(imt_collect::SystemRunner),
            monitor: Arc::new(imt_monitor::ProcessMonitor),
            event_log: imt_plugin_windows::eventlog::system(),
            windows: cfg!(windows),
        }
    }
}

struct Shared {
    knobs: Arc<settings::Knobs>,
    tools: Vec<Arc<dyn Tool>>,
    counters: Arc<dyn imt_api_perf::CounterSource>,
}

/// The plugin. Clones share their tools and state.
#[derive(Clone)]
pub struct ToolsPlugin {
    shared: Arc<Shared>,
}

impl Default for ToolsPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolsPlugin {
    pub fn new() -> Self {
        Self::with_env(ToolsEnv::system())
    }

    pub fn with_env(env: ToolsEnv) -> Self {
        let knobs = Arc::new(settings::Knobs::default());
        let sets = perfmon_tools::SetsTool::new(env.runner.clone(), env.windows, knobs.clone());
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(perfmon_tools::TemplatesTool),
            Arc::new(sets),
            Arc::new(monitor_tool::MonitorTool::new(env.monitor)),
            Arc::new(eventlog_tool::EventLogTool::new(env.event_log)),
            Arc::new(collect_tool::DiagnosticsTool::new(env.runner.clone())),
            Arc::new(elevation::ElevationTool::new(env.runner)),
        ];
        ToolsPlugin {
            shared: Arc::new(Shared {
                knobs,
                tools,
                counters: counters::system(),
            }),
        }
    }
}

impl Plugin for ToolsPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn name(&self) -> &'static str {
        "System tools"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// Optionally uses the Windows plugin (it imports the `.blg` logs a
    /// collector set writes); provides live performance counters.
    fn manifest(&self) -> Manifest {
        Manifest::new(self.id(), self.version())
            .optionally("net.decryptology.windows", ">=0.1")
            .provides(imt_api_perf::SERVICE_ID, imt_api_perf::VERSION)
    }

    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        for tool in &self.shared.tools {
            registry.add_tool(tool.clone())?;
        }
        let counters = self.shared.counters.clone();
        registry.provide::<dyn imt_api_perf::CounterSource>(imt_api_perf::SERVICE_ID, imt_api_perf::VERSION, counters)?;
        Ok(())
    }

    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        Some(Arc::new(settings::ToolsSettings(self.shared.knobs.clone())))
    }
}

#[cfg(test)]
mod tests;
