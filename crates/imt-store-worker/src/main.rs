//! The IMT store worker. IMT starts it and tells it, on stdin, its private endpoint and a one-time token; it
//! connects, serves the store protocol (`imt_store_client::protocol`, ADR 0017 frames) and exits when IMT closes
//! the connection or stdin (IMT is gone).
//!
//! Importer plugins run in their own processes (ADR 0017), started by this worker on first use, so events go
//! from a plugin to the store without passing through IMT.

use imt_plugin_host::Builtins;
use std::sync::{Arc, OnceLock};

#[global_allocator]
static GLOBAL: imt_alloc::MiMalloc = imt_alloc::MiMalloc;

fn main() {
    imt_alloc::tune();
    let mode = imt_store_client::MemoryMode::from_args(std::env::args());
    if mode == imt_store_client::MemoryMode::Low {
        imt_alloc::purge_at_once();
    }
    let builtins: Arc<OnceLock<Builtins>> = Arc::default();
    let registry: imt_store_worker::server::RegistryFactory = Arc::new(move |disabled, safe_mode| {
        if safe_mode {
            return imt_core::PluginRegistry::new();
        }
        let plugins = builtins
            .get_or_init(|| Builtins::load(&imt_plugin_host::builtins::default_binary()))
            .plugins();
        imt_store_worker::server::install_plugins(plugins, disabled)
    });
    if let Err(e) = imt_store_worker::server::run(registry, mode) {
        eprintln!("imt-store-worker: {e}");
        std::process::exit(1);
    }
}
