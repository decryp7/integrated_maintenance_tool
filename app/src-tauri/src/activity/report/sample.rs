//! `samples/diagnostics/imt-diagnostics-sample.zip`: a diagnostics report for demos and tests, written by
//! the real report writer so it never drifts from the format. Fully reproducible: fixed times on
//! 14 Oct 2026 (an analyst looking into the samples' incident that afternoon), fixed pids, synthetic
//! system information, and zip entries in a fixed order with a fixed time.
//!
//! Scenario: IMT starts, the user opens the incident samples and runs a query; the AI engine starts and
//! its llama-server crashes (exit code 3) and is restarted; the out-of-process plugin
//! `com.example.orderaudit` crashes three times and is quarantined, which leaves an error snapshot.
//!
//! Regenerate: `IMT_REGEN_SAMPLES=1 cargo test -p imt-app --lib activity::report::sample` (the test fails
//! when the committed file differs from what the writer makes now).

use super::{breadcrumbs_from_log, write, PluginLine, ReportInputs};
use crate::snapshots::{Limits, LogRef, Sidecar, SnapshotStore};
use chrono::{TimeZone, Utc};
use imt_ipc::child_log::{ChildEvent, ChildKind, ChildLog, ChildLogSpec, Exit};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The committed sample.
pub fn sample_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/diagnostics/imt-diagnostics-sample.zip")
}

/// 2026-10-14T13:00:00Z plus `secs` (and `ms`).
fn at(secs: u64, ms: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_791_982_800 + secs) + Duration::from_millis(ms)
}

fn t(secs: u64, ms: u64) -> String {
    imt_ipc::child_log::rfc3339(at(secs, ms))
}

/// The snapshot's id (a valid `SnapshotStore` id: time and 8 hex digits).
pub const SNAPSHOT_ID: &str = "20261014T130411250-0badc0de";
/// The plugin that is quarantined.
pub const PLUGIN: &str = "com.example.orderaudit";

// ---- a small PNG, written by hand (stored deflate) ----

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// An RGB PNG of `w`×`h` pixels.
fn png(w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            raw.extend_from_slice(&pixel(x, y));
        }
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push(u8::from(i + 1 == blocks.len()));
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A tiny "screenshot": a dark window, a red error bar, a few grid rows and a timeline strip.
fn screenshot() -> Vec<u8> {
    png(160, 100, |x, y| match (x, y) {
        (_, 0..=9) => [0x2b, 0x2d, 0x3a],
        (_, 10..=21) => [0xf2, 0x54, 0x5b],
        (_, 80..=99) if x % 16 < 3 => [0x39, 0x87, 0xe5],
        (_, 80..=99) => [0x24, 0x26, 0x33],
        (_, y) if y % 8 == 0 => [0x3a, 0x3d, 0x4d],
        _ => [0x1e, 0x1f, 0x2a],
    })
}

// ---- the scenario ----

fn activity(secs: u64, op: &str, trigger: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut v = json!({
        "@t": t(secs, 0),
        "@mt": "User {Operation} via {Trigger}",
        "@m": format!("User {op} via {trigger}"),
        "SourceContext": "IMT.Activity",
        "Operation": op,
        "Trigger": trigger,
        "Window": "main",
        "OperationId": format!("op-{secs}"),
    });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    v
}

fn imt_line(secs: u64, level: &str, context: &str, message: &str) -> serde_json::Value {
    json!({ "@t": t(secs, 0), "@l": level, "@m": message, "SourceContext": context, "Version": "0.1.0", "SessionId": "00000000-0000-4000-8000-000000000001" })
}

