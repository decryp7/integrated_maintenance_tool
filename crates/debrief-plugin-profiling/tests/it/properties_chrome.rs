//! Property-based tests: the Chrome importers (`.cpuprofile` and trace
//! event JSON) survive arbitrary, mutated and structurally random input
//! (never panic, never hang) and keep their sniff scores within 0..=100.
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p debrief-plugin-profiling --no-default-features --test it properties_chrome::`.

use debrief_plugin_profiling::chrome::{CpuProfileImporter, TraceEventImporter};
use debrief_profile::{CaptureImporter, Limits};
use proptest::prelude::*;
use std::path::Path;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

const CASES: u32 = 256;
const HANG_LIMIT: Duration = Duration::from_secs(15);
const SEEDS: &[&str] = &[
    "samples/profiling/orders.cpuprofile",
    "samples/profiling/orders.trace.json",
    "fuzz/corpus/fuzz_prof_cpuprofile",
    "fuzz/corpus/fuzz_prof_chrome_trace",
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

/// A random node list (ids may repeat, children and parents may dangle or loop).
fn nodes() -> impl Strategy<Value = String> {
    let node = (
        0u64..8,
        prop::sample::select(vec!["(root)", "(program)", "(idle)", "(garbage collector)", "", "f", "g"]),
        prop::sample::select(vec!["", "file:///a.js", "node:internal/x", "/n/node_modules/p.js", "http://h/"]),
        -2i64..50,
        prop::collection::vec(0u64..9, 0..3),
        prop::option::of(0u64..9),
    );
    prop::collection::vec(node, 0..10).prop_map(|ns| {
        let ns: Vec<String> = ns
            .iter()
            .map(|(id, f, url, line, children, parent)| {
                let parent = parent.map(|p| format!(r#","parent":{p}"#)).unwrap_or_default();
                format!(
                    r#"{{"id":{id},"callFrame":{{"functionName":"{f}","url":"{url}","lineNumber":{line},"scriptId":0}},"children":{children:?}{parent},"hitCount":{line}}}"#
                )
            })
            .collect();
        format!("[{}]", ns.join(","))
    })
}

fn samples() -> impl Strategy<Value = (Vec<u64>, Vec<i64>)> {
    (prop::collection::vec(0u64..9, 0..20), prop::collection::vec(-50i64..500, 0..21))
}

/// Structurally valid `.cpuprofile` and trace JSON with random contents.
fn structured() -> impl Strategy<Value = Vec<u8>> {
    let cpuprofile = (nodes(), samples(), -5i64..5, -5i64..10_000).prop_map(|(nodes, (s, d), start, end)| {
        format!(r#"{{"nodes":{nodes},"startTime":{start},"endTime":{end},"samples":{s:?},"timeDeltas":{d:?}}}"#).into_bytes()
    });
    let event = (
        prop::sample::select(vec!["X", "B", "E", "i", "I", "P", "M", "b"]),
        prop::sample::select(vec!["Profile", "ProfileChunk", "CpuProfile", "MinorGC", "thread_name", "task", ""]),
        -10i64..1000,
        prop::option::of(-10i64..500),
        0u8..3,
        0u8..3,
        0u8..3,
        prop::option::of((nodes(), samples())),
    );
    let trace = (prop::collection::vec(event, 0..16), prop::bool::ANY).prop_map(|(events, array)| {
        let events: Vec<String> = events
            .iter()
            .map(|(ph, name, ts, dur, pid, tid, id, chunk)| {
                let dur = dur.map(|d| format!(r#","dur":{d}"#)).unwrap_or_default();
                let args = match chunk {
                    Some((nodes, (s, d))) => format!(
                        r#","args":{{"name":"t{tid}","data":{{"startTime":{ts},"cpuProfile":{{"nodes":{nodes},"samples":{s:?}}},"timeDeltas":{d:?}}}}}"#
                    ),
                    None => format!(r#","args":{{"name":"t{tid}"}}"#),
                };
                format!(r#"{{"ph":"{ph}","name":"{name}","ts":{ts}{dur},"pid":{pid},"tid":{tid},"id":"0x{id}"{args}}}"#)
            })
            .collect();
        let events = events.join(",");
        if array {
            format!("[{events}").into_bytes()
        } else {
            format!(r#"{{"traceEvents":[{events}]}}"#).into_bytes()
        }
    });
    prop_oneof![cpuprofile, trace]
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
    let head = &bytes[..bytes.len().min(64 * 1024)];
    for (name, imp) in [
        ("p.cpuprofile", &CpuProfileImporter as &dyn CaptureImporter),
        ("t.json", &TraceEventImporter),
    ] {
        let score = imp.sniff(name, head);
        assert!(score <= 100, "{} sniff score {score}", imp.id());
        // Import must be safe even when sniff says no (the user can force the format).
        let _ = imp.import(name, bytes, &limits());
    }
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
            let cpu = CpuProfileImporter.import("p.cpuprofile", &s, &limits());
            let trace = TraceEventImporter.import("t.json", &s, &limits());
            assert!(cpu.is_ok() || trace.is_ok(), "{cpu:?} / {trace:?}");
        });
    }
}
