//! The service-health model: a closed state per service, a plain cause and a repair action, and the snapshot and
//! event shapes windows read. No Debrief domain types and no Tauri: this module is written to move into the shared
//! `paddock-health` crate (Paddock ADR 0010) unchanged.
//!
//! Wire contract: `app/src/app/health/service-health.types.ts` (camelCase JSON). Change both sides together.

use serde::{Deserialize, Serialize};

/// The event every window listens to; its payload is a [`Snapshot`].
pub const SERVICE_HEALTH_EVENT: &str = "debrief://service-health";

/// How a service is doing. A closed set: a UI never has to guess from text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceState {
    /// Working.
    Ok,
    /// Starting; not an outage.
    Starting,
    /// Working with a limit (slow, or one part failed).
    Degraded,
    /// Not working.
    Down,
    /// Stopped by the supervisor after repeated failures, until it is reset.
    Quarantined,
    /// A repair is running.
    Repairing,
}

impl ServiceState {
    /// Whether things that need the service can be asked of it.
    pub fn is_ok(self) -> bool {
        matches!(self, ServiceState::Ok | ServiceState::Starting)
    }

    /// Whether the user has something to know (anything but `ok` and `starting`).
    pub fn is_problem(self) -> bool {
        !self.is_ok()
    }
}

/// What the shell can do to bring a service back. It is always served by the shell, never by the broken service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepairAction {
    /// Start the process again.
    Restart,
    /// Forget the failure history and start again.
    Reset,
    /// Let a quarantined part run again.
    LiftQuarantine,
    /// Install what is missing.
    Install,
    /// Run the failed work again.
    Retry,
    /// Use the fallback instead of the service that went away (the AI's built-in engine instead of Pitwall).
    UseFallback,
}

/// Advice to raise a memory cap that stopped a process: the UI offers "Raise the cap…" (it opens the setting with this value;
/// the shell cannot change a user's setting by itself).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RaiseCap {
    /// Which process: the app's own id (`storeWorker`, `ai`, `plugin:<id>`).
    pub target: String,
    pub suggested_mb: u32,
}

/// Why a service is not ok, in words for the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cause {
    /// One plain sentence.
    pub message: String,
    /// The next step for this cause.
    pub hint: Option<String>,
    /// The technical chain, for Details and bug reports.
    pub detail: Option<String>,
    /// Set when a memory cap stopped the process: the UI offers "Raise the cap…" next to the repair.
    pub raise_cap: Option<RaiseCap>,
}

impl Cause {
    pub fn new(message: impl Into<String>) -> Cause {
        Cause {
            message: message.into(),
            hint: None,
            detail: None,
            raise_cap: None,
        }
    }

    pub fn raise_cap(mut self, target: impl Into<String>, suggested_mb: u32) -> Cause {
        self.raise_cap = Some(RaiseCap {
            target: target.into(),
            suggested_mb,
        });
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Cause {
        self.hint = Some(hint.into());
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Cause {
        let d = detail.into();
        self.detail = (!d.trim().is_empty()).then_some(d);
        self
    }
}

/// One service as windows see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceHealth {
    /// Stable id (`store`, `ai`, ...); the notification cause is `health:<id>`.
    pub service: String,
    /// What the user calls it ("the log store").
    pub label: String,
    pub state: ServiceState,
    /// Set while the state is not `ok`.
    pub cause: Option<Cause>,
    /// What "Repair" does now; none when the user cannot do anything.
    pub repair: Option<RepairAction>,
}

/// Every service, in a fixed order, and a counter that grows with each change (a window keeps the newest).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub services: Vec<ServiceHealth>,
}

/// One service changed state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub service: String,
    pub from: ServiceState,
    pub to: ServiceState,
}

/// The notification cause of a service (one notification per service outage).
pub fn cause_key(service: &str) -> String {
    format!("health:{service}")
}
