//! Diagnostics collector for IMT.
//!
//! Gathers application logs, Windows event logs, `msinfo32`, registry keys,
//! a performance-counter sample and environment info into a single `.imtz`
//! bundle (a zip with `project.imtproj`, `manifest.json` and the collected
//! files) that IMT opens directly. Text is redacted while it is copied.
//!
//! External tools run behind [`CommandRunner`] so the logic is testable on any
//! platform; Windows-only steps are skipped with a manifest note elsewhere.

mod bundle;
pub mod cli;
mod collector;
mod env;
mod manifest;
mod plan;
mod redact;
mod runner;
mod select;

pub use collector::{safe_segment, CollectResult, Collector, NoProgress, Platform, Progress};
pub use manifest::{CommandRecord, CommandStatus, Item, ItemKind, Manifest, Skipped, StepTiming, MANIFEST_SCHEMA};
pub use plan::{CollectPlan, CustomPattern, LogSpec, PlanError, RedactConfig};
pub use redact::{luhn_ok, Counts, Redactor};
pub use runner::{CommandError, CommandOutput, CommandRunner, SystemRunner};
pub use select::{select_logs, Rejected, Selected};
