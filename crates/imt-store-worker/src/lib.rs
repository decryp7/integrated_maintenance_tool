//! The store worker (ADR 0014 §1): one process per app instance hosts the DuckDB stores of all windows and
//! their full-text indexes. [`server`] is the worker side and [`plugins`] the importer plugins it starts; IMT's
//! side (the protocol, the supervisor and `RemoteStore`) is `imt-store-client`, which links no database.

pub mod plugins;
pub mod server;

pub use imt_store_client::{protocol, RemoteStore, StoreWorker, WorkerConfig};

/// The worker program: serves IMT with the built-in plugins next to it and the installed third-party plugins IMT
/// names (ADR 0027), each started on first use. Exits with status 1 when it cannot connect.
pub fn main() {
    let plugins = plugins::WorkerPlugins::new(imt_plugin_host::builtins::default_binary());
    if let Err(e) = server::run(std::sync::Arc::new(plugins)) {
        eprintln!("imt-store-worker: {e}");
        std::process::exit(1);
    }
}
