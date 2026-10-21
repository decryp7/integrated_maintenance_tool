//! Tools: typed request/response operations a plugin contributes and the
//! shell dispatches generically (ADR 0002), such as controlling Performance
//! Monitor, running the process monitor or collecting a diagnostics bundle.
//!
//! A [`Tool`] describes itself once ([`ToolInfo`]): its operations, each with
//! an argument [`Schema`] and a time budget, the platforms it runs on and
//! whether it usually needs administrator rights. The host
//! ([`ToolHost`], from [`crate::PluginRegistry::tools`]) then
//!
//! * lists every tool with its availability: available, unsupported on this
//!   platform (with the reason), or refused because its plugin is inactive,
//!   quarantined or not responding (with the plugin's reason);
//! * validates the arguments against the operation's schema before the tool
//!   sees them, so a tool only parses well-formed input;
//! * runs the operation under the plugin's guard on a helper thread: a panic
//!   becomes an error counted towards quarantine, and an operation that runs
//!   past its budget is abandoned and marks the plugin hung.
//!
//! Long-running work (a running monitor, a started collector set) is reported
//! through [`ToolCtx::tasks`] so the shell can list it (tray, status bar) and
//! stop it later through [`ToolHost::stop`], which calls [`Tool::stop`].

use crate::guard::{CallKind, Guard};
use serde::Serialize;
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

// The tool contract (descriptions, argument schemas with typed paths, contexts, failures) is part of the plugin
// contract (ADR 0017).
pub use imt_plugin_api::tools::{
    failure_reason, FailureReason, Field, NoPaths, NoTasks, OpSpec, PathAccess, PathKind, PathPolicy, Platform, Schema, SchemaError,
    TaskSink, Tool, ToolCtx, ToolFailure, ToolInfo, ToolTask, DEFAULT_OP_BUDGET, MAX_ARGS_BYTES, MAX_ID_LEN,
};

/// A tool as registered: its description and the guard of its plugin.
pub(crate) struct Registered {
    pub owner: String,
    pub guard: Option<Guard>,
    pub info: ToolInfo,
    pub tool: Arc<dyn Tool>,
}

/// Whether a tool can be called now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Availability {
    Available,
    /// Not on this platform.
    Unsupported {
        reason: String,
    },
    /// Its plugin is inactive, quarantined or not responding.
    PluginUnavailable {
        reason: String,
    },
}

/// A tool as listed for the UI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolEntry {
    #[serde(flatten)]
    pub info: ToolInfo,
    /// The plugin that contributes it.
    pub plugin: String,
    pub availability: Availability,
}

/// Why a tool call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// No installed (enabled) plugin contributes this tool.
    UnknownTool(String),
    UnknownOp {
        tool: String,
        op: String,
        known: Vec<String>,
    },
    /// The tool does not run on this platform.
    Unsupported(String),
    InvalidArgs {
        tool: String,
        op: String,
        error: SchemaError,
    },
    /// The plugin refused (inactive, quarantined, hung) or the call failed, crashed or overran.
    Failed(String),
    /// The call needs rights the user lacks (administrator, a group): the next step is to restart elevated.
    Denied(String),
    /// A program the tool runs is not installed.
    Missing(String),
    /// A path argument names something the user did not choose (`at` is where in the arguments).
    PathRefused {
        tool: String,
        op: String,
        error: SchemaError,
    },
}

