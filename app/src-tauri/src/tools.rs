//! Generic dispatch of plugin tools (ADR 0002): `tools_list` lists the tools
//! of the calling window's plugins with their availability, and
//! `tool_invoke { tool, op, args }` runs one operation. The shell knows no
//! tool: arguments are validated by the host against the tool's schema, and
//! the call runs under the plugin's guard and the operation's time budget.
//!
//! Background work a tool reports goes through [`ShellTasks`]: what runs until
//! stopped (the process monitor, a PerfMon collector set) is listed in
//! [`BackgroundTasks`] (tray, status bar); a collection that ends by itself (a
//! diagnostics collection) is a collection job (one at a time; a second one is
//! stopped with a reason). Stopping either calls the tool back through the same host.

use crate::background::{BackgroundTasks, TaskKind, TaskSpec};
use crate::windows::CurrentWindow;
use crate::{AppState, CmdResult, State};
use debrief_core::plugin::tools::{TaskSink, ToolCtx, ToolEntry, ToolError, ToolHost, ToolTask, MAX_ID_LEN};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};

/// Lists background work of tools in [`BackgroundTasks`]; its stop action
/// calls [`ToolHost::stop`].
pub(crate) struct ShellTasks<R: Runtime> {
    app: AppHandle<R>,
    host: ToolHost,
    data_dir: Option<PathBuf>,
}

impl<R: Runtime> Clone for ShellTasks<R> {
    fn clone(&self) -> Self {
        ShellTasks {
            app: self.app.clone(),
            host: self.host.clone(),
            data_dir: self.data_dir.clone(),
        }
    }
}

impl<R: Runtime> ShellTasks<R> {
    /// The context a call of `host` gets.
    pub(crate) fn context(app: &AppHandle<R>, host: &ToolHost) -> ToolCtx {
        let data_dir = crate::app_dirs::data(app).ok();
        let sink = ShellTasks {
            app: app.clone(),
            host: host.clone(),
            data_dir: data_dir.clone(),
        };
        ToolCtx::new(data_dir, Arc::new(sink))
    }

    fn tasks(&self) -> Option<tauri::State<'_, BackgroundTasks>> {
        self.app.try_state::<BackgroundTasks>()
    }
}

/// The collection jobs of tool tasks, by task id: the job ends when the tool reports the task finished.
#[derive(Default)]
/// Each entry keeps the task's kind hint: one task per hint runs at a time (a second diagnostics collection is
/// stopped), while tasks of different tools (a collection and a .NET capture) may run together.
struct CollectionJobs(std::sync::Mutex<std::collections::HashMap<String, CollectionJob>>);

/// A collection job: its id, the sender that ends it, and the task's kind hint.
type CollectionJob = (crate::jobs::JobId, std::sync::mpsc::Sender<()>, String);

impl CollectionJobs {
    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, CollectionJob>> {
        crate::sync::lock(&self.0)
    }
}

/// Whether a tool task runs until stopped (a background task) rather than ending by itself (a job).
fn is_service(hint: &str) -> bool {
    matches!(hint, "process-monitor" | "perfmon")
}

impl<R: Runtime> ShellTasks<R> {
    fn stop_task(&self, tool: &str, id: &str) -> anyhow::Result<()> {
        let ctx = ToolCtx::new(self.data_dir.clone(), Arc::new(self.clone()));
        self.host.stop(tool, id, &ctx).map_err(anyhow::Error::from)
    }

