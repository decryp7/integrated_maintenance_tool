//! Debrief's services in the registry: what reports into each and what repairs it.

use super::model::{Cause, RepairAction, ServiceState, Snapshot};
use super::registry::{Registry, Sink};
use crate::engine::store_failure::StoreFailure;
use crate::sync::lock;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

pub const STORE: &str = "store";
pub const AI: &str = "ai";
pub const PLUGINS: &str = "plugins";
pub const BROWSER: &str = "browser";
pub const DOWNLOADS: &str = "downloads";

/// Every service and what the user calls it, in the order the status bar lists them.
pub const CATALOG: [(&str, &str); 5] = [
    (STORE, "the log store"),
    (AI, "the AI worker"),
    (PLUGINS, "plugins"),
    (BROWSER, "the research browser"),
    (DOWNLOADS, "downloads"),
];

/// Brings a service back. Runs on a background thread; `Err` is why it is still down.
pub type Repair = Arc<dyn Fn() -> Result<(), Cause> + Send + Sync>;

#[derive(Default)]
struct Extra {
    store_failure: Option<StoreFailure>,
    /// Quarantined plugins: id, then what happened.
    plugins: BTreeMap<String, String>,
    /// Download jobs that are not "downloads" (the browser install reports as the browser).
    other_jobs: HashSet<u64>,
    failed_download: Option<u64>,
}

/// What the AI worker's state means for health, kept apart from the AI module's DTO so this stays testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiHealth {
    /// Loaded, loading, idle or no model chosen: nothing is wrong.
    Fine,
    /// The worker crashed repeatedly and was switched off for the session.
    Switched,
    /// The last model load failed.
    LoadFailed,
}

pub struct Health {
    registry: Registry,
    repairs: Mutex<HashMap<&'static str, Repair>>,
    extra: Mutex<Extra>,
}

static CURRENT: RwLock<Option<Arc<Health>>> = RwLock::new(None);

#[cfg(test)]
thread_local! {
    static TEST: std::cell::RefCell<Option<Arc<Health>>> = const { std::cell::RefCell::new(None) };
}

