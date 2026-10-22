//! End to end against the generated Windows samples in `/samples/windows`
//! (event logs, ProcMon and PerfMon CSV, the crash dump) and the msinfo32
//! reports in `/samples/configs`, opened by a workspace with this plugin.
//!
//! `samples/windows/WEB02/OrderService.exe.4120.dmp` is built here, from the
//! synthetic minidump builder, not by `samples/generate.py`. Regenerate it with
//! `DEBRIEF_REGENERATE_SAMPLES=1 cargo test -p debrief-plugin-windows --test samples -- --ignored`;
//! [`committed_crash_dump_is_current`] fails while the committed file is stale.

mod common;

#[path = "../../debrief-plugin-windows/tests/support/synth_minidump.rs"]
mod synth_minidump;

use debrief_core::api::{SourceInfo, TimelineRequest};
use debrief_core::config::{snapshot, CompareOptions, ConfigParsers, Limits};
use debrief_core::model::{Level, Record};
use debrief_core::plugin::annotate::AnnotationsRequest;
use debrief_core::query::Query;
use debrief_core::{PluginRegistry, Profile, Workspace};
use debrief_plugin_api::importer::{ImportCtx, TextImporter};
use debrief_plugin_api::text::decode;
use debrief_plugin_api::time::TimestampParser;
use serde_json::Value;
use std::path::{Path, PathBuf};
use synth_minidump::*;

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// 2026-10-14 09:00 and 12:00 UTC, the scenario window of `samples/generate.py`.
const START: i64 = 1_791_968_400_000;
const END: i64 = 1_791_979_200_000;
/// WEB02's OrderService (pid 4120) crashes at 10:41:24.050 and restarts at 10:42:10.412 as pid 5532.
const CRASH: i64 = START + 6_084_050;
const RESTART: i64 = START + 6_130_412;
/// BATCH01 loses power at 10:52:14.380 and the OS starts again at 10:54:57.500.
const POWER_LOSS: i64 = START + 6_734_380;
const BOOT: i64 = START + 6_897_500;

const DUMP: &str = "windows/WEB02/OrderService.exe.4120.dmp";

fn registry() -> PluginRegistry {
    common::registry(&[common::WINDOWS])
}

fn workspace() -> Workspace {
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry()).unwrap();
    ws.open(&[samples().join("windows"), samples().join("configs")]).unwrap();
    ws
}

fn source<'a>(sources: &'a [SourceInfo], suffix: &str) -> &'a SourceInfo {
    sources
        .iter()
        .find(|s| s.path.replace('\\', "/").ends_with(suffix))
        .unwrap_or_else(|| panic!("{suffix} not opened: {:?}", sources.iter().map(|s| &s.name).collect::<Vec<_>>()))
}

fn records<'a>(ws: &'a Workspace, src: &SourceInfo) -> Vec<&'a Record> {
    ws.records().iter().filter(|r| r.source == src.id).collect()
}

fn text<'a>(r: &'a Record, field: &str) -> &'a str {
    r.fields.get(field).and_then(Value::as_str).unwrap_or("")
}

fn num(r: &Record, field: &str) -> i64 {
    r.fields.get(field).and_then(Value::as_i64).unwrap_or(-1)
}

#[test]
fn every_windows_sample_is_read_by_its_importer() {
    let ws = workspace();
    let s = ws.summary();
    let timed = [
        ("windows/WEB02/System.evtx", "evtx"),
        ("windows/WEB02/Application.evtx", "evtx"),
        ("windows/BATCH01/System.evtx", "evtx"),
        ("windows/WEB02/procmon-OrderService.csv", "procmon-csv"),
        ("windows/WEB02/perfmon-OrderService_000001.csv", "perfmon-csv"),
        (DUMP, "minidump"),
    ];
    for (suffix, importer) in timed {
        let src = source(&s.sources, suffix);
        assert_eq!(src.importer, importer, "{suffix}");
        assert_eq!(src.skipped, 0, "{suffix}");
        assert!(src.records > 0, "{suffix}");
        let (from, to) = (src.from.unwrap(), src.to.unwrap());
        assert!(START <= from && to <= END, "{suffix}: {from}..{to}");
    }
    for suffix in ["windows/msinfo32.nfo", "configs/WEB01/msinfo32.nfo", "configs/WEB02/msinfo32.nfo"] {
        let src = source(&s.sources, suffix);
        assert_eq!((src.importer.as_str(), src.skipped, src.from), ("msinfo", 0, None), "{suffix}");
        assert!(src.records >= 1, "{suffix}");
    }
    // The registry exports need the registry plugin.
    assert!(s.unrecognized.iter().any(|u| u.name.ends_with("registry-export.reg")));
}

