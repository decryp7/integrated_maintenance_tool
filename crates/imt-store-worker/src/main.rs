//! The IMT store worker. IMT starts it and tells it, on stdin, its private endpoint and a one-time token; it
//! connects, serves the store protocol (`imt_store_worker::protocol`, ADR 0017 frames) and exits when IMT closes
//! the connection or stdin (IMT is gone).

fn main() {
    if let Err(e) = imt_store_worker::server::run() {
        eprintln!("imt-store-worker: {e}");
        std::process::exit(1);
    }
}
