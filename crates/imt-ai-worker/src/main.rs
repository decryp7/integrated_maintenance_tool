//! The IMT inference worker. IMT starts it and tells it, on stdin, a private local address and a one-time
//! token; it then serves gRPC (`proto/llm.proto`) on that Unix socket or named pipe. It exits when stdin
//! closes (IMT is gone), never touches the network, and only reads the model file it is told to load. Running llama.cpp here keeps a native crash out of IMT.

mod llama;

fn lower_priority() {
    #[cfg(unix)]
    // SAFETY: setpriority takes plain integers and has no memory effects.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
    // On Windows IMT starts the worker with BELOW_NORMAL_PRIORITY_CLASS.
}

fn main() {
    lower_priority();
    if let Err(e) = imt_ai::worker::run_worker(|| Box::new(llama::LlamaBackend::new())) {
        eprintln!("imt-ai-worker: {e}");
        std::process::exit(1);
    }
}
