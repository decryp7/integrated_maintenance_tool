//! The plugin adapter: the `dotnet-profiler` tool and the `.etl` importer,
//! contributed through the plugin registry (ADR 0019). Captures are app-wide
//! within a process: every window's registry gets the same capture list.

mod captures;
mod importer;
mod tool;

pub use importer::EtlImporter;
pub use tool::{TASK_KIND, TOOL_ID};

use crate::{Collector, Limits, SystemCollector, PLUGIN_ID};
use anyhow::Result;
use imt_core::plugin::Manifest;
use imt_core::{Plugin, PluginRegistry};
use imt_profile::{CaptureImporter, IMPORTERS_POINT};
use std::sync::{Arc, OnceLock};

/// The plugin. `DotnetPlugin::default()` uses the real collector and the process-wide capture list.
#[derive(Clone)]
pub struct DotnetPlugin {
    captures: Arc<captures::Captures>,
    limits: Limits,
}

impl Default for DotnetPlugin {
    fn default() -> Self {
        static SHARED: OnceLock<Arc<captures::Captures>> = OnceLock::new();
        DotnetPlugin {
            captures: SHARED.get_or_init(|| captures::Captures::new(Arc::new(SystemCollector))).clone(),
            limits: Limits::default(),
        }
    }
}

impl DotnetPlugin {
    /// A plugin with its own capture list over `collector` (tests, other hosts).
    pub fn with_collector(collector: Arc<dyn Collector>, limits: Limits) -> DotnetPlugin {
        DotnetPlugin {
            captures: captures::Captures::new(collector),
            limits,
        }
    }
}

impl Plugin for DotnetPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn name(&self) -> &'static str {
        ".NET profiling"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// The `.etl` importer goes to the profiling plugin's extension point.
    fn manifest(&self) -> Manifest {
        Manifest::new(self.id(), self.version()).requires(imt_plugin_profiling::PLUGIN_ID, ">=0.1")
    }

    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        registry.add_tool(Arc::new(tool::DotnetTool::new(self.captures.clone(), self.limits)))?;
        registry.contribute::<dyn CaptureImporter>(IMPORTERS_POINT, Arc::new(EtlImporter));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_the_tool_and_the_importer() {
        let mut registry = PluginRegistry::new();
        registry.install(&imt_plugin_profiling::ProfilingPlugin).unwrap();
        registry.install(&DotnetPlugin::default()).unwrap();
        registry.resolve();
        let info = registry.plugins().into_iter().find(|p| p.id == PLUGIN_ID).unwrap();
        assert!(info.inactive_reason.is_none(), "{:?}", info.inactive_reason);
        let importers = registry.contributions::<dyn CaptureImporter>(IMPORTERS_POINT);
        assert!(importers.iter().any(|i| i.call(|i| i.id()).ok() == Some("etl")));
        let tools = registry.tools().list();
        assert!(
            tools.iter().any(|t| t.info.id == TOOL_ID),
            "{:?}",
            tools.iter().map(|t| &t.info.id).collect::<Vec<_>>()
        );
        // The default plugin shares one capture list.
        assert!(Arc::ptr_eq(&DotnetPlugin::default().captures, &DotnetPlugin::default().captures));
    }
}
