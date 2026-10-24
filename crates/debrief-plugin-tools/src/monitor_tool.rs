//! The process monitor as a tool: one app-wide run writing rolling CLEF files.

use crate::ids::PROCESS_MONITOR;
use crate::run_slot::{Run, RunSlot, Seen};
use anyhow::{anyhow, bail, Result};
use debrief_core::plugin::Collector;
use debrief_plugin_api::clef::ClefFileSink;
use debrief_plugin_api::tools::{OpSpec, PathAccess, PathKind, Schema, Tool, ToolCtx, ToolInfo, ToolTask};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Id of the running monitor in the background task list.
pub(crate) const TASK: &str = "monitor";
/// Rolling CLEF files: at most this many of this size.
const FILE_BYTES: u64 = 10 * 1024 * 1024;
const FILES: usize = 10;

/// A running monitor: its thread and the flag that stops it.
struct MonitorRun {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Result<()>>,
}

impl Run for MonitorRun {
    fn is_running(&self) -> bool {
        !self.thread.is_finished()
    }

    fn snapshot(&self) -> Value {
        Value::Null
    }

    fn finish(self) -> Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.join().map_err(|_| anyhow!("the monitor thread panicked"))?
    }
}

pub(crate) struct MonitorTool {
    collector: Arc<dyn Collector>,
    run: RunSlot<MonitorRun>,
}

impl MonitorTool {
    pub fn new(collector: Arc<dyn Collector>) -> MonitorTool {
        MonitorTool {
            collector,
            run: RunSlot::default(),
        }
    }

    fn start(&self, args: Value, ctx: &ToolCtx) -> Result<Value> {
        let mut started = PathBuf::new();
        self.run.begin("the process monitor is already running", || {
            let config = args.get("config").cloned().unwrap_or(Value::Null);
            debrief_monitor::config::Config::from_json(&config)?;
            let dir = match args.get("dir").and_then(Value::as_str) {
                Some(d) => PathBuf::from(d),
                None => ctx
                    .data_dir()
                    .ok_or_else(|| anyhow!("choose a folder for the monitor's files"))?
                    .join("monitor")
                    .join(chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string()),
            };
            let sink = ClefFileSink::new(&dir, "process-monitor", FILE_BYTES, FILES)?;
            let stop = Arc::new(AtomicBool::new(false));
            let (flag, collector, done) = (stop.clone(), self.collector.clone(), ctx.clone());
            let thread = std::thread::Builder::new().name("debrief-process-monitor".into()).spawn(move || {
                // The collector is plugin code on its own thread: contain a panic.
                let result = debrief_plugin_api::clef::catch("the process monitor", || collector.run(&config, &sink, &flag));
                if !flag.load(Ordering::SeqCst) {
                    // Ended on its own (an error): it is no longer background work.
                    done.task_finished(TASK);
                }
                result
            })?;
            started = dir.clone();
            Ok((MonitorRun { stop, thread }, dir))
        })?;
        ctx.task_started(ToolTask {
            id: TASK.into(),
            label: "Process monitor".into(),
            kind: "process-monitor".into(),
            stoppable: true,
        });
        Ok(json!(started.to_string_lossy()))
    }

    /// Stops the run; its folder, `None` when nothing was running, or the failure of a run that ended on its own.
    fn stop_run(&self, ctx: &ToolCtx) -> Result<Option<String>> {
        let r = self.run.end();
        if !matches!(r, Ok(None)) {
            ctx.task_finished(TASK);
        }
        r
    }

    fn status(&self) -> Value {
        self.run.look(|s| match s {
            Seen::Idle => json!({ "running": false, "dir": null }),
            Seen::Running { dir, .. } => json!({ "running": true, "dir": dir }),
            Seen::Ended { dir, error, .. } => json!({ "running": false, "dir": dir, "error": error }),
        })
    }
}

impl Tool for MonitorTool {
    fn id(&self) -> &str {
        PROCESS_MONITOR
    }

    fn describe(&self) -> ToolInfo {
        ToolInfo::new(PROCESS_MONITOR, "Process monitor")
            .describe("Samples processes (and, on Windows, performance counters) into rolling CLEF files that open with live tail.")
            .op(OpSpec::new("start", "Start monitoring")
                .args(
                    Schema::object()
                        .field("config", Schema::Any)
                        .optional("dir", Schema::path(PathKind::Dir, PathAccess::Write)),
                )
                .budget(Some(Duration::from_secs(10))))
            .op(OpSpec::new("stop", "Stop monitoring"))
            .op(OpSpec::new("status", "Whether it runs").budget(Some(Duration::from_secs(5))))
    }

    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value> {
        match op {
            "start" => self.start(args, ctx),
            "stop" => Ok(json!(self.stop_run(ctx)?)),
            "status" => Ok(self.status()),
            other => bail!("unknown operation \"{other}\""),
        }
    }