    /// Lists a collection as a collection job, or stops it when another of the same kind runs.
    fn track_collection(&self, tool: &str, task: &ToolTask) {
        let Some(jobs) = self.app.try_state::<crate::jobs::Jobs>() else {
            return;
        };
        if self.app.try_state::<CollectionJobs>().is_none() {
            self.app.manage(CollectionJobs::default());
        }
        let Some(registry) = self.app.try_state::<CollectionJobs>() else {
            return;
        };
        if task.kind != CAPTURE_HINT && registry.lock().values().any(|(_, _, hint)| *hint == task.kind) {
            log::warn!(Tool = tool; "A second collection was stopped: one of each kind runs at a time");
            let (sink, tool, id) = (self.clone(), tool.to_string(), task.id.clone());
            // Off the caller's thread: stopping may run a system tool.
            let _ = std::thread::Builder::new().name("debrief-tool-stop".into()).spawn(move || {
                if let Err(e) = sink.stop_task(&tool, &id) {
                    log::warn!(Tool = tool.as_str(); "could not stop the second collection: {e:#}");
                }
            });
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let mut spec = crate::jobs::JobSpec::new(job_kind(&task.kind), task.label.clone());
        if !task.stoppable {
            spec = spec.not_cancellable();
        }
        let (sink, t, id) = (self.clone(), tool.to_string(), task.id.clone());
        let controls = crate::jobs::Controls::cancel(move || {
            if let Err(e) = sink.stop_task(&t, &id) {
                log::warn!(Tool = t.as_str(); "could not stop the collection: {e:#}");
            }
        });
        let rx = std::sync::Mutex::new(rx);
        match jobs.submit_work(spec, controls, move |_| {
            // The tool says when it is done (or the sender goes with Debrief).
            let _ = crate::sync::lock(&rx).recv();
            crate::jobs::queue::JobEnd::Done(None)
        }) {
            Ok(job) => {
                registry.lock().insert(task.id.clone(), (job, tx, task.kind.clone()));
            }
            Err(e) => log::warn!(Tool = tool; "collection not listed as a job: {e}"),
        }
    }
}

/// The job kind of a tool task that ends by itself: profiler captures (hint `capture`) run several at once;
/// everything else is a collection.
fn job_kind(hint: &str) -> crate::jobs::JobKind {
    match hint {
        CAPTURE_HINT => crate::jobs::JobKind::Capture,
        _ => crate::jobs::JobKind::Collection,
    }
}

/// The task kind hint of profiler captures.
const CAPTURE_HINT: &str = "capture";

/// The tray icon of a tool's task kind hint.
fn task_kind(hint: &str) -> TaskKind {
    match hint {
        "process-monitor" => TaskKind::ProcessMonitor,
        "perfmon" => TaskKind::Perfmon,
        _ => TaskKind::Other,
    }
}

impl<R: Runtime> TaskSink for ShellTasks<R> {
    fn started(&self, tool: &str, task: &ToolTask) {
        if !is_service(&task.kind) {
            return self.track_collection(tool, task);
        }
        let Some(tasks) = self.tasks() else { return };
        let stop: Option<crate::background::StopFn> = task.stoppable.then(|| {
            let (sink, tool, id) = (self.clone(), tool.to_string(), task.id.clone());
            let stop: crate::background::StopFn = Arc::new(move || {
                let ctx = ToolCtx::new(sink.data_dir.clone(), Arc::new(sink.clone()));
                sink.host.stop(&tool, &id, &ctx).map_err(anyhow::Error::from)
            });
            stop
        });
        let spec = TaskSpec {
            id: task.id.clone(),
            label: task.label.clone(),
            kind: task_kind(&task.kind),
            window: None,
        };
        if let Err(e) = tasks.register(spec, stop) {
            log::warn!(Tool = tool; "could not list background work: {e:#}");
        }
    }

    fn progress(&self, _tool: &str, task: &str, fraction: Option<f32>) {
        let job = self
            .app
            .try_state::<CollectionJobs>()
            .and_then(|r| r.lock().get(task).map(|(id, _, _)| *id));
        match (job, self.app.try_state::<crate::jobs::Jobs>()) {
            (Some(id), Some(jobs)) => jobs.report(
                id,
                crate::jobs::JobProgress {
                    fraction,
                    ..Default::default()
                },
            ),
            _ => {
                if let Some(tasks) = self.tasks() {
                    tasks.set_progress(task, fraction);
                }
            }
        }
    }