fn imt_log() -> String {
    let lines = [
        imt_line(0, "Information", "imt_app", "Application starting — IMT 0.1.0"),
        activity(5, "file.open", "menu", json!({ "Files": "6", "Folders": "3" })),
        imt_line(9, "Information", "imt_app::workspace_commands", "Workspace opened"),
        activity(70, "query.change", "keyboard", json!({ "QueryKind": "ql", "DurationMs": 38 })),
        activity(115, "ai.chat.send", "button", json!({ "View": "AI assistant" })),
        imt_line(
            180,
            "Warning",
            "imt_app::plugin_isolation",
            "Plugin process crashed and was restarted",
        ),
        imt_line(
            220,
            "Warning",
            "imt_app::plugin_isolation",
            "Plugin process crashed and was restarted",
        ),
        imt_line(
            250,
            "Error",
            "imt_app::plugin_isolation",
            "Plugin quarantined after repeated failures",
        ),
        imt_line(
            332,
            "Error",
            "imt_ai",
            "AI engine call failed: the engine crashed and was restarted",
        ),
        activity(340, "ai.chat.send", "button", json!({ "View": "AI assistant", "Outcome": "ok" })),
    ];
    lines.iter().map(|l| format!("{l}\n")).collect()
}

fn child_logs(dir: &Path) -> Vec<imt_ipc::child_log::ProcessSummary> {
    let worker = ChildLog::open(
        Some(dir),
        &ChildLogSpec::new("ai-worker", ChildKind::AiWorker).with_version("0.1.0"),
    );
    let llama = ChildLog::open(
        Some(dir),
        &ChildLogSpec::new("llama-server", ChildKind::LlamaServer).with_version("b6500"),
    );
    let plugin = ChildLog::open(
        Some(dir),
        &ChildLogSpec::new(format!("plugin-{PLUGIN}"), ChildKind::Plugin).with_version("2.3.0"),
    );
    let out = |l: &ChildLog, s: u64, ms: u64, text: &str| l.record_at(at(s, ms), &ChildEvent::Output(text.into()));
    let crash = |l: &ChildLog, s: u64, pid: u32, exit: Exit| l.record_at(at(s, 0), &ChildEvent::Exit { pid: Some(pid), exit });

    // The plugin process: starts with the workspace, crashes three times, is quarantined.
    let mut pid = 5120;
    plugin.record_at(at(3, 0), &ChildEvent::Start { pid });
    out(&plugin, 3, 200, "orderaudit 2.3.0 listening");
    for (crash_at, restart_at) in [(180, Some(181)), (220, Some(222)), (250, None)] {
        out(
            &plugin,
            crash_at,
            0,
            "thread 'main' panicked at src/rules.rs:88:17: index out of bounds: the len is 0 but the index is 0",
        );
        crash(&plugin, crash_at, pid, Exit::Code(101));
        if let Some(r) = restart_at {
            pid += 1;
            plugin.record_at(at(r, 0), &ChildEvent::Start { pid });
        }
    }
    plugin.record_at(
        at(250, 100),
        &ChildEvent::Quarantined {
            reason: format!("plugin \"{PLUGIN}\" failed 3 times in 10 minutes (last: it crashed (exit code 101)) and was stopped; reset it to try again"),
        },
    );

    // The AI engine and its llama-server, which crashes once and is started again.
    worker.record_at(at(112, 0), &ChildEvent::Start { pid: 4242 });
    llama.record_at(at(112, 400), &ChildEvent::Start { pid: 4243 });
    out(&llama, 113, 0, "main: loading model");
    out(&llama, 118, 500, "main: model loaded");
    out(&llama, 118, 600, "main: HTTP endpoint ready on 127.0.0.1:52731");
    out(
        &llama,
        331,
        800,
        "ggml_cuda_compute_forward: error: out of memory allocating 512.00 MiB on device 0",
    );
    out(&llama, 331, 900, "CUDA error: out of memory");
    crash(&llama, 332, 4243, Exit::Code(3));
    llama.record_at(at(334, 0), &ChildEvent::Start { pid: 4251 });
    out(&llama, 334, 300, "main: loading model");
    out(&llama, 339, 0, "main: model loaded");
    vec![worker.status(), llama.status(), plugin.status()]
}

