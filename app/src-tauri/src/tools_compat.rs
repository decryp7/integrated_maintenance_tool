//! **Deprecated** tool commands, kept so existing UIs and scripts keep
//! working. Each one is a thin wrapper over `tool_invoke` (see
//! [`crate::tools`]) with the same name, arguments and result as before; the
//! tools themselves live in the tools plugin (`imt-plugin-tools`), so they
//! are disabled, quarantined and absent in safe mode like any plugin.
//!
//! | Deprecated command | Tool, operation |
//! |---|---|
//! | `perfmon_presets` | `perfmon.templates`, `presets` |
//! | `perfmon_generate { template, xmlPath?, overwrite? }` | `perfmon.templates`, `generate` |
//! | `perfmon_install { template }` | `perfmon.sets`, `install` |
//! | `perfmon_control { name, action }` | `perfmon.sets`, `control` |
//! | `perfmon_outputs { name }` | `perfmon.sets`, `outputs` |
//! | `monitor_start { config, dir? }` | `process-monitor`, `start` |
//! | `monitor_stop` | `process-monitor`, `stop` |
//! | `monitor_status` | `process-monitor`, `status` |
//! | `collect_default_plan` | `diagnostics`, `default-plan` |
//! | `collect_diagnostics { plan, outDir, overwrite? }` | `diagnostics`, `collect` |
//!
//! New code calls `tool_invoke` instead. (`elevation` stays a shell command:
//! it asks the `elevation` tool and falls back when no plugin answers.)

use crate::tools::invoke;
use crate::windows::CurrentWindow;
use crate::{AppState, CmdResult, State};
use serde_json::{json, Value};
use tauri::{AppHandle, Runtime};

/// Deprecated: `tool_invoke { tool: "perfmon.templates", op: "presets" }`.
#[tauri::command]
pub async fn perfmon_presets<R: Runtime>(app: AppHandle<R>, window: CurrentWindow, state: State<'_, AppState>) -> CmdResult<Value> {
    invoke(&app, &window, state.safe_mode, "perfmon.templates", "presets", json!({})).await
}

/// Deprecated: `tool_invoke { tool: "perfmon.templates", op: "generate" }`.
#[tauri::command]
pub async fn perfmon_generate<R: Runtime>(
    app: AppHandle<R>,
    window: CurrentWindow,
    state: State<'_, AppState>,
    template: Value,
    xml_path: Option<String>,
    overwrite: Option<bool>,
) -> CmdResult<Value> {
    let args = json!({ "template": template, "xmlPath": xml_path, "overwrite": overwrite });
    invoke(&app, &window, state.safe_mode, "perfmon.templates", "generate", args).await
}

/// Deprecated: `tool_invoke { tool: "perfmon.sets", op: "install" }`.
#[tauri::command]
pub async fn perfmon_install<R: Runtime>(
    app: AppHandle<R>,
    window: CurrentWindow,
    state: State<'_, AppState>,
    template: Value,
) -> CmdResult<Value> {
    invoke(
        &app,
        &window,
        state.safe_mode,
        "perfmon.sets",
        "install",
        json!({ "template": template }),
    )
    .await
}

/// Deprecated: `tool_invoke { tool: "perfmon.sets", op: "control" }`.
#[tauri::command]
pub async fn perfmon_control<R: Runtime>(
    app: AppHandle<R>,
    window: CurrentWindow,
    state: State<'_, AppState>,
    name: String,
    action: String,
) -> CmdResult<Value> {
    let args = json!({ "name": name, "action": action });
    invoke(&app, &window, state.safe_mode, "perfmon.sets", "control", args).await
}

/// Deprecated: `tool_invoke { tool: "perfmon.sets", op: "outputs" }`.
#[tauri::command]
pub async fn perfmon_outputs<R: Runtime>(
    app: AppHandle<R>,
    window: CurrentWindow,
    state: State<'_, AppState>,
    name: String,
) -> CmdResult<Value> {
    invoke(&app, &window, state.safe_mode, "perfmon.sets", "outputs", json!({ "name": name })).await
}

/// Deprecated: `tool_invoke { tool: "process-monitor", op: "start" }`.
#[tauri::command]
pub async fn monitor_start<R: Runtime>(
    app: AppHandle<R>,
    window: CurrentWindow,
    state: State<'_, AppState>,
    config: Value,
    dir: Option<String>,
) -> CmdResult<Value> {
    let args = json!({ "config": config, "dir": dir });
    invoke(&app, &window, state.safe_mode, "process-monitor", "start", args).await
}

/// Deprecated: `tool_invoke { tool: "process-monitor", op: "stop" }`.
#[tauri::command]
pub async fn monitor_stop<R: Runtime>(app: AppHandle<R>, window: CurrentWindow, state: State<'_, AppState>) -> CmdResult<Value> {
    invoke(&app, &window, state.safe_mode, "process-monitor", "stop", json!({})).await
}

/// Deprecated: `tool_invoke { tool: "process-monitor", op: "status" }`.
#[tauri::command]
pub async fn monitor_status<R: Runtime>(app: AppHandle<R>, window: CurrentWindow, state: State<'_, AppState>) -> CmdResult<Value> {
    invoke(&app, &window, state.safe_mode, "process-monitor", "status", json!({})).await
}

/// Deprecated: `tool_invoke { tool: "diagnostics", op: "default-plan" }`.
#[tauri::command]
pub async fn collect_default_plan<R: Runtime>(app: AppHandle<R>, window: CurrentWindow, state: State<'_, AppState>) -> CmdResult<Value> {
    invoke(&app, &window, state.safe_mode, "diagnostics", "default-plan", json!({})).await
}

