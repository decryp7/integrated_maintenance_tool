//! `debrief-agent`: headless scheduled diagnostics collection. See `debrief-agent --help`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    // Ctrl+C and SIGTERM (systemd, launchd) stop the agent after the running actions wind down.
    let _ = ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed));
    let mut out = std::io::stdout();
    if let Err((code, message)) = debrief_agent::cli::run(&args, &mut out, stop) {
        eprintln!("debrief-agent: {message}");
        std::process::exit(code);
    }
}
