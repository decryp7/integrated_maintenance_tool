//! The `dotnet-profiler` tool: what the UI's "Profile .NET process…" dialog
//! and the `.etl` process picker call through the generic `tool_invoke`.
//!
//! | Op | Args | Result |
//! |---|---|---|
//! | `processes` | — | `DotnetProcess[]` |
//! | `start` | `DotnetCaptureRequest` | `DotnetCaptureStatus` (a running capture is a tool task → a Job) |
//! | `status` | — | `{ captures: DotnetCaptureStatus[], outputs }` (polled with the quiet `tool_poll`) |
//! | `stop` | `{ id }` | `DotnetCaptureStatus` (kept; cancel is the Job's: `jobs_cancel`) |
//! | `etl-processes` | `{ path }` | `EtlProcess[]` (Windows) |
//! | `etl-convert` | `{ path, pid }` | `{ path, notes }`: a NetTrace to open with `profiling_open` |

use super::captures::{Captures, Request};
use crate::Limits;
use anyhow::{anyhow, bail, Context, Result};
use imt_core::plugin::tools::{OpSpec, Schema, Tool, ToolCtx, ToolFailure, ToolInfo, ToolTask};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The tool id.
pub const TOOL_ID: &str = "dotnet-profiler";
/// Task kind hint (the shell lists such tasks as Jobs).
pub const TASK_KIND: &str = "capture";
/// Largest `.etl` file converted.
pub const MAX_ETL_BYTES: u64 = 8 << 30;
const MAX_PATH: usize = 4096;

pub(crate) struct DotnetTool {
    captures: Arc<Captures>,
    limits: Limits,
    conversions: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl DotnetTool {
    pub fn new(captures: Arc<Captures>, limits: Limits) -> DotnetTool {
        DotnetTool {
            captures,
            limits,
            conversions: Mutex::new(HashMap::new()),
        }
    }

    fn out_dir(ctx: &ToolCtx, sub: &str) -> Result<PathBuf> {
        Ok(ctx
            .data_dir()
            .ok_or_else(|| anyhow!("IMT has no data folder to write captures to"))?
            .join("captures")
            .join(sub))
    }

    fn start(&self, args: Value, ctx: &ToolCtx) -> Result<Value> {
        let req: Request = serde_json::from_value(args).context("invalid capture request")?;
        let dir = Self::out_dir(ctx, "dotnet")?;
        let status = self.captures.start(req, &dir, self.limits).map_err(|e| anyhow!(e))?;
        if status.needs_elevation {
            // Typed, so the shell answers with the "denied" code and the restart-as-administrator hint.
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
                .name("imt-dotnet-watch".into())
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

    fn etl_path(args: &Value) -> Result<PathBuf> {
        let raw = args.get("path").and_then(Value::as_str).unwrap_or_default();
        let path = Path::new(raw).canonicalize().with_context(|| format!("cannot open {raw}"))?;
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("etl")) {
            bail!("{} is not an .etl file", path.display());
        }
        let len = std::fs::metadata(&path)?.len();
        if len > MAX_ETL_BYTES {
            bail!(
                "{} is larger than {} GB; record a shorter trace",
                path.display(),
                MAX_ETL_BYTES >> 30
            );
        }
        Ok(path)
    }

    fn etl_processes(&self, args: Value) -> Result<Value> {
        let path = Self::etl_path(&args)?;
        etl::processes(&path).map(|p| json!(p))
    }

    fn etl_convert(&self, args: Value, ctx: &ToolCtx) -> Result<Value> {
        let path = Self::etl_path(&args)?;
        let pid = args
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|p| u32::try_from(p).ok())
            .ok_or_else(|| anyhow!("choose a process"))?;
        let dir = Self::out_dir(ctx, "etl")?;
        std::fs::create_dir_all(&dir)?;
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let out = dir.join(format!("{stem}-{pid}.nettrace"));
        let id = format!("etl-{pid}-{}", self.conversions.lock().unwrap_or_else(|p| p.into_inner()).len());
        let cancel = Arc::new(AtomicBool::new(false));
        self.conversions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), cancel.clone());
        ctx.task_started(ToolTask {
            id: id.clone(),
            label: format!(
                "Converting {} (process {pid})",
                path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
            ),
            kind: TASK_KIND.into(),
            stoppable: true,
        });
        let result = etl::convert(&path, pid, &out, self.limits.max_bytes, &cancel);
        ctx.task_finished(&id);
        self.conversions.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
        let notes = result?;
        Ok(json!({ "path": out.to_string_lossy(), "notes": notes }))
    }
}

