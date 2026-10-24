//! The elevated process' output files and standard error (ADR 0017 §7, amendment 2026-10-22), with the fake
//! plugin's elevated role run as an ordinary process (the administrator prompt cannot run in tests): the bytes
//! are streamed to the host, which saves them in the data folder it chose; bad names, oversized streams and a
//! crash mid-stream leave nothing behind; standard-error lines reach the plugin's elevated log, bounded.

use debrief_core::plugin::tools::{NoTasks, ToolCtx};
use debrief_core::PluginRegistry;
use debrief_ipc::{IpcError, Json};
use debrief_plugin_api::protocol::{PluginRequest, ToolFail, MAX_LOG_LINE};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ID: &str = "test.fake";
/// The log the host keeps of the fake plugin's elevated process.
const ELEVATED_LOG: &str = "plugin-test.fake-elevated";

struct Fixture {
    process: Arc<debrief_plugin_host::PluginProcess>,
    registry: PluginRegistry,
    data: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        let exe = Path::new(env!("CARGO_BIN_EXE_debrief-plugin-host-fake"));
        let manifest = debrief_plugin_host::builtins::load_manifests(exe)
            .unwrap()
            .into_iter()
            .find(|m| &*m.id == ID)
            .unwrap();
        let mut spec = debrief_plugin_host::PluginSpec::new(exe, ID);
        spec.elevate = false;
        let process = debrief_plugin_host::PluginProcess::new(manifest, spec);
        let mut registry = PluginRegistry::new();
        registry
            .install_shared(Arc::new(debrief_plugin_host::RemotePlugin::new(process.clone())))
            .unwrap();
        registry.resolve();
        // The data folder sits in a folder of its own, so a file escaping it would be visible next to it.
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir(data.path().join("data")).unwrap();
        Fixture { process, registry, data }
    }

    fn data_dir(&self) -> PathBuf {
        self.data.path().join("data")
    }

    fn invoke(&self, op: &str, args: Value) -> Result<Value, String> {
        let ctx = ToolCtx::new(Some(self.data_dir()), Arc::new(NoTasks));
        self.registry.tools().invoke("fake.tool", op, args, &ctx).map_err(|e| e.to_string())
    }

    /// Every file under the fixture's folder (the data folder and next to it), relative.
    fn files(&self) -> Vec<String> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else {
                    out.push(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        let mut out = Vec::new();
        walk(self.data.path(), self.data.path(), &mut out);
        out.sort();
        out
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.process.shutdown();
    }
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

/// Waits up to 10 s for `cond`.
fn eventually(mut cond: impl FnMut() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(60) {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn output_bytes_arrive_intact_in_the_folder_the_host_chose() {
    let f = Fixture::new();
    // Several pieces and credits' worth, not a multiple of the piece size.
    let size = 1_300_007;
    let out = f
        .invoke("admin-save", json!({"folder": "fake", "name": "trace.bin", "size": size}))
        .unwrap();
    assert_eq!(out, json!({"name": "trace.bin", "bytes": size}));
    let saved = f.data_dir().join("captures").join("fake").join("trace.bin");
    assert_eq!(std::fs::read(&saved).unwrap(), pattern(size));
    assert_eq!(f.files(), ["data/captures/fake/trace.bin"], "no partial file is left");
    // An empty output is a file too, and an existing file is never replaced.
    f.invoke("admin-save", json!({"folder": "fake", "name": "empty.bin", "size": 0}))
        .unwrap();
    assert_eq!(std::fs::metadata(f.data_dir().join("captures/fake/empty.bin")).unwrap().len(), 0);
    let e = f
        .invoke("admin-save", json!({"folder": "fake", "name": "trace.bin", "size": 5}))
        .unwrap_err();
    assert!(e.contains("already exists"), "{e}");
    assert_eq!(std::fs::read(&saved).unwrap(), pattern(size));
}

#[test]
fn an_output_over_its_limit_is_cut_off_with_a_clear_error_and_removed() {
    let f = Fixture::new();
    let e = f
        .invoke(
            "admin-save",
            json!({"folder": "fake", "name": "big.bin", "size": 900_000, "max": 300_000}),
        )
        .unwrap_err();
    assert!(e.contains("exceeds its limit of 300000 bytes"), "{e}");
    assert!(eventually(|| f.files().is_empty()), "{:?}", f.files());
    // The elevated process is still fine afterwards.
    f.invoke("admin-save", json!({"folder": "fake", "name": "ok.bin", "size": 10}))
        .unwrap();
}

#[test]
fn malicious_output_names_are_refused_by_the_host() {
    let f = Fixture::new();
    for (folder, name) in [
        ("fake", "../evil.bin"),
        ("fake", "..\\evil.bin"),
        ("fake", "sub/evil.bin"),
        ("fake", "C:\\evil.bin"),
        ("fake", "/tmp/evil.bin"),
        ("fake", "con"),
        ("fake", ".hidden"),
        ("fake", ""),
        ("..", "evil.bin"),
        ("../..", "evil.bin"),
        ("a/b", "evil.bin"),
    ] {
        let e = f
            .invoke("admin-save", json!({"folder": folder, "name": name, "size": 4}))
            .unwrap_err();
        assert!(e.contains("invalid output"), "{folder:?} {name:?}: {e}");
    }
    assert!(f.files().is_empty(), "{:?}", f.files());
}

#[test]
fn a_crash_in_the_middle_of_a_stream_is_a_typed_error_and_leaves_no_partial_file() {
    let f = Fixture::new();
    // A first call tells the host the tool's data folder.
    f.invoke("admin-save", json!({"folder": "fake", "name": "first.bin", "size": 3}))
        .unwrap();
    let req = PluginRequest::InvokeTool {
        tool: "fake.tool".into(),
        op: "admin-save-crash".into(),
        args: Json(json!({"folder": "fake", "name": "crash.bin", "size": 2_000_000})),
        data_dir: Some(f.data_dir().to_string_lossy().into_owned()),
    };
    match f.process.call::<Result<Json, ToolFail>>(&req, Some(Duration::from_secs(30)), true) {
        Err(IpcError::Died(m)) => assert!(m.contains("elevated"), "{m}"),
        other => panic!("expected the elevated process to have died: {other:?}"),
    }
    assert!(eventually(|| f.files() == ["data/captures/fake/first.bin"]), "{:?}", f.files());
    // Why the stream failed is in the elevated log.
    assert!(eventually(|| elevated_messages().iter().any(|m| m.contains("crash.bin broke off"))));
}

/// The messages of the elevated log's newest lines.
fn elevated_messages() -> Vec<String> {
    debrief_ipc::child_log::find(ELEVATED_LOG)
        .map(|l| l.tail_text())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v["@m"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn standard_error_is_forwarded_cut_and_capped() {
    let f = Fixture::new();
    f.invoke("admin-log", json!({"marker": "m1", "lines": 3, "long": 20_000})).unwrap();
    assert!(
        eventually(|| {
            let m = elevated_messages();
            m.iter().any(|l| l == "m1 line 2") && m.iter().any(|l| l.starts_with("m1 long "))
        }),
        "{:?}",
        elevated_messages()
    );
    let long = elevated_messages().into_iter().find(|l| l.starts_with("m1 long ")).unwrap();
    assert!(long.len() <= MAX_LOG_LINE, "a long line is cut ({} bytes)", long.len());
    // A flood: the plugin drains its standard error, sends what its bounds allow and reports the rest as dropped.
    f.invoke("admin-log", json!({"marker": "m2", "lines": 50_000})).unwrap();
    assert!(
        eventually(|| elevated_messages()
            .iter()
            .any(|l| l.contains("lines of the elevated process were dropped"))),
        "{:?}",
        elevated_messages().last()
    );
    // Still responsive.
    f.invoke("admin-log", json!({"marker": "m3", "lines": 1})).unwrap();
}
