//! Property-based tests: the nettrace importer must survive arbitrary and
//! mutated input (never panic, never hang) and keep its sniff score within 0..=100.
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p imt-plugin-profiling --test properties_nettrace`.

use imt_plugin_profiling::nettrace::writer::{ids, payload as p, Meta, Row, TraceHeader, Writer, RUNDOWN, RUNTIME, SAMPLE_PROFILER};
use imt_plugin_profiling::nettrace::NettraceImporter;
use imt_profile::{Capture, CaptureImporter, Limits};
use proptest::prelude::*;
use std::path::Path;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

const CASES: u32 = 256;
const HANG_LIMIT: Duration = Duration::from_secs(15);

/// Runs `f` on a helper thread and fails if it panics or takes longer than [`HANG_LIMIT`].
fn guard<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
    });
    match rx.recv_timeout(HANG_LIMIT) {
        Ok(Ok(v)) => v,
        Ok(Err(p)) => std::panic::resume_unwind(p),
        Err(_) => panic!("hang: no result within {HANG_LIMIT:?}"),
    }
}

/// Byte-level mutations applied to a seed: flips, insertions, deletions, truncation, splices.
fn mutate(seed: &[u8], ops: &[(u8, usize, u8)]) -> Vec<u8> {
    let mut v = seed.to_vec();
    for &(kind, pos, byte) in ops {
        if v.is_empty() {
            v.push(byte);
            continue;
        }
        let i = pos % v.len();
        match kind % 6 {
            0 => v[i] ^= 1 << (byte % 8),
            1 => v[i] = byte,
            2 => v.insert(i, byte),
            3 => {
                v.remove(i);
            }
            4 => v.truncate(i),
            _ => {
                let j = (pos / 7 + byte as usize) % v.len();
                let chunk: Vec<u8> = v[j..(j + 16).min(v.len())].to_vec();
                for (k, b) in chunk.into_iter().enumerate() {
                    v.insert((i + k).min(v.len()), b);
                }
            }
        }
    }
    v
}

fn ops() -> impl Strategy<Value = Vec<(u8, usize, u8)>> {
    prop::collection::vec((any::<u8>(), any::<usize>(), any::<u8>()), 0..16)
}

/// A writer-generated trace touching every block type and event the importer reads.
fn generated(compressed: bool) -> Vec<u8> {
    let meta = |id, provider, event_id| Meta {
        id,
        provider,
        event_id,
        name: "",
    };
    let mut w = Writer::new(&TraceHeader::default());
    w.metadata(
        compressed,
        &[
            meta(1, SAMPLE_PROFILER, ids::THREAD_SAMPLE),
            meta(2, RUNDOWN, ids::METHOD_DC_END_VERBOSE),
            meta(3, RUNDOWN, ids::MODULE_DC_END),
            meta(4, RUNTIME, ids::GC_START),
            meta(5, RUNTIME, ids::GC_END),
            meta(6, RUNTIME, ids::EXCEPTION_THROWN),
            meta(7, RUNTIME, ids::CONTENTION_START),
            meta(8, RUNTIME, ids::CONTENTION_STOP),
            meta(9, RUNTIME, ids::METHOD_JITTING_STARTED),
            meta(10, RUNTIME, ids::METHOD_LOAD_VERBOSE),
        ],
    );
    w.stacks(1, &[vec![0x2010, 0x1010], vec![0x9999, 0x2000]]);
    let row = |metadata_id, thread, stack_id, ts, payload| Row {
        metadata_id,
        sequence: 1,
        thread,
        capture_thread: thread,
        stack_id,
        ts,
        payload,
        ..Row::default()
    };
    let t = TraceHeader::default().sync_qpc;
    w.events(
        compressed,
        &[
            row(9, 1, 0, t + 5, p::jitting_started(2, 7, "Shop.Orders.OrderService", "PlaceOrder")),
            row(
                10,
                1,
                0,
                t + 50,
                p::method_verbose(2, 7, 0x2000, 0x100, "Shop.Orders.OrderService", "PlaceOrder"),
            ),
            row(1, 1, 1, t + 100, p::sample(true)),
            row(1, 2, 2, t + 200, p::sample(false)),
            row(4, 1, 0, t + 300, p::gc_start(1, 0, 0)),
            row(5, 1, 0, t + 400, p::gc_end(1, 0)),
            row(6, 2, 2, t + 500, p::exception("System.InvalidOperationException", "nope")),
            row(7, 2, 1, t + 600, p::contention(false)),
            row(8, 2, 1, t + 700, p::contention(false)),
        ],
    );
    w.sequence_point(t + 800, &[(1, 3), (2, 4)]);
    w.events(
        compressed,
        &[
            row(3, 3, 0, t + 900, p::module_load(7, "/app/Shop.dll")),
            row(2, 3, 0, t + 900, p::method_verbose(1, 7, 0x1000, 0x100, "Shop.Program", "Main")),
        ],
    );
    w.finish()
}

/// The committed sample's first 64 KiB plus writer-generated traces.
fn seeds() -> &'static Vec<Vec<u8>> {
    static S: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    S.get_or_init(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/profiling/orders.nettrace");
        let mut sample = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        sample.truncate(64 * 1024);
        vec![sample, generated(true), generated(false)]
    })
}

/// Arbitrary bytes, a mutated seed, or a seed cut at an arbitrary point.
fn input() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..2048),
        prop::collection::vec(any::<u8>(), 0..512).prop_map(|tail| {
            let mut v = b"Nettrace\x14\0\0\0!FastSerialization.1".to_vec();
            v.extend(tail);
            v
        }),
        (any::<prop::sample::Index>(), ops()).prop_map(|(i, o)| {
            let s = seeds();
            mutate(&s[i.index(s.len())], &o)
        }),
        (any::<prop::sample::Index>(), any::<prop::sample::Index>()).prop_map(|(i, cut)| {
            let s = seeds();
            let seed = &s[i.index(s.len())];
            seed[..cut.index(seed.len() + 1)].to_vec()
        }),
    ]
}

fn small_limits() -> Limits {
    Limits {
        max_input_bytes: 4 << 20,
        max_frames: 20_000,
        max_stack_nodes: 50_000,
        max_samples: 200_000,
        ..Limits::default()
    }
}

fn exercise(bytes: &[u8]) {
    let score = NettraceImporter.sniff("fuzz.nettrace", &bytes[..bytes.len().min(64 * 1024)]);
    assert!(score <= 100, "sniff score {score}");
    // Import must be safe even when sniff says no (the user can force the format).
    if let Ok(Capture::Profile(p)) = NettraceImporter.import("fuzz.nettrace", bytes, &small_limits()) {
        assert_eq!(p.samples.values.len(), p.samples.len() * p.metrics.len());
        assert!(p.samples.time.windows(2).all(|w| w[0] <= w[1]), "samples sorted by time");
        for &s in &p.samples.stack {
            assert!(!p.stack_frames(s).is_empty());
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn nettrace_never_panics(bytes in input()) {
        guard(move || exercise(&bytes));
    }
}

#[test]
fn seeds_import_cleanly() {
    for s in seeds().iter().skip(1) {
        let c = NettraceImporter.import("seed.nettrace", s, &Limits::default()).unwrap();
        let Capture::Profile(p) = c else { panic!("not a profile") };
        assert!(!p.samples.is_empty());
    }
    // The sample's 64 KiB prefix is truncated (an error, not a panic) unless the sample is smaller.
    let prefix = &seeds()[0];
    assert_eq!(
        NettraceImporter.import("cut.nettrace", prefix, &Limits::default()).is_err(),
        prefix.len() == 64 * 1024
    );
}
