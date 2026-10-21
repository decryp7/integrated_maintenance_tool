//! Tests: the store in process (the DuckDB store on a temp folder; ADR 0014 "tests use the real adapter"),
//! with the importers of the given plugins (their processes, as in the store worker).

use super::{SharedStore, StoreHost};
use imt_core::package::ProjectManifest;
use imt_core::ports::StoreResult;
use imt_core::{PluginRegistry, Profile};
use imt_plugin_host::Builtins;
use imt_store_duck::DuckStore;
use std::path::Path;
use std::sync::Arc;

pub(crate) struct LocalHost {
    plugins: Option<Arc<Builtins>>,
}

/// Built-in importers only.
pub(crate) fn host() -> Arc<dyn StoreHost> {
    Arc::new(LocalHost { plugins: None })
}

/// With the importers of `plugins` (none in safe mode).
pub(crate) fn with_plugins(plugins: Arc<Builtins>, safe_mode: bool) -> Arc<dyn StoreHost> {
    Arc::new(LocalHost {
        plugins: (!safe_mode).then_some(plugins),
    })
}

impl LocalHost {
    fn registry(&self) -> PluginRegistry {
        match &self.plugins {
            Some(p) => crate::plugin_config::install(p.plugins(), false, &[]).unwrap_or_else(|_| PluginRegistry::new()),
            None => PluginRegistry::new(),
        }
    }
}

impl StoreHost for LocalHost {
    fn create(&self, dir: &Path, profile: &Profile) -> StoreResult<SharedStore> {
        Ok(Arc::new(DuckStore::create(dir, profile.clone(), self.registry())?))
    }

    fn open(&self, dir: &Path, profile: &Profile) -> StoreResult<SharedStore> {
        Ok(Arc::new(DuckStore::open(dir, profile.clone(), self.registry())?))
    }

    fn open_project(&self, path: &Path, dest_root: &Path, profile: &Profile) -> StoreResult<(SharedStore, ProjectManifest)> {
        std::fs::create_dir_all(dest_root)?;
        let (store, manifest) = DuckStore::open_project(path, dest_root, profile.clone(), self.registry())?;
        Ok((Arc::new(store), manifest))
    }
}
