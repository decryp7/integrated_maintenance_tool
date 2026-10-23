//! The `dotnet-profiler` tool: what the UI's "Profile .NET process…" dialog
//! calls through the generic `tool_invoke`. `.etl` traces are the native profiler's (one importer and one
//! process picker for .NET and native processes, ADR 0022 §9).
//!
//! | Op | Args | Result |
//! |---|---|---|
//! | `processes` | — | `DotnetProcess[]` |
//! | `start` | `DotnetCaptureRequest` | `DotnetCaptureStatus` (a running capture is a tool task → a Job) |
//! | `status` | — | `{ captures: DotnetCaptureStatus[], outputs }` (polled with the quiet `tool_poll`) |
//! | `stop` | `{ id }` | `DotnetCaptureStatus` (kept; cancel is the Job's: `jobs_cancel`) |
//! | `etw-start`, `etw-poll`, `etw-stop`, `etw-cancel` | see [`super::elevated`] | elevated: ETW recordings, called by this tool's `start` through the host |

use super::captures::{Captures, Request};
use super::elevated::{self, EtwHost, Router};
use crate::Limits;
use anyhow::{anyhow, bail, Context, Result};
use debrief_plugin_api::tools::{OpSpec, Schema, Tool, ToolCtx, ToolFailure, ToolInfo, ToolTask};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The tool id.
pub const TOOL_ID: &str = "dotnet-profiler";
/// Task kind hint (the shell lists such tasks as Jobs).
pub const TASK_KIND: &str = "capture";

pub(crate) struct DotnetTool {
    captures: Arc<Captures>,
    /// Where ETW recordings go (the elevated process); `None` with a test collector.
    router: Option<Arc<Router>>,
    /// The ETW recordings when this is the elevated process (created on its first operation).
    etw: std::sync::OnceLock<EtwHost>,
    limits: Limits,
}

impl DotnetTool {
    pub fn new(captures: Arc<Captures>, router: Option<Arc<Router>>, limits: Limits) -> DotnetTool {
        DotnetTool {
            captures,
            router,
            etw: std::sync::OnceLock::new(),
            limits,
        }
    }

    fn out_dir(ctx: &ToolCtx, sub: &str) -> Result<PathBuf> {
        Ok(ctx
            .data_dir()
            .ok_or_else(|| anyhow!("Debrief has no data folder to write captures to"))?
            .join("captures")
            .join(sub))
    }

    fn start(&self, args: Value, ctx: &ToolCtx) -> Result<Value> {
        let req: Request = serde_json::from_value(args).context("invalid capture request")?;
        if let Some(router) = &self.router {
            router.connect(ctx.elevated());
        }
        let dir = Self::out_dir(ctx, "dotnet")?;
        let status = self.captures.start(req, &dir, self.limits).map_err(|e| anyhow!(e))?;
        if status.needs_elevation {
            // The administrator prompt was declined (or there is no elevated process). Typed, so the shell answers
            // with the "denied" code.
            return Err(anyhow!(ToolFailure::denied(status.error.clone().unwrap_or_default())));
        }
        if self.captures.is_running(&status.id) {
            ctx.task_started(ToolTask {
                id: status.id.clone(),
                label: format!("Recording {} of {} ({})", req.kind.label(), status.process_name, status.pid),
                kind: TASK_KIND.into(),
                stoppable: true,
            });
            // Finalise the capture when it ends by itself, and end its task.
            let (captures, ctx, id) = (self.captures.clone(), ctx.clone(), status.id.clone());
            std::thread::Builder::new()
                .name("debrief-dotnet-watch".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_millis(250));
                    captures.reap();
                    if !captures.is_running(&id) {
                        ctx.task_finished(&id);
                        break;
                    }
                })
                .context("cannot watch the capture")?;
        }
        Ok(serde_json::to_value(status)?)
    }
}

fn id_arg() -> Schema {
    Schema::object().field("id", Schema::text(64))
}

impl Tool for DotnetTool {
    fn id(&self) -> &str {
        TOOL_ID
    }

