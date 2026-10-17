//! The `manifest.json` that documents what a bundle contains and what happened.

use crate::plan::CollectPlan;
use crate::redact::Counts;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MANIFEST_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Log,
    EventLog,
    Msinfo,
    Registry,
    Counters,
    Environment,
    UnifiedLog,
    Journal,
}

/// A file inside the bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// Path inside the bundle.
    pub path: String,
    pub kind: ItemKind,
    /// Where it came from on the machine (file path or the command used).
    pub origin: String,
    /// Size on the machine and size written to the bundle (after redaction).
    pub original_bytes: u64,
    pub bytes: u64,
    /// True when the content was scanned and masked; false for binary files
    /// and when redaction is disabled.
    pub redacted: bool,
    pub redactions: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub note: Option<String>,
}

/// Something that was looked at but left out, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub kind: ItemKind,
    pub what: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommandStatus {
    Ok,
    Failed,
    TimedOut,
    NotFound,
}

/// One external tool invocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRecord {
    pub program: String,
    pub args: Vec<String>,
    pub status: CommandStatus,
    pub exit_code: Option<i32>,
    pub stderr: String,
    pub ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepTiming {
    pub name: String,
    pub ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: u32,
    pub name: String,
    pub host: String,
    pub platform: String,
    pub created_at: String,
    pub tool_versions: BTreeMap<String, String>,
    pub items: Vec<Item>,
    pub skipped: Vec<Skipped>,
    pub commands: Vec<CommandRecord>,
    /// Matches per redaction rule, over the whole bundle.
    pub redaction: Counts,
    pub errors: Vec<String>,
    pub steps: Vec<StepTiming>,
    pub total_bytes: u64,
    pub max_bundle_bytes: u64,
    pub duration_ms: u64,
    /// The plan that produced the bundle.
    pub plan: CollectPlan,
}
