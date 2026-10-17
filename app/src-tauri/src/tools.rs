//! Commands driving external tooling: Windows Performance Monitor (templates
//! and `logman` control), the process monitor collector and the diagnostics
//! collector. Long-running work happens off the UI thread. A running process
//! monitor and started PerfMon collector sets are listed as background work
//! (`crate::background`), which the tray can stop.

use crate::background::{BackgroundTasks, TaskKind, TaskSpec};
use crate::{err, AppState, CmdResult, State};
use imt_core::export::ClefFileSink;
use imt_core::plugin::Collector;
use imt_plugin_windows::command::SystemRunner;
use imt_plugin_windows::perfmon_control::{output_files, PerfmonController};
use imt_plugin_windows::perfmon_template::{guidance, presets, CollectorTemplate};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A running process-monitor session.
pub struct MonitorSession {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<anyhow::Result<()>>,
    pub dir: PathBuf,
}

#[derive(Default)]
pub struct Tools {
    pub monitor: Mutex<Option<MonitorSession>>,
}

/// Counter presets with one line of guidance per counter.
#[tauri::command]
pub fn perfmon_presets() -> Value {
    let list: Vec<Value> = presets()
        .into_iter()
        .map(|p| {
            let counters: Vec<Value> = p.counters.iter().map(|c| json!({ "path": c, "guidance": guidance(c) })).collect();
            json!({ "id": p.id, "name": p.name, "description": p.description, "counters": counters })
        })
        .collect();
    Value::Array(list)
}

/// The Data Collector Set XML for a template, plus the matching `logman` commands.
#[tauri::command]
pub fn perfmon_generate(template: CollectorTemplate, xml_path: Option<String>) -> CmdResult<Value> {
    let xml = template.to_xml().map_err(err)?;
    let path = xml_path.unwrap_or_else(|| format!("{}.xml", template.name));
    if let Some(dir) = Path::new(&path).parent().filter(|d| !d.as_os_str().is_empty()) {
        if dir.exists() {
            std::fs::write(&path, &xml).map_err(err)?;
        }
    }
    Ok(json!({ "xml": xml, "path": path, "commands": template.logman_commands(&path) }))
}

fn controller() -> PerfmonController<SystemRunner> {
    PerfmonController::new(SystemRunner)
}

/// Creates (or replaces) the collector set in Performance Monitor from a template.
#[tauri::command]
pub async fn perfmon_install(template: CollectorTemplate) -> CmdResult<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let xml = template.to_xml().map_err(err)?;
        let file = std::env::temp_dir().join(format!("imt-{}.xml", template.name));
        std::fs::write(&file, xml).map_err(err)?;
        let c = controller();
        c.import(&template.name, &file, true).map_err(err)?;
        let _ = std::fs::remove_file(&file);
        serde_json::to_value(c.query(&template.name).map_err(err)?).map_err(err)
    })
    .await
    .map_err(err)?
}

/// `start`, `stop`, `delete` or `query` a collector set; returns its status (null after delete).
#[tauri::command]
pub async fn perfmon_control<R: tauri::Runtime>(app: tauri::AppHandle<R>, name: String, action: String) -> CmdResult<Value> {
    let status = control(name.clone(), action.clone()).await?;
    if let Some(tasks) = tauri::Manager::try_state::<BackgroundTasks>(&app) {
        match action.as_str() {
            "start" => register_perfmon(&tasks, &name),
            "stop" | "delete" => {
                tasks.remove(&perfmon_task(&name));
            }
            _ => {}
        }
    }
    Ok(status)
}

fn perfmon_task(name: &str) -> String {
    format!("perfmon:{name}")
}

/// Lists a started collector set as background work; stopping runs `logman stop`.
fn register_perfmon(tasks: &BackgroundTasks, name: &str) {
    let spec = TaskSpec {
        id: perfmon_task(name),
        label: format!("PerfMon collection \u{201c}{name}\u{201d}"),
        kind: TaskKind::Perfmon,
        window: None,
    };
    let set = name.to_string();
    let stop = Arc::new(move || controller().stop(&set).map_err(anyhow::Error::from));
    if let Err(e) = tasks.register(spec, Some(stop)) {
        log::warn!("could not list PerfMon collection as background work: {e:#}");
    }
}

