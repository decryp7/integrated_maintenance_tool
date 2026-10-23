//! Native profiling for real (Windows): the `native-busy` fixture (built by `cargo test` as an example) is profiled
//! by suspend-and-sample, symbolised from its PDB, written as NetTrace, imported, and the engine's findings name
//! its hot function and its lock. No administrator rights are needed.
//!
//! The ETW tests need an elevated shell and opt in with `DEBRIEF_ETW_TESTS=1`:
//!
//! ```text
//! set DEBRIEF_ETW_TESTS=1
//! cargo test -p debrief-plugin-native --test real_process -- --ignored --nocapture
//! ```
#![cfg(windows)]

use debrief_plugin_api::output::{FolderOutputs, Outputs};
use debrief_plugin_native::captures::Control;
use debrief_plugin_native::etw::{record as etw_record, EtwSpec};
use debrief_plugin_native::sampling::live::{record, SamplingSpec};
use debrief_plugin_native::sampling::write_nettrace;
use debrief_plugin_native::symbolize::symbolize;
use debrief_plugin_profiling::nettrace::NettraceImporter;
use debrief_profile::api::*;
use debrief_profile::*;
use debrief_symbols::SymbolConfig;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// The fixture next to this test binary (`target/<profile>/examples/native-busy.exe`).
fn fixture() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let profile_dir = exe.parent().and_then(|d| d.parent()).unwrap();
    let p = profile_dir.join("examples").join("native-busy.exe");
    assert!(p.is_file(), "{} is built by `cargo test -p debrief-plugin-native`", p.display());
    p
}

/// A running fixture; killed when dropped (its stdin closes too).
struct Busy(Child);