/// Deprecated: `tool_invoke { tool: "diagnostics", op: "collect" }`.
#[tauri::command]
pub async fn collect_diagnostics<R: Runtime>(
    app: AppHandle<R>,
    window: CurrentWindow,
    state: State<'_, AppState>,
    plan: String,
    out_dir: String,
    overwrite: Option<bool>,
) -> CmdResult<Value> {
    let args = json!({ "plan": plan, "outDir": out_dir, "overwrite": overwrite });
    invoke(&app, &window, state.safe_mode, "diagnostics", "collect", args).await
}

#[cfg(test)]
mod tests {
    use crate::background::{BackgroundTasks, TaskKind};
    use crate::test_support::{test_app_with_commands, window, TempDir};
    use crate::tools::tests::ipc;
    use serde_json::{json, Value};
    use tauri::Manager;

    #[test]
    fn the_old_commands_answer_as_before() {
        let app = test_app_with_commands(None);
        let main = window(&app, "main");
        let presets = ipc(&main, "perfmon_presets", json!({})).unwrap();
        assert!(presets.as_array().unwrap().iter().any(|x| x["id"] == "memory"));
        assert!(presets[0]["counters"][0]["guidance"].is_string());
        let dir = TempDir::new("compat-perfmon");
        let path = dir.0.join("t.xml").to_string_lossy().into_owned();
        let template = json!({ "name": "IMT-test", "processes": ["OrderService"] });
        let out = ipc(&main, "perfmon_generate", json!({ "template": template, "xmlPath": path })).unwrap();
        assert!(out["xml"].as_str().unwrap().contains("<DataCollectorSet>"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("OrderService"));
        // The file exists now: replaced only when confirmed.
        let again = ipc(&main, "perfmon_generate", json!({ "template": template, "xmlPath": path })).unwrap_err();
        assert_eq!(again, json!(format!("{path}{}", imt_core::overwrite::FILE_EXISTS_SUFFIX)));
        let confirmed = json!({ "template": template, "xmlPath": path, "overwrite": true });
        assert!(ipc(&main, "perfmon_generate", confirmed).is_ok());
        let bad = ipc(
            &main,
            "perfmon_generate",
            json!({ "template": { "name": "", "sampleIntervalSecs": 0 } }),
        )
        .unwrap_err();
        assert!(bad.as_str().unwrap().contains("collector name"), "{bad}");
        assert!(ipc(&main, "collect_default_plan", json!({}))
            .unwrap()
            .as_str()
            .unwrap()
            .contains("redact"));
        assert_eq!(ipc(&main, "monitor_stop", json!({})).unwrap(), Value::Null);
        assert_eq!(
            ipc(&main, "monitor_status", json!({})).unwrap(),
            json!({ "running": false, "dir": null })
        );
        let e = ipc(&main, "collect_diagnostics", json!({ "plan": "{ nope", "outDir": dir.0 })).unwrap_err();
        assert!(e.is_string());
        // A bundle of the same name is replaced only when confirmed (the name has a one-second stamp).
        let out = dir.0.join("bundles");
        let plan = json!({ "name": "compat", "eventLogs": [], "environment": false }).to_string();
        let first = ipc(&main, "collect_diagnostics", json!({ "plan": plan, "outDir": out })).unwrap();
        let bundle = first["bundle"].as_str().unwrap().to_string();
        let prefix = &bundle[..bundle.len() - "yyyyMMdd-HHmmss.imtz".len()];
        let now = chrono::Local::now();
        for s in 0..10 {
            let at = now + chrono::Duration::seconds(s);
            let _ = std::fs::write(format!("{prefix}{}.imtz", at.format("%Y%m%d-%H%M%S")), "mine");
        }
        let refused = ipc(&main, "collect_diagnostics", json!({ "plan": plan, "outDir": out })).unwrap_err();
        assert!(
            refused.as_str().unwrap().ends_with(imt_core::overwrite::FILE_EXISTS_SUFFIX),
            "{refused}"
        );
        let confirmed = json!({ "plan": plan, "outDir": out, "overwrite": true });
        assert!(ipc(&main, "collect_diagnostics", confirmed).is_ok());
        #[cfg(not(windows))]
        for (cmd, body) in [
            ("perfmon_install", json!({ "template": template })),
            ("perfmon_control", json!({ "name": "IMT-test", "action": "query" })),
            ("perfmon_outputs", json!({ "name": "IMT-test" })),
        ] {
            let e = ipc(&main, cmd, body).unwrap_err();
            assert!(e.as_str().unwrap().contains("only available on Windows"), "{cmd}: {e}");
        }
    }

    #[test]
    fn the_monitor_is_app_wide_and_listed_as_background_work() {
        let app = test_app_with_commands(None);
        let (a, b) = (window(&app, "main"), window(&app, "w-2"));
        let dir = TempDir::new("compat-monitor");
        let config = json!({ "targets": [{ "pid": std::process::id() }] });
        let started = ipc(&a, "monitor_start", json!({ "config": config, "dir": dir.0 })).unwrap();
        assert_eq!(ipc(&b, "monitor_status", json!({})).unwrap()["running"], true);
        let again = ipc(&b, "monitor_start", json!({ "config": config })).unwrap_err();
        assert_eq!(again, json!("the process monitor is already running"));
        let listed = app.state::<BackgroundTasks>().list();
        assert_eq!((listed[0].id.as_str(), listed[0].kind), ("monitor", TaskKind::ProcessMonitor));
        assert_eq!(ipc(&b, "monitor_stop", json!({})).unwrap(), started);
        assert!(app.state::<BackgroundTasks>().list().is_empty());
        let bad = ipc(&a, "monitor_start", json!({ "config": { "targets": [] } })).unwrap_err();
        assert!(bad.as_str().unwrap().contains("at least one entry"), "{bad}");
    }
}