async fn control(name: String, action: String) -> CmdResult<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let c = controller();
        match action.as_str() {
            "start" => c.start(&name).map_err(err)?,
            "stop" => c.stop(&name).map_err(err)?,
            "delete" => {
                c.delete(&name).map_err(err)?;
                return Ok(Value::Null);
            }
            "query" => {}
            other => return Err(format!("unknown action \"{other}\"")),
        }
        serde_json::to_value(c.query(&name).map_err(err)?).map_err(err)
    })
    .await
    .map_err(err)?
}

/// The `.blg`/`.csv`/`.tsv` files a collector set produced, newest first.
#[tauri::command]
pub async fn perfmon_outputs(name: String) -> CmdResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        let status = controller().query(&name).map_err(err)?;
        Ok(output_files(&status)
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect())
    })
    .await
    .map_err(err)?
}

/// Starts the process monitor with a config (see imt-monitor), writing CLEF
/// files to `dir` (default: the app's data folder). Returns the folder, which
/// the UI opens with live tail on.
#[tauri::command]
pub fn monitor_start<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    tools: State<'_, Tools>,
    config: Value,
    dir: Option<String>,
) -> CmdResult<String> {
    use tauri::Manager;
    let mut slot = tools.monitor.lock().map_err(err)?;
    if slot.is_some() {
        return Err("the process monitor is already running".into());
    }
    imt_monitor::config::Config::from_json(&config).map_err(err)?;
    let dir = match dir {
        Some(d) => PathBuf::from(d),
        None => app
            .path()
            .app_data_dir()
            .map_err(err)?
            .join("monitor")
            .join(chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string()),
    };
    let sink = ClefFileSink::new(&dir, "process-monitor", 10 * 1024 * 1024, 10).map_err(err)?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::spawn(move || imt_monitor::ProcessMonitor.run(&config, &sink, &flag));
    log::info!("Process monitor started");
    *slot = Some(MonitorSession {
        stop,
        thread,
        dir: dir.clone(),
    });
    drop(slot);
    if let Some(tasks) = app.try_state::<BackgroundTasks>() {
        let handle = app.clone();
        let stop = Arc::new(move || {
            let tools = handle.try_state::<Tools>().ok_or_else(|| anyhow::anyhow!("IMT is shutting down"))?;
            stop_monitor(&tools).map(|_| ()).map_err(|e| anyhow::anyhow!(e))
        });
        let spec = TaskSpec {
            id: MONITOR_TASK.into(),
            label: "Process monitor".into(),
            kind: TaskKind::ProcessMonitor,
            window: None,
        };
        if let Err(e) = tasks.register(spec, Some(stop)) {
            log::warn!("could not list the process monitor as background work: {e:#}");
        }
    }
    Ok(dir.to_string_lossy().into_owned())
}

/// Id of the process monitor in the background task registry.
const MONITOR_TASK: &str = "monitor";

/// Stops the process monitor; returns its folder.
#[tauri::command]
pub fn monitor_stop<R: tauri::Runtime>(app: tauri::AppHandle<R>, tools: State<'_, Tools>) -> CmdResult<Option<String>> {
    let out = stop_monitor(&tools);
    if let Some(tasks) = tauri::Manager::try_state::<BackgroundTasks>(&app) {
        tasks.remove(MONITOR_TASK);
    }
    out
}

fn stop_monitor(tools: &Tools) -> CmdResult<Option<String>> {
    let Some(session) = tools.monitor.lock().unwrap_or_else(|p| p.into_inner()).take() else {
        return Ok(None);
    };
    session.stop.store(true, Ordering::SeqCst);
    let result = session.thread.join().map_err(|_| "the monitor thread panicked".to_string())?;
    log::info!("Process monitor stopped");
    result.map_err(err)?;
    Ok(Some(session.dir.to_string_lossy().into_owned()))
}