    fn finished(&self, _tool: &str, task: &str) {
        if let Some((_, done, _)) = self.app.try_state::<CollectionJobs>().and_then(|r| r.lock().remove(task)) {
            let _ = done.send(());
            return;
        }
        if let Some(tasks) = self.tasks() {
            tasks.remove(task);
        }
    }
}

/// A tool error as the command's error: its kind is the code (missing rights and programs are typed by the
/// tool, see `ToolFailure`); a missing tool says what the user can do.
fn missing(error: ToolError, safe_mode: bool) -> crate::CmdError {
    use crate::{CmdError, ErrorCode};
    match error {
        ToolError::UnknownTool(tool) if safe_mode => CmdError::unavailable(format!(
            "\"{tool}\" is not available in safe mode (plugins are off); restart normally to use it"
        )),
        ToolError::UnknownTool(tool) => CmdError::not_found(format!(
            "no installed plugin provides the tool \"{tool}\" (is the System tools plugin disabled in the Plugins panel?)"
        )),
        e @ ToolError::UnknownOp { .. } => CmdError::not_found(e.to_string()),
        e @ ToolError::InvalidArgs { .. } => CmdError::invalid(e.to_string()).field("args"),
        e @ ToolError::PathRefused { .. } => {
            CmdError::new(ErrorCode::NotGranted, e.to_string()).hint("Choose the file or folder with the dialog first.")
        }
        ToolError::Unsupported(m) => CmdError::new(ErrorCode::Unsupported, m),
        ToolError::Denied(m) => CmdError::new(ErrorCode::Denied, m).hint("Restart Debrief as administrator, then try again."),
        ToolError::Missing(m) => CmdError::new(ErrorCode::Missing, m),
        ToolError::Failed(m) => CmdError::failed(m),
    }
}

/// Runs `tool`'s `op` for the calling window, off the UI thread.
pub(crate) async fn invoke<R: Runtime>(
    app: &AppHandle<R>,
    window: &CurrentWindow,
    safe_mode: bool,
    tool: &str,
    op: &str,
    args: Value,
) -> CmdResult<Value> {
    if tool.len() > MAX_ID_LEN || op.len() > MAX_ID_LEN {
        return Err("tool and operation names are at most 64 characters".into());
    }
    let host = window.tools();
    // Path arguments must be what this window's user chose (ADR 0016 grants, checked by the tool host).
    let grants = crate::paths::WindowGrants::of(app, window.label());
    let ctx = ShellTasks::context(app, &host).with_paths(Arc::new(grants));
    let (tool, op) = (tool.to_string(), op.to_string());
    tauri::async_runtime::spawn_blocking(move || {
        host.invoke(&tool, &op, args, &ctx).map_err(|e| {
            log_failure(&tool, &op, &e);
            missing(e, safe_mode)
        })
    })
    .await
    .map_err(crate::err)?
}

/// Longest tool error text logged (its tail: an external program's stderr ends with the cause).
const MAX_ERROR_LOG: usize = 1024;

/// The last `max` bytes of `text` (at a character boundary), marked when cut.
fn tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

/// Logs a failed tool operation: bad requests and unavailable tools at Warning, failures of the tool or
/// of the external program it runs (exit code, timeout, stderr) at Error; missing rights or programs at Warning.
/// The text is redacted and bounded.
fn log_failure(tool: &str, op: &str, e: &ToolError) {
    let full = crate::activity::privacy::redact(&e.to_string());
    let timeout = full.contains("did not finish within") || full.contains("timed out");
    let message = tail(&full, MAX_ERROR_LOG);
    let kind = match e {
        ToolError::UnknownTool(_) => "unknown-tool",
        ToolError::UnknownOp { .. } => "unknown-op",
        ToolError::Unsupported(_) => "unsupported",
        ToolError::InvalidArgs { .. } => "invalid-args",
        ToolError::PathRefused { .. } => "path-refused",
        ToolError::Failed(_) => "failed",
        ToolError::Denied(_) => "denied",
        ToolError::Missing(_) => "missing",
    };
    // Missing rights and programs are the user's to fix (warnings); other failures are errors.
    if matches!(e, ToolError::Failed(_)) {
        log::error!(Tool = tool, Op = op, ErrorKind = kind, Timeout = timeout, Error = message.as_str(); "Tool {tool} {op} failed");
    } else {
        log::warn!(Tool = tool, Op = op, ErrorKind = kind, Timeout = timeout, Error = message.as_str(); "Tool {tool} {op} failed");
    }
}

/// Runs one operation of a plugin tool: `{ tool, op, args }`. `args` is
/// validated against the operation's schema (see `tools_list`).
#[tauri::command]
pub async fn tool_invoke<R: Runtime>(
    app: AppHandle<R>,
    window: CurrentWindow,
    state: State<'_, AppState>,
    tool: String,
    op: String,
    args: Option<Value>,
) -> CmdResult<Value> {
    let result = invoke(&app, &window, state.safe_mode, &tool, &op, args.unwrap_or(Value::Null)).await?;
    // What a tool wrote (a collected bundle, an event-log capture folder, a dump) may be opened from this window.
    crate::paths::grant_read_all(&app, window.label(), &tool_outputs(&result), crate::paths::GrantSource::Output);
    Ok(result)
}

/// The files and folders a tool reports it wrote: the top-level `bundle`, `dir`, `path` and `file` of its result and
/// the paths listed in `outputs` (the tool output contract; plugin code, not the webview, names them).
pub(crate) fn tool_outputs(result: &Value) -> Vec<PathBuf> {
    let listed = result
        .get("outputs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    ["bundle", "dir", "path", "file"]
        .iter()
        .filter_map(|k| result.get(*k).and_then(Value::as_str))
        .chain(listed)
        .filter(|p| crate::paths::parse(p).is_ok())
        .map(PathBuf::from)
        .collect()
}

/// The tools of the calling window's plugins, with their availability
/// (unsupported platform or inactive/quarantined plugin, with the reason).
/// Empty in safe mode.
#[tauri::command]
pub async fn tools_list(window: CurrentWindow) -> Vec<ToolEntry> {
    window.tools().list()
}

/// Whether `tool`'s `op` is declared a pollable status operation.
fn is_poll_op(window: &CurrentWindow, tool: &str, op: &str) -> bool {
    window
        .tools()
        .list()
        .iter()
        .find(|t| t.info.id == tool)
        .is_some_and(|t| t.info.ops.iter().any(|o| o.name == op && o.poll))
}

/// Runs a tool's read-only status operation (declared with `OpSpec::polling`): what a dialog polls while
/// work runs. Quiet in the activity log; any other operation is refused (use `tool_invoke`).
#[tauri::command]
pub async fn tool_poll<R: Runtime>(
    app: AppHandle<R>,
    window: CurrentWindow,
    state: State<'_, AppState>,
    tool: String,
    op: String,
    args: Option<Value>,
) -> CmdResult<Value> {
    if !is_poll_op(&window, &tool, &op) {
        return Err(crate::CmdError::invalid(format!(
            "\"{tool}\" \"{op}\" is not a status operation; call it with tool_invoke"
        )));
    }
    let result = invoke(&app, &window, state.safe_mode, &tool, &op, args.unwrap_or(Value::Null)).await?;
    crate::paths::grant_read_all(&app, window.label(), &tool_outputs(&result), crate::paths::GrantSource::Output);
    Ok(result)
}

/// The job of a running tool task (a capture, a collection), so the UI cancels it with `jobs_cancel`: Jobs
/// are the one cancel authority. `None` when the task has no job (it ended, or runs as a background task).
#[tauri::command]
pub async fn tool_task_job<R: Runtime>(app: AppHandle<R>, task: String) -> CmdResult<Option<u64>> {
    if task.len() > MAX_ID_LEN {
        return Err(crate::CmdError::invalid("task ids are at most 64 characters").field("task"));
    }
    Ok(app
        .try_state::<CollectionJobs>()
        .and_then(|r| r.lock().get(&task).map(|(id, _, _)| id.0)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_support::{test_app_with_commands, window};
    use serde_json::json;
    use tauri::test::MockRuntime;

    /// Invokes a command through the real IPC path; `Err` carries the rejection.
    pub fn ipc(w: &tauri::WebviewWindow<MockRuntime>, cmd: &str, body: Value) -> Result<Value, Value> {
        use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
        use tauri::test::{get_ipc_response, INVOKE_KEY};
        use tauri::webview::InvokeRequest;
        let res = get_ipc_response(
            w,
            InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: crate::test_support::ipc_url(),
                body: InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        );
        res.map(|body| match body {
            InvokeResponseBody::Json(s) => serde_json::from_str::<Value>(&s).unwrap(),
            InvokeResponseBody::Raw(_) => Value::Null,
        })
    }

    /// Like [`ipc`], with the UI's operation id in the IPC header.
    pub fn ipc_op(w: &tauri::WebviewWindow<MockRuntime>, cmd: &str, body: Value, op: &str) -> Result<Value, Value> {
        use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
        use tauri::test::{get_ipc_response, INVOKE_KEY};
        let mut headers = tauri::http::HeaderMap::new();
        headers.insert(crate::activity::OP_HEADER, op.parse().unwrap());
        let res = get_ipc_response(
            w,
            tauri::webview::InvokeRequest {
                cmd: cmd.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: crate::test_support::ipc_url(),
                body: InvokeBody::Json(body),
                headers,
                invoke_key: INVOKE_KEY.to_string(),
            },
        );
        res.map(|body| match body {
            InvokeResponseBody::Json(s) => serde_json::from_str::<Value>(&s).unwrap(),
            InvokeResponseBody::Raw(_) => Value::Null,
        })
    }

    /// The domain events captured since `test_capture` (needs its guard held).
    pub fn domain_events() -> Vec<crate::activity::Captured> {
        crate::activity::captured()
            .into_iter()
            .filter(|c| c.props.get("Kind").is_some_and(|k| k == "domain"))
            .collect()
    }

    /// The one domain event `event` of operation `op`.
    pub fn domain_event(event: &str, op: &str) -> crate::activity::Captured {
        let all = domain_events();
        all.iter()
            .find(|c| c.props["Operation"] == event && c.props.get("OperationId").is_some_and(|o| o == op))
            .cloned()
            .unwrap_or_else(|| panic!("no {event} for {op} in {all:?}"))
    }

    #[test]
    fn tools_are_listed_and_invoked_through_ipc() {
        let app = test_app_with_commands(None);
        let main = window(&app, "main");
        let all = ipc(&main, "tools_list", json!({})).unwrap();
        // Other built-in plugins contribute tools too (the native profiler); these are the System tools.
        assert!(all.as_array().unwrap().iter().any(|t| t["id"] == "native-profiler"));
        let listed: Vec<Value> = all
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["plugin"] == "net.decryptology.tools")
            .cloned()
            .collect();
        let ids: Vec<&str> = listed.iter().map(|t| t["id"].as_str().unwrap()).collect();
        assert_eq!(
            ids,
            [
                "perfmon.templates",
                "perfmon.sets",
                "process-monitor",
                "event-log-follower",
                "diagnostics",
                "elevation"
            ]
        );
        let sets = &listed[1];
        assert_eq!(sets["plugin"], "net.decryptology.tools");
        assert_eq!(sets["needsElevation"], true);
        #[cfg(not(windows))]
        assert_eq!(sets["availability"]["state"], "unsupported");
        assert_eq!(listed[0]["availability"]["state"], "available");

        let presets = ipc(&main, "tool_invoke", json!({ "tool": "perfmon.templates", "op": "presets" })).unwrap();
        assert!(presets.as_array().unwrap().iter().any(|p| p["id"] == "memory"));
        let status = ipc(
            &main,
            "tool_invoke",
            json!({ "tool": "process-monitor", "op": "status", "args": {} }),
        )
        .unwrap();
        assert_eq!(status["running"], false);
        // Schema errors, unknown tools and operations are rejections with a message.
        let bad = ipc(
            &main,
            "tool_invoke",
            json!({ "tool": "perfmon.templates", "op": "generate", "args": { "template": 3 } }),
        );
        assert_eq!(
            bad.unwrap_err()["message"],
            json!("perfmon.templates generate: args.template must be an object, not a number")
        );
        let unknown = ipc(&main, "tool_invoke", json!({ "tool": "nope", "op": "x" })).unwrap_err();
        assert!(
            unknown["message"].as_str().unwrap().contains("is the System tools plugin disabled"),
            "{unknown}"
        );
        let op = ipc(&main, "tool_invoke", json!({ "tool": "elevation", "op": "dance" })).unwrap_err();
        assert!(op["message"].as_str().unwrap().contains("has no operation \"dance\""), "{op}");
        let long = ipc(&main, "tool_invoke", json!({ "tool": "x".repeat(65), "op": "x" })).unwrap_err();
        assert!(long["message"].as_str().unwrap().contains("at most 64"), "{long}");
        #[cfg(not(windows))]
        {
            let e = ipc(
                &main,
                "tool_invoke",
                json!({ "tool": "perfmon.sets", "op": "outputs", "args": { "name": "Debrief" } }),
            )
            .unwrap_err();
            assert!(e["message"].as_str().unwrap().contains("only available on Windows"), "{e}");
        }
    }

    #[test]
    fn a_disabled_tools_plugin_contributes_no_tools() {
        let dir = crate::test_support::TempDir::new("tools-disabled");
        std::fs::write(dir.0.join("plugins.json"), r#"{"disabled":["net.decryptology.tools"]}"#).unwrap();
        let app = test_app_with_commands(Some(dir.0.join("profiles")));
        let main = window(&app, "main");
        let listed = ipc(&main, "tools_list", json!({})).unwrap();
        assert!(
            listed.as_array().unwrap().iter().all(|t| t["plugin"] != "net.decryptology.tools"),
            "{listed}"
        );
        let e = ipc(&main, "tool_invoke", json!({ "tool": "elevation", "op": "info" })).unwrap_err();
        assert!(
            e["message"].as_str().unwrap().contains("is the System tools plugin disabled"),
            "{e}"
        );
    }

    #[test]
    fn background_work_is_listed_and_stopped_from_the_task_registry() {
        let app = test_app_with_commands(None);
        let main = window(&app, "main");
        let dir = crate::test_support::TempDir::new("tool-monitor");
        crate::paths::grant(&app, "main", &dir.0, crate::paths::Access::Read, crate::paths::GrantSource::Dialog).unwrap();
        let started = ipc(
            &main,
            "tool_invoke",
            json!({ "tool": "process-monitor", "op": "start", "args": { "config": { "targets": [{ "pid": std::process::id() }] }, "dir": dir.0 } }),
        )
        .unwrap();
        assert_eq!(started, json!(dir.0.to_string_lossy()));
        let tasks = app.state::<BackgroundTasks>();
        let listed = tasks.list();
        assert_eq!(
            (listed[0].id.as_str(), listed[0].kind, listed[0].stoppable),
            ("monitor", TaskKind::ProcessMonitor, true)
        );
        // The tray's Stop goes through the tool.
        tasks.stop("monitor").unwrap();
        assert!(tasks.list().is_empty());
        let status = ipc(&main, "tool_invoke", json!({ "tool": "process-monitor", "op": "status" })).unwrap();
        assert_eq!(status["running"], false);
        assert_eq!(task_kind("perfmon"), TaskKind::Perfmon);
        assert_eq!(task_kind("collect"), TaskKind::Other);
    }

    #[test]
    fn collections_are_collection_jobs_one_at_a_time() {
        let app = test_app_with_commands(None);
        let host = crate::test_support::main_window(&app).tools();
        let sink = ShellTasks {
            app: app.handle().clone(),
            host,
            data_dir: None,
        };
        let task = |id: &str| ToolTask {
            id: id.into(),
            label: "Collecting diagnostics".into(),
            kind: "collect".into(),
            stoppable: true,
        };
        let jobs = app.state::<crate::jobs::Jobs>().inner().clone();
        let collections = || jobs.find_active(|j| j.kind == crate::jobs::JobKind::Collection);
        sink.started("diag", &task("collect-1"));
        crate::jobs::wait_until("the collection job", || collections().len() == 1);
        assert!(app.state::<BackgroundTasks>().list().is_empty(), "not a background service");
        let id = collections()[0];
        sink.progress("diag", "collect-1", Some(0.5));
        assert_eq!(jobs.info(id).unwrap().progress.fraction, Some(0.5));
        // A second collection while one runs is stopped (through the tool), not queued behind a running tool.
        let _g = crate::activity::test_capture();
        sink.started("diag", &task("collect-2"));
        assert_eq!(collections().len(), 1);
        assert!(crate::activity::captured().iter().any(|c| c.message.contains("second collection")));
        // Captures are jobs of their own kind, several at once, beside a collection; each one's job is known.
        let captures = || jobs.find_active(|j| j.kind == crate::jobs::JobKind::Capture);
        let capture = |id: &str| ToolTask {
            id: id.into(),
            label: "Recording CPU of Shop.exe".into(),
            kind: CAPTURE_HINT.into(),
            stoppable: true,
        };
        sink.started("dotnet-profiler", &capture("dotnet-1"));
        sink.started("dotnet-profiler", &capture("dotnet-2"));
        crate::jobs::wait_until("the captures' jobs", || captures().len() == 2);
        assert_eq!(collections().len(), 1);
        let job = app.state::<CollectionJobs>().lock().get("dotnet-1").map(|(id, _, _)| *id);
        assert!(job.is_some_and(|j| captures().contains(&j)));
        assert_eq!(job_kind("collect"), crate::jobs::JobKind::Collection);
        sink.finished("dotnet-profiler", "dotnet-1");
        sink.finished("dotnet-profiler", "dotnet-2");
        crate::jobs::wait_until("the captures to end", || captures().is_empty());
        sink.finished("diag", "collect-1");
        crate::jobs::wait_until("the job to end", || collections().is_empty());
        // Unknown tasks are ignored.
        sink.progress("diag", "nope", None);
        sink.finished("diag", "nope");
        assert!(is_service("perfmon") && !is_service("collect"));
    }

    /// `tool_invoke` for `tool`'s `op`.
    fn call(w: &tauri::WebviewWindow<MockRuntime>, tool: &str, op: &str, args: Value) -> Result<Value, Value> {
        ipc(w, "tool_invoke", json!({ "tool": tool, "op": op, "args": args }))
    }

    #[test]
    fn collector_templates_and_diagnostics_replace_files_only_when_confirmed() {
        use crate::test_support::TempDir;
        let app = test_app_with_commands(None);
        let main = window(&app, "main");
        let presets = call(&main, "perfmon.templates", "presets", json!({})).unwrap();
        assert!(presets[0]["counters"][0]["guidance"].is_string());
        let dir = TempDir::new("tool-perfmon");
        let path = dir.0.join("t.xml").to_string_lossy().into_owned();
        // The user chose the file and the folders with the dialogs (ADR 0017: path arguments need a grant).
        let grant = |p: &std::path::Path, access| {
            crate::paths::grant(&app, "main", p, access, crate::paths::GrantSource::Dialog).unwrap();
        };
        grant(std::path::Path::new(&path), crate::paths::Access::Write);
        grant(&dir.0, crate::paths::Access::Read);
        let template = json!({ "name": "Debrief-test", "processes": ["OrderService"] });
        let gen = |args: Value| call(&main, "perfmon.templates", "generate", args);
        let out = gen(json!({ "template": template, "xmlPath": path })).unwrap();
        assert!(out["xml"].as_str().unwrap().contains("<DataCollectorSet>"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("OrderService"));
        // The file exists now: replaced only when confirmed.
        let again = gen(json!({ "template": template, "xmlPath": path })).unwrap_err();
        assert_eq!(
            again["message"],
            json!(format!("{path}{}", debrief_core::overwrite::FILE_EXISTS_SUFFIX))
        );
        assert!(gen(json!({ "template": template, "xmlPath": path, "overwrite": true })).is_ok());
        let bad = gen(json!({ "template": { "name": "", "sampleIntervalSecs": 0 } })).unwrap_err();
        assert!(bad["message"].as_str().unwrap().contains("collector name"), "{bad}");

        let plan = call(&main, "diagnostics", "default-plan", json!({})).unwrap();
        assert!(plan.as_str().unwrap().contains("redact"));
        let e = call(&main, "diagnostics", "collect", json!({ "plan": "{ nope", "outDir": dir.0 })).unwrap_err();
        assert!(e["message"].is_string() && e["code"] == "failed");
        // A bundle of the same name is replaced only when confirmed (the name has a one-second stamp).
        let out = dir.0.join("bundles");
        std::fs::create_dir(&out).unwrap();
        grant(&out, crate::paths::Access::Read);
        let plan = json!({ "name": "tool", "eventLogs": [], "environment": false }).to_string();
        let first = call(&main, "diagnostics", "collect", json!({ "plan": plan, "outDir": out })).unwrap();
        let bundle = first["bundle"].as_str().unwrap().to_string();
        let prefix = &bundle[..bundle.len() - "yyyyMMdd-HHmmss.debriefz".len()];
        let now = chrono::Local::now();
        for s in 0..10 {
            let at = now + chrono::Duration::seconds(s);
            let _ = std::fs::write(format!("{prefix}{}.debriefz", at.format("%Y%m%d-%H%M%S")), "mine");
        }
        let refused = call(&main, "diagnostics", "collect", json!({ "plan": plan, "outDir": out })).unwrap_err();
        assert!(
            refused["message"]
                .as_str()
                .unwrap()
                .ends_with(debrief_core::overwrite::FILE_EXISTS_SUFFIX),
            "{refused}"
        );
        assert!(call(
            &main,
            "diagnostics",
            "collect",
            json!({ "plan": plan, "outDir": out, "overwrite": true })
        )
        .is_ok());
        #[cfg(not(windows))]
        for (tool, op, args) in [
            ("perfmon.sets", "install", json!({ "template": template })),
            ("perfmon.sets", "control", json!({ "name": "Debrief-test", "action": "query" })),
            ("perfmon.sets", "outputs", json!({ "name": "Debrief-test" })),
        ] {
            let e = call(&main, tool, op, args).unwrap_err();
            assert!(e["message"].as_str().unwrap().contains("only available on Windows"), "{op}: {e}");
        }
    }

    #[test]
    fn the_monitor_is_app_wide_and_listed_as_background_work() {
        let app = test_app_with_commands(None);
        let (a, b) = (window(&app, "main"), window(&app, "w-2"));
        let dir = crate::test_support::TempDir::new("tool-monitor-shared");
        crate::paths::grant(&app, "main", &dir.0, crate::paths::Access::Read, crate::paths::GrantSource::Dialog).unwrap();
        let config = json!({ "targets": [{ "pid": std::process::id() }] });
        let started = call(&a, "process-monitor", "start", json!({ "config": config, "dir": dir.0 })).unwrap();
        assert_eq!(call(&b, "process-monitor", "status", json!({})).unwrap()["running"], true);
        let again = call(&b, "process-monitor", "start", json!({ "config": config })).unwrap_err();
        assert_eq!(again["message"], json!("the process monitor is already running"));
        let listed = app.state::<BackgroundTasks>().list();
        assert_eq!((listed[0].id.as_str(), listed[0].kind), ("monitor", TaskKind::ProcessMonitor));
        assert_eq!(call(&b, "process-monitor", "stop", json!({})).unwrap(), started);
        assert!(app.state::<BackgroundTasks>().list().is_empty());
        let bad = call(&a, "process-monitor", "start", json!({ "config": { "targets": [] } })).unwrap_err();
        assert!(bad["message"].as_str().unwrap().contains("at least one entry"), "{bad}");
    }

    #[test]
    fn tool_failures_are_logged_with_severity() {
        let _g = crate::activity::test_capture();
        let app = test_app_with_commands(None);
        let main = window(&app, "main");
        let _ = ipc(&main, "tool_invoke", json!({ "tool": "elevation", "op": "dance-log-test" })).unwrap_err();
        log_failure(
            "perfmon.sets",
            "start",
            &ToolError::Failed("logman failed (exit code 3): boom password=hunter2".into()),
        );
        log_failure(
            "perfmon.sets",
            "stop",
            &ToolError::Denied("logman failed: Access is denied.".into()),
        );
        log_failure(
            "relog",
            "convert",
            &ToolError::Failed(format!("relog did not finish within 120s{}", "x".repeat(4000))),
        );
        let logged = crate::activity::captured();
        let find = |op: &str| {
            logged
                .iter()
                .find(|c| c.props.get("Op").is_some_and(|o| o == op))
                .unwrap_or_else(|| panic!("{op} logged"))
        };
        let bad = find("dance-log-test");
        assert_eq!((bad.level, bad.props["ErrorKind"].as_str()), (log::Level::Warn, "unknown-op"));
        let failed = find("start");
        assert_eq!(failed.level, log::Level::Error);
        assert!(
            failed.props["Error"].contains("exit code 3") && !failed.props["Error"].contains("hunter2"),
            "{:?}",
            failed.props
        );
        assert_eq!(find("stop").level, log::Level::Warn);
        let slow = find("convert");
        assert_eq!((slow.props["Timeout"].as_str(), slow.level), ("true", log::Level::Error));
        assert!(slow.props["Error"].len() <= MAX_ERROR_LOG + 3);
        assert_eq!(tail("abc", 10), "abc");
        assert_eq!(tail("aé€", 3), "…€");
    }

    #[test]
    fn safe_mode_has_no_tools_and_says_why() {
        let app = crate::test_support::test_app_safe_mode();
        let main = window(&app, "main");
        assert_eq!(ipc(&main, "tools_list", json!({})).unwrap(), json!([]));
        let e = ipc(&main, "tool_invoke", json!({ "tool": "process-monitor", "op": "status" })).unwrap_err();
        assert!(e["message"].as_str().unwrap().contains("not available in safe mode"), "{e}");
        // Elevation falls back to the shell's own token check (not a blanket "not elevated").
        let info = ipc(&main, "elevation", json!({})).unwrap();
        assert_eq!(info["canRestartElevated"], cfg!(windows));
        assert_eq!(info["elevated"], crate::system::process_elevated());
        assert_eq!(missing(ToolError::Failed("x".into()), true).message, "x");
        assert_eq!(missing(ToolError::Denied("d".into()), false).code, crate::ErrorCode::Denied);
        assert_eq!(missing(ToolError::Missing("m".into()), false).code, crate::ErrorCode::Missing);
        assert_eq!(
            missing(ToolError::UnknownTool("t".into()), true).code,
            crate::ErrorCode::Unavailable
        );
        assert_eq!(
            missing(ToolError::Unsupported("u".into()), false).code,
            crate::ErrorCode::Unsupported
        );
    }

    #[test]
    fn polling_is_only_for_status_operations_and_task_jobs_are_found() {
        let app = test_app_with_commands(None);
        let main = window(&app, "main");
        // The .NET profiler runs on every platform (the native profiler is Windows-only and answers "unsupported").
        let status = ipc(&main, "tool_poll", json!({ "tool": "dotnet-profiler", "op": "status" })).unwrap();
        assert!(status["captures"].is_array(), "{status}");
        let e = ipc(&main, "tool_poll", json!({ "tool": "elevation", "op": "info" })).unwrap_err();
        assert_eq!(e["code"], "invalid", "{e}");
        let e = ipc(&main, "tool_poll", json!({ "tool": "nope", "op": "status" })).unwrap_err();
        assert_eq!(e["code"], "invalid", "{e}");
        assert_eq!(ipc(&main, "tool_task_job", json!({ "task": "sampling-99" })).unwrap(), json!(null));
        // A non-status operation of a tool with a status operation is refused too.
        let e = ipc(&main, "tool_poll", json!({ "tool": "dotnet-profiler", "op": "processes" })).unwrap_err();
        assert_eq!(e["code"], "invalid", "{e}");
        let e = ipc(&main, "tool_task_job", json!({ "task": "x".repeat(65) })).unwrap_err();
        assert_eq!(e["code"], "invalid", "{e}");
        assert_eq!(crate::commands::kind_of("tool_poll"), crate::commands::Kind::Quiet);
    }

    #[test]
    fn tool_outputs_are_the_reported_bundle_and_folder() {
        let abs = std::env::temp_dir().join("out").to_string_lossy().into_owned();
        let found = tool_outputs(&json!({ "bundle": abs, "dir": "relative", "other": abs, "manifest": {} }));
        assert_eq!(found, vec![PathBuf::from(&abs)]);
        assert_eq!(
            tool_outputs(&json!({ "file": abs })),
            vec![PathBuf::from(&abs)],
            "a dump the capture tool wrote"
        );
        assert!(tool_outputs(&json!(null)).is_empty());
        let listed = tool_outputs(&json!({ "path": abs, "outputs": [abs, "relative", 3] }));
        assert_eq!(listed, vec![PathBuf::from(&abs), PathBuf::from(&abs)]);
    }
}
