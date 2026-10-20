//! The Tauri side of the AI update checks ([`super::updates`]): the scheduler thread, the install jobs, the
//! "Update / Later / Skip this version" message and the commands `ai_updates_status`, `ai_updates_check`,
//! `ai_updates_apply`, `ai_updates_skip`, `ai_updates_later`. Event: `imt://ai-updates` (the status, to every
//! window). Wire types: `app/src/app/ai/ai.types.ts` (`AiUpdatesStatus`).

use super::commands::core;
use super::core::AiCore;
use super::error::{AiCmdError, AiErrorCode, AiResult};
use super::runtime_commands::start_install_with;
use super::updates::{system_now, AiUpdates, AiUpdatesStatus, BusyProbe, UpdateEffects};
use super::{STATUS_EVENT, UPDATES_EVENT};
use crate::jobs::{self, Controls, Done, JobKind, JobLink, JobProgress, JobResult, JobSpec};
use crate::notification_centre::centre::{NewNotice, NoticeAction, NoticeScope, NoticeSeverity, NoticeSource};
use imt_ai::setup::SetupStage;
use imt_ai::updates::{UpdateCandidate, UpdateKind};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// The scheduler's beat; the first one comes a minute after start-up (never during it).
const TICK: Duration = Duration::from_secs(60);
const MAX_KEY_CHARS: usize = 160;

/// The app-wide update checks, managed as Tauri state.
pub struct AiUpdatesState(pub Arc<AiUpdates>);

/// A chat turn, a use case or web research is running: an automatic install or a model switch waits.
pub fn chat_busy<R: Runtime>(app: &AppHandle<R>) -> BusyProbe {
    let app = app.clone();
    Arc::new(move || {
        app.try_state::<jobs::Jobs>()
            .is_some_and(|j| !j.find_active(|job| matches!(job.kind, JobKind::Ai | JobKind::Research)).is_empty())
    })
}

struct TauriEffects<R: Runtime> {
    app: AppHandle<R>,
}

fn updates<R: Runtime>(app: &AppHandle<R>) -> AiResult<Arc<AiUpdates>> {
    app.try_state::<AiUpdatesState>()
        .map(|s| s.0.clone())
        .ok_or_else(|| AiCmdError::unavailable("AI updates are not available in this run"))
}

fn size_text(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{:.0} MB", (bytes as f64 / 1e6).max(1.0))
    }
}

/// The message of an update the user is asked about.
pub fn update_notice(u: &UpdateCandidate) -> NewNotice {
    let (title, body) = match u.kind {
        UpdateKind::Runtime => (
            format!("AI runtime update: {}", u.name),
            format!("llama.cpp {} replaces {} ({} download).", u.to, u.from, size_text(u.download_bytes)),
        ),
        UpdateKind::Model => (
            format!("AI model update: {}", u.name),
            format!("{} replaces {} ({} download).", u.to, u.from, size_text(u.download_bytes)),
        ),
    };
    NewNotice::new(NoticeSource::Ai, NoticeSeverity::Info, title, NoticeScope::App)
        .body(body)
        .primary(NoticeAction::UpdateAi { key: u.key.clone() })
        .secondary(NoticeAction::LaterAiUpdate { key: u.key.clone() })
        .tertiary(NoticeAction::SkipAiUpdate { key: u.key.clone() })
        .dedupe(format!("ai-update:{}:{}", u.kind.as_str(), u.id))
}

impl<R: Runtime> UpdateEffects for TauriEffects<R> {
    fn install(&self, u: &UpdateCandidate) -> AiResult<()> {
        match u.kind {
            UpdateKind::Runtime => {
                let (app, update) = (self.app.clone(), u.clone());
                start_install_with(
                    self.app.clone(),
                    u.id.clone(),
                    vec![],
                    Some(format!("Updating the {} inference runtime", u.name)),
                    Some(Box::new(move |r| {
                        if let Ok(updates) = updates(&app) {
                            updates.finished(&update, &r.as_ref().map(|_| ()).map_err(Clone::clone));
                        }
                    })),
                )
            }
            UpdateKind::Model => start_model_update(&self.app, u.clone()),
        }
    }

    fn announce(&self, u: &UpdateCandidate) {
        crate::notification_centre::post(&self.app, None, update_notice(u), None);
    }

    fn changed(&self, status: &AiUpdatesStatus) {
        let _ = self.app.emit(UPDATES_EVENT, status);
    }
}

