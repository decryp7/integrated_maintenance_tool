//! The Tauri side of service health: the `debrief://service-health` event to every window, the notification a
//! transition raises (one, with the cause `health:<service>`), the two commands and the status probes.

use super::hub::{self, AiHealth, Health, StoreProbe, AI, BROWSER, DOWNLOADS, PLUGINS, STORE};
use super::model::{cause_key, Cause, Change, RepairAction, ServiceState, Snapshot, SERVICE_HEALTH_EVENT};
use crate::ai::EngineStateDto;
use crate::app_state::AppState;
use crate::notification_centre::centre::{NewNotice, NoticeAction, NoticeScope, NoticeSeverity, NoticeSource};
use crate::notify::NotificationKind;
use crate::{CmdError, CmdResult};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// How often the supervisors' state is looked at (a crash is also reported at once by the failing call).
const PROBE_EVERY: Duration = Duration::from_secs(3);

/// What a change means for the notification centre: the message of a service that is not working, or nothing.
pub fn notice_for(snapshot: &Snapshot, change: &Change) -> Option<NewNotice> {
    let entry = snapshot.services.iter().find(|s| s.service == change.service)?;
    // The repair in progress keeps the message; the outcome (ok / down again) changes it.
    if !entry.state.is_problem() || entry.state == ServiceState::Repairing {
        return None;
    }
    let cause = entry.cause.as_ref()?;
    let severity = if entry.state == ServiceState::Degraded {
        NoticeSeverity::Warning
    } else {
        NoticeSeverity::Error
    };
    let mut n = NewNotice::new(
        cause_key(&entry.service),
        NoticeSource::Health,
        severity,
        cause.message.clone(),
        NoticeScope::App,
    )
    .body(cause.hint.clone().unwrap_or_default());
    match (entry.repair, &cause.settings) {
        (Some(RepairAction::OpenSettings), Some(section)) => {
            n = n.primary(NoticeAction::OpenSettings {
                section: section.to_string(),
            });
        }
        (Some(RepairAction::OpenSettings), None) | (None, _) => {}
        (Some(_), _) => {
            n = n.primary(NoticeAction::RepairService {
                service: entry.service.clone(),
            });
        }
    }
    Some(n)
}

/// A service changed: every window hears it, and the notification centre raises or resolves its one message.
fn announce<R: Runtime>(app: &AppHandle<R>, snapshot: &Snapshot, change: &Change) {
    if let Err(e) = app.emit(SERVICE_HEALTH_EVENT, snapshot) {
        log::warn!("The service-health event could not be sent: {e}");
    }
    log::info!(Service = change.service.as_str(), State = format!("{:?}", change.to).as_str(); "Service health changed");
    if change.to == ServiceState::Ok {
        crate::notification_centre::resolve_cause(app, &cause_key(&change.service));
    } else if let Some(n) = notice_for(snapshot, change) {
        crate::notification_centre::post(app, None, n, Some(NotificationKind::Attention));
    }
}

/// Creates the registry (announcing through the app's windows and notification centre), wires the repairs and manages it
/// as state. Without the probe or the global handle: tests build the app with this.
pub fn setup<R: Runtime>(app: &AppHandle<R>) -> Arc<Health> {
    let handle = app.clone();
    let health = Health::new(Box::new(move |snapshot, change| announce(&handle, snapshot, change)));
    wire_repairs(app, &health);
    app.manage(health.clone());
    health
}

/// `setup`, plus the process-wide handle for code without an app handle (the engine's error mapping) and the probe of the
/// supervisors. Call from `setup`, after the notification centre.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let health = setup(app);
    hub::install(health.clone());
    spawn_probe(app.clone(), health);
}

/// The AI worker's health after the user reset it (`ai_reset`), without waiting for the next probe.
pub fn refresh_ai<R: Runtime>(app: &AppHandle<R>, state: EngineStateDto, reason: Option<&str>) {
    if let Some(h) = app.try_state::<Arc<Health>>() {
        h.clear_cap_stop(AI);
        h.ai(ai_health(state), reason);
    }
}

fn wire_repairs<R: Runtime>(app: &AppHandle<R>, health: &Arc<Health>) {
    let h = app.clone();
    health.set_repair(STORE, Arc::new(move || repair_store(&h)));
    let h = app.clone();
    health.set_repair(AI, Arc::new(move || repair_ai(&h)));
    let (h, hl) = (app.clone(), health.clone());
    health.set_repair(PLUGINS, Arc::new(move || repair_plugins(&h, &hl)));
    let h = app.clone();
    health.set_repair(BROWSER, Arc::new(move || crate::browser::runtime::repair_install(&h)));
    let (h, hl) = (app.clone(), health.clone());
    health.set_repair(DOWNLOADS, Arc::new(move || repair_downloads(&h, &hl)));
}

/// Restarts the store worker from the shell: forgets the crash history, then starts it and waits for it to answer.
fn repair_store<R: Runtime>(app: &AppHandle<R>) -> Result<(), Cause> {
    let worker = app
        .try_state::<AppState>()
        .and_then(|s| s.performance.worker())
        .ok_or_else(|| Cause::new("The log store is not set up in this run."))?;
    restart_store_worker(&worker)
}