#[test]
fn the_web02_crash_is_in_both_event_logs() {
    let ws = workspace();
    let s = ws.summary();
    let app = records(&ws, source(&s.sources, "WEB02/Application.evtx"));
    let crash = app.iter().find(|r| text(r, "Provider") == "Application Error").unwrap();
    assert_eq!((num(crash, "EventId"), crash.level), (1000, Level::Error));
    assert!((crash.ts.unwrap() - CRASH).abs() < 1_000);
    assert_eq!(text(crash, "AppName"), "OrderService.exe");
    assert_eq!(text(crash, "ExceptionCode"), "e0434352");
    let dotnet = app.iter().find(|r| text(r, "Provider") == ".NET Runtime").unwrap();
    assert!(dotnet.message.contains("System.TimeoutException"), "{}", dotnet.message);
    let wer = app.iter().find(|r| text(r, "Provider") == "Windows Error Reporting").unwrap();
    assert!(text(wer, "AttachedFiles").ends_with("OrderService.exe.4120.dmp"));

    let system = records(&ws, source(&s.sources, "WEB02/System.evtx"));
    let scm: Vec<&&Record> = system.iter().filter(|r| text(r, "param1") == "OrderService").collect();
    let ids: Vec<i64> = scm.iter().map(|r| num(r, "EventId")).collect();
    assert_eq!(ids, [7036, 7031, 7036], "running, crashed, running again");
    assert!(scm[1].ts.unwrap() > CRASH && scm[2].ts.unwrap() > RESTART);
    assert!(system.iter().all(|r| text(r, "Computer") == "WEB02.contoso.local"));
    assert!(system
        .iter()
        .any(|r| text(r, "Provider") == "Microsoft-Windows-Time-Service" && r.level == Level::Warning));

    // Markers on the timeline: the crash kinds of the plugin.
    let tl = ws
        .timeline(&TimelineRequest {
            query: Query::default(),
            buckets: 200,
            always_show_lifecycle: true,
        })
        .unwrap();
    let kinds: Vec<&str> = tl.lanes.iter().flat_map(|l| l.markers.iter().map(|m| m.kind.as_str())).collect();
    for kind in [
        "app-crash",
        "dotnet-crash",
        "service-crash",
        "wer-report",
        "unexpected-shutdown",
        "kernel-power",
    ] {
        assert!(kinds.iter().any(|k| k.ends_with(kind)), "no {kind} marker in {kinds:?}");
    }
}

