//! The native profiling plugin as IMT runs it (its own `imt-plugins` process, over the real IPC, sandboxed with
//! the rights it asks for): it lists processes, samples this test process by suspend-and-sample while one of its
//! threads spins in a known function, reports the capture as a tool task, and writes a NetTrace with symbols.
#![cfg(windows)]

mod common;

use imt_core::plugin::tools::{NoTasks, TaskSink, ToolCtx, ToolTask};
use imt_plugin_profiling::nettrace::NettraceImporter;
use imt_profile::{Capture, CaptureImporter, Limits};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NATIVE: &str = "net.decryptology.native";

#[inline(never)]
fn native_e2e_spin(stop: &AtomicBool) -> u64 {
    let mut x = 0x2545_f491_4f6c_dd1du64;
    while !stop.load(Ordering::Relaxed) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
    }
    x
}

#[derive(Default)]
struct Tasks(Mutex<Vec<String>>);

impl TaskSink for Tasks {
    fn started(&self, tool: &str, task: &ToolTask) {
        self.0.lock().unwrap().push(format!("started {tool} {} {}", task.id, task.kind));
    }
    fn progress(&self, _: &str, _: &str, _: Option<f32>) {}
    fn finished(&self, tool: &str, task: &str) {
        self.0.lock().unwrap().push(format!("finished {tool} {task}"));
    }
}

#[test]
fn samples_a_process_out_of_process_and_reports_the_capture_as_a_task() {
    let registry = common::registry(&[NATIVE]);
    let tools = registry.tools();
    let listed: Vec<String> = tools.list().into_iter().map(|t| t.info.id).collect();
    assert!(listed.contains(&"native-profiler".to_string()) && listed.contains(&"native-profiler.etw".to_string()));
    let plain = ToolCtx::new(None, Arc::new(NoTasks));
    let defaults = tools.invoke("native-profiler", "defaults", json!({}), &plain).unwrap();
    assert_eq!(defaults["supported"], true);
    assert_eq!(defaults["symbolServer"], false, "downloads are off by default");
    let me = std::process::id();
    let procs = tools.invoke("native-profiler", "processes", json!({}), &plain).unwrap();
    let mine = procs.as_array().unwrap().iter().find(|p| p["pid"] == me).cloned();
    assert_eq!(mine.as_ref().map(|p| p["accessible"].clone()), Some(json!(true)), "{mine:?}");

    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let spinner = std::thread::spawn(move || native_e2e_spin(&s2));
    let data = tempfile::tempdir().unwrap();
    let tasks = Arc::new(Tasks::default());
    let ctx = ToolCtx::new(Some(data.path().to_path_buf()), tasks.clone());
    let started = tools
        .invoke(
            "native-profiler",
            "start",
            json!({ "pid": me, "durationSec": 2, "rateHz": 200 }),
            &ctx,
        )
        .unwrap();
    assert_eq!(started["state"], "recording", "{started}");
    let id = started["id"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(60);
    let done: Value = loop {
        // Task reports go to the context of the tool's last call: poll with the same one, as the shell does.
        let status = tools.invoke("native-profiler", "status", json!({}), &ctx).unwrap();
        let c = status["captures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id.as_str())
            .cloned()
            .unwrap();
        if c["state"] != "recording" && c["state"] != "stopping" {
            assert!(status["outputs"].as_array().unwrap().iter().any(|o| o == &c["path"]), "{status}");
            break c;
        }
        assert!(Instant::now() < deadline, "the capture did not end: {c}");
        std::thread::sleep(Duration::from_millis(200));
    };
    stop.store(true, Ordering::Relaxed);
    spinner.join().unwrap();
    assert_eq!(done["state"], "done", "{done}");
    let path = done["path"].as_str().unwrap();
    assert!(path.starts_with(&*data.path().to_string_lossy()), "{path}");
    let bytes = std::fs::read(path).unwrap();
    let Capture::Profile(p) = NettraceImporter.import("me.nettrace", &bytes, &Limits::default()).unwrap() else {
        panic!("a profile");
    };
    assert!(
        p.frames.iter().any(|f| f.function.ends_with("native_e2e_spin") && f.file.is_some()),
        "{:?}",
        p.notes
    );
    // The task started and ended (the shell turns it into a Job).
    std::thread::sleep(Duration::from_millis(500));
    let seen = tasks.0.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|t| t.starts_with("started native-profiler") && t.ends_with("capture")),
        "{seen:?}"
    );
    assert!(seen.iter().any(|t| t.starts_with("finished native-profiler")), "{seen:?}");
    // Starting a capture of a process that does not exist is an error, not a crash of the plugin.
    assert!(tools
        .invoke("native-profiler", "start", json!({ "pid": u32::MAX - 9 }), &ctx)
        .is_err());
    assert!(tools.invoke("native-profiler", "defaults", json!({}), &plain).is_ok());
}
