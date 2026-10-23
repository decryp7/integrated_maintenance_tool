//! The shared, bounded pool of the pipelined analyses (patterns, exception groups): `cores - 1` threads at a lower
//! priority, built once, so several analyses at once share it instead of each starting its own.

use debrief_core::stages::{Stages, DEFAULT_CHUNK};
use debrief_core::threads::{lower_priority, threads, WorkKind};
use std::sync::OnceLock;

fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads(WorkKind::Cpu, None))
            .thread_name(|i| format!("debrief-mine-{i}"))
            .start_handler(|_| {
                lower_priority();
            })
            .build()
            .unwrap_or_else(|_| rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("a one-thread pool"))
    })
}

/// The pipeline of a mining analysis: read in order on the caller's thread, work on the shared pool, commit in order.
pub(crate) fn stages() -> Stages<'static> {
    Stages {
        pool: pool(),
        chunk: DEFAULT_CHUNK,
        cancel: None,
    }
}
