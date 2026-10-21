//! The IMT store worker. IMT starts it and tells it, on stdin, a private local address and a one-time token;
//! it then serves the store protocol (`proto/store.proto`) there and exits when stdin closes (IMT is gone).

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    if let Err(e) = imt_store_worker::server::run() {
        eprintln!("imt-store-worker: {e}");
        std::process::exit(1);
    }
}
