//! The activity log: every user operation and every IPC command, correlated by an operation id, so a
//! problem a user hit can be reproduced and fixed.
//!
//! Two layers write to IMT's own CLEF log under `SourceContext = "IMT.Activity"`:
//! - **UI intent** (Angular, `app/src/app/core/activity`): one entry per user operation at the point of
//!   intent (`User {Operation} via {Trigger}`), with an `OperationId` that travels to Rust as the
//!   [`OP_HEADER`] IPC header.
//! - **Backend execution** (here): [`logged`] wraps the Tauri invoke handler and logs every command with
//!   its window, operation id and kind ([`CommandKind`]: queries at Debug, mutations at Info); commands
//!   log domain events (`profile saved`, `export written`, ...) with [`domain_event`].
//!
//! [`ActivityLogger`] sits in front of the log plugin: it keeps a ring of the latest entries per window
//! ([`Breadcrumbs`]), attaches the last ones to every error, deduplicates repeated errors, and the CLEF
//! enrichment (`crate::clef`) tags every line with version, commit, OS and session id. [`steps`] turns
//! breadcrumbs into human-readable steps to reproduce.
//!
//! Privacy ([`privacy`]): no file contents, log lines, record messages or AI/web text are ever logged.
//! Query text and paths are reduced to fingerprints in the default [`Detail::Normal`] mode; secrets are
//! always redacted.

mod breadcrumbs;
mod config;
mod ipc;
mod logger;
pub mod privacy;
pub mod report;
mod steps;

pub use breadcrumbs::{Breadcrumb, BREADCRUMBS_PER_ERROR};
pub use config::{build_commit, config, session_id, set_config, ActivitySettings, Detail};
pub(crate) use ipc::current_window;
#[cfg(test)]
pub(crate) use ipc::OP_HEADER;
pub use ipc::{current_op, logged, OpId};
pub use logger::{breadcrumbs, ActivityLogger};
#[cfg(test)]
pub(crate) use logger::{captured, test_capture, Captured};
pub use steps::{steps, steps_text};

/// `SourceContext` of every activity entry (UI and Rust), the filter of Help ▸ Show activity log.
pub const SOURCE_CONTEXT: &str = "IMT.Activity";

/// Logs a domain event caused by a user command (profile saved, plugin enabled, export written, ...) under
/// [`SOURCE_CONTEXT`], with the operation id of the command that caused it. `props` must already be safe
/// (see [`privacy`]): never file contents, query text in normal mode or secrets.
pub fn domain_event(event: &str, op: Option<&str>, props: &[(&str, &str)]) {
    let op = op.map(str::to_string).or_else(current_op).unwrap_or_default();
    let window = ipc::current_window().unwrap_or_default();
    let mut kvs: Vec<(&str, log::kv::Value)> = vec![
        ("SourceContext", log::kv::Value::from(SOURCE_CONTEXT)),
        ("Operation", log::kv::Value::from(event)),
        ("Trigger", log::kv::Value::from("engine")),
        ("Kind", log::kv::Value::from("domain")),
    ];
    if !op.is_empty() {
        kvs.push(("OperationId", log::kv::Value::from(op.as_str())));
    }
    if !window.is_empty() {
        kvs.push(("Window", log::kv::Value::from(window.as_str())));
    }
    let props: Vec<(String, String)> = props
        .iter()
        .take(privacy::MAX_PARAMS)
        .map(|(k, v)| ((*k).to_string(), privacy::safe_param(k, v)))
        .collect();
    for (k, v) in &props {
        kvs.push((k.as_str(), log::kv::Value::from(v.as_str())));
    }
    log::logger().log(
        &log::Record::builder()
            .level(log::Level::Info)
            .target(SOURCE_CONTEXT)
            .key_values(&kvs)
            .args(format_args!("{event}"))
            .build(),
    );
}
