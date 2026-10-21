//! Property-based tests: every importer of this plugin must survive arbitrary and
//! mutated input (never panic, never hang) and keep its sniff score within 0..=100.
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p imt-plugin-registry --test properties`.

use imt_plugin_api::importer::{ImportCtx, TextImporter};
use imt_plugin_api::time::TimestampParser;
use proptest::prelude::*;
use std::path::{Path, PathBuf};
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

/// Fixture files (first 64 KiB of each) used as mutation seeds.
fn seeds() -> &'static Vec<Vec<u8>> {
    static S: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    S.get_or_init(|| {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"));
        let paths: Vec<PathBuf> = ["../../samples/windows/registry-export.reg"].iter().map(|p| base.join(p)).collect();
        let mut out: Vec<Vec<u8>> = paths
            .iter()
            .map(|p| {
                let mut b = std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                b.truncate(64 * 1024);
                b
            })
            .collect();
        out.extend([
            "Windows Registry Editor Version 5.00\n\n[HKEY_LOCAL_MACHINE\\SOFTWARE\\X]\n\"a\"=dword:00000001\n\"b\"=hex:01,02,\\\n  03\n@=\"d\"\n\"c\"=hex(7):41,00,00,00\n[-HKEY_X\\Y]\n",
        ].iter().map(|s: &&str| s.as_bytes().to_vec()));
        out
    })
}

/// Arbitrary bytes, or a mutated fixture.
fn input() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..2048),
        (any::<prop::sample::Index>(), ops()).prop_map(|(i, o)| {
            let s = seeds();
            mutate(&s[i.index(s.len())], &o)
        }),
        // Fixture prefix cut at an arbitrary point.
        (any::<prop::sample::Index>(), any::<prop::sample::Index>()).prop_map(|(i, cut)| {
            let s = seeds();
            let seed = &s[i.index(s.len())];
            seed[..cut.index(seed.len() + 1)].to_vec()
        }),
    ]
}

fn timestamps() -> &'static TimestampParser {
    static P: OnceLock<TimestampParser> = OnceLock::new();
    P.get_or_init(TimestampParser::default)
}

fn ctx() -> ImportCtx<'static> {
    ImportCtx::new(Path::new("fuzz"), timestamps())
}

fn exercise_text(imp: &dyn TextImporter, bytes: &[u8]) {
    let Some(text) = imt_plugin_api::text::decode(bytes) else { return };
    let c = ctx();
    let head: String = text.chars().take(64 * 1024).collect();
    let score = imp.sniff(&c, &head);
    assert!(score <= 100, "{} sniff score {score}", imp.id());
    // Import must be safe even when sniff says no (the user can force the format).
    if let Ok(imported) = imp.import(&c, &text) {
        for r in &imported.records {
            let _ = serde_json::to_string(&r.fields).unwrap();
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn importers_never_panic(bytes in input()) {
        guard(move || {
            exercise_text(&imt_plugin_registry::RegistryImporter, &bytes);
        });
    }

    #[test]
    fn importers_never_panic_on_arbitrary_text(text in any::<String>()) {
        guard(move || {
            let bytes = text.into_bytes();
            exercise_text(&imt_plugin_registry::RegistryImporter, &bytes);
        });
    }
}

#[test]
fn fixtures_import_cleanly() {
    for s in seeds() {
        let bytes = s.clone();
        exercise_text(&imt_plugin_registry::RegistryImporter, &bytes);
    }
}
