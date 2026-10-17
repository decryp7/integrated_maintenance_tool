//! The collection plan: what to gather, how much, and what to redact.
//!
//! A plan is plain data (JSON or TOML, camelCase keys). Unknown keys are
//! rejected so a typo never silently disables a collection step.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why a plan could not be used. The messages tell the user what to fix.
#[derive(Debug, Error, PartialEq)]
pub enum PlanError {
    #[error("the plan is not valid JSON: {0}")]
    Json(String),
    #[error("the plan is not valid TOML: {0}")]
    Toml(String),
    #[error("{0}")]
    Invalid(String),
}

/// One folder (or single file) of application logs to collect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogSpec {
    /// A directory (searched recursively) or a single file.
    pub path: String,
    /// Glob patterns matched against the path relative to `path` or the file
    /// name. Empty means everything.
    #[serde(default = "default_include")]
    pub include: Vec<String>,
    /// Only files modified within this many days. `None` means any age.
    #[serde(default)]
    pub max_age_days: Option<u32>,
    /// Keep the newest files until their sizes add up to this many MB.
    #[serde(default)]
    pub max_total_mb: Option<u64>,
}

fn default_include() -> Vec<String> {
    vec!["*.log".into(), "*.txt".into(), "*.json".into(), "*.clef".into(), "*.log.gz".into()]
}

/// A user-defined redaction rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomPattern {
    pub name: String,
    pub regex: String,
    /// Replacement text; `$1` / `${name}` refer to capture groups.
    #[serde(default = "default_replace")]
    pub replace: String,
}

fn default_replace() -> String {
    "[REDACTED]".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RedactConfig {
    /// Built-in rules: e-mail, bearer/API tokens, passwords, connection-string
    /// secrets, credit-card-like numbers (Luhn checked).
    #[serde(default = "yes")]
    pub built_in: bool,
    /// Also mask IPv4/IPv6 addresses (part of the built-ins; off keeps them).
    #[serde(default = "yes")]
    pub ip_addresses: bool,
    #[serde(default)]
    pub patterns: Vec<CustomPattern>,
}

impl Default for RedactConfig {
    fn default() -> Self {
        RedactConfig {
            built_in: true,
            ip_addresses: true,
            patterns: Vec::new(),
        }
    }
}

fn yes() -> bool {
    true
}

/// Everything the collector does, in one document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectPlan {
    /// Used for the project name and the bundle file name.
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default)]
    pub logs: Vec<LogSpec>,
    /// Windows event log channels to export (`wevtutil epl`).
    #[serde(default = "default_event_logs")]
    pub event_logs: Vec<String>,
    /// Only export events from the last N days.
    #[serde(default = "default_event_days")]
    pub event_log_days: u32,
    /// Windows: `msinfo32 /nfo`.
    #[serde(default)]
    pub msinfo: bool,
    /// Windows: registry keys to `reg export`, e.g. `HKLM\SOFTWARE\Contoso`.
    #[serde(default)]
    pub registry: Vec<String>,
    /// Windows: a short `typeperf` sample.
    #[serde(default)]
    pub counters: bool,
    /// Counter paths for `typeperf`.
    #[serde(default = "default_counter_paths")]
    pub counter_paths: Vec<String>,
    /// OS, host, CPU, memory, disks, uptime and allow-listed environment variables.
    #[serde(default = "yes")]
    pub environment: bool,
    /// Environment variable names to include (case-insensitive).
    #[serde(default = "default_env_allowlist")]
    pub env_allowlist: Vec<String>,
    /// macOS: `log show --last <recentLogHours>h --style json`.
    #[serde(default)]
    pub macos_unified_log: bool,
    /// Linux: `journalctl -o json --since -<recentLogHours>h`.
    #[serde(default)]
    pub linux_journal: bool,
    #[serde(default = "default_recent_hours")]
    pub recent_log_hours: u32,
    #[serde(default)]
    pub redact: RedactConfig,
    /// Upper bound for the collected (uncompressed) content; oldest logs go first.
    #[serde(default = "default_max_bundle_mb")]
    pub max_bundle_mb: u64,
    /// Timeout for each external tool.
    #[serde(default = "default_timeout")]
    pub command_timeout_secs: u64,
}