fn snapshot(dir: &Path, crumbs: &[crate::activity::Breadcrumb]) {
    std::fs::create_dir_all(dir).unwrap();
    let image = screenshot();
    let message = format!("Plugin \"{PLUGIN}\" was stopped after 3 crashes");
    let sidecar = Sidecar {
        version: 1,
        id: SNAPSHOT_ID.into(),
        created_at: t(251, 250),
        reason: "engine-error".into(),
        message: message.clone(),
        stack: None,
        view: Some("Timeline".into()),
        manual: false,
        app_version: "0.1.0".into(),
        os: "windows".into(),
        arch: "x86_64".into(),
        mime: "image/png".into(),
        bytes: image.len() as u64,
        log_refs: vec![LogRef {
            line: 8,
            time: Some(t(250, 0)),
        }],
        steps: crate::activity::steps(crumbs, Some(&message)),
        breadcrumbs: crumbs.to_vec(),
    };
    std::fs::write(dir.join(format!("{SNAPSHOT_ID}.png")), &image).unwrap();
    std::fs::write(
        dir.join(format!("{SNAPSHOT_ID}.json")),
        serde_json::to_vec_pretty(&sidecar).unwrap(),
    )
    .unwrap();
}

fn system_info() -> serde_json::Value {
    json!({
        "appVersion": "0.1.0",
        "commit": "sample",
        "sessionId": "00000000-0000-4000-8000-000000000001",
        "os": "windows",
        "osVersion": "Windows 11 Pro 24H2 (sample)",
        "arch": "x86_64",
        "cpuCores": 8,
        "totalMemoryMb": 16384,
        "availableMemoryMb": 9120,
        "safeMode": false,
        "activityDetail": "normal",
        "plugins": [
            { "id": "net.decryptology.web", "version": "0.1.0", "status": "active" },
            { "id": "net.decryptology.windows", "version": "0.1.0", "status": "active" },
            { "id": PLUGIN, "version": "2.3.0", "status": "quarantined" },
        ],
        "configuration": { "settings": { "version": 1 }, "diagnostics": null, "plugins": { "disabled": [] } },
        "unreadable": [],
    })
}

/// Writes the sample into `work` (a scratch folder) and returns the zip's path.
pub fn build(work: &Path) -> PathBuf {
    let _ = std::fs::remove_dir_all(work);
    let logs = work.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("imt.2026-10-14.log"), imt_log()).unwrap();
    let live = child_logs(&super::children_dir(&logs));
    let crumbs = breadcrumbs_from_log(imt_log().as_bytes(), 200);
    snapshot(&logs.join("snapshots"), &crumbs[..crumbs.len().min(3)]);
    let inputs = ReportInputs {
        log_dir: Some(logs.clone()),
        snapshots: Some(SnapshotStore::new(logs.join("snapshots"), None, Limits::default())),
        plugins: vec![PluginLine {
            id: PLUGIN.into(),
            version: "2.3.0".into(),
            status: "quarantined".into(),
        }],
        breadcrumbs: crumbs,
        live_children: live,
        previous_crash: Some(false),
        created_at: Utc.with_ymd_and_hms(2026, 10, 14, 13, 6, 0).unwrap(),
        system_override: Some(system_info()),
        ..ReportInputs::empty()
    };
    let out = work.join("imt-diagnostics-sample.zip");
    write(
        &inputs,
        "The order-audit plugin keeps crashing after I open the incident samples, and the AI assistant restarted once.",
        &out,
    )
    .unwrap();
    out
}

#[cfg(test)]
mod tests {
    use super::super::open::{detect, divert_in, Detected, REPORTS_DIR};
    use super::*;
    use imt_ipc::child_log::ChildRunState;

    #[test]
    fn the_png_is_valid() {
        let p = screenshot();
        assert!(p.starts_with(&[0x89, b'P', b'N', b'G']));
        // Known CRC-32 and Adler-32 values.
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert!(p.len() < 64 * 1024);
    }

