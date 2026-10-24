//! Property-based tests: the speedscope importer survives arbitrary, mutated
//! and structurally random input (never panics, never hangs) and keeps its
//! sniff score within 0..=100.
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p debrief-plugin-profiling --no-default-features --test it properties_speedscope::`.

use debrief_plugin_profiling::speedscope::SpeedscopeImporter;
use debrief_profile::{CaptureImporter, Limits};
use proptest::prelude::*;
use std::path::Path;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

const CASES: u32 = 256;
const HANG_LIMIT: Duration = Duration::from_secs(15);
const SEEDS: &[&str] = &[
    "samples/profiling/orders.speedscope.json",
    "samples/profiling/orders-evented.speedscope.json",
    "fuzz/corpus/fuzz_prof_speedscope",
];

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

/// Sample files and fuzz seeds (files of a directory), first 64 KiB of each.
fn seeds() -> &'static Vec<Vec<u8>> {
    static S: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    S.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut out = Vec::new();
        for p in SEEDS {
            let p = root.join(p);
            let files = if p.is_dir() {
                std::fs::read_dir(&p).unwrap().map(|e| e.unwrap().path()).collect()
            } else {
                vec![p]
            };
            for f in files {
                let mut b = std::fs::read(&f).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
                b.truncate(64 * 1024);
                out.push(b);
            }
        }
        out
    })
}

/// Structurally valid JSON with random indices, units, event kinds and times.
fn structured() -> impl Strategy<Value = Vec<u8>> {
    let unit = prop::sample::select(vec!["none", "nanoseconds", "microseconds", "milliseconds", "seconds", "bytes", "x"]);
    let event = (prop::sample::select(vec!["O", "C", "Z"]), 0u32..6, -5i64..200);
    let sample = prop::collection::vec(0u32..6, 0..8);
    (
        unit,
        prop::bool::ANY,
        prop::collection::vec(event, 0..40),
        prop::collection::vec(sample, 0..20),
        prop::collection::vec(-2.0f64..5.0, 0..22),
        -10i64..10,
        -10i64..300,
    )
        .prop_map(|(unit, evented, events, samples, weights, start, end)| {
            let frames = r#"[{"name":"a","file":"a.js","line":1},{"name":"b"},{"name":"","line":-3},{"name":"d","file":""},{"name":"e","line":1e20}]"#;
            let events: Vec<String> = events.iter().map(|(k, f, at)| format!(r#"{{"type":"{k}","frame":{f},"at":{at}}}"#)).collect();
            let kind = if evented { "evented" } else { "sampled" };
            format!(
                r#"{{"$schema":"https://www.speedscope.app/file-format-schema.json","shared":{{"frames":{frames}}},"profiles":[{{"type":"{kind}","name":"p","unit":"{unit}","startValue":{start},"endValue":{end},"events":[{}],"samples":{},"weights":{}}}]}}"#,
                events.join(","),
                serde_json::to_string(&samples).unwrap(),
                serde_json::to_string(&weights).unwrap(),
            )
            .into_bytes()
        })
}

/// Arbitrary bytes, a mutated seed, a seed prefix, or structured JSON.
fn input() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..2048),
        (any::<prop::sample::Index>(), ops()).prop_map(|(i, o)| {
            let s = seeds();
            mutate(&s[i.index(s.len())], &o)
        }),
        (any::<prop::sample::Index>(), any::<prop::sample::Index>()).prop_map(|(i, cut)| {
            let s = seeds();
            let seed = &s[i.index(s.len())];
            seed[..cut.index(seed.len() + 1)].to_vec()
        }),
        structured(),
    ]
}

fn limits() -> Limits {
    Limits {
        max_input_bytes: 4 << 20,
        max_frames: 20_000,
        max_stack_nodes: 50_000,
        max_samples: 200_000,
        max_stack_depth: 64,
        ..Limits::default()
    }
}

fn exercise(bytes: &[u8]) {
    let imp = SpeedscopeImporter;
    let score = imp.sniff("p.json", &bytes[..bytes.len().min(64 * 1024)]);
    assert!(score <= 100, "sniff score {score}");
    // Import must be safe even when sniff says no (the user can force the format).
    let _ = imp.import("p.json", bytes, &limits());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn never_panics(bytes in input()) {
        guard(move || exercise(&bytes));
    }
}

#[test]
fn seeds_import_cleanly() {
    for s in seeds() {
        let s = s.clone();
        guard(move || {
            SpeedscopeImporter.import("p.json", &s, &limits()).unwrap();
        });
    }
}