#[tauri::command]
pub fn monitor_status(tools: State<'_, Tools>) -> CmdResult<Value> {
    let slot = tools.monitor.lock().map_err(err)?;
    Ok(match slot.as_ref() {
        Some(s) => json!({ "running": !s.thread.is_finished(), "dir": s.dir }),
        None => json!({ "running": false, "dir": null }),
    })
}

/// The default diagnostics-collection plan (JSON).
#[tauri::command]
pub fn collect_default_plan() -> String {
    imt_collect::CollectPlan::default_json()
}

/// Collects a diagnostics bundle (.imtz) into `out_dir`; returns its path and manifest.
#[tauri::command]
pub async fn collect_diagnostics(_state: State<'_, AppState>, plan: String, out_dir: String) -> CmdResult<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        let plan = imt_collect::CollectPlan::parse(&plan).map_err(err)?;
        let runner = imt_collect::SystemRunner;
        let collector = imt_collect::Collector::new(plan, &runner, &imt_collect::NoProgress);
        let result = collector.run(Path::new(&out_dir)).map_err(err)?;
        log::info!("Diagnostics bundle collected");
        Ok(json!({ "bundle": result.bundle, "manifest": result.manifest }))
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_templates() {
        let p = perfmon_presets();
        assert!(p.as_array().unwrap().iter().any(|x| x["id"] == "memory"));
        assert!(p[0]["counters"][0]["guidance"].is_string());
        let dir = std::env::temp_dir().join(format!("imt-perfmon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.xml").to_string_lossy().into_owned();
        let template: CollectorTemplate = serde_json::from_value(json!({ "name": "IMT-test", "processes": ["OrderService"] })).unwrap();
        let out = perfmon_generate(template.clone(), Some(path.clone())).unwrap();
        assert!(out["xml"].as_str().unwrap().contains("<DataCollectorSet>"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("OrderService"));
        assert!(out["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap().contains("logman import")));
        let bad: CollectorTemplate = serde_json::from_value(json!({ "name": "", "sampleIntervalSecs": 0 })).unwrap();
        assert!(perfmon_generate(bad, None).is_err());
        std::fs::remove_dir_all(dir).unwrap();
        #[cfg(not(windows))]
        {
            let r = tauri::async_runtime::block_on(control("IMT-test".into(), "query".into()));
            assert!(r.unwrap_err().contains("only be managed on Windows"));
            let r = tauri::async_runtime::block_on(control("IMT-test".into(), "explode".into()));
            assert!(r.unwrap_err().contains("unknown action"));
            assert!(tauri::async_runtime::block_on(perfmon_install(template)).is_err());
            assert!(tauri::async_runtime::block_on(perfmon_outputs("IMT-test".into())).is_err());
        }
        assert!(collect_default_plan().contains("redact"));
    }

    #[test]
    fn collections_are_listed_as_background_work() {
        let app = crate::test_support::test_app(None);
        let h = app.handle().clone();
        tauri::Manager::manage(&app, Tools::default());
        let tasks = || tauri::Manager::state::<BackgroundTasks>(&app);
        register_perfmon(&tasks(), "IMT-test");
        let listed = tasks().list();
        assert_eq!(
            (listed[0].id.as_str(), listed[0].kind, listed[0].stoppable),
            ("perfmon:IMT-test", TaskKind::Perfmon, true)
        );
        // Stopping runs `logman stop`; where that fails the task stays listed with the reason.
        #[cfg(not(windows))]
        {
            assert!(tasks().stop("perfmon:IMT-test").is_err());
            assert_eq!(tasks().list().len(), 1);
            // A failed start registers nothing; stop and delete remove the entry.
            assert!(tauri::async_runtime::block_on(perfmon_control(h.clone(), "x".into(), "start".into())).is_err());
            assert!(tauri::async_runtime::block_on(perfmon_control(h.clone(), "IMT-test".into(), "stop".into())).is_err());
        }
        tasks().remove("perfmon:IMT-test");
        // Stopping a monitor that is not running is fine.
        assert_eq!(monitor_stop(h.clone(), tauri::Manager::state::<Tools>(&app)).unwrap(), None);
        let status = monitor_status(tauri::Manager::state::<Tools>(&app)).unwrap();
        assert_eq!(status["running"], false);
    }
}
