//! The inference runtime commands (ADR 0009): list, install (a download job streaming
//! `debrief://ai-runtime`), cancel, delete and choose llama-server builds. Thin: they validate the IPC
//! arguments, run the work off the UI thread and turn errors into [`AiCmdError`]s.

use super::commands::core;
use super::core::AiCore;
use super::error::{AiCmdError, AiErrorCode, AiResult};
use super::runtimes::RuntimeEvent;
use super::{RUNTIME_EVENT, STATUS_EVENT};
use crate::jobs::{self, Controls, Done, JobKind, JobLink, JobProgress, JobResult, JobSpec};
use crate::settings::AI_SECTION;
use debrief_ai::runtimes::builds::label;
use debrief_ai::runtimes::{InstalledRuntime, RuntimeOverview};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Runtime};

const MAX_ID_CHARS: usize = 32;
/// A build has one or two archives; a few more are refused with a clear message by the store.
const MAX_ARCHIVES: usize = 8;
/// How often the download ticker reports progress.
const TICK: Duration = Duration::from_millis(250);

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AiResult<T> + Send + 'static) -> AiResult<T> {
    crate::sync::blocking("the AI task", f).await
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

/// Downloads, verifies and unpacks a build. Returns at once; progress arrives as `debrief://ai-runtime`
/// events ending in `ready`, `failed` or `cancelled`. Runs as a download job, returned (`jobs_cancel` stops it).
#[tauri::command]
pub async fn ai_runtime_install<R: Runtime>(app: AppHandle<R>, id: String) -> AiResult<u64> {
    install(app, id, vec![])
}

/// Installs a build from its archives the user downloaded themselves ("Download manually"): they are
/// checked against the pinned SHA-256 and unpacked like a download. Events and job as [`ai_runtime_install`].
#[tauri::command]
pub async fn ai_runtime_add_archives<R: Runtime>(
    app: AppHandle<R>,
    grants: crate::paths::WindowGrants,
    id: String,
    paths: Vec<String>,
) -> AiResult<u64> {
    if paths.is_empty() || paths.len() > MAX_ARCHIVES {
        return Err(AiCmdError::invalid("choose the downloaded archives of the build"));
    }
    let archives = paths.iter().map(|p| archive_path(&grants, p)).collect::<AiResult<Vec<_>>>()?;
    install(app, id, archives)
}

/// A file the user picked in the open dialog (granted to the window), canonical.
fn archive_path(grants: &crate::paths::WindowGrants, path: &str) -> AiResult<PathBuf> {
    if !crate::paths::parse(path)?.exists() {
        return Err(AiCmdError::not_found(format!("cannot open {path}: it does not exist")));
    }
    grants.read_file(path, &[]).map(|g| g.path().to_path_buf())
}

/// Runs after an install ended (successfully or not), e.g. to retire the build an update replaced.
pub(crate) type AfterInstall = Box<dyn FnOnce(&AiResult<InstalledRuntime>) + Send>;

/// Starts an install job (returned; `jobs_cancel` stops it).
pub(crate) fn install<R: Runtime>(app: AppHandle<R>, id: String, archives: Vec<PathBuf>) -> AiResult<u64> {
    start_install_with(app, id, archives, None, None)
}

/// [`install`] with the job's title and a hook that runs when the install ended.
pub(crate) fn start_install_with<R: Runtime>(
    app: AppHandle<R>,
    id: String,
    archives: Vec<PathBuf>,
    title: Option<String>,
    after: Option<AfterInstall>,
) -> AiResult<u64> {
    let core = core(&app)?;
    let id = short("an inference build", &id)?;
    let progress = core.begin_runtime_install(&id)?;
    let verb = if archives.is_empty() { "Downloading" } else { "Installing" };
    let title = title.unwrap_or_else(|| format!("{verb} the {} inference runtime", label(&id)));
    let spec = JobSpec::new(JobKind::Download, title)
        .link(JobLink::Model { id: id.clone() })
        .retry_with(serde_json::json!({"op": "runtime", "runtimeId": id}));
    let c = core.clone();
    let controls = Controls::cancel(move || {
        c.cancel_runtime_install();
    });
    let (a, c) = (app.clone(), core.clone());
    let queued = jobs::spawn(&app, spec, controls, move |ctx: &jobs::JobCtx| -> AiResult<Done<()>> {
        let (app, core) = (a, c);
        let result = std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                let a = app.clone();
                let mut on_stage = |st| {
                    let _ = a.emit(RUNTIME_EVENT, RuntimeEvent::of_stage(&id, st));
                };
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    core.install_runtime(&id, &archives, &progress, &mut on_stage)
                }))
                .unwrap_or_else(|_| Err(AiCmdError::new(AiErrorCode::Engine, "the runtime install stopped unexpectedly")))
            });
            let mut last = None;
            while !worker.is_finished() {
                std::thread::sleep(TICK);
                let ev = RuntimeEvent::of_progress(&id, &progress);
                if let RuntimeEvent::Progress {
                    bytes, total, verifying, ..
                } = &ev
                {
                    if *total > 0 && last != Some((*bytes, *total)) {
                        last = Some((*bytes, *total));
                        ctx.progress(phase_progress(*verifying, *bytes, *total));
                        let _ = app.emit(RUNTIME_EVENT, ev);
                    }
                }
            }
            worker
                .join()
                .unwrap_or_else(|_| Err(AiCmdError::new(AiErrorCode::Engine, "the runtime install stopped unexpectedly")))
        });
        core.end_runtime_install();
        match &result {
            Ok(r) => log::info!(Build = id.as_str(), Tag = r.tag.as_str(); "Inference runtime installed"),
            Err(e) => {
                log::warn!(Build = id.as_str(), Code = format!("{:?}", e.code).as_str(); "Inference runtime not installed: {}", e.message)
            }
        }
        let _ = app.emit(RUNTIME_EVENT, RuntimeEvent::of_result(&id, &result));
        if let Some(after) = after {
            after(&result);
        }
        emit_status(&app, &core);
        result.map(|_| Done::new(()).with_result(JobResult::Model { detail: None, file_name: None }))
    });
    queued.map(|j| j.0).map_err(|e| {
        core.end_runtime_install();
        AiCmdError::busy(e)
    })
}

/// Job progress for one artifact: bytes, labelled with the phase so a checksum pass reads as "Checking", not as a
/// second download restarting from zero under the same title.
pub(crate) fn phase_progress(verifying: bool, bytes: u64, total: u64) -> JobProgress {
    JobProgress {
        current: Some(if verifying { "Checking" } else { "Downloading" }.to_string()),
        ..JobProgress::bytes(bytes, total)
    }
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
        let overview = Ok::<_, AiCmdError>(crate::settings::overview(&a))?;
        let mut values = overview
            .sections
            .into_iter()
            .find(|s| s.section.id == AI_SECTION)
            .map(|s| s.values)
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Default::default()));
        values["inference_build"] = Value::from(choice);
        let r = crate::settings::apply_section(&a, AI_SECTION, values)?;
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