    fn describe(&self) -> ToolInfo {
        let long = Some(Duration::from_secs(120));
        ToolInfo::new(TOOL_ID, ".NET profiler")
            .describe(
                "Records CPU, allocations and heap snapshots of running .NET processes through ETW (.NET Framework, \
                 needs administrator rights) or EventPipe (.NET Core / .NET 5+),. \
                 Nothing is loaded into the target process.",
            )
            .op(OpSpec::new("processes", "List .NET processes").args(Schema::object()))
            .op(OpSpec::new("start", "Start a capture").args(
                Schema::object()
                    .field("pid", Schema::int(1, i64::from(u32::MAX)))
                    .field("kind", Schema::one_of(["cpu", "allocations", "heap"]))
                    .optional("durationSec", Schema::int(1, super::captures::MAX_DURATION_SEC as i64)),
            ))
            .op(OpSpec::new("status", "List captures").args(Schema::object()).polling())
            .op(OpSpec::new("stop", "Stop a capture").args(id_arg()).budget(long))
            .op(OpSpec::new(elevated::START, "Start an ETW recording (elevated)")
                .args(
                    Schema::object()
                        .field("pid", Schema::int(1, i64::from(u32::MAX)))
                        .field("kind", Schema::one_of(["cpu", "allocations", "heap"]))
                        .field("name", Schema::text(255))
                        .field("maxBytes", Schema::int(1, i64::MAX))
                        .field("maxDurationMs", Schema::int(1, i64::MAX)),
                )
                .elevated())
            .op(OpSpec::new(elevated::POLL, "ETW recording status (elevated)")
                .args(id_arg())
                .elevated())
            .op(OpSpec::new(elevated::STOP, "Stop an ETW recording (elevated)")
                .args(id_arg())
                .budget(Some(elevated::STOP_TIMEOUT))
                .elevated())
            .op(OpSpec::new(elevated::CANCEL, "Cancel an ETW recording (elevated)")
                .args(id_arg())
                .elevated())
    }

    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value> {
        match op {
            "processes" => Ok(json!(self.captures.collector().list_processes())),
            "start" => self.start(args, ctx),
            "status" => {
                self.captures.reap();
                let captures = self.captures.list();
                // Finished files are tool outputs: the shell lets this window open them.
                let outputs: Vec<&str> = captures.iter().filter_map(|c| c.path.as_deref()).collect();
                Ok(json!({ "captures": captures, "outputs": outputs }))
            }
            "stop" => {
                let id = args.get("id").and_then(Value::as_str).unwrap_or_default();
                Ok(serde_json::to_value(self.captures.stop(id).map_err(|e| anyhow!(e))?)?)
            }
            elevated::START | elevated::POLL | elevated::STOP | elevated::CANCEL => {
                self.etw
                    .get_or_init(EtwHost::default)
                    .invoke(op, args, ctx.data_dir(), ctx.outputs())
            }
            other => bail!("unknown operation {other}"),
        }
    }

    /// Cancel from the Jobs pane (`jobs_cancel`, the one cancel authority): a capture is stopped at once and its
    /// file deleted. Stopping a capture and keeping it is the `stop` operation.
    fn stop(&self, task: &str, _ctx: &ToolCtx) -> Result<()> {
        self.captures.cancel(task).map(|_| ()).map_err(|e| anyhow!(e))
    }
}

#[cfg(test)]
mod tests {
    use super::super::captures::tests::fake;
    use super::*;
    use debrief_plugin_api::tools::{NoTasks, TaskSink};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Sink(Mutex<Vec<String>>);

    impl TaskSink for Sink {
        fn started(&self, _: &str, t: &ToolTask) {
            self.0.lock().unwrap().push(format!("started {} {}", t.id, t.kind));
        }
        fn progress(&self, _: &str, _: &str, _: Option<f32>) {}
        fn finished(&self, _: &str, t: &str) {
            self.0.lock().unwrap().push(format!("finished {t}"));
        }
    }

    fn tool(ends: Option<Duration>) -> DotnetTool {
        DotnetTool::new(Captures::new(fake(ends)), None, Limits::default())
    }

    #[test]
    fn describes_valid_ops() {
        let info = tool(None).describe();
        assert!(info.check().is_ok(), "{:?}", info.check());
        assert_eq!(info.ops.len(), 8);
        assert_eq!(info.ops.iter().filter(|o| o.elevated).count(), 4);
        assert!(
            !info.ops.iter().any(|o| o.name.starts_with("etl-")),
            ".etl is the native profiler's"
        );
        assert!(info.ops.iter().any(|o| o.name == "status" && o.poll));
        assert!(info.ops.iter().filter(|o| o.poll).count() == 1);
    }

