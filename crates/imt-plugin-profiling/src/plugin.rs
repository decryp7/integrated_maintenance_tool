//! The plugin adapter: declares the extension point and contributes the built-in importers.

use crate::{builtin_importers, PLUGIN_ID};
use anyhow::Result;
use imt_core::{Plugin, PluginRegistry};
use imt_profile::{CaptureImporter, IMPORTERS_POINT};

pub struct ProfilingPlugin;

impl Plugin for ProfilingPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn name(&self) -> &'static str {
        "Profiling"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn manifest(&self) -> imt_core::plugin::Manifest {
        imt_core::plugin::Manifest::new(self.id(), self.version())
    }

    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        registry.declare_extension_point::<dyn CaptureImporter>(IMPORTERS_POINT)?;
        for importer in builtin_importers() {
            registry.contribute::<dyn CaptureImporter>(IMPORTERS_POINT, importer);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_the_point_and_every_importer() {
        let mut registry = PluginRegistry::new();
        registry.install(&ProfilingPlugin).unwrap();
        registry.resolve();
        let importers = registry.contributions::<dyn CaptureImporter>(IMPORTERS_POINT);
        assert_eq!(importers.len(), builtin_importers().len());
        let info = registry.plugins().into_iter().find(|p| p.id == PLUGIN_ID).unwrap();
        assert!(info.inactive_reason.is_none(), "{:?}", info.inactive_reason);
        assert!(info.warnings.is_empty(), "{:?}", info.warnings);
        assert_eq!(importers[0].call(|i| i.id()).unwrap(), builtin_importers()[0].id());
    }
}
