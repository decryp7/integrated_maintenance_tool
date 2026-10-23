//! The Debrief inference worker. Debrief starts it and tells it, on stdin, a private local address and a one-time
//! token; it connects back over that Unix socket or named pipe, serves the frame protocol (`debrief_ipc`, ADR 0017)
//! and runs inference in a child `llama-server` (the official prebuilt llama.cpp build, ADR 0009) on 127.0.0.1. It exits when stdin
//! closes (Debrief is gone), taking llama-server with it, and only reads the model file it is told to load.
//!
//! `debrief-ai-worker --list-builds` prints the installed inference builds and whether each works here (JSON).

use debrief_ai_worker::locate::LlamaConfig;
use debrief_ai_worker::server::ServerBackend;
use std::time::Duration;

fn lower_priority() {
    #[cfg(unix)]
    // SAFETY: setpriority takes plain integers and has no memory effects.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
    // On Windows Debrief starts the worker with BELOW_NORMAL_PRIORITY_CLASS.
}

fn main() {
    lower_priority();
    let cfg = LlamaConfig::from_env();
    if std::env::args().nth(1).as_deref() == Some("--list-builds") {
        let report = debrief_ai_worker::listing::report(&cfg, Duration::from_secs(30));
        match serde_json::to_string(&report) {
            Ok(json) => println!("{json}"),
            Err(e) => {
                eprintln!("debrief-ai-worker: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Err(e) = debrief_ai::worker::run_worker(move || Box::new(ServerBackend::new(cfg))) {
        eprintln!("debrief-ai-worker: {e}");
        std::process::exit(1);
    }
}