/// The app's health, for code that has no app handle (the engine's error mapping, the renderer). Tests set their own
/// for the calling thread with [`use_in_this_thread`].
pub fn current() -> Option<Arc<Health>> {
    #[cfg(test)]
    if let Some(h) = TEST.with(|t| t.borrow().clone()) {
        return Some(h);
    }
    CURRENT.read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Makes `h` the app's health (call once from `setup`).
pub fn install(h: Arc<Health>) {
    *CURRENT.write().unwrap_or_else(|p| p.into_inner()) = Some(h);
}

#[cfg(test)]
pub fn use_in_this_thread(h: Arc<Health>) {
    TEST.with(|t| *t.borrow_mut() = Some(h));
}

impl Health {
    pub fn new(sink: Sink) -> Arc<Health> {
        Arc::new(Health {
            registry: Registry::new(&CATALOG, sink),
            repairs: Mutex::new(HashMap::new()),
            extra: Mutex::new(Extra::default()),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.registry.snapshot()
    }

    pub fn state(&self, service: &str) -> Option<ServiceState> {
        self.registry.get(service).map(|s| s.state)
    }

    /// What repairs `service` (the shell's own code: it never asks the service).
    pub fn set_repair(&self, service: &'static str, repair: Repair) {
        lock(&self.repairs).insert(service, repair);
    }

    // ---- the log store ----

    /// The store worker did not start or stopped (`detail` is the technical chain).
    pub fn store_failed(&self, failure: StoreFailure, detail: &str) {
        lock(&self.extra).store_failure = Some(failure);
        let cause = Cause::new(failure.message()).hint(failure.hint()).detail(detail);
        let state = if failure == StoreFailure::Quarantined {
            ServiceState::Quarantined
        } else {
            ServiceState::Down
        };
        let repair = if state == ServiceState::Quarantined {
            RepairAction::Reset
        } else {
            RepairAction::Restart
        };
        self.registry.set(STORE, state, Some(cause), Some(repair));
    }

    /// The store worker runs.
    pub fn store_running(&self) {
        lock(&self.extra).store_failure = None;
        self.registry.ok(STORE);
    }

    /// What the store probe should do now, given the supervisor's state (see [`probe_store`]).
    pub fn store_failure(&self) -> Option<StoreFailure> {
        lock(&self.extra).store_failure
    }

    // ---- the AI worker ----

    pub fn ai(&self, health: AiHealth, reason: Option<&str>) {
        let reason = reason.unwrap_or("").trim();
        match health {
            AiHealth::Fine => {
                self.registry.ok(AI);
            }
            AiHealth::Switched => {
                let cause = Cause::new("The AI worker was switched off after it failed several times in a row.")
                    .hint("Choose Try again to start it. If it stops again, report the problem and include the details.")
                    .detail(reason);
                self.registry
                    .set(AI, ServiceState::Quarantined, Some(cause), Some(RepairAction::Reset));
            }
            AiHealth::LoadFailed => {
                let cause = Cause::new("The AI model could not be loaded.")
                    .hint("Choose Try again. If it keeps failing, pick another model in Settings ▸ AI.")
                    .detail(reason);
                self.registry.set(AI, ServiceState::Down, Some(cause), Some(RepairAction::Reset));
            }
        }
    }

    // ---- plugins ----

    /// Plugin `id` ("name") was switched off after repeated failures. One service for all plugins: the cause lists them.
    pub fn plugin_quarantined(&self, id: &str, name: &str, message: &str) {
        lock(&self.extra).plugins.insert(id.to_string(), format!("{name}: {message}"));
        self.publish_plugins();
    }

    /// Plugin `id` may run again.
    pub fn plugin_reset(&self, id: &str) {
        lock(&self.extra).plugins.remove(id);
        self.publish_plugins();
    }

    /// The ids of the plugins switched off now.
    pub fn quarantined_plugins(&self) -> Vec<String> {
        lock(&self.extra).plugins.keys().cloned().collect()
    }

    fn publish_plugins(&self) {
        let plugins = lock(&self.extra).plugins.clone();
        if plugins.is_empty() {
            self.registry.ok(PLUGINS);
            return;
        }
        let ids: Vec<&str> = plugins.keys().map(String::as_str).collect();
        let message = match ids.len() {
            1 => format!("The plugin {} was switched off after it failed several times.", ids[0]),
            n => format!("{n} plugins were switched off after they failed several times: {}.", ids.join(", ")),
        };
        let detail = plugins.values().cloned().collect::<Vec<_>>().join("\n");
        let cause = Cause::new(message)
            .hint("Choose Allow again to let them run. If they stop again, update or disable the plugin in Plugins.")
            .detail(detail);
        self.registry
            .set(PLUGINS, ServiceState::Quarantined, Some(cause), Some(RepairAction::LiftQuarantine));
    }

    // ---- the research browser ----

    /// No browser could be started (`why` is the engine's message).
    pub fn browser_missing(&self, why: &str) {
        let cause = Cause::new("The research browser is not available.")
            .hint("Install the downloaded Chromium (Repair), or install Edge, Chrome or Chromium.")
            .detail(why);
        self.registry.down(BROWSER, cause, Some(RepairAction::Install));
    }

    pub fn browser_available(&self) {
        self.registry.ok(BROWSER);
    }

    // ---- downloads ----

    /// A download (model, runtime) failed; `job` is the job to run again.
    pub fn download_failed(&self, job: u64, title: &str, error: &str) {
        {
            let mut x = lock(&self.extra);
            if x.other_jobs.contains(&job) {
                return;
            }
            x.failed_download = Some(job);
        }
        let cause = Cause::new(format!("A download failed: {title}."))
            .hint("Check the connection, then choose Try again.")
            .detail(error);
        self.registry.down(DOWNLOADS, cause, Some(RepairAction::Retry));
    }

    /// A download finished.
    pub fn download_done(&self, job: u64) {
        let mut x = lock(&self.extra);
        if x.other_jobs.contains(&job) {
            return;
        }
        x.failed_download = None;
        drop(x);
        self.registry.ok(DOWNLOADS);
    }

    /// The job the download repair runs again.
    pub fn failed_download(&self) -> Option<u64> {
        lock(&self.extra).failed_download
    }

    /// A download job that reports as another service (the browser install): downloads ignores it.
    pub fn ignore_download_job(&self, job: u64) {
        lock(&self.extra).other_jobs.insert(job);
    }

    // ---- repair ----

    /// Starts the repair of `service` on a background thread (Paddock rule 10) and returns at once. The state is
    /// `repairing` until it ends, then `ok` or `down` again with the new cause.
    pub fn repair(self: &Arc<Self>, service: &str) -> Result<(), String> {
        let id = self.begin_repair(service)?;
        let health = self.clone();
        std::thread::Builder::new()
            .name(format!("repair-{id}"))
            .spawn(move || health.finish_repair(id))
            .map(|_| ())
            .map_err(|e| format!("the repair could not start: {e}"))
    }

    /// Marks the repair as running; returns the service's static id. Refuses an unknown service, one that works, or one
    /// nothing repairs.
    pub fn begin_repair(&self, service: &str) -> Result<&'static str, String> {
        let id = CATALOG
            .iter()
            .map(|(id, _)| *id)
            .find(|id| *id == service)
            .ok_or_else(|| "there is no such service".to_string())?;
        let current = self.registry.get(id).ok_or_else(|| "there is no such service".to_string())?;
        match current.state {
            ServiceState::Ok | ServiceState::Starting => return Err(format!("{} is working; there is nothing to repair", current.label)),
            ServiceState::Repairing => return Ok(id),
            _ => {}
        }
        let action = current
            .repair
            .ok_or_else(|| format!("nothing repairs {} from here", current.label))?;
        if !lock(&self.repairs).contains_key(id) {
            return Err(format!("nothing repairs {} from here", current.label));
        }
        self.registry.set(id, ServiceState::Repairing, current.cause, Some(action));
        Ok(id)
    }

    /// Runs the repair (blocking) and records how it ended.
    pub fn finish_repair(&self, id: &'static str) {
        let repair = lock(&self.repairs).get(id).cloned();
        let result = match repair {
            Some(r) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| r()))
                .unwrap_or_else(|_| Err(Cause::new("The repair failed unexpectedly."))),
            None => Err(Cause::new("Nothing repairs this from here.")),
        };
        match result {
            Ok(()) => match id {
                STORE => self.store_running(),
                PLUGINS => {
                    lock(&self.extra).plugins.clear();
                    self.registry.ok(PLUGINS);
                }
                DOWNLOADS => {
                    lock(&self.extra).failed_download = None;
                    self.registry.ok(DOWNLOADS);
                }
                other => {
                    self.registry.ok(other);
                }
            },
            Err(cause) => {
                let repair = self.registry.get(id).and_then(|s| s.repair);
                self.registry.set(id, ServiceState::Down, Some(cause), repair);
            }
        }
    }
}

