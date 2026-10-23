//! Property-based tests: every importer of this plugin must survive arbitrary and
//! mutated input (never panic, never hang) and keep its sniff score within 0..=100.
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p debrief-plugin-windows --test properties`.

use debrief_plugin_api::importer::{BinaryImporter, ImportCtx, TextImporter};
use debrief_plugin_api::time::TimestampParser;
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
        let paths: Vec<PathBuf> = [
            "tests/fixtures/msinfo.nfo",
            "tests/fixtures/perfmon.csv",
            "tests/fixtures/procmon.csv",
            "tests/fixtures/security.evtx",
            "tests/fixtures/crash.dmp",
            "../../samples/csv/perfmon-export.csv",
        ]
        .iter()
        .map(|p| base.join(p))
        .collect();
        let mut out: Vec<Vec<u8>> = paths
            .iter()
            .map(|p| {
                let mut b = std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                b.truncate(64 * 1024);
                b
            })
            .collect();
        out.extend(Vec::<&str>::new().iter().map(|s: &&str| s.as_bytes().to_vec()));
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
    let Some(text) = debrief_plugin_api::text::decode(bytes) else {
        return;
    };
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

fn exercise_binary(imp: &dyn BinaryImporter, bytes: &[u8]) {
    let c = ctx();
    let score = imp.sniff(&c, &bytes[..bytes.len().min(4096)]);
    assert!(score <= 100, "{} sniff score {score}", imp.id());
    if let Ok(imported) = imp.import(&c, bytes) {
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
            exercise_text(&debrief_plugin_windows::NfoImporter, &bytes);
            exercise_text(&debrief_plugin_windows::PerfmonCsvImporter, &bytes);
            exercise_text(&debrief_plugin_windows::ProcmonCsvImporter, &bytes);
            procmon_with_file_date(&bytes);
            exercise_binary(&debrief_plugin_windows::EvtxImporter, &bytes);
            exercise_binary(&debrief_plugin_windows::MinidumpImporter, &bytes);
            // BLG import shells out to `relog` on Windows; only sniff there.
            #[cfg(not(windows))]
            exercise_binary(&debrief_plugin_windows::BlgImporter::default(), &bytes);
        });
    }

    #[test]
    fn importers_never_panic_on_arbitrary_text(text in any::<String>()) {
        guard(move || {
            let bytes = text.into_bytes();
            exercise_text(&debrief_plugin_windows::NfoImporter, &bytes);
            exercise_text(&debrief_plugin_windows::PerfmonCsvImporter, &bytes);
            exercise_text(&debrief_plugin_windows::ProcmonCsvImporter, &bytes);
            procmon_with_file_date(&bytes);
            exercise_binary(&debrief_plugin_windows::EvtxImporter, &bytes);
            exercise_binary(&debrief_plugin_windows::MinidumpImporter, &bytes);
            // BLG import shells out to `relog` on Windows; only sniff there.
            #[cfg(not(windows))]
            exercise_binary(&debrief_plugin_windows::BlgImporter::default(), &bytes);
        });
    }
}

#[test]
fn fixtures_import_cleanly() {
    for s in seeds() {
        let bytes = s.clone();
        exercise_text(&debrief_plugin_windows::NfoImporter, &bytes);
        exercise_text(&debrief_plugin_windows::PerfmonCsvImporter, &bytes);
        exercise_text(&debrief_plugin_windows::ProcmonCsvImporter, &bytes);
        procmon_with_file_date(&bytes);
        exercise_binary(&debrief_plugin_windows::EvtxImporter, &bytes);
        exercise_binary(&debrief_plugin_windows::MinidumpImporter, &bytes);
        // BLG import shells out to `relog` on Windows; only sniff there.
        #[cfg(not(windows))]
        exercise_binary(&debrief_plugin_windows::BlgImporter::default(), &bytes);
    }
}

/// The ProcMon importer with a file date, so time-of-day rows exercise the date rollover.
fn procmon_with_file_date(bytes: &[u8]) {
    let Some(text) = debrief_plugin_api::text::decode(bytes) else {
        return;
    };
    let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 14);
    if let Ok(out) = debrief_plugin_windows::procmon::parse_procmon(&text, timestamps(), date) {
        for r in &out.records {
            let _ = serde_json::to_string(&r.fields).unwrap();
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    /// Header-valid ProcMon files with arbitrary cells: every row path is reached.
    #[test]
    fn procmon_rows_never_panic(rows in prop::collection::vec(prop::collection::vec(".{0,24}", 0..9), 0..40)) {
        guard(move || {
            let mut text = String::from("Time of Day,Date & Time,Process Name,PID,Operation,Path,Result,Duration,TID\n");
            for row in rows {
                let cells: Vec<String> = row.iter().map(|c| format!("\"{}\"", c.replace('"', "\"\""))).collect();
                text.push_str(&cells.join(","));
                text.push('\n');
            }
            procmon_with_file_date(text.as_bytes());
            exercise_text(&debrief_plugin_windows::ProcmonCsvImporter, text.as_bytes());
        });
    }
}

#[path = "support/synth_minidump.rs"]
mod synth_minidump;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    /// Structurally valid minidumps with arbitrary values (stack words, registers, module
    /// layout, exception code and parameters) reach the stack scan and module lookups.
    #[test]
    fn minidumps_with_arbitrary_values_never_panic(
        arch in prop_oneof![Just(synth_minidump::ARCH_AMD64), Just(synth_minidump::ARCH_X86), any::<u16>()],
        words in prop::collection::vec(any::<u64>(), 0..64),
        ip in any::<u64>(),
        sp in any::<u64>(),
        stack_base in any::<u64>(),
        modules in prop::collection::vec((any::<u64>(), any::<u32>(), ".{0,40}"), 0..8),
        code in any::<u32>(),
        params in prop::collection::vec(any::<u64>(), 0..20),
        thread in any::<u32>(),
        times in (any::<u32>(), any::<u32>()),
    ) {
        guard(move || {
            use synth_minidump::*;
            let ctx = if arch == ARCH_X86 { x86_context(ip as u32, sp as u32) } else { amd64_context(ip, sp) };
            let mods: Vec<synth_minidump::ModuleSpec> =
                modules.iter().map(|m| (m.0, m.1, m.2.as_str(), Some((1, 0, 0, 0)))).collect();
            let dump = DumpBuilder::new()
                .system_info(arch, (10, 0, 1), 4, "")
                .misc_info(1, times.0)
                .module_list(&mods)
                .thread_list(thread, stack_base, &stack64(&words), &ctx)
                .exception(thread, code, ip, &params, &ctx)
                .finish(times.1);
            exercise_binary(&debrief_plugin_windows::MinidumpImporter, &dump);
        });
    }
}