    #[test]
    fn capture_lifecycle_through_the_tool_reports_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Arc::new(Sink::default());
        let ctx = ToolCtx::new(Some(dir.path().to_path_buf()), sink.clone()).for_tool(TOOL_ID);
        let t = tool(None);
        let procs = t.invoke("processes", json!({}), &ctx).unwrap();
        assert_eq!(procs.as_array().unwrap().len(), 2);
        let s = t
            .invoke("start", json!({"pid": 10, "kind": "cpu", "durationSec": null}), &ctx)
            .unwrap();
        let id = s["id"].as_str().unwrap().to_string();
        assert_eq!(s["state"], "recording");
        assert!(t.invoke("start", json!({"pid": 10, "kind": "cpu"}), &ctx).is_err());
        assert_eq!(
            t.invoke("status", json!({}), &ctx).unwrap()["captures"].as_array().unwrap().len(),
            1
        );
        // The stop operation keeps the capture; its file is a tool output.
        t.invoke("stop", json!({"id": id}), &ctx).unwrap();
        let st = t.invoke("status", json!({}), &ctx).unwrap();
        assert_eq!(st["captures"][0]["state"], "done");
        assert!(st["captures"][0]["path"].as_str().unwrap().contains("captures"));
        assert_eq!(st["outputs"][0], st["captures"][0]["path"]);
        let start = std::time::Instant::now();
        while !sink.0.lock().unwrap().iter().any(|e| e.starts_with("finished")) && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let events = sink.0.lock().unwrap().clone();
        assert_eq!(events, [format!("started {id} {TASK_KIND}"), format!("finished {id}")]);
        let s = t.invoke("start", json!({"pid": 11, "kind": "heap"}), &ctx).unwrap();
        // Cancel from the Jobs pane discards the capture.
        t.stop(s["id"].as_str().unwrap(), &ctx).unwrap();
        assert_eq!(t.invoke("status", json!({}), &ctx).unwrap()["captures"][0]["state"], "cancelled");
        assert!(
            t.invoke("cancel", json!({"id": s["id"]}), &ctx).is_err(),
            "no cancel operation: Jobs cancel"
        );
        assert!(t.invoke("stop", json!({"id": "nope"}), &ctx).is_err());
        assert!(t.invoke("bogus", json!({}), &ctx).is_err());
        assert!(t.stop("nope", &ctx).is_err());
        assert!(t.invoke("start", json!({"pid": 10, "kind": "nope"}), &ctx).is_err());
    }

    #[test]
    fn self_ending_captures_finish_their_task_and_need_a_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        let sink = Arc::new(Sink::default());
        let ctx = ToolCtx::new(Some(dir.path().to_path_buf()), sink.clone()).for_tool(TOOL_ID);
        let t = tool(Some(Duration::from_millis(30)));
        let s = t.invoke("start", json!({"pid": 10, "kind": "cpu"}), &ctx).unwrap();
        let start = std::time::Instant::now();
        while !sink.0.lock().unwrap().iter().any(|e| e.starts_with("finished")) && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(t.invoke("status", json!({}), &ctx).unwrap()["captures"][0]["state"], "done");
        assert_eq!(s["kind"], "cpu");
        let no_dir = ToolCtx::new(None, Arc::new(NoTasks));
        assert!(t
            .invoke("start", json!({"pid": 11, "kind": "cpu"}), &no_dir)
            .unwrap_err()
            .to_string()
            .contains("data folder"));
    }

    #[test]
    fn missing_rights_are_a_typed_denied_failure() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(Some(dir.path().to_path_buf()), Arc::new(NoTasks));
        let f = fake(None);
        f.deny.store(true, std::sync::atomic::Ordering::SeqCst);
        let t = DotnetTool::new(Captures::new(f), None, Limits::default());
        let e = t.invoke("start", json!({"pid": 10, "kind": "cpu"}), &ctx).unwrap_err();
        let failure = e.downcast_ref::<ToolFailure>().expect("a typed failure");
        assert_eq!(*failure, ToolFailure::denied(failure.message.clone()));
        assert!(failure.message.contains("administrator"));
        // The failed capture is still listed.
        assert_eq!(t.invoke("status", json!({}), &ctx).unwrap()["captures"][0]["state"], "failed");
    }
}