/// A failed call as a [`ToolError`]: a [`ToolFailure`] or an OS "permission denied" in its chain keeps its reason.
fn failed(e: anyhow::Error) -> ToolError {
    let message = format!("{e:#}");
    match failure_reason(&e) {
        Some(FailureReason::Denied) => ToolError::Denied(message),
        Some(FailureReason::Missing) => ToolError::Missing(message),
        None => ToolError::Failed(message),
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::UnknownTool(t) => write!(f, "no installed plugin provides the tool \"{t}\""),
            ToolError::UnknownOp { tool, op, known } => {
                write!(f, "tool \"{tool}\" has no operation \"{op}\" (it has: {})", known.join(", "))
            }
            ToolError::Unsupported(reason) => write!(f, "{reason}"),
            ToolError::InvalidArgs { tool, op, error } | ToolError::PathRefused { tool, op, error } => write!(f, "{tool} {op}: {error}"),
            ToolError::Failed(message) | ToolError::Denied(message) | ToolError::Missing(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ToolError {}

/// The tools of a registry, for the shell: listing, dispatch and stopping
/// background work. Cheap to clone and usable without the workspace lock.
#[derive(Clone)]
pub struct ToolHost {
    tools: Arc<[Arc<Registered>]>,
    platform: Platform,
}

impl fmt::Debug for ToolHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolHost")
            .field("tools", &self.tools.iter().map(|t| t.info.id.as_str()).collect::<Vec<_>>())
            .field("platform", &self.platform)
            .finish()
    }
}

impl ToolHost {
    pub(crate) fn new(tools: &[Arc<Registered>]) -> ToolHost {
        ToolHost {
            tools: tools.into(),
            platform: Platform::current(),
        }
    }

    /// The same host judging availability for another platform (tests).
    pub fn on_platform(mut self, platform: Platform) -> ToolHost {
        self.platform = platform;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Every registered tool with its availability, in registration order.
    pub fn list(&self) -> Vec<ToolEntry> {
        self.tools
            .iter()
            .map(|t| ToolEntry {
                info: t.info.clone(),
                plugin: t.owner.clone(),
                availability: self.availability(t),
            })
            .collect()
    }

    fn availability(&self, t: &Registered) -> Availability {
        if let Some(reason) = t.info.unsupported_on(self.platform) {
            return Availability::Unsupported { reason };
        }
        match t.guard.as_ref().map(|g| g.health().availability(g.owner())) {
            Some(Err(reason)) => Availability::PluginUnavailable { reason },
            _ => Availability::Available,
        }
    }

    fn find(&self, tool: &str) -> std::result::Result<&Arc<Registered>, ToolError> {
        self.tools
            .iter()
            .find(|t| t.info.id == tool)
            .ok_or_else(|| ToolError::UnknownTool(tool.chars().take(MAX_ID_LEN).collect()))
    }

    /// Validates `args` against the op's schema and runs the op under the
    /// plugin's guard and the op's budget.
    pub fn invoke(&self, tool: &str, op: &str, args: Value, ctx: &ToolCtx) -> std::result::Result<Value, ToolError> {
        let t = self.find(tool)?;
        let spec = t.info.ops.iter().find(|o| o.name == op).ok_or_else(|| ToolError::UnknownOp {
            tool: t.info.id.clone(),
            op: op.chars().take(MAX_ID_LEN).collect(),
            known: t.info.ops.iter().map(|o| o.name.clone()).collect(),
        })?;
        if let Some(reason) = t.info.unsupported_on(self.platform) {
            return Err(ToolError::Unsupported(reason));
        }
        let invalid = |error| ToolError::InvalidArgs {
            tool: t.info.id.clone(),
            op: spec.name.clone(),
            error,
        };
        let size = serde_json::to_vec(&args).map(|v| v.len()).unwrap_or(usize::MAX);
        if size > MAX_ARGS_BYTES {
            return Err(invalid(SchemaError {
                at: "args".into(),
                message: format!("are too large ({size} bytes, at most {MAX_ARGS_BYTES})"),
            }));
        }
        // No arguments at all is the same as an empty object.
        let mut args = if args.is_null() { Value::Object(Default::default()) } else { args };
        spec.args.validate(&args).map_err(invalid)?;
        // Path arguments must be what the user chose (the calling window's grants); they go on canonical.
        let paths = ctx.paths().clone();
        spec.args
            .check_paths(&mut args, &|p, kind, access| paths.check(p, kind, access))
            .map_err(|error| ToolError::PathRefused {
                tool: t.info.id.clone(),
                op: spec.name.clone(),
                error,
            })?;
        let (inner, ctx, op_name) = (t.tool.clone(), ctx.for_tool(&t.info.id), spec.name.clone());
        let what = format!("tool {} {}", t.info.id, spec.name);
        let run = move || inner.invoke(&op_name, args, &ctx);
        match &t.guard {
            Some(g) => g.call_detached_within(CallKind::Tool, spec.budget, &what, run),
            None => crate::guard::catch(&what, run),
        }
        .map_err(failed)
    }

    /// Stops background work of `tool` (from the tray or status bar).
    pub fn stop(&self, tool: &str, task: &str, ctx: &ToolCtx) -> std::result::Result<(), ToolError> {
        let t = self.find(tool)?;
        let (inner, ctx, id) = (t.tool.clone(), ctx.for_tool(&t.info.id), task.to_string());
        let what = format!("tool {} stop", t.info.id);
        let run = move || inner.stop(&id, &ctx);
        match &t.guard {
            Some(g) => g.call_detached_within(CallKind::Tool, Some(DEFAULT_OP_BUDGET), &what, run),
            None => crate::guard::catch(&what, run),
        }
        .map_err(failed)
    }
}

#[cfg(test)]
mod tests;
