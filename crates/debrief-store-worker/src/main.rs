//! The Debrief store worker. Debrief starts it and tells it, on stdin, its private endpoint and a one-time token; it
//! connects, serves the store protocol (`debrief_store_client::protocol`, ADR 0017 frames) and exits when Debrief closes
//! the connection or stdin (Debrief is gone).
//!
//! Importer plugins run in their own processes (ADR 0017), started by this worker on first use (the built-ins and
//! the installed third-party plugins Debrief names, ADR 0027), so events go from a plugin to the store without passing
//! through Debrief.

#[global_allocator]
static GLOBAL: debrief_alloc::MiMalloc = debrief_alloc::MiMalloc;

fn main() {
    debrief_alloc::tune();
    debrief_store_worker::main();
}