/// A model update as a download job (Cancel stops it); it holds the set-up slot, so it does not run beside a set-up.
fn start_model_update<R: Runtime>(app: &AppHandle<R>, u: UpdateCandidate) -> AiResult<()> {
    let core = core(app)?;
    let progress = core.begin_setup()?;
    let spec = JobSpec::new(JobKind::Download, format!("Updating AI model {}", u.name)).link(JobLink::Model { id: u.id.clone() });
    let c = core.clone();
    let controls = Controls::cancel(move || {
        c.cancel_setup();
    });
    let (a, c) = (app.clone(), core.clone());
    let queued = jobs::spawn(app, spec, controls, move |ctx: &jobs::JobCtx| -> AiResult<Done<()>> {
        let (app, core) = (a, c);
        let busy = chat_busy(&app);
        let mut on_stage = |st: SetupStage| match st {
            SetupStage::Downloading { bytes, total, .. } | SetupStage::Verifying { bytes, total } => {
                ctx.progress(JobProgress::counts(bytes, total))
            }
            _ => {}
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            core.install_model_update(&u, &progress, &mut on_stage, &*busy)
        }))
        .unwrap_or_else(|_| Err(AiCmdError::new(AiErrorCode::Engine, "the model update stopped unexpectedly")));
        core.end_setup();
        let _ = app.emit(STATUS_EVENT, core.status());
        if let Ok(updates) = updates(&app) {
            updates.finished(&u, &r);
        }
        r.map(|_| {
            Done::new(()).with_result(JobResult::Model {
                file_name: Some(u.to.clone()),
            })
        })
    });
    queued.map(|_| ()).map_err(|e| {
        core.end_setup();
        AiCmdError::busy(e)
    })
}

/// Creates the update checks for `core` (epoch seconds from `now`) and manages them, without the scheduler.
pub fn manage<R: Runtime>(app: &AppHandle<R>, core: Arc<AiCore>, now: Box<dyn Fn() -> i64 + Send + Sync>) -> Arc<AiUpdates> {
    let record_dir = core.config_dir().map(|d| d.to_path_buf());
    let updates = Arc::new(AiUpdates::new(
        core,
        record_dir,
        chat_busy(app),
        now,
        Box::new(TauriEffects { app: app.clone() }),
    ));
    app.manage(AiUpdatesState(updates.clone()));
    updates
}

/// Creates the update checks for `core` and starts the scheduler. Call after [`super::manage`].
pub fn init<R: Runtime>(app: &AppHandle<R>, core: Arc<AiCore>) {
    let updates = manage(app, core, Box::new(system_now));
    let spawned = std::thread::Builder::new().name("imt-ai-updates".into()).spawn(move || loop {
        std::thread::sleep(TICK);
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| updates.tick())).is_err() {
            log::error!("AI update check stopped unexpectedly; it runs again at the next beat");
        }
    });
    if let Err(e) = spawned {
        log::warn!("Automatic AI update checks are off: {e}");
    }
}

fn check_key(key: &str) -> AiResult<&str> {
    let k = key.trim();
    if k.is_empty() || k.chars().count() > MAX_KEY_CHARS {
        return Err(AiCmdError::invalid("that is not an AI update"));
    }
    Ok(k)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> AiResult<T> + Send + 'static) -> AiResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AiCmdError::new(AiErrorCode::Engine, format!("the AI task stopped unexpectedly ({e})")))?
}

#[tauri::command]
pub fn ai_updates_status<R: Runtime>(app: AppHandle<R>) -> AiResult<AiUpdatesStatus> {
    Ok(updates(&app)?.status())
}

/// "Check now": checks `kind` (`runtime` or `model`), or both, whatever the policy.
#[tauri::command]
pub async fn ai_updates_check<R: Runtime>(app: AppHandle<R>, kind: Option<String>) -> AiResult<AiUpdatesStatus> {
    let updates = updates(&app)?;
    core(&app)?.availability()?;
    let kinds = match kind.as_deref() {
        None => UpdateKind::ALL.to_vec(),
        Some(k) => vec![UpdateKind::parse(k).ok_or_else(|| AiCmdError::invalid("check runtime or model updates"))?],
    };
    blocking(move || {
        let mut status = updates.status();
        for k in kinds {
            status = updates.check(k);
        }
        Ok(status)
    })
    .await
}

/// "Update": installs the update now as a download job.
#[tauri::command]
pub fn ai_updates_apply<R: Runtime>(app: AppHandle<R>, key: String) -> AiResult<AiUpdatesStatus> {
    updates(&app)?.apply(check_key(&key)?)
}

/// "Skip this version".
#[tauri::command]
pub fn ai_updates_skip<R: Runtime>(app: AppHandle<R>, key: String) -> AiResult<AiUpdatesStatus> {
    updates(&app)?.skip(check_key(&key)?)
}

/// "Later": asked again at the next check.
#[tauri::command]
pub fn ai_updates_later<R: Runtime>(app: AppHandle<R>, key: String) -> AiResult<AiUpdatesStatus> {
    updates(&app)?.later(check_key(&key)?)
}
