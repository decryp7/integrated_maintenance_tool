//! The inference runtime commands (ADR 0009): list, install (a background task streaming
//! `imt://ai-runtime`), cancel, delete and choose llama-server builds. Thin: they validate the IPC
//! arguments, run the work off the UI thread and turn errors into [`AiCmdError`]s.

use super::commands::core;
use super::core::AiCore;
use super::error::{AiCmdError, AiErrorCode, AiResult};
use super::runtimes::RuntimeEvent;
use super::{RUNTIME_EVENT, STATUS_EVENT};
use crate::background::{BackgroundTasks, TaskKind, TaskSpec};
use crate::settings::AI_SECTION;
use imt_ai::runtimes::builds::label;
use imt_ai::runtimes::RuntimeOverview;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Id of the runtime download in the background task list.
pub const RUNTIME_TASK: &str = "ai:runtime";
const MAX_ID_CHARS: usize = 32;
/// How often the download ticker reports progress.
const TICK: Duration = Duration::from_millis(250);

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AiResult<T> + Send + 'static) -> AiResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AiCmdError::new(AiErrorCode::Engine, format!("the AI task stopped unexpectedly ({e})")))?
}

fn short(what: &str, s: &str) -> AiResult<String> {
    let t = s.trim();
    if t.is_empty() || t.len() > MAX_ID_CHARS {
        return Err(AiCmdError::invalid(format!("'{t}' is not {what}")));
    }
    Ok(t.to_ascii_lowercase())
}

fn emit_status<R: Runtime>(app: &AppHandle<R>, core: &AiCore) {
    let _ = app.emit(STATUS_EVENT, core.status());
}

/// Every build for this platform with whether it is installed and works, plus older ones still on disk.
#[tauri::command]
pub async fn ai_runtimes<R: Runtime>(app: AppHandle<R>) -> AiResult<RuntimeOverview> {
    let core = core(&app)?;
    blocking(move || core.runtime_overview()).await
}

/// Downloads, verifies and unpacks a build. Returns at once; progress arrives as `imt://ai-runtime`
/// events ending in `ready`, `failed` or `cancelled`. Listed as a background task (stop = cancel).
#[tauri::command]
pub fn ai_runtime_install<R: Runtime>(app: AppHandle<R>, id: String) -> AiResult<()> {
    let core = core(&app)?;
    let id = short("an inference build", &id)?;
    let progress = core.begin_runtime_install(&id)?;
    if let Some(t) = app.try_state::<BackgroundTasks>() {
        let c = core.clone();
        let stop: crate::background::StopFn = Arc::new(move || {
            c.cancel_runtime_install();
            Ok(())
        });
        let spec = TaskSpec {
            id: RUNTIME_TASK.into(),
            label: format!("Downloading the {} inference runtime", label(&id)),
            kind: TaskKind::Download,
            window: None,
        };
        if let Err(e) = t.register(spec, Some(stop)) {
            log::warn!("AI runtime download not listed as a background task: {e:#}");
        }
    }
    let spawned = std::thread::Builder::new().name("imt-ai-runtime".into()).spawn(move || {
        let result = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let a = app.clone();
                let mut on_stage = |st| {
                    let _ = a.emit(RUNTIME_EVENT, RuntimeEvent::of_stage(&id, st));
                };
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.install_runtime(&id, &progress, &mut on_stage)))
                    .unwrap_or_else(|_| Err(AiCmdError::new(AiErrorCode::Engine, "the runtime install stopped unexpectedly")))
            });
            let mut last = None;
            while !worker.is_finished() {
                std::thread::sleep(TICK);
                let ev = RuntimeEvent::of_progress(&id, &progress);
                if let RuntimeEvent::Progress { bytes, total, .. } = &ev {
                    if *total > 0 && last != Some((*bytes, *total)) {
                        last = Some((*bytes, *total));
                        if let Some(t) = app.try_state::<BackgroundTasks>() {
                            t.set_progress(RUNTIME_TASK, Some(*bytes as f32 / *total as f32));
                        }
                        let _ = app.emit(RUNTIME_EVENT, ev);
                    }
                }
            }
            worker
                .join()
                .unwrap_or_else(|_| Err(AiCmdError::new(AiErrorCode::Engine, "the runtime install stopped unexpectedly")))
        });
        core.end_runtime_install();
        if let Some(t) = app.try_state::<BackgroundTasks>() {
            t.remove(RUNTIME_TASK);
        }
        match &result {
            Ok(r) => log::info!(Build = id.as_str(), Tag = r.tag.as_str(); "Inference runtime installed"),
            Err(e) => {
                log::warn!(Build = id.as_str(), Code = format!("{:?}", e.code).as_str(); "Inference runtime not installed: {}", e.message)
            }
        }
        let _ = app.emit(RUNTIME_EVENT, RuntimeEvent::of_result(&id, &result));
        emit_status(&app, &core);
    });
    spawned
        .map(|_| ())
        .map_err(|e| AiCmdError::new(AiErrorCode::Io, format!("cannot start the download: {e}")))
}

/// Stops the running runtime install. False when none runs.
#[tauri::command]
pub fn ai_runtime_cancel<R: Runtime>(app: AppHandle<R>) -> AiResult<bool> {
    Ok(core(&app)?.cancel_runtime_install())
}

/// Deletes an installed build (of the pinned release or an older one); stops the engine first if it may
/// be using it.
#[tauri::command]
pub async fn ai_runtime_delete<R: Runtime>(app: AppHandle<R>, tag: String, id: String) -> AiResult<()> {
    let core = core(&app)?;
    let tag = short("a release tag", &tag)?;
    let id = short("an inference build", &id)?;
    let c = core.clone();
    blocking(move || {
        c.availability()?;
        c.delete_runtime(&tag, &id)
    })
    .await?;
    emit_status(&app, &core);
    Ok(())
}

/// Makes `id` (an installed build) or `auto` the setting *Inference build* and restarts the engine with it.
/// Saved through the settings service, like a change on the Settings screen.
#[tauri::command]
pub async fn ai_runtime_choose<R: Runtime>(app: AppHandle<R>, id: String) -> AiResult<()> {
    let core = core(&app)?;
    let id = short("an inference build", &id)?;
    let c = core.clone();
    let a = app.clone();
    blocking(move || {
        c.availability()?;
        let choice = c.check_build_choice(&id)?;
        let overview = crate::settings::settings_sections(a.clone()).map_err(|e| AiCmdError::new(AiErrorCode::Io, e))?;
        let mut values = overview
            .sections
            .into_iter()
            .find(|s| s.section.id == AI_SECTION)
            .map(|s| s.values)
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Default::default()));
        values["inference_build"] = Value::from(choice);
        let r = crate::settings::set_settings(a, AI_SECTION.into(), values).map_err(|e| AiCmdError::new(AiErrorCode::Io, e))?;
        if !r.applied {
            let why: Vec<String> = r.errors.into_iter().map(|e| e.message).collect();
            return Err(AiCmdError::invalid(why.join("; ")));
        }
        Ok(())
    })
    .await?;
    emit_status(&app, &core);
    Ok(())
}
