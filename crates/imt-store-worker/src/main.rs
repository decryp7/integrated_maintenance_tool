//! The IMT store worker. IMT starts it and tells it, on stdin, its private endpoint and a one-time token; it
//! connects, serves the store protocol (`imt_store_client::protocol`, ADR 0017 frames) and exits when IMT closes
//! the connection or stdin (IMT is gone).
//!
//! Importer plugins run in their own processes (ADR 0017), started by this worker on first use (the built-ins and
//! the installed third-party plugins IMT names, ADR 0027), so events go from a plugin to the store without passing
//! through IMT.

#[global_allocator]
static GLOBAL: imt_alloc::MiMalloc = imt_alloc::MiMalloc;

fn main() {
    imt_alloc::tune();
    imt_store_worker::main();
}