/// What the store probe does for the supervisor's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreProbe {
    Nothing,
    /// The worker runs: the service is ok.
    Running,
    /// The supervisor stopped it for repeated crashes.
    Quarantined,
    /// It crashed and nobody asked it since: start it again (the supervisor's crash ledger bounds this).
    Restart,
}

/// Decides from the supervisor's state, the service's state now and why it failed.
pub fn probe_store(running: bool, quarantined: bool, now: ServiceState, failure: Option<StoreFailure>) -> StoreProbe {
    if quarantined {
        return if now == ServiceState::Quarantined {
            StoreProbe::Nothing
        } else {
            StoreProbe::Quarantined
        };
    }
    if running {
        return if now == ServiceState::Ok {
            StoreProbe::Nothing
        } else {
            StoreProbe::Running
        };
    }
    match (now, failure) {
        (ServiceState::Down, Some(StoreFailure::Crashed | StoreFailure::Other)) => StoreProbe::Restart,
        _ => StoreProbe::Nothing,
    }
}

#[cfg(test)]
mod tests;

// ---- reporting for code without a handle on the Health value ----

/// The store worker did not start or stopped.
pub fn report_store_failed(failure: StoreFailure, detail: &str) {
    if let Some(h) = current() {
        h.store_failed(failure, detail);
    }
}

/// No browser could be started.
pub fn report_browser_missing(why: &str) {
    if let Some(h) = current() {
        h.browser_missing(why);
    }
}

/// A browser can be started now.
pub fn report_browser_available() {
    if let Some(h) = current() {
        h.browser_available();
    }
}

/// Plugin `id` was switched off after repeated failures.
pub fn report_plugin_quarantined(id: &str, message: &str) {
    if let Some(h) = current() {
        h.plugin_quarantined(id, id, message);
    }
}

/// Plugin `id` may run again.
pub fn report_plugin_reset(id: &str) {
    if let Some(h) = current() {
        h.plugin_reset(id);
    }
}
