//! Real-runtime tests: the C# fixture (`tests/fixtures/Busy`) built with the installed .NET SDK,
//! recorded for real and opened with the nettrace importer.
//!
//! - EventPipe (no elevation): run with every `cargo test`; skipped (not failed) when no `dotnet`
//!   is found (`$DEBRIEF_DOTNET`, then `dotnet` on PATH, then `~/.dotnet/dotnet`).
//! - ETW (Windows, elevated): `#[ignore]`d; run once from an elevated prompt with
//!   `DEBRIEF_ETW_TESTS=1 cargo test -p debrief-plugin-dotnet --test it real_runtime:: -- --ignored --nocapture`.

// Tests start the fixture and the SDK directly (no console window concerns).
#![allow(clippy::disallowed_methods)]

use debrief_plugin_dotnet::{Limits, Preset};
use debrief_profile::{Capture, CaptureImporter, MetricId, Profile};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

fn dotnet() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DEBRIEF_DOTNET") {
        return Some(PathBuf::from(p));
    }
    let probe = |p: &Path| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success());
    if probe(Path::new("dotnet")) {
        return Some(PathBuf::from("dotnet"));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let p = PathBuf::from(home)
        .join(".dotnet")
        .join(if cfg!(windows) { "dotnet.exe" } else { "dotnet" });
    probe(&p).then_some(p)
}

fn out_dir() -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    base.join("dotnet-fixtures")
}

/// Builds the fixture for `tfm` (and `x86` when asked); `None` (with a message) when it cannot be built.
fn fixture(tfm: &str, x86: bool) -> Option<PathBuf> {
    static BUILDS: OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    let key = format!("{tfm}-{}", if x86 { "x86" } else { "x64" });
    let map = BUILDS.get_or_init(Default::default);
    let mut map = map.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(v) = map.get(&key) {
        return v.clone();
    }
    let built = (|| {
        let dotnet = dotnet().or_else(|| {
            eprintln!("skipped: no dotnet found (set DEBRIEF_DOTNET)");
            None
        })?;
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/Busy");
        let out = out_dir().join(&key);
        let mut cmd = Command::new(&dotnet);
        cmd.current_dir(&src)
            .args(["build", "-c", "Release", "-f", tfm, "-o"])
            .arg(&out)
            .arg(format!(
                "-p:BaseIntermediateOutputPath={}/",
                out_dir().join(format!("obj-{key}")).display()
            ));
        if x86 {
            cmd.arg("-p:PlatformTarget=x86");
        }
        let o = cmd.output().ok()?;
        if !o.status.success() {
            eprintln!(
                "skipped: the {key} fixture did not build (for net462 the Microsoft.NETFramework.ReferenceAssemblies package must restore):\n{}",
                String::from_utf8_lossy(&o.stdout)
            );
            return None;
        }
        let exe = out.join(if tfm == "net462" || cfg!(windows) { "Busy.exe" } else { "Busy" });
        exe.exists().then_some(exe)
    })();
    map.insert(key, built.clone());
    built
}

/// The running fixture; killed when dropped.
struct Target(Child, u32);

