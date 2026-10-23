//! Property-based tests: the pprof importer must survive arbitrary and
//! mutated input (never panic, never hang), keep its sniff score within
//! 0..=100 and only produce well-formed profiles.
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p debrief-plugin-profiling --no-default-features --test properties_pprof`.

use debrief_plugin_profiling::pprof;
use debrief_profile::{Capture, CaptureImporter, Limits};
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

/// Sample files (first 64 KiB of each) and small valid inputs used as mutation seeds.
fn seeds() -> &'static Vec<Vec<u8>> {
    static S: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    S.get_or_init(|| {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/profiling");
        let mut out: Vec<Vec<u8>> = ["orders.pb.gz", "orders-alloc.pb.gz"]
            .iter()
            .map(|n| {
                let p = base.join(n);
                let mut b = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                b.truncate(64 * 1024);
                b
            })
            .collect();
        out.extend(extra_seeds());
        out
    })
}

/// Raw (uncompressed) profiles: mutations then hit the protobuf decoder directly.
fn extra_seeds() -> Vec<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::new();
    for s in seeds_gz() {
        let mut raw = Vec::new();
        if flate2::read::GzDecoder::new(&s[..]).read_to_end(&mut raw).is_ok() {
            out.push(raw);
        }
    }
    out
}

fn seeds_gz() -> Vec<Vec<u8>> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/profiling");
    ["orders.pb.gz", "orders-alloc.pb.gz"]
        .iter()
        .filter_map(|n| std::fs::read(base.join(n)).ok())
        .collect()
}

/// Arbitrary bytes, or a mutated seed.
fn input() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..2048),
        (any::<prop::sample::Index>(), ops()).prop_map(|(i, o)| {
            let s = seeds();
            mutate(&s[i.index(s.len())], &o)
        }),
        // Seed prefix cut at an arbitrary point.
        (any::<prop::sample::Index>(), any::<prop::sample::Index>()).prop_map(|(i, cut)| {
            let s = seeds();
            let seed = &s[i.index(s.len())];
            seed[..cut.index(seed.len() + 1)].to_vec()
        }),
    ]
}

/// Small limits, like the fuzz targets, so limit paths are exercised too.
fn limits() -> Limits {
    Limits {
        max_input_bytes: 4 << 20,
        max_frames: 2_000,
        max_stack_nodes: 5_000,
        max_samples: 20_000,
        max_stack_depth: 64,
        ..Limits::default()
    }
}

fn exercise(bytes: &[u8]) {
    let imp = pprof::PprofImporter;
    for name in ["fuzz", "cpu.pb.gz", "heap.pprof"] {
        let score = imp.sniff(name, &bytes[..bytes.len().min(64 * 1024)]);
        assert!(score <= 100, "sniff score {score}");
    }
    // Import must be safe even when sniff says no (the user can force the format).
    if let Ok(Capture::Profile(p)) = imp.import("fuzz", bytes, &limits()) {
        assert_eq!(p.samples.values.len(), p.samples.len() * p.metrics.len());
        assert!(p.samples.stack.iter().all(|&s| (s as usize) < p.stacks.len()));
        assert!(p.samples.thread.iter().all(|&t| (t as usize) < p.threads.len()));
        assert!(p.stacks.iter().all(|n| (n.frame as usize) < p.frames.len()));
        assert!(p
            .stacks
            .iter()
            .all(|n| n.parent == debrief_profile::NONE || (n.parent as usize) < p.stacks.len()));
        if p.has_time {
            assert!(p.samples.time.windows(2).all(|w| w[0] <= w[1]));
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn importer_never_panics(bytes in input()) {
        guard(move || exercise(&bytes));
    }

    #[test]
    fn importer_never_panics_on_arbitrary_text(text in any::<String>()) {
        guard(move || exercise(text.as_bytes()));
    }
}

#[test]
fn seeds_import_cleanly() {
    for s in seeds() {
        let bytes = s.clone();
        guard(move || exercise(&bytes));
    }
}
