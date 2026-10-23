//! Property-based tests: the V8 heap snapshot importer survives arbitrary,
//! mutated and structurally random input (never panics, never hangs) and
//! keeps its sniff score within 0..=100.
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p debrief-plugin-profiling --no-default-features --test properties_heapsnapshot`.

use debrief_plugin_profiling::heapsnapshot::HeapSnapshotImporter;
use debrief_profile::{CaptureImporter, Limits};
use proptest::prelude::*;
use std::path::Path;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

const CASES: u32 = 256;
const HANG_LIMIT: Duration = Duration::from_secs(15);
const SEEDS: &[&str] = &["samples/profiling/heap-1.heapsnapshot", "fuzz/corpus/fuzz_prof_heapsnapshot"];

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

/// The standard V8 meta with random node and edge arrays (indices may be out of range,
/// edge counts may not add up, `to_node` may not be divisible).
fn structured() -> impl Strategy<Value = Vec<u8>> {
    (
        prop::collection::vec(0u32..16, 0..70),
        prop::collection::vec(prop_oneof![0u32..10, (0u32..10).prop_map(|n| n * 7)], 0..60),
        prop::option::of(0u64..12),
        prop::option::of(0u64..20),
        prop::collection::vec("[a-z()]{0,6}", 0..12),
    )
        .prop_map(|(nodes, edges, node_count, edge_count, strings)| {
            let counts = match (node_count, edge_count) {
                (Some(n), Some(e)) => format!(r#","node_count":{n},"edge_count":{e}"#),
                (Some(n), None) => format!(r#","node_count":{n}"#),
                _ => String::new(),
            };
            format!(
                r#"{{"snapshot":{{"meta":{{"node_fields":["type","name","id","self_size","edge_count","trace_node_id","detachedness"],
"node_types":[["hidden","array","string","object","code","closure","regexp","number","native","synthetic"],"string","number","number","number","number","number"],
"edge_fields":["type","name_or_index","to_node"],"edge_types":[["context","element","property","internal","hidden","shortcut","weak"],"string_or_number","node"]}}{counts}}},
"nodes":{nodes:?},"edges":{edges:?},"strings":{}}}"#,
                serde_json::to_string(&strings).unwrap()
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
        max_objects: 50_000,
        max_edges: 200_000,
        ..Limits::default()
    }
}

fn exercise(bytes: &[u8]) {
    let imp = HeapSnapshotImporter;
    let score = imp.sniff("h.heapsnapshot", &bytes[..bytes.len().min(64 * 1024)]);
    assert!(score <= 100, "sniff score {score}");
    // Import must be safe even when sniff says no (the user can force the format).
    if let Ok(debrief_profile::Capture::Heap(h)) = imp.import("h.heapsnapshot", bytes, &limits()) {
        // The graph's invariants hold: edges and roots point at objects.
        let g = h.graph.expect("V8 snapshots have a graph");
        let n = g.len() as u32;
        assert!(g.edge_to.iter().chain(&g.roots).all(|&t| t < n));
        assert_eq!(g.edge_start.len(), g.len() + 1);
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
            HeapSnapshotImporter.import("h.heapsnapshot", &s, &limits()).unwrap();
        });
    }
}