impl Drop for Target {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run(exe: &Path) -> Target {
    let mut cmd = Command::new(exe);
    // A framework-dependent fixture finds the runtime of a per-user install (`~/.dotnet`) only through DOTNET_ROOT.
    if let Some(root) = dotnet().and_then(|d| d.parent().map(Path::to_path_buf)).filter(|r| r.is_absolute()) {
        cmd.env("DOTNET_ROOT", root);
    }
    let mut child = cmd
        .arg("120")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the fixture");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let pid: u32 = line.trim().strip_prefix("READY ").expect("READY line").parse().unwrap();
    // Let the workers warm up; a .NET Core target also needs its diagnostics port answering (slow on a busy machine).
    std::thread::sleep(Duration::from_millis(1500));
    if !exe.extension().is_some_and(|e| e == "exe") || exe.with_extension("dll").exists() {
        let t = std::time::Instant::now();
        while debrief_plugin_dotnet::eventpipe::process_info(pid, Duration::from_secs(5)).is_err() && t.elapsed() < Duration::from_secs(60)
        {
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    Target(child, pid)
}

fn import(path: &Path) -> Capture {
    let bytes = std::fs::read(path).unwrap();
    debrief_plugin_profiling::nettrace::NettraceImporter
        .import(&path.to_string_lossy(), &bytes, &debrief_profile::Limits::default())
        .unwrap_or_else(|e| panic!("import {}: {e}", path.display()))
}

fn functions(p: &Profile) -> Vec<String> {
    p.frames.iter().map(|f| f.function.clone()).collect()
}

fn limits() -> Limits {
    Limits {
        connect_timeout: Duration::from_secs(30),
        ..Limits::default()
    }
}

fn record_eventpipe(pid: u32, preset: Preset, out: &Path, secs: u64) -> debrief_plugin_dotnet::Finished {
    use debrief_plugin_dotnet::Recording;
    let r = debrief_plugin_dotnet::eventpipe::record(pid, preset, out, limits()).expect("start EventPipe");
    let start = std::time::Instant::now();
    while !r.is_finished() && start.elapsed() < Duration::from_secs(secs) {
        std::thread::sleep(Duration::from_millis(100));
    }
    let f = Box::new(r).stop().expect("stop EventPipe");
    eprintln!("{preset:?}: {} bytes, notes {:?}", f.bytes, f.notes);
    f
}

#[test]
fn eventpipe_cpu_allocations_and_heap_of_a_net10_process() {
    let Some(exe) = fixture("net10.0", false) else { return };
    let dir = tempfile::tempdir().unwrap();
    let target = run(&exe);

    let info = debrief_plugin_dotnet::eventpipe::process_info(target.1, Duration::from_secs(5)).expect("ProcessInfo");
    assert_eq!(info.pid, u64::from(target.1));
    assert!(info.runtime_version.as_deref().is_some_and(|v| v.starts_with("10.")), "{info:?}");

    let cpu = dir.path().join("cpu.nettrace");
    record_eventpipe(target.1, Preset::Cpu, &cpu, 3);
    let Capture::Profile(p) = import(&cpu) else { panic!("a profile") };
    let names = functions(&p);
    assert!(
        names.iter().any(|n| n.contains("Busy.Program.HotLoop")),
        "{:?}",
        &names[..names.len().min(40)]
    );
    assert!(p.metrics.iter().any(|m| m.id == MetricId::Exceptions), "exceptions recorded");
    assert!(p.events.iter().any(|e| e.label.contains("InvalidOperationException")));

    let alloc = dir.path().join("alloc.nettrace");
    record_eventpipe(target.1, Preset::Allocations, &alloc, 3);
    let Capture::Profile(p) = import(&alloc) else { panic!("a profile") };
    assert!(
        p.metrics.iter().any(|m| m.id == MetricId::AllocBytes),
        "allocation ticks: {:?}",
        p.notes
    );
    assert!(functions(&p).iter().any(|n| n.starts_with("System.Byte[]")));

    let heap = dir.path().join("heap.nettrace");
    let f = record_eventpipe(target.1, Preset::HeapSnapshot, &heap, 60);
    assert!(f.bytes > 0);
    let Capture::Heap(h) = import(&heap) else {
        panic!("a heap snapshot")
    };
    assert!(
        h.types.iter().any(|t| t.name == "Busy.Node" && t.count >= 2000),
        "{:?}",
        h.types.iter().take(20).collect::<Vec<_>>()
    );
    assert!(h.graph.as_ref().is_some_and(|g| !g.roots.is_empty()));
    assert!(h
        .types
        .iter()
        .any(|t| t.name == "[Static variables]" || t.name == "[Handles]" || t.name == "[Stack roots]"));
}

/// ETW, elevated only: .NET Framework 4.6.2 x64 and x86 and .NET 10 targets, CPU / allocations / heap,
/// plus an `.etl` file recorded by the same session code, then scanned and converted.
#[cfg(windows)]
#[test]
#[ignore = "needs an elevated prompt: DEBRIEF_ETW_TESTS=1 cargo test -p debrief-plugin-dotnet --test it real_runtime:: -- --ignored --nocapture"]
fn etw_framework_and_core_targets_elevated() {
    use debrief_plugin_dotnet::Recording;
    if std::env::var_os("DEBRIEF_ETW_TESTS").is_none() {
        eprintln!("skipped: set DEBRIEF_ETW_TESTS=1 (and run elevated)");
        return;
    }
    assert!(
        debrief_plugin_dotnet::etw::session::can_record(),
        "run this test from an elevated prompt"
    );
    let dir = tempfile::tempdir().unwrap();
    let mut targets = Vec::new();
    for (tfm, x86) in [("net462", false), ("net462", true), ("net10.0", false)] {
        if let Some(exe) = fixture(tfm, x86) {
            targets.push((format!("{tfm}-{}", if x86 { "x86" } else { "x64" }), exe));
        }
    }
    assert!(!targets.is_empty(), "no fixture could be built");
    for (label, exe) in targets {
        let target = run(&exe);
        let info = debrief_plugin_dotnet::process::info(target.1).expect("the fixture is a .NET process");
        eprintln!("{label}: {info:?}");
        assert_eq!(info.arch.as_deref(), Some(if label.ends_with("x86") { "x86" } else { "x64" }));
        for preset in [Preset::Cpu, Preset::Allocations, Preset::HeapSnapshot] {
            let out = dir.path().join(format!("{label}-{preset:?}.nettrace"));
            let r = debrief_plugin_dotnet::etw::session::record(
                &info,
                preset,
                Box::new(debrief_plugin_dotnet::recording::FileSink::create(&out).unwrap()),
                &out,
                limits(),
            )
            .expect("start ETW");
            let start = std::time::Instant::now();
            let secs = if preset == Preset::HeapSnapshot { 60 } else { 4 };
            while !r.is_finished() && start.elapsed() < Duration::from_secs(secs) {
                std::thread::sleep(Duration::from_millis(100));
            }
            let f = Box::new(r).stop().expect("stop ETW");
            eprintln!("{label} {preset:?}: {} bytes, notes {:?}", f.bytes, f.notes);
            match (preset, import(&out)) {
                (Preset::HeapSnapshot, Capture::Heap(h)) => {
                    assert!(h.types.iter().any(|t| t.name == "Busy.Node"), "{label}");
                }
                (Preset::Cpu, Capture::Profile(p)) => {
                    let names = functions(&p);
                    assert!(
                        names.iter().any(|n| n.contains("HotLoop")),
                        "{label}: {:?}",
                        &names[..names.len().min(40)]
                    );
                    assert!(
                        p.events.iter().any(|e| e.kind == debrief_profile::EventKind::Wait),
                        "{label}: waits"
                    );
                }
                (Preset::Allocations, Capture::Profile(p)) => {
                    assert!(p.metrics.iter().any(|m| m.id == MetricId::AllocBytes), "{label}");
                }
                (p, _) => panic!("{label} {p:?}: wrong capture kind"),
            }
        }
        // An .etl of the same process, read back with the file reader.
        let etl = dir.path().join(format!("{label}.etl"));
        debrief_plugin_dotnet::etw::session::record_etl(&info, &etl, Duration::from_secs(4)).expect("record .etl");
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let procs = debrief_plugin_dotnet::etw::etl::scan(&etl, debrief_etw::scan::Order::DotnetFirst, &cancel).expect("scan .etl");
        let me = procs.iter().find(|p| p.pid == target.1).expect("the fixture is in the .etl");
        assert!(me.is_dotnet());
        let out = dir.path().join(format!("{label}-etl.nettrace"));
        let mut no_symbols = |_: &[debrief_plugin_profiling::nettrace::ext::ImageLoad], _: &[u64]| Vec::new();
        let c = debrief_plugin_dotnet::etw::etl::convert(&etl, me, &out, 1 << 30, &cancel, Default::default(), &mut no_symbols)
            .expect("convert .etl");
        eprintln!("{label} .etl: {c:?}");
        let Capture::Profile(p) = import(&out) else { panic!("a profile") };
        assert!(functions(&p).iter().any(|n| n.contains("HotLoop")), "{label} .etl");
        // Keep a copy of a small .etl for the importer's tests when asked.
        if let Some(keep) = std::env::var_os("DEBRIEF_KEEP_ETL") {
            let _ = std::fs::copy(&etl, PathBuf::from(keep).join(format!("{label}.etl")));
        }
    }
}