fn default_name() -> String {
    "imt-diagnostics".into()
}
fn default_event_logs() -> Vec<String> {
    vec!["System".into(), "Application".into()]
}
fn default_event_days() -> u32 {
    7
}
fn default_counter_paths() -> Vec<String> {
    vec![
        r"\Processor(_Total)\% Processor Time".into(),
        r"\Memory\Available MBytes".into(),
        r"\PhysicalDisk(_Total)\% Disk Time".into(),
    ]
}
fn default_env_allowlist() -> Vec<String> {
    [
        "PATH",
        "OS",
        "PROCESSOR_ARCHITECTURE",
        "NUMBER_OF_PROCESSORS",
        "COMPUTERNAME",
        "USERDOMAIN",
        "LANG",
        "TZ",
        "SHELL",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}
fn default_recent_hours() -> u32 {
    1
}
fn default_max_bundle_mb() -> u64 {
    500
}
fn default_timeout() -> u64 {
    120
}

impl Default for CollectPlan {
    fn default() -> Self {
        // An empty document yields every field's default.
        CollectPlan {
            name: default_name(),
            logs: Vec::new(),
            event_logs: default_event_logs(),
            event_log_days: default_event_days(),
            msinfo: false,
            registry: Vec::new(),
            counters: false,
            counter_paths: default_counter_paths(),
            environment: true,
            env_allowlist: default_env_allowlist(),
            macos_unified_log: false,
            linux_journal: false,
            recent_log_hours: default_recent_hours(),
            redact: RedactConfig::default(),
            max_bundle_mb: default_max_bundle_mb(),
            command_timeout_secs: default_timeout(),
        }
    }
}

impl CollectPlan {
    /// The documented default plan, as pretty JSON (`--print-default-plan`).
    /// It collects the environment and the System/Application event logs
    /// (Windows only) and one example log folder to adapt.
    pub fn default_json() -> String {
        let mut plan = CollectPlan::default();
        plan.logs.push(LogSpec {
            path: "/var/log/myapp".into(),
            include: default_include(),
            max_age_days: Some(7),
            max_total_mb: Some(200),
        });
        serde_json::to_string_pretty(&plan).unwrap_or_default()
    }

    /// Parses JSON (text starting with `{`) or TOML, then validates.
    pub fn parse(text: &str) -> Result<CollectPlan, PlanError> {
        let plan: CollectPlan = if text.trim_start().starts_with('{') {
            serde_json::from_str(text).map_err(|e| PlanError::Json(e.to_string()))?
        } else {
            toml::from_str(text).map_err(|e| PlanError::Toml(e.to_string()))?
        };
        plan.validate()?;
        Ok(plan)
    }

    /// Checks everything that can be checked without touching the machine.
    pub fn validate(&self) -> Result<(), PlanError> {
        let bad = |m: String| Err(PlanError::Invalid(m));
        if self.name.trim().is_empty() {
            return bad("name must not be empty".into());
        }
        if self.max_bundle_mb == 0 {
            return bad("maxBundleMb must be at least 1".into());
        }
        if self.command_timeout_secs == 0 {
            return bad("commandTimeoutSecs must be at least 1".into());
        }
        for l in &self.logs {
            if l.path.trim().is_empty() {
                return bad("a log entry has an empty path".into());
            }
            for g in &l.include {
                if let Err(e) = globset::Glob::new(g) {
                    return bad(format!("invalid glob '{g}' for {}: {e}", l.path));
                }
            }
        }
        for n in &self.event_logs {
            if n.trim().is_empty() || n.starts_with('-') || n.starts_with('/') {
                return bad(format!("invalid event log name '{n}'"));
            }
        }
        for k in &self.registry {
            let root = k.split('\\').next().unwrap_or("").to_ascii_uppercase();
            if !root.starts_with("HK") {
                return bad(format!("registry key '{k}' must start with a hive such as HKLM\\"));
            }
        }
        for p in &self.redact.patterns {
            if p.name.trim().is_empty() {
                return bad("a redaction pattern has no name".into());
            }
            if let Err(e) = imt_core::user_regex::compile(&p.regex, false) {
                return bad(format!("redaction pattern '{}': {e}", p.name));
            }
        }
        Ok(())
    }
}