impl Busy {
    // Tests may start programs directly (clippy.toml).
    #[allow(clippy::disallowed_methods)]
    fn start(mode: &str) -> Busy {
        let mut c = Command::new(fixture())
            .args([mode, "60"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(c.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert!(line.starts_with("ready "), "{line}");
        std::thread::sleep(Duration::from_millis(200));
        Busy(c)
    }

    fn pid(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn import(bytes: &[u8]) -> Profile {
    match NettraceImporter.import("native-busy.nettrace", bytes, &Limits::default()).unwrap() {
        Capture::Profile(p) => p,
        Capture::Heap(_) => panic!("a profile"),
    }
}

fn findings(p: Profile) -> Vec<ProfileFinding> {
    let mut store = Captures::new(Vec::new(), Limits::default());
    let id = store.insert(Capture::Profile(p), None).unwrap().id();
    store
        .findings(&FindingsRequest {
            profile_id: id,
            baseline_id: None,
            view: ProfileView::default(),
            thresholds: FindingThresholds::default(),
        })
        .unwrap()
}

#[test]
fn suspend_and_sample_finds_the_hot_function_and_the_lock_with_symbols() {
    let busy = Busy::start("all");
    let spec = SamplingSpec {
        rate_hz: 200,
        duration: Duration::from_millis(2500),
        max_samples: 1_000_000,
    };
    let (set, notes) = record(busy.pid(), &spec, &|| false, &|_| {}).unwrap();
    assert!(set.samples.len() > 100, "{} samples; {notes:?}", set.samples.len());
    assert!(set.images.iter().any(|i| i.path.ends_with("native-busy.exe")));
    let (symbols, sym_notes) = symbolize(&SymbolConfig::default(), &set.images, &set.addresses(), 100_000);
    assert!(!symbols.is_empty(), "{sym_notes:?}");
    let (bytes, stats) = write_nettrace(&set, &symbols, Vec::new()).unwrap();
    assert!(stats.samples > 0 && stats.waits > 0, "{stats:?}");
    // The target keeps running after being sampled (every thread was resumed).
    std::thread::sleep(Duration::from_millis(100));
    assert!(debrief_etw::session::process_alive(busy.pid()));

    let p = import(&bytes);
    let spin = p
        .frames
        .iter()
        .find(|f| f.function.ends_with("native_busy_spin"))
        .unwrap_or_else(|| {
            panic!(
                "no native_busy_spin frame: {:?}",
                p.frames.iter().map(|f| &f.function).collect::<Vec<_>>()
            )
        });
    assert_eq!(spin.kind, FrameKind::Native);
    assert!(spin.file.as_deref().is_some_and(|f| f.ends_with("main.rs")), "{spin:?}");
    assert_eq!(spin.module.as_deref(), Some("native-busy.exe"));
    assert!(p.frames.iter().any(|f| f.function.ends_with("native_busy_contend")));

    let f = findings(p);
    let hot = f
        .iter()
        .find(|f| f.kind == FindingKind::HotNativeFunction)
        .unwrap_or_else(|| panic!("{:#?}", f.iter().map(|f| &f.title).collect::<Vec<_>>()));
    assert!(hot.title.contains("native_busy_spin"), "{}", hot.title);
    let wait = f
        .iter()
        .find(|f| f.kind == FindingKind::WaitHotSpot)
        .unwrap_or_else(|| panic!("{:#?}", f.iter().map(|f| &f.title).collect::<Vec<_>>()));
    assert!(
        wait.title.contains("native_busy_contend") && wait.title.contains("lock"),
        "{}",
        wait.title
    );
}

#[test]
fn stopping_ends_the_capture_and_a_dead_target_is_reported() {
    let busy = Busy::start("busy");
    let pid = busy.pid();
    let spec = SamplingSpec {
        rate_hz: 1000,
        duration: Duration::from_secs(30),
        max_samples: 50,
    };
    // The sample limit ends it.
    let (set, notes) = record(pid, &spec, &|| false, &|_| {}).unwrap();
    assert!(set.is_full(), "{notes:?}");
    // A stop request ends it at once.
    let t = std::time::Instant::now();
    let (_, _) = record(
        pid,
        &SamplingSpec {
            max_samples: 1_000_000,
            ..spec
        },
        &|| true,
        &|_| {},
    )
    .unwrap();
    assert!(t.elapsed() < Duration::from_secs(5));
    // The target exits during the capture.
    let mut busy = busy;
    let _ = busy.0.kill();
    let _ = busy.0.wait();
    let gone = record(pid, &spec, &|| false, &|_| {});
    assert!(gone.is_err() || gone.unwrap().1.iter().any(|n| n.contains("exited")));
}

fn etw_enabled() -> bool {
    std::env::var("DEBRIEF_ETW_TESTS").is_ok_and(|v| v == "1") && debrief_etw::session::can_record()
}

/// ETW for real: kernel samples with stack walks, waits and the heap provider for the fixture. Needs an
/// elevated shell and `DEBRIEF_ETW_TESTS=1` (see the module documentation).
#[test]
#[ignore = "needs administrator rights and DEBRIEF_ETW_TESTS=1"]
fn etw_records_cpu_waits_and_heap_growth() {
    if !etw_enabled() {
        eprintln!("skipped: set DEBRIEF_ETW_TESTS=1 in an elevated shell");
        return;
    }
    let busy = Busy::start("all");
    let dir = tempfile::tempdir().unwrap();
    // The elevated process streams its capture to the host, which saves it below the data folder.
    let outputs = FolderOutputs(dir.path().to_path_buf());
    let out = outputs.create("native", "busy.nettrace", 2 << 30).unwrap();
    let saved_at = dir.path().join("captures").join("native").join("busy.nettrace");
    let ctl = Control::default();
    let spec = EtwSpec {
        heap: true,
        duration: Duration::from_secs(4),
        max_bytes: 1 << 30,
        max_addresses: 200_000,
    };
    let done = etw_record(busy.pid(), &spec, out, &saved_at, &SymbolConfig::default(), &ctl).unwrap();
    eprintln!("{:#?}", done.notes);
    let p = import(&std::fs::read(&done.path).unwrap());
    assert_eq!(done.path, saved_at);
    assert!(p.frames.iter().any(|f| f.function.ends_with("native_busy_spin")));
    assert!(p.metrics.iter().any(|m| m.id == MetricId::WaitNs));
    let f = findings(p);
    let kinds: Vec<FindingKind> = f.iter().map(|f| f.kind).collect();
    assert!(kinds.contains(&FindingKind::HotNativeFunction), "{kinds:?}");
    assert!(kinds.contains(&FindingKind::NativeHeapGrowth), "{kinds:?}");
}