#[cfg(windows)]
mod etl {
    use crate::etw::etl;
    use crate::etw::scan::EtlProcess;
    use anyhow::{anyhow, Result};
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    pub(super) fn processes(path: &Path) -> Result<Vec<EtlProcess>> {
        Ok(etl::scan(path, &AtomicBool::new(false))?)
    }

    pub(super) fn convert(path: &Path, pid: u32, out: &Path, max_bytes: u64, cancel: &AtomicBool) -> Result<Vec<String>> {
        let procs = etl::scan(path, cancel)?;
        let p = procs
            .iter()
            .find(|p| p.pid == pid)
            .ok_or_else(|| anyhow!("process {pid} is not in the trace"))?;
        let c = etl::convert(path, p, out, max_bytes, cancel).inspect_err(|_| {
            let _ = std::fs::remove_file(out);
        })?;
        Ok(c.notes)
    }
}

#[cfg(not(windows))]
mod etl {
    use anyhow::{bail, Result};
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    pub(super) fn processes(_: &Path) -> Result<Vec<crate::etw::scan::EtlProcess>> {
        bail!("ETL import needs Windows")
    }

    pub(super) fn convert(_: &Path, _: u32, _: &Path, _: u64, _: &AtomicBool) -> Result<Vec<String>> {
        bail!("ETL import needs Windows")
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
                 needs administrator rights) or EventPipe (.NET Core / .NET 5+), and converts .etl traces. \
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
            .op(OpSpec::new("etl-processes", "List the processes in an .etl trace")
                .args(Schema::object().field("path", Schema::text(MAX_PATH)))
                .budget(Some(Duration::from_secs(1800))))
            .op(OpSpec::new("etl-convert", "Convert one process of an .etl trace")
                .args(
                    Schema::object()
                        .field("path", Schema::text(MAX_PATH))
                        .field("pid", Schema::int(1, i64::from(u32::MAX))),
                )
                .budget(Some(Duration::from_secs(3600))))
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
            "etl-processes" => self.etl_processes(args),
            "etl-convert" => self.etl_convert(args, ctx),
            other => bail!("unknown operation {other}"),
        }
    }

    /// Cancel from the Jobs pane (`jobs_cancel`, the one cancel authority): a capture is stopped at once and its
    /// file deleted; a conversion is cancelled. Stopping a capture and keeping it is the `stop` operation.
    fn stop(&self, task: &str, _ctx: &ToolCtx) -> Result<()> {
        if let Some(flag) = self.conversions.lock().unwrap_or_else(|p| p.into_inner()).get(task) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            return Ok(());
        }
        self.captures.cancel(task).map(|_| ()).map_err(|e| anyhow!(e))
    }
}

#[cfg(test)]
mod tests {
    use super::super::captures::tests::fake;
    use super::*;
    use imt_core::plugin::tools::{NoTasks, TaskSink};

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
        DotnetTool::new(Captures::new(fake(ends)), Limits::default())
    }

    #[test]
    fn describes_valid_ops() {
        let info = tool(None).describe();
        assert!(info.check().is_ok(), "{:?}", info.check());
        assert_eq!(info.ops.len(), 6);
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
        let t = DotnetTool::new(Captures::new(f), Limits::default());
        let e = t.invoke("start", json!({"pid": 10, "kind": "cpu"}), &ctx).unwrap_err();
        let failure = e.downcast_ref::<ToolFailure>().expect("a typed failure");
        assert_eq!(*failure, ToolFailure::denied(failure.message.clone()));
        assert!(failure.message.contains("administrator"));
        // The failed capture is still listed.
        assert_eq!(t.invoke("status", json!({}), &ctx).unwrap()["captures"][0]["state"], "failed");
    }

    #[test]
    fn etl_paths_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolCtx::new(Some(dir.path().to_path_buf()), Arc::new(NoTasks));
        let t = tool(None);
        let txt = dir.path().join("a.txt");
        std::fs::write(&txt, b"x").unwrap();
        assert!(t
            .invoke("etl-processes", json!({"path": txt}), &ctx)
            .unwrap_err()
            .to_string()
            .contains("not an .etl"));
        assert!(t
            .invoke("etl-processes", json!({"path": dir.path().join("missing.etl")}), &ctx)
            .is_err());
        let junk = dir.path().join("junk.etl");
        std::fs::write(&junk, vec![1u8; 100]).unwrap();
        let e = t.invoke("etl-processes", json!({"path": junk}), &ctx).unwrap_err().to_string();
        if !cfg!(windows) {
            assert_eq!(e, "ETL import needs Windows");
        }
        assert!(t.invoke("etl-convert", json!({"path": junk, "pid": 5}), &ctx).is_err());
        assert!(t.invoke("etl-convert", json!({"path": junk}), &ctx).is_err());
    }
}
