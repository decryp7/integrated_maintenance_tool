//! Debrief agent: headless, scheduled and triggered diagnostics collection.
//!
//! The agent reads a versioned `debrief-agent.toml` ([`config`]), decides when each
//! schedule is due ([`schedule`] for cron and interval timers, [`trigger`] for
//! process thresholds and log events), and runs its actions ([`action`]: the
//! diagnostics collector, a process-monitor recording, copying logs, a
//! Performance Monitor capture). Every run gets its own folder with a
//! `manifest.json` ([`run`]); old runs are pruned by count, size and age
//! ([`retention`]).
//!
//! Robustness: a schedule never overlaps itself ([`lock`]: lock file with a
//! PID liveness check), free disk space is checked before a run ([`disk`]),
//! corrupt state falls back to defaults ([`state`]), the agent logs CLEF to
//! rolling files ([`log`]) and every external tool runs with a timeout.
//!
//! The agent only reads and writes local files: it opens no network listener.
//! [`service`] holds the shared service loop plus thin OS adapters (Windows
//! Service, systemd unit and launchd plist generation); [`cli`] is the
//! `debrief-agent` command line.

pub mod action;
pub mod agent;
pub mod cli;
pub mod clock;
pub mod config;
pub mod disk;
pub mod job;
pub mod lock;
pub mod log;
pub mod retention;
pub mod run;
pub mod schedule;
pub mod schema;
pub mod service;
pub mod state;
pub mod toml_merge;
pub mod trigger;

pub use agent::{Agent, Deps};
pub use config::{AgentConfig, AgentConfigFile, Issue, CONFIG_VERSION};
pub use run::{RunManifest, RunStatus, RunSummary};

#[cfg(test)]
pub(crate) mod testing;
