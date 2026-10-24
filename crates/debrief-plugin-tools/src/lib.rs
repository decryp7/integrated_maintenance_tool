//! Debrief tools plugin (`net.decryptology.tools`): the system tools the shell
//! used to call directly, contributed as [`debrief_plugin_api::tools::Tool`]s so
//! they can be disabled, quarantined and left out in safe mode (ADR 0002).
//!
//! | Tool id | Platforms | Operations |
//! |---|---|---|
//! | [`ids::PERFMON_TEMPLATES`] | all | `presets`, `generate` (Data Collector Set XML and `logman` commands) |
//! | [`ids::PERFMON_SETS`] | Windows | `install`, `control` (`start`/`stop`/`delete`/`query`), `outputs` |
//! | [`ids::PROCESS_MONITOR`] | all | `start`, `stop`, `status` (rolling CLEF files, see `debrief-monitor`) |
//! | [`ids::EVENT_LOG`] | Windows | `channels`, `validate`, `start`, `stop`, `pause`, `resume`, `status` (live event logs to rolling CLEF files, see `debrief_plugin_windows::eventlog`) |
//! | [`ids::DIAGNOSTICS`] | all | `default-plan`, `collect` (an `.debriefz` bundle, see `debrief-collect`) |
//! | [`ids::ELEVATION`] | all | `info` (administrator rights, platform) |
//!
//! It declares an optional dependency on the Windows plugin, which imports the
//! `.blg` files a collector set writes.
//!
//! One [`ToolsPlugin`] value is shared by every window's registry (clones
//! share state), so a running process monitor is the same in every window.
//! External programs (`logman`, `wevtutil`, `id`, ...) run only through the
//! injected [`Runner`], and the monitor collector is injected too: tests use
//! fakes for both ([`ToolsEnv`]).

mod collect_tool;
mod elevation;
mod eventlog_tool;
#[cfg(test)]
pub(crate) mod fake;
mod monitor_tool;
pub mod perfmon_control;
pub mod perfmon_template;
mod perfmon_tools;
mod run_slot;
mod settings;

use anyhow::Result;
use debrief_core::plugin::Collector;
use debrief_plugin_api::manifest::{Permission, PermissionRequest};
use debrief_plugin_api::settings::Configurable;
use debrief_plugin_api::tools::Tool;
use debrief_plugin_api::{Contributions, Plugin};
use std::sync::Arc;

/// Runs external programs (with a timeout, never through a shell).
pub type Runner = Arc<dyn debrief_collect::CommandRunner + Send + Sync>;

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
    /// The process monitor (`debrief_monitor::ProcessMonitor` in the app).
    pub monitor: Arc<dyn Collector>,
    /// The Windows Event Log (`debrief_plugin_windows::eventlog::system()` in the app).
    pub event_log: Arc<dyn debrief_plugin_windows::eventlog::EventLog>,
    /// Whether Performance Monitor (`logman`) can be driven here.
    pub windows: bool,
}

impl ToolsEnv {
    /// Real processes, the real monitor, the current platform.
    pub fn system() -> ToolsEnv {
        ToolsEnv {
            runner: Arc::new(debrief_collect::SystemRunner),
            monitor: Arc::new(debrief_monitor::ProcessMonitor),
            event_log: debrief_plugin_windows::eventlog::system(),
            windows: cfg!(windows),
        }
    }
}

struct Shared {
    knobs: Arc<settings::Knobs>,
    tools: Vec<Arc<dyn Tool>>,
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
            shared: Arc::new(Shared { knobs, tools }),
        }
    }
}

impl Plugin for ToolsPlugin {
    fn id(&self) -> &str {
        PLUGIN_ID
    }

    fn name(&self) -> &str {
        "System tools"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        for tool in &self.shared.tools {
            c.add_tool(tool.clone());
        }
        c.set_settings(Arc::new(settings::ToolsSettings(self.shared.knobs.clone())) as Arc<dyn Configurable>);
        Ok(())
    }

    /// The tools run system programs (`logman`, `wevtutil`, ...) and write their output (collector sets, the
    /// process monitor's and event-log follower's rolling files, bundles) into folders the user chose.
    fn permissions(&self) -> Vec<PermissionRequest> {
        vec![
            PermissionRequest {
                permission: Permission::RunPrograms,
                reason: "Performance Monitor, the Event Log and the diagnostics collector are driven by system programs".into(),
            },
            PermissionRequest {
                permission: Permission::WriteFolders,
                reason: "collectors write their logs and bundles into the folders you choose".into(),
            },
        ]
    }
}

#[cfg(test)]
mod tests;
