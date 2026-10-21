//! The IMT store worker. IMT starts it and tells it, on stdin, its private endpoint and a one-time token; it
//! connects, serves the store protocol (`imt_store_client::protocol`, ADR 0017 frames) and exits when IMT closes
//! the connection or stdin (IMT is gone).

fn main() {
    let registry: imt_store_worker::server::RegistryFactory = std::sync::Arc::new(|_| imt_core::PluginRegistry::new());
    if let Err(e) = imt_store_worker::server::run(registry) {
        eprintln!("imt-store-worker: {e}");
        std::process::exit(1);
    }
}
