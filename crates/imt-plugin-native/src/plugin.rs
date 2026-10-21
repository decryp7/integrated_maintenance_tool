//! The plugin: its tools, settings and the `.etl` importer, and the rights its processes need.

use crate::captures::Captures;
use crate::importer::EtlImporter;
use crate::settings::{Knobs, NativeSettings};
use crate::tool::{EtwTool, NativeTool};
use crate::{Limits, PLUGIN_ID};
use anyhow::Result;
use imt_plugin_api::manifest::{Permission, PermissionRequest};
use imt_plugin_api::{Contributions, Plugin};
use std::sync::Arc;

/// Memory of a plugin process: DbgHelp loads large PDBs, conversions hold symbol tables.
const MEMORY_LIMIT: u64 = 2 << 30;

/// The native profiling plugin. One value per process: the normal process records samples, the elevated one ETW.
#[derive(Default)]
pub struct NativePlugin {
    captures: Arc<Captures>,
    knobs: Arc<Knobs>,
}

impl NativePlugin {
    pub fn new() -> NativePlugin {
        NativePlugin::default()
    }
}

impl Drop for NativePlugin {
    fn drop(&mut self) {
        self.captures.stop_all();
    }
}

impl Plugin for NativePlugin {
    fn id(&self) -> &str {
        PLUGIN_ID
    }

    fn name(&self) -> &str {
        "Native profiling"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        let limits = Limits::default();
        c.add_tool(Arc::new(NativeTool::new(self.captures.clone(), self.knobs.clone(), limits)));
        c.add_tool(Arc::new(EtwTool::new(self.captures.clone(), limits)));
        c.set_settings(Arc::new(NativeSettings(self.knobs.clone())));
        c.add_profile_importer(EtlImporter { knobs: self.knobs.clone() });
        Ok(())
    }

    fn permissions(&self) -> Vec<PermissionRequest> {
        vec![
            PermissionRequest {
                permission: Permission::WriteFolders,
                reason: "Writes captures and the symbol cache into IMT's data folder, and opens the processes you profile \
                         (which needs a normal, not a low, integrity level)"
                    .into(),
            },
            PermissionRequest {
                permission: Permission::Network,
                reason: "Downloads PDBs of Windows modules from the Microsoft symbol server, only when you enable it in Settings ▸ Native profiling".into(),
            },
        ]
    }

    fn memory_limit(&self) -> Option<u64> {
        Some(MEMORY_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_two_tools_settings_and_the_etl_importer() {
        let p = NativePlugin::new();
        let mut c = Contributions::default();
        p.register(&mut c).unwrap();
        let m = c.manifest(&p);
        m.check().unwrap();
        let tools: Vec<&str> = m.tools.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(tools, [crate::TOOL_ID, crate::ETW_TOOL_ID]);
        assert!(m.has_elevated_ops());
        assert!(m.settings.is_some());
        assert_eq!(&*m.profile_importers[0].id, "etl");
        assert!(m.asks_for(Permission::WriteFolders) && m.asks_for(Permission::Network));
        assert_eq!(m.memory_limit, Some(MEMORY_LIMIT));
        assert_eq!(p.name(), "Native profiling");
    }
}
