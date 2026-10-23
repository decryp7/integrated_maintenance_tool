//! Real .NET Framework processes monitored through PDH (Windows only). The fixture is a tiny
//! C# console app compiled with the Framework's `csc.exe`; the test is skipped when the
//! compiler is missing.

use super::*;
use crate::dotnet::Runtime;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;

const FIXTURE: &str = r#"
using System;
using System.Collections.Generic;
using System.Threading;

static class Program {
    static void Main(string[] args) {
        int seconds = args.Length > 0 ? int.Parse(args[0]) : 30;
        int throws = args.Length > 1 ? int.Parse(args[1]) : 5;
        var keep = new List<byte[]>();
        var end = DateTime.UtcNow.AddSeconds(seconds);
        while (DateTime.UtcNow < end) {
            for (int i = 0; i < 200; i++) keep.Add(new byte[1024]);
            keep.Add(new byte[100 * 1024]);
            if (keep.Count > 5000) keep.Clear();
            for (int i = 0; i < throws; i++) {
                try { throw new InvalidOperationException("fixture"); } catch (InvalidOperationException) { }
            }
            Thread.Sleep(20);
        }
    }
}
"#;

fn csc() -> Option<PathBuf> {
    let windir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    ["Framework64", "Framework"]
        .iter()
        .map(|f| windir.join("Microsoft.NET").join(f).join("v4.0.30319").join("csc.exe"))
        .find(|p| p.is_file())
}

fn compile(csc: &Path, dir: &Path) -> PathBuf {
    let src = dir.join("DebriefClrFixture.cs");
    let exe = dir.join("DebriefClrFixture.exe");
    std::fs::write(&src, FIXTURE).unwrap();
    let out = debrief_core::process::quiet_command(csc)
        .arg("/nologo")
        .arg(format!("/out:{}", exe.display()))
        .arg(&src)
        .output()
        .unwrap();
    assert!(out.status.success(), "csc failed: {}", String::from_utf8_lossy(&out.stdout));
    exe
}

/// Kills the fixture even when an assertion fails.
struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Collect<'a> {
    records: Mutex<Vec<Record>>,
    writes: AtomicUsize,
    limit: usize,
    stop: &'a AtomicBool,
}
impl EventSink for Collect<'_> {
    fn write(&self, r: &[Record]) -> Result<()> {
        self.records.lock().unwrap().extend_from_slice(r);
        if self.writes.fetch_add(1, Ordering::SeqCst) + 1 >= self.limit {
            self.stop.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

#[test]
fn real_dotnet_framework_processes_get_runtime_fields() {
    let Some(csc) = csc() else {
        eprintln!("skipped: .NET Framework csc.exe not found");
        return;
    };
    let problems = crate::dotnet::open().unwrap().problems;
    if !problems.is_empty() {
        eprintln!("skipped: .NET CLR performance counters unavailable: {problems:?}");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let exe = compile(&csc, dir.path());
    // Two instances share a name, so PDH names them `DebriefClrFixture` and `DebriefClrFixture#1`.
    // They throw at different rates, so a mix-up of instances would show.
    let spawn = |throws: &str| {
        Kill(
            debrief_core::process::quiet_command(&exe)
                .args(["60", throws])
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let (a, b) = (spawn("8"), spawn("1"));
    let pids = [a.0.id(), b.0.id()];
    let me = std::process::id();
    // Let the CLR start and publish its counters.
    std::thread::sleep(Duration::from_millis(1500));

    let stop = AtomicBool::new(false);
    let sink = Collect {
        records: Mutex::new(vec![]),
        writes: AtomicUsize::new(0),
        limit: 8,
        stop: &stop,
    };
    let config = json!({"targets": [{"pid": pids[0]}, {"pid": pids[1]}, {"pid": me}], "intervalMs": 500, "dotnet": true});
    ProcessMonitor.run(&config, &sink, &stop).unwrap();
    let recs = sink.records.lock().unwrap();
    let stats = |pid: u32| -> Vec<&Record> {
        recs.iter()
            .filter(|r| r.message.starts_with("Process stats") && r.fields["ProcessId"] == pid)
            .collect()
    };
    let all: Vec<_> = recs.iter().map(|r| (&r.message, &r.fields)).collect();
    let mut peak = Vec::new();
    for pid in pids {
        let s = stats(pid);
        assert!(s.len() >= 3, "stats for {pid}: {all:#?}");
        let last = &s.last().unwrap().fields;
        eprintln!("{pid}: {}", serde_json::to_string(last).unwrap_or_default());
        assert_eq!(last["DotnetRuntime"], "framework", "{pid}: {last:#?}");
        let num = |k: &str| {
            last.get(k)
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| panic!("{k} missing: {last:#?}"))
        };
        assert!(num("DotnetHeapBytes") > 0.0);
        assert!(num("DotnetGen0Collections") >= 1.0, "the fixture allocates constantly");
        assert!(num("DotnetLohBytes") >= 0.0);
        assert!((0.0..=100.0).contains(&num("DotnetTimeInGcPercent")));
        assert!(num("DotnetThreads") >= 1.0);
        assert!(num("DotnetAssemblies") >= 1.0);
        for k in [
            "DotnetGen1Collections",
            "DotnetGen2Collections",
            "DotnetGen0HeapBytes",
            "DotnetPinnedObjects",
            "DotnetPhysicalThreads",
        ] {
            num(k);
        }
        // ~250 resp. ~30 exceptions/s; allow for scheduling noise.
        let rates: Vec<f64> = s
            .iter()
            .filter_map(|r| r.fields.get("DotnetExceptionsPerSec").and_then(|v| v.as_f64()))
            .collect();
        assert!(rates.iter().any(|r| *r > 5.0), "exception rates for {pid}: {rates:?}");
        peak.push(rates.iter().copied().fold(0.0, f64::max));
        assert!(s.iter().any(|r| r.fields.get("DotnetContentionPerSec").is_some()));
        assert!(s.iter().any(|r| r.fields.get("DotnetQueueLengthPerSec").is_some()));
    }
    assert!(peak[0] > peak[1] * 3.0, "8 vs 1 throws per loop: {peak:?}");
    // The native test process gets no .NET fields and no notice.
    assert!(stats(me).iter().all(|r| r.fields.get("DotnetRuntime").is_none()));
    assert!(!recs
        .iter()
        .any(|r| r.fields.get("ProcessId") == Some(&me.into()) && r.level == Level::Warning));
}

#[test]
fn module_probe_detects_the_framework_and_native_processes() {
    let mut opened = crate::dotnet::open().unwrap();
    // This test binary is native Rust.
    assert_eq!(opened.source.runtime(std::process::id()), None);
    // A PID that does not exist cannot be read.
    assert_eq!(opened.source.runtime(4_000_000), None);
    let Some(csc) = csc() else {
        eprintln!("skipped: .NET Framework csc.exe not found");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let exe = compile(&csc, dir.path());
    let child = Kill(
        debrief_core::process::quiet_command(&exe)
            .arg("20")
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let pid = child.0.id();
    let mut found = None;
    for _ in 0..50 {
        found = opened.source.runtime(pid);
        if found.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(found, Some(Runtime::Framework));
}