    fn stop(&self, task: &str, ctx: &ToolCtx) -> Result<()> {
        if task != TASK {
            bail!("the process monitor has no background task \"{task}\"");
        }
        self.stop_run(ctx).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_plugin_api::clef::EventSink;
    use debrief_plugin_api::model::Record;
    use debrief_plugin_api::tools::TaskSink;
    use std::sync::Mutex;

    /// Writes one event per tick until stopped; or fails / panics on demand.
    struct FakeMonitor(&'static str);

    impl Collector for FakeMonitor {
        fn id(&self) -> &str {
            "process-monitor"
        }
        fn name(&self) -> &str {
            "fake"
        }
        fn run(&self, _: &Value, sink: &dyn EventSink, stop: &AtomicBool) -> Result<()> {
            match self.0 {
                "fail" => bail!("sampling failed"),
                "panic" => panic!("sampler bug"),
                _ => {
                    // At least one tick, however soon it is stopped (a loaded test machine).
                    loop {
                        let mut r = Record::new(0);
                        r.message = "tick".into();
                        sink.write(&[r])?;
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Ok(())
                }
            }
        }
    }

    #[derive(Default)]
    struct Tasks(Mutex<Vec<String>>);
    impl TaskSink for Tasks {
        fn started(&self, tool: &str, t: &ToolTask) {
            self.0.lock().unwrap().push(format!("+{tool} {} {}", t.id, t.kind));
        }
        fn progress(&self, _: &str, _: &str, _: Option<f32>) {}
        fn finished(&self, tool: &str, task: &str) {
            self.0.lock().unwrap().push(format!("-{tool} {task}"));
        }
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("debrief-tools-monitor-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn config() -> Value {
        json!({ "targets": [{ "name": "OrderService" }] })
    }

    #[test]
    fn starts_once_writes_clef_and_stops() {
        let tool = MonitorTool::new(Arc::new(FakeMonitor("ok")));
        assert!(tool.describe().check().is_ok());
        let data = temp("data");
        let tasks = Arc::new(Tasks::default());
        let ctx = ToolCtx::new(Some(data.clone()), tasks.clone()).for_tool(PROCESS_MONITOR);
        assert_eq!(
            tool.invoke("status", json!({}), &ctx).unwrap(),
            json!({ "running": false, "dir": null })
        );
        let dir = tool.invoke("start", json!({ "config": config() }), &ctx).unwrap();
        let dir = PathBuf::from(dir.as_str().unwrap());
        assert!(dir.starts_with(data.join("monitor")), "{dir:?}");
        assert_eq!(tool.invoke("status", json!({}), &ctx).unwrap()["running"], true);
        let again = tool.invoke("start", json!({ "config": config() }), &ctx).unwrap_err();
        assert_eq!(again.to_string(), "the process monitor is already running");
        std::thread::sleep(Duration::from_millis(30));
        let stopped = tool.invoke("stop", json!({}), &ctx).unwrap();
        assert_eq!(stopped, json!(dir.to_string_lossy()));
        let written: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert!(!written.is_empty(), "CLEF files were written");
        // Stopping again is fine; nothing runs.
        assert_eq!(tool.invoke("stop", json!({}), &ctx).unwrap(), Value::Null);
        assert_eq!(
            *tasks.0.lock().unwrap(),
            ["+process-monitor monitor process-monitor", "-process-monitor monitor"]
        );
        let _ = std::fs::remove_dir_all(data);
    }

    #[test]
    fn the_tray_stops_it_through_the_tool() {
        let tool = MonitorTool::new(Arc::new(FakeMonitor("ok")));
        let dir = temp("tray");
        let ctx = ToolCtx::new(None, Arc::new(Tasks::default()));
        tool.invoke("start", json!({ "config": config(), "dir": dir }), &ctx).unwrap();
        assert!(tool.stop("perfmon:x", &ctx).is_err());
        tool.stop(TASK, &ctx).unwrap();
        assert_eq!(tool.status()["running"], false);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_configs_and_missing_folders_are_refused() {
        let tool = MonitorTool::new(Arc::new(FakeMonitor("ok")));
        let ctx = ToolCtx::new(None, Arc::new(Tasks::default()));
        let e = tool.invoke("start", json!({ "config": { "targets": [] } }), &ctx).unwrap_err();
        assert!(e.to_string().contains("at least one entry"), "{e}");
        let e = tool.invoke("start", json!({ "config": config() }), &ctx).unwrap_err();
        assert_eq!(e.to_string(), "choose a folder for the monitor's files");
        assert!(tool.invoke("nope", json!({}), &ctx).is_err());
    }

    /// Waits (bounded) until the run is no longer running.
    fn wait_ended(tool: &MonitorTool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while tool.status()["running"] == true {
            assert!(std::time::Instant::now() < deadline, "the failed run ended");
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_run_that_failed_on_its_own_is_over_and_the_next_start_works() {
        for mode in ["fail", "panic"] {
            let tool = MonitorTool::new(Arc::new(FakeMonitor(mode)));
            let dir = temp(&format!("again-{mode}"));
            let tasks = Arc::new(Tasks::default());
            let ctx = ToolCtx::new(None, tasks.clone()).for_tool(PROCESS_MONITOR);
            tool.invoke("start", json!({ "config": config(), "dir": dir }), &ctx).unwrap();
            wait_ended(&tool);
            let status = tool.status();
            assert_eq!(status["running"], false, "{mode}");
            assert!(
                status["error"].as_str().is_some_and(|e| !e.is_empty()),
                "{mode}: the status says why: {status}"
            );
            // Start again without Stop: the failed run does not block it.
            tool.invoke("start", json!({ "config": config(), "dir": dir }), &ctx)
                .unwrap_or_else(|e| panic!("{mode}: {e}"));
            wait_ended(&tool);
            let _ = tool.invoke("stop", json!({}), &ctx);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_failing_or_crashing_monitor_ends_its_task_and_reports_on_stop() {
        for (mode, message) in [("fail", "sampling failed"), ("panic", "sampler bug")] {
            let tool = MonitorTool::new(Arc::new(FakeMonitor(mode)));
            let dir = temp(mode);
            let tasks = Arc::new(Tasks::default());
            let ctx = ToolCtx::new(None, tasks.clone()).for_tool(PROCESS_MONITOR);
            tool.invoke("start", json!({ "config": config(), "dir": dir }), &ctx).unwrap();
            wait_ended(&tool);
            assert!(tasks.0.lock().unwrap().contains(&"-process-monitor monitor".to_string()));
            let e = tool.invoke("stop", json!({}), &ctx).unwrap_err().to_string();
            assert!(e.contains(message), "{e}");
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