#[test]
fn batch01_reboot_and_web02_service_become_spans() {
    let ws = workspace();
    let all = ws.annotations(&AnnotationsRequest::default());
    let reboot = all
        .iter()
        .find(|a| a.annotator == "windows.reboots" && a.on.as_ref().is_some_and(|o| o.value == "BATCH01.contoso.local"))
        .unwrap_or_else(|| panic!("no BATCH01 downtime in {all:#?}"));
    assert_eq!(reboot.from, POWER_LOSS - 380, "from the last event before the power loss");
    assert_eq!(reboot.to, BOOT, "to Kernel-General 12");

    let orders = all
        .iter()
        .filter(|a| a.label.starts_with("OrderService running"))
        .map(|a| (a.from, a.to, a.label.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(orders.len(), 2, "{orders:?}");
    assert_eq!(orders[0].0, START + 750);
    assert_eq!(orders[0].2, "OrderService running (then crashed)");
    assert_eq!(orders[1].0, RESTART + 700);
    assert!(orders[1].1 >= END - 60_000, "open until the newest event");
}

#[test]
fn procmon_capture_shows_the_exit_the_dump_and_denied_operations() {
    let ws = workspace();
    let s = ws.summary();
    let rows = records(&ws, source(&s.sources, "procmon-OrderService.csv"));
    let exit = rows
        .iter()
        .find(|r| text(r, "Operation") == "Process Exit" && num(r, "ProcessId") == 4120)
        .unwrap();
    assert_eq!(exit.ts, Some(CRASH));
    assert!(rows
        .iter()
        .any(|r| text(r, "ProcessName") == "WerFault.exe" && text(r, "Path").ends_with(".dmp")));
    let start = rows
        .iter()
        .find(|r| text(r, "Operation") == "Process Start" && num(r, "ProcessId") == 5532)
        .unwrap();
    assert!(start.ts.unwrap() < RESTART);
    for result in ["ACCESS DENIED", "SHARING VIOLATION"] {
        assert!(
            rows.iter()
                .any(|r| text(r, "Result") == result && matches!(r.level, Level::Warning | Level::Error)),
            "{result}"
        );
    }
}

#[test]
fn perfmon_counters_follow_the_process_across_the_restart() {
    let ws = workspace();
    let s = ws.summary();
    let rows = records(&ws, source(&s.sources, "perfmon-OrderService_000001.csv"));
    let pids = |from: i64, to: i64| -> Vec<i64> {
        let mut p: Vec<i64> = rows
            .iter()
            .filter(|r| (from..to).contains(&r.ts.unwrap()) && text(r, "Object") == "Process")
            .map(|r| num(r, "ProcessId"))
            .collect();
        p.sort();
        p.dedup();
        p
    };
    assert_eq!(pids(START, CRASH), [4120]);
    assert_eq!(
        pids(CRASH, RESTART),
        Vec::<i64>::new(),
        "process counters are blank while it is down"
    );
    assert_eq!(pids(RESTART + 15_000, END + 1), [5532]);
    let peak = rows
        .iter()
        .filter(|r| text(r, "Counter") == "% Processor Time" && text(r, "Object") == "Process")
        .filter_map(|r| r.fields.get("Value").and_then(Value::as_f64))
        .fold(0.0, f64::max);
    assert!(peak > 700.0, "WEB02 saturates its 8 cores before the crash: {peak}");
}

#[test]
fn the_crash_dump_summarises_the_clr_exception() {
    let ws = workspace();
    let s = ws.summary();
    let rows = records(&ws, source(&s.sources, DUMP));
    assert!(rows.iter().all(|r| r.ts == Some(CRASH / 1000 * 1000)), "stamped with the dump time");
    let ex = rows.iter().find(|r| text(r, "Kind") == "exception").unwrap();
    assert_eq!(ex.level, Level::Error);
    assert!(ex.message.contains("KERNELBASE.dll+"), "{}", ex.message);
    let modules: Vec<&str> = rows
        .iter()
        .filter(|r| text(r, "Kind") == "module")
        .map(|r| text(r, "Module"))
        .collect();
    assert!(
        modules.contains(&"OrderService.exe") && modules.contains(&"coreclr.dll"),
        "{modules:?}"
    );
}

#[test]
fn msinfo_reports_of_web01_and_web02_differ_in_memory_and_build() {
    let parsers = ConfigParsers::with_registry(&registry());
    let configs = samples().join("configs");
    let d = snapshot::diff_folders(
        &parsers,
        &configs.join("WEB01"),
        &configs.join("WEB02"),
        &CompareOptions::default(),
        &Limits::default(),
    )
    .unwrap();
    let nfo = d.files.iter().find(|f| f.rel_path == "msinfo32.nfo").unwrap();
    assert_eq!(nfo.status, snapshot::FileStatus::Changed);
    assert!(nfo.counts.as_ref().unwrap().changed >= 3, "{nfo:?}");
}

/// The scenario's crash: `OrderService.exe` (pid 4120, started at 09:00:00) throws a CLR
/// exception (0xE0434352, raised in KERNELBASE.dll) on thread 0x1a2c at 10:41:24.
fn orderservice_crash() -> Vec<u8> {
    const KERNELBASE: u64 = 0x7FFB_1000_0000;
    const CORECLR: u64 = 0x7FFD_4A20_0000;
    const CORELIB: u64 = 0x7FFD_2000_0000;
    let rsp = STACK_BASE + 0x10;
    let stack = stack64(&[
        0x0,
        0x0,
        0x0,
        CORECLR + 0x1_2F40,
        0x0000_01D2_4C1E_7A90,
        CORELIB + 0x7_7310,
        APP_BASE + 0x4_A21C,
        KERNEL32_BASE + 0x1_7344,
        NTDLL_BASE + 0x5_26B1,
    ]);
    let ctx = amd64_context(KERNELBASE + 0x3_EA8C, rsp);
    let start = (START / 1000) as u32;
    DumpBuilder::new()
        .system_info(ARCH_AMD64, (10, 0, 20348), 8, "")
        .misc_info(4120, start)
        .module_list(&[
            (
                APP_BASE,
                0x5_2000,
                r"C:\Program Files\Contoso\OrderService\OrderService.exe",
                Some((3, 14, 2, 0)),
            ),
            (
                CORECLR,
                0x4E_5000,
                r"C:\Program Files\dotnet\shared\Microsoft.NETCore.App\9.0.1\coreclr.dll",
                Some((9, 0, 124, 61010)),
            ),
            (
                CORELIB,
                0xF2_0000,
                r"C:\Program Files\dotnet\shared\Microsoft.NETCore.App\9.0.1\System.Private.CoreLib.dll",
                Some((9, 0, 124, 61010)),
            ),
            (
                KERNELBASE,
                0x3A_0000,
                r"C:\Windows\System32\KERNELBASE.dll",
                Some((10, 0, 20348, 2762)),
            ),
            (
                KERNEL32_BASE,
                0xC_0000,
                r"C:\Windows\System32\kernel32.dll",
                Some((10, 0, 20348, 2762)),
            ),
            (NTDLL_BASE, 0x1F_8000, r"C:\Windows\System32\ntdll.dll", Some((10, 0, 20348, 2762))),
        ])
        .thread_list(0x1A2C, STACK_BASE, &stack, &ctx)
        .exception(0x1A2C, 0xE043_4352, KERNELBASE + 0x3_EA8C, &[0x8013_1505, 0, 0, 0, CORECLR], &ctx)
        .finish((CRASH / 1000) as u32)
}

#[test]
fn committed_crash_dump_is_current() {
    let committed = std::fs::read(samples().join(DUMP)).unwrap_or_default();
    assert!(
        committed == orderservice_crash(),
        "{DUMP} is stale; regenerate with DEBRIEF_REGENERATE_SAMPLES=1 cargo test -p debrief-plugin-windows --test samples -- --ignored"
    );
}

#[test]
#[ignore = "writes samples/windows/WEB02/OrderService.exe.4120.dmp; run with DEBRIEF_REGENERATE_SAMPLES=1"]
fn regenerate_crash_dump() {
    if std::env::var("DEBRIEF_REGENERATE_SAMPLES").as_deref() != Ok("1") {
        eprintln!("set DEBRIEF_REGENERATE_SAMPLES=1 to write the sample");
        return;
    }
    std::fs::write(samples().join(DUMP), orderservice_crash()).unwrap();
}

// ---- snapshots of the text formats ----

/// A compact projection of an import: counts, range and the first records.
fn snapshot_of(rel: &str, importer: &dyn TextImporter) -> Value {
    let bytes = std::fs::read(samples().join(rel)).unwrap();
    let content = decode(&bytes).unwrap();
    let ts = TimestampParser::default();
    let mut ctx = ImportCtx::new(Path::new(rel), &ts);
    ctx.file_date = None;
    let head: String = content.chars().take(64 * 1024).collect();
    assert!(importer.sniff(&ctx, &head) > 80, "{rel} is not recognised by {}", importer.id());
    let out = importer.import(&ctx, &content).unwrap();
    let stamps: Vec<i64> = out.records.iter().filter_map(|r| r.ts).collect();
    let mut fields: Vec<&String> = out.records.iter().flat_map(|r| r.fields.keys()).collect();
    fields.sort();
    fields.dedup();
    serde_json::json!({
        "sample": rel,
        "importer": importer.id(),
        "records": out.records.len(),
        "skipped": out.skipped,
        "first_ts": stamps.iter().min(),
        "last_ts": stamps.iter().max(),
        "fields": fields,
        "first": out.records.iter().take(3).map(|r| serde_json::json!({
            "ts": r.ts, "level": r.level.as_str(), "message": r.message, "fields": r.fields,
        })).collect::<Vec<_>>(),
    })
}

#[test]
fn text_format_snapshots() {
    insta::assert_yaml_snapshot!(
        "procmon",
        snapshot_of(
            "windows/WEB02/procmon-OrderService.csv",
            &debrief_plugin_windows::ProcmonCsvImporter
        )
    );
    insta::assert_yaml_snapshot!(
        "perfmon",
        snapshot_of(
            "windows/WEB02/perfmon-OrderService_000001.csv",
            &debrief_plugin_windows::PerfmonCsvImporter
        )
    );
    insta::assert_yaml_snapshot!(
        "msinfo-web02",
        snapshot_of("configs/WEB02/msinfo32.nfo", &debrief_plugin_windows::NfoImporter)
    );
}
