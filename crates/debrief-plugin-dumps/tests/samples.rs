//! The committed dump samples and fuzz seeds come from the synthetic builder (`debrief_plugin_dumps::synth`); the
//! UMDH samples from `samples/generate.py`. Regenerate the dumps with
//! `DEBRIEF_REGENERATE_SAMPLES=1 cargo test -p debrief-plugin-dumps --test samples -- --ignored regenerate_` (or
//! `cargo xtask gen-samples`); a normal run fails while a committed file is stale.

use debrief_dump::model::{Document, DumpKind, UmdhKind};
use debrief_dump::{ImportOptions, Limits};
use debrief_plugin_dumps::synth;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every generated file: its path from the repository root and its bytes.
fn generated() -> Vec<(PathBuf, Vec<u8>)> {
    let umdh_seed = b"*- - - - - - - - - - Start of data for heap @ 00150000 - - - - - - - - - -\r\n\r\n00000100 bytes + 00000010 at 00157C40 by BackTrace00A1\r\n\tntdll!RtlAllocateHeap+00000021\r\n\tShop!Cache::Add+1D (c:\\src\\cache.cpp, 42)\r\n\t7FF600001100\r\n".to_vec();
    let diff_seed = b"// + BYTES_DELTA (NEW_BYTES - OLD_BYTES) NEW_COUNT allocs BackTrace TRACEID\r\n+   80000 (  80000 -      0)      1 allocs\tBackTrace4920\r\n+       1 (      1 -      0)\tBackTrace4920\tallocations\r\n\r\n\tntdll!RtlAllocateHeap+00000021\r\n\r\nTotal increase == 80000 requested + 30 overhead = 80030\r\n".to_vec();
    vec![
        (PathBuf::from("samples/dumps/Billing.exe.900.hang.dmp"), synth::sample_hang_x86()),
        (
            PathBuf::from("samples/dumps/OrderService.exe.4120.full.dmp"),
            synth::sample_full(16 << 10),
        ),
        (PathBuf::from("fuzz/corpus/fuzz_dump_minidump/crash.dmp"), synth::sample_crash()),
        (
            PathBuf::from("fuzz/corpus/fuzz_dump_minidump/hang-x86.dmp"),
            synth::sample_hang_x86(),
        ),
        (PathBuf::from("fuzz/corpus/fuzz_dump_minidump/full.dmp"), synth::sample_full(1024)),
        (PathBuf::from("fuzz/corpus/fuzz_dump_umdh/snapshot.txt"), umdh_seed),
        (PathBuf::from("fuzz/corpus/fuzz_dump_umdh/diff.txt"), diff_seed),
        (
            PathBuf::from("fuzz/corpus/fuzz_dump_wire/dump.bin"),
            debrief_dump::wire::encode_result(&Ok(Document::Dump(debrief_dump::testing::sample_dump()))),
        ),
        (
            PathBuf::from("fuzz/corpus/fuzz_dump_wire/umdh.bin"),
            debrief_dump::wire::encode_result(&Ok(Document::Umdh(debrief_dump::testing::sample_umdh(
                UmdhKind::Diff,
                0,
                &[(1, 100, 2)],
            )))),
        ),
    ]
}

#[test]
#[ignore = "writes samples/dumps/*.dmp and the dump fuzz seeds; run with DEBRIEF_REGENERATE_SAMPLES=1"]
fn regenerate_samples() {
    if std::env::var_os("DEBRIEF_REGENERATE_SAMPLES").is_none() {
        eprintln!("set DEBRIEF_REGENERATE_SAMPLES=1 to write the samples");
        return;
    }
    for (path, bytes) in generated() {
        let p = root().join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
    }
}

#[test]
fn committed_samples_match_their_generator() {
    for (path, bytes) in generated() {
        let on_disk = std::fs::read(root().join(&path)).unwrap_or_default();
        assert!(
            on_disk == bytes,
            "{} is stale: run DEBRIEF_REGENERATE_SAMPLES=1 cargo test -p debrief-plugin-dumps --test samples -- --ignored regenerate_",
            path.display()
        );
    }
}

fn options() -> ImportOptions {
    ImportOptions {
        limits: Limits::default(),
        ..ImportOptions::default()
    }
}

#[test]
fn every_dump_sample_reads() {
    let dir = root().join("samples/dumps");
    let mut dumps = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "dmp") {
            let bytes = std::fs::read(&p).unwrap();
            let Document::Dump(d) = debrief_plugin_dumps::import::import_dump(&bytes, &options(), &[]).unwrap() else {
                panic!("{} is not a dump", p.display())
            };
            assert!(!d.threads.is_empty(), "{}", p.display());
            dumps += 1;
        }
    }
    assert_eq!(dumps, 2);
    let full = std::fs::read(dir.join("OrderService.exe.4120.full.dmp")).unwrap();
    let Document::Dump(d) = debrief_plugin_dumps::import::import_dump(&full, &options(), &[]).unwrap() else {
        panic!()
    };
    assert_eq!(d.kind, DumpKind::Full);
    let windows = std::fs::read(root().join("samples/windows/WEB02/OrderService.exe.4120.dmp")).unwrap();
    assert!(debrief_plugin_dumps::import::import_dump(&windows, &options(), &[]).is_ok());
}

#[test]
fn every_umdh_sample_reads() {
    let dir = root().join("samples/dumps/umdh");
    let mut kinds = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(debrief_plugin_dumps::umdh_reader::sniff(&bytes) >= 90);
        let Document::Umdh(u) = debrief_plugin_dumps::import::import_umdh(&bytes, &options()).unwrap() else {
            panic!()
        };
        assert!(u.stacks.len() >= 3);
        kinds.push(u.kind);
    }
    kinds.sort_by_key(|k| *k == UmdhKind::Snapshot);
    assert_eq!(kinds, [UmdhKind::Diff, UmdhKind::Snapshot, UmdhKind::Snapshot, UmdhKind::Snapshot]);
}
