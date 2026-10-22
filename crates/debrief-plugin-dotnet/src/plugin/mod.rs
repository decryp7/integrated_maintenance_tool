//! The plugin adapter (ADR 0019 on the plugin contract, ADR 0017): the `dotnet-profiler` tool. Debrief runs the plugin
//! in its own process; captures are app-wide within it. `.etl` traces open through the native profiling plugin
//! (one importer for .NET and native processes, ADR 0022 §9).
//!
//! ETW needs administrator rights, so ETW recordings run in the plugin's **elevated process** (ADR 0017 §7): the
//! normal process asks the host to run the tool's elevated `etw-*` operations there, and the host starts that
//! process through the administrator prompt on first use. EventPipe and listing processes stay in the normal process.

mod captures;
mod elevated;
mod tool;

pub use tool::{TASK_KIND, TOOL_ID};

use crate::{Collector, Limits, PLUGIN_ID};
use anyhow::Result;
use debrief_plugin_api::manifest::{Permission, PermissionRequest};
use debrief_plugin_api::{Contributions, Plugin};
use std::sync::{Arc, OnceLock};

/// Memory of the plugin's process: `.etl` imports hold the file and its conversion.
const MEMORY_LIMIT: u64 = 3 << 30;

/// The plugin. `DotnetPlugin::default()` records with the real backends and the process-wide capture list.
#[derive(Clone)]
pub struct DotnetPlugin {
    captures: Arc<captures::Captures>,
    /// How ETW recordings reach the elevated process (`None`: a test collector records everything).
    router: Option<Arc<elevated::Router>>,
    limits: Limits,
}

impl Default for DotnetPlugin {
    fn default() -> Self {
        static SHARED: OnceLock<(Arc<captures::Captures>, Arc<elevated::Router>)> = OnceLock::new();
        let (captures, router) = SHARED.get_or_init(|| {
            let router = Arc::new(elevated::Router::default());
            (captures::Captures::new(router.clone()), router)
        });
        DotnetPlugin {
            captures: captures.clone(),
            router: Some(router.clone()),
            limits: Limits::default(),
        }
    }
}

impl DotnetPlugin {
    /// A plugin with its own capture list over `collector` (tests, other hosts).
    pub fn with_collector(collector: Arc<dyn Collector>, limits: Limits) -> DotnetPlugin {
        DotnetPlugin {
            captures: captures::Captures::new(collector),
            router: None,
            limits,
        }
    }
}

impl Plugin for DotnetPlugin {
    fn id(&self) -> &str {
        PLUGIN_ID
    }

    fn name(&self) -> &str {
        ".NET profiling"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_tool(Arc::new(tool::DotnetTool::new(
            self.captures.clone(),
            self.router.clone(),
            self.limits,
        )));
        Ok(())
    }

    fn permissions(&self) -> Vec<PermissionRequest> {
        vec![PermissionRequest {
            permission: Permission::WriteFolders,
            reason: "captures are written to Debrief's data folder, and EventPipe connects to the diagnostics channel of \
                     the process you record"
                .into(),
        }]
    }

    fn memory_limit(&self) -> Option<u64> {
        Some(MEMORY_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_the_tool() {
        let plugin = DotnetPlugin::default();
        let mut c = Contributions::default();
        plugin.register(&mut c).unwrap();
        let m = c.manifest(&plugin);
        m.check().unwrap();
        assert!(m.profile_importers.is_empty(), ".etl is the native profiler's");
        let tool = m.tools.iter().find(|t| t.id == TOOL_ID).unwrap();
        // ETW runs in the elevated process; nothing the UI calls does.
        let elevated: Vec<&str> = tool.ops.iter().filter(|o| o.elevated).map(|o| o.name.as_str()).collect();
        assert_eq!(elevated, ["etw-start", "etw-poll", "etw-stop", "etw-cancel"]);
        assert!(m.asks_for(Permission::WriteFolders) && !m.asks_for(Permission::Network));
        assert_eq!(m.memory_limit, Some(MEMORY_LIMIT));
        // The default plugin shares one capture list.
        assert!(Arc::ptr_eq(&DotnetPlugin::default().captures, &DotnetPlugin::default().captures));
    }
}