    #[test]
    fn the_committed_sample_is_reproducible() {
        let base = std::env::temp_dir().join(format!("imt-diag-sample-{}", std::process::id()));
        let a = std::fs::read(build(&base.join("a"))).unwrap();
        let b = std::fs::read(build(&base.join("b"))).unwrap();
        assert!(a == b, "two builds differ");
        let committed = sample_path();
        if std::env::var_os("IMT_REGEN_SAMPLES").is_some_and(|v| v == "1") {
            std::fs::create_dir_all(committed.parent().unwrap()).unwrap();
            std::fs::write(&committed, &a).unwrap();
        }
        let on_disk = std::fs::read(&committed).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&base);
        assert!(
            on_disk == a,
            "{} is out of date; regenerate it with IMT_REGEN_SAMPLES=1 cargo test -p imt-app --lib activity::report::sample",
            committed.display()
        );
    }

    #[test]
    fn opening_the_sample_gives_a_diagnostics_workspace() {
        let zip = sample_path();
        let Detected::Report(m) = detect(&zip) else {
            panic!("the sample is not detected as a report")
        };
        assert_eq!(m.created_at, "2026-10-14T13:06:00.000Z");
        let work = std::env::temp_dir().join(format!("imt-diag-sample-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        let done = divert_in(&work.join(REPORTS_DIR), vec![zip.to_string_lossy().into_owned()]);
        let o = done.opened.expect("opened as a report");

        // The panel: processes, steps, breadcrumbs, the snapshot.
        let v = &o.view;
        let p = |name: &str| v.processes.iter().find(|p| p.name == name).cloned().unwrap();
        let llama = p("llama-server");
        assert_eq!(
            (llama.state, llama.restarts, llama.last_exit.as_deref()),
            (ChildRunState::Running, 1, Some("exit code 3"))
        );
        let plugin = p(&format!("plugin-{PLUGIN}"));
        assert!(plugin.quarantined && plugin.state == ChildRunState::Quarantined);
        assert_eq!(plugin.restarts, 2);
        assert_eq!(p("ai-worker").state, ChildRunState::Running);
        assert!(v.steps[0].starts_with("1. Opened files"), "{:?}", v.steps);
        assert_eq!(v.breadcrumbs.len(), 4);
        assert!(v.description.as_deref().is_some_and(|d| d.contains("order-audit")));
        assert_eq!(v.snapshots.len(), 1);
        assert_eq!(v.snapshots[0].id, SNAPSHOT_ID);
        assert!(v.unreadable.is_empty(), "{:?}", v.unreadable);
        assert!(super::super::open::read_image(&zip, &m, SNAPSHOT_ID)
            .unwrap()
            .starts_with("data:image/png;base64,"));

        // The workspace: one source (lane) per log, the JSON files not imported, crash and restart markers.
        let mut ws = imt_core::workspace::Workspace::new(imt_core::Profile::default_profile()).unwrap();
        ws.open(std::slice::from_ref(&o.logs)).unwrap();
        let s = ws.summary();
        let mut names: Vec<String> = s.sources.iter().map(|x| x.name.replace('\\', "/")).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "activity.log",
                "children/ai-worker.log",
                "children/llama-server.log",
                &format!("children/plugin-{PLUGIN}.log"),
                "imt.2026-10-14.log",
            ],
            "{:?}",
            s.unrecognized
        );
        let kind_bit = |kind: &str| s.marker_kinds.iter().position(|k| k.kind == kind).map(|i| 1u64 << i).unwrap();
        let marked = |kind: &str, starts: &str| {
            ws.records()
                .iter()
                .filter(|r| r.markers & kind_bit(kind) != 0 && r.message.starts_with(starts))
                .count()
        };
        assert_eq!(marked("fatal", "Crashed: llama-server"), 1);
        assert_eq!(marked("fatal", &format!("Crashed: plugin-{PLUGIN}")), 3);
        assert_eq!(marked("start", "Started llama-server"), 2);
        assert_eq!(marked("start", &format!("Started plugin-{PLUGIN}")), 3);
        assert_eq!(marked("error", &format!("Quarantined plugin-{PLUGIN}")), 1);
        let _ = std::fs::remove_dir_all(work);
    }
}
