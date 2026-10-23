//! Timing of the streaming import against reading a complete list first (ADR 0051), on a synthetic tree built here.
//! `cargo test -p debrief-core --release --test stream_bench -- --ignored --nocapture`; `DEBRIEF_BENCH_FILES` sets the
//! number of small files (default 100,000).

use anyhow::Result;
use debrief_core::batch::{EventId, EventSchema, PendingBatch, SourceId};
use debrief_core::importers::Importers;
use debrief_core::ingest::{
    find_files, import_files_parallel, import_stream, ImportEnv, NewSource, ParallelOptions, SharedSink, SourceReport, StreamOptions,
    StreamSpec,
};
use debrief_core::parser::ParserSet;
use debrief_core::profile::CompiledProfile;
use debrief_core::workspace::ImportProgress;
use debrief_core::Profile;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LINE: &str = "2026-10-14 10:00:00 INFO synthetic line\n";

struct Clock {
    start: Instant,
    first_event: Mutex<Option<Duration>>,
    next_source: AtomicU32,
    next_id: AtomicU64,
}

impl SharedSink for Clock {
    fn reserve_sources(&self, n: u32) -> Result<SourceId> {
        Ok(SourceId(self.next_source.fetch_add(n, Ordering::SeqCst)))
    }
    fn begin_source(&self, _: &NewSource, id: Option<SourceId>) -> Result<SourceId> {
        Ok(id.unwrap_or_else(|| SourceId(self.next_source.fetch_add(1, Ordering::SeqCst))))
    }
    fn write(&self, batch: PendingBatch) -> Result<()> {
        self.first_event.lock().unwrap().get_or_insert_with(|| self.start.elapsed());
        let first = self.next_id.fetch_add(batch.len() as u64, Ordering::SeqCst);
        batch.assign_ids(EventId(first))?;
        Ok(())
    }
    fn finish_source(&self, _: &SourceReport) -> Result<()> {
        Ok(())
    }
    fn abort_source(&self, _: SourceId) -> Result<()> {
        Ok(())
    }
}

#[test]
#[ignore = "timing run"]
fn first_event_and_total_time_of_the_streaming_import_against_gather_first() {
    let n: usize = std::env::var("DEBRIEF_BENCH_FILES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);
    let root = std::env::temp_dir().join(format!("debrief-stream-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for i in 0..n {
        let d = root.join(format!("a{}/b{}/c{}", i % 20, (i / 20) % 20, (i / 400) % 50));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("f{i}.log")), LINE.repeat(3)).unwrap();
    }
    std::fs::create_dir_all(root.join("big")).unwrap();
    for i in 0..3 {
        std::fs::write(root.join(format!("big/big{i}.log")), LINE.repeat(300_000)).unwrap();
    }
    let root = std::fs::canonicalize(root).unwrap();
    let importers = Importers::default();
    let profile = CompiledProfile::compile(Profile::default_profile()).unwrap();
    let parsers = ParserSet::of_compiled(CompiledProfile::compile(Profile::default_profile()).unwrap());
    let roots = vec![root.clone()];
    let threads = 7;
    for round in 0..2 {
        // Gather first (before): the whole sorted list, then the sizes and archive entries, then the reading.
        let progress = ImportProgress::default();
        let _run = progress.begin();
        let env = ImportEnv {
            encodings: &Default::default(),
            importers: &importers,
            parsers: &parsers,
            schema: Arc::new(EventSchema::plain()),
            cancel: progress.cancel_flag(),
            window: None,
        };
        let sink = Clock {
            start: Instant::now(),
            first_event: Mutex::new(None),
            next_source: AtomicU32::new(0),
            next_id: AtomicU64::new(0),
        };
        let files = find_files(&roots, &roots, &profile, &HashSet::new(), &HashSet::new(), &progress, 14);
        let opts = ParallelOptions {
            threads,
            ..ParallelOptions::default()
        };
        import_files_parallel(&env, &files, &HashSet::new(), &progress, &opts, &sink);
        eprintln!(
            "round {round} gather-first: first event {:?}, total {:?}",
            sink.first_event.lock().unwrap().unwrap(),
            sink.start.elapsed()
        );
        drop(_run);
        // Streaming (after).
        let progress = ImportProgress::default();
        let _run = progress.begin();
        let env = ImportEnv {
            cancel: progress.cancel_flag(),
            ..env
        };
        let sink = Clock {
            start: Instant::now(),
            first_event: Mutex::new(None),
            next_source: AtomicU32::new(0),
            next_id: AtomicU64::new(0),
        };
        let (removed, known) = (HashSet::new(), HashSet::new());
        let spec = StreamSpec {
            paths: &roots,
            roots: &roots,
            profile: &profile,
            removed: &removed,
            known: &known,
        };
        let opts = StreamOptions {
            threads,
            walkers: 14,
            ..StreamOptions::default()
        };
        import_stream(&env, &spec, &removed, &progress, &opts, &sink);
        eprintln!(
            "round {round} streaming:    first event {:?}, total {:?}",
            sink.first_event.lock().unwrap().unwrap(),
            sink.start.elapsed()
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
