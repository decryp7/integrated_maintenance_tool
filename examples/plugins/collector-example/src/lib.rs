//! An example collector plugin: a tool whose `start` operation runs in the background, reports itself as a task
//! (listed in IMT's Jobs pane and tray, with Stop), and writes what it collects into the plugin's private data
//! folder. Third-party plugins never write elsewhere: what they collect reaches the user through IMT.
//!
//! - `start { count, intervalMs }`: records `count` heartbeats `intervalMs` apart into `heartbeats.csv`;
//! - `samples`: the heartbeats so far (polled by the UI);
//! - stopping the task ends it early.

use anyhow::{bail, Context, Result};
use imt_plugin_api::prelude::*;
use imt_plugin_api::tools::{OpSpec, Schema, ToolTask};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The task's id.
pub const TASK: &str = "heartbeat";

/// The collector's state, shared with its background thread.
#[derive(Default)]
pub struct Heartbeat {
    samples: Arc<Mutex<Vec<u64>>>,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

impl Tool for Heartbeat {
    fn id(&self) -> &str {
        "heartbeat"
    }

    fn describe(&self) -> ToolInfo {
        ToolInfo::new("heartbeat", "Heartbeat collector")
            .op(OpSpec::new("start", "Start").args(
                Schema::object()
                    .field("count", Schema::int(1, 10_000))
                    .field("intervalMs", Schema::int(10, 60_000)),
            ))
            .op(OpSpec::new("samples", "Samples").polling())
    }

    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value> {
        match op {
            "start" => {
                if self.running.swap(true, Ordering::SeqCst) {
                    bail!("the collector is already running");
                }
                let count = args["count"].as_u64().unwrap_or(1);
                let interval = Duration::from_millis(args["intervalMs"].as_u64().unwrap_or(1000));
                let opened = ctx
                    .data_dir()
                    .context("IMT gave the plugin no data folder")
                    .and_then(|d| std::fs::File::create(d.join("heartbeats.csv")).context("cannot write heartbeats.csv"));
                let mut file = match opened {
                    Ok(f) => f,
                    Err(e) => {
                        self.running.store(false, Ordering::SeqCst);
                        return Err(e);
                    }
                };
                let (samples, stop, running, ctx) = (self.samples.clone(), self.stop.clone(), self.running.clone(), ctx.clone());
                stop.store(false, Ordering::SeqCst);
                if let Ok(mut s) = samples.lock() {
                    s.clear();
                }
                ctx.task_started(ToolTask {
                    id: TASK.into(),
                    label: "Heartbeat collector".into(),
                    kind: "collector".into(),
                    stoppable: true,
                });
                std::thread::spawn(move || {
                    for i in 0..count {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let t = now_ms();
                        let _ = writeln!(file, "{i},{t}");
                        if let Ok(mut s) = samples.lock() {
                            s.push(t);
                        }
                        ctx.task_progress(TASK, Some((i + 1) as f32 / count as f32));
                        std::thread::sleep(interval);
                    }
                    let _ = file.flush();
                    running.store(false, Ordering::SeqCst);
                    ctx.task_finished(TASK);
                });
                Ok(json!({ "task": TASK }))
            }
            "samples" => {
                let s = self.samples.lock().map(|s| s.clone()).unwrap_or_default();
                Ok(json!({ "running": self.running.load(Ordering::SeqCst), "samples": s }))
            }
            _ => bail!("no operation \"{op}\""),
        }
    }

    fn stop(&self, task: &str, _ctx: &ToolCtx) -> Result<()> {
        if task != TASK {
            bail!("no task \"{task}\"");
        }
        self.stop.store(true, Ordering::SeqCst);
        Ok(())
    }
}

/// The plugin.
pub struct HeartbeatPlugin;

impl Plugin for HeartbeatPlugin {
    fn id(&self) -> &str {
        "net.decryptology.example.collector"
    }

    fn name(&self) -> &str {
        "Heartbeat collector (example)"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_tool(Arc::new(Heartbeat::default()));
        Ok(())
    }
}
