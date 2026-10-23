//! Headless process monitor: `debrief-monitor --config monitor.json --out ./monitor-logs [--duration 10m]`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn main() {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    if let Err(e) = ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed)) {
        eprintln!("warning: cannot install Ctrl+C handler: {e}");
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = debrief_monitor::cli::run(&args, &stop) {
        eprintln!("debrief-monitor: {e:#}");
        std::process::exit(1);
    }
}