/// Forgets the worker's crash history and starts it, waiting for it to answer. The supervisor lives in the shell, so this does
/// not need the store (or anything the broken worker held) to work.
fn restart_store_worker(worker: &debrief_store_client::StoreWorker) -> Result<(), Cause> {
    worker.reset();
    worker.worker_status().map(|_| ()).map_err(|e| {
        let (failure, detail) = crate::engine::classify_worker_failure(&e.to_string());
        Cause::new(failure.message()).hint(failure.hint()).detail(detail)
    })
}

/// Lets the AI worker run again (the engine's own reset) and checks it is no longer switched off.
fn repair_ai<R: Runtime>(app: &AppHandle<R>) -> Result<(), Cause> {
    let core = crate::ai::commands::core(app).map_err(|e| Cause::new(e.message))?;
    core.reset();
    if let Some(jobs) = app.try_state::<crate::jobs::Jobs>() {
        jobs.pump();
    }
    match core.quarantine_reason() {
        Some(reason) => Err(Cause::new("The AI worker is still switched off.").detail(reason)),
        None => Ok(()),
    }
}

/// Lifts every plugin quarantine.
fn repair_plugins<R: Runtime>(app: &AppHandle<R>, health: &Health) -> Result<(), Cause> {
    let (Some(state), Some(windows)) = (app.try_state::<AppState>(), app.try_state::<crate::windows::Windows>()) else {
        return Err(Cause::new("Debrief is not ready yet."));
    };
    for id in health.quarantined_plugins() {
        crate::plugin_config::reset_plugin_everywhere(&windows, &state, &id);
    }
    Ok(())
}

/// Runs the download that failed again (as a new job, which reports its own outcome).
fn repair_downloads<R: Runtime>(app: &AppHandle<R>, health: &Health) -> Result<(), Cause> {
    let id = health
        .failed_download()
        .ok_or_else(|| Cause::new("There is no failed download to run again.").hint("Start it again from Settings."))?;
    let jobs = app
        .try_state::<crate::jobs::Jobs>()
        .ok_or_else(|| Cause::new("Debrief is not ready yet."))?;
    match jobs.retry(crate::jobs::JobId(id)) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(Cause::new("That download cannot be run again.").hint("Start it again from Settings.")),
        Err(e) => Err(Cause::new("The download could not be started again.").detail(e)),
    }
}

/// What the AI worker's state means for health.
pub fn ai_health(state: EngineStateDto) -> AiHealth {
    match state {
        EngineStateDto::Disabled => AiHealth::Switched,
        EngineStateDto::Failed => AiHealth::LoadFailed,
        _ => AiHealth::Fine,
    }
}

/// One look at the supervisors: the store worker and the AI worker.
fn probe<R: Runtime>(app: &AppHandle<R>, health: &Health) {
    if let Some(worker) = app.try_state::<AppState>().and_then(|s| s.performance.worker()) {
        use debrief_ipc::supervisor::ChildState;
        let status = worker.status();
        let now = health.state(STORE).unwrap_or(ServiceState::Ok);
        match hub::probe_store(
            status.state == ChildState::Running,
            status.state == ChildState::Quarantined,
            now,
            health.store_failure(),
        ) {
            StoreProbe::Nothing => {}
            StoreProbe::Running => health.store_running(),
            StoreProbe::Quarantined => health.store_failed(
                crate::engine::store_failure::StoreFailure::Quarantined,
                status.reason.as_deref().unwrap_or_default(),
            ),
            StoreProbe::Restart => {
                // The crash ledger bounds this: after repeated crashes the supervisor quarantines the worker.
                if let Err(e) = worker.worker_status() {
                    let (failure, detail) = crate::engine::classify_worker_failure(&e.to_string());
                    health.store_failed(failure, &detail);
                }
            }
        }
    }
    if let Ok(core) = crate::ai::commands::core(app) {
        let s = core.status();
        if s.available {
            health.ai(ai_health(s.state), s.reason.as_deref());
        }
    }
}

fn spawn_probe<R: Runtime>(app: AppHandle<R>, health: Arc<Health>) {
    let spawned = std::thread::Builder::new().name("service-health-probe".into()).spawn(move || loop {
        std::thread::sleep(PROBE_EVERY);
        probe(&app, &health);
    });
    if let Err(e) = spawned {
        log::warn!("The service-health probe could not start: {e}");
    }
}

fn health_of<R: Runtime>(app: &AppHandle<R>) -> CmdResult<Arc<Health>> {
    app.try_state::<Arc<Health>>()
        .map(|h| h.inner().clone())
        .ok_or_else(|| CmdError::unavailable("Debrief is not ready yet"))
}

/// Every service's state now. A window asks when it starts: a new window must not assume everything is fine.
#[tauri::command]
pub async fn health_snapshot<R: Runtime>(app: AppHandle<R>) -> CmdResult<Snapshot> {
    Ok(health_of(&app)?.snapshot())
}

/// Repairs `service` from the shell (never through the service itself). Returns at once; the state is `repairing`
/// until it ends and the outcome arrives as the usual event.
#[tauri::command]
pub async fn health_repair<R: Runtime>(app: AppHandle<R>, service: String) -> CmdResult<Snapshot> {
    let health = health_of(&app)?;
    health.repair(&service).map_err(CmdError::invalid)?;
    crate::activity::domain_event("health.repair", None, &[("Service", &service)]);
    Ok(health.snapshot())
}

#[cfg(test)]
mod tests;
