//! `debrief-agent.toml`: the file format ([`AgentConfigFile`], also the JSON the
//! UI edits, camelCase) and the validated, typed configuration
//! ([`AgentConfig`]) the agent runs. Untrusted text is parsed once here;
//! every problem is reported with the path of the offending key so the UI can
//! show it next to the field. See `debrief-agent.example.toml` for the documented
//! format.

use crate::schedule::{parse_duration, TimeZoneChoice, Timer};
use debrief_collect::{CollectPlan, LogSpec};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Current format version. Older versions are read; newer ones are refused.
pub const CONFIG_VERSION: u32 = 1;
/// Largest config file read.
pub const MAX_CONFIG_BYTES: u64 = 1 << 20;
pub const MAX_SCHEDULES: usize = 64;
pub const MAX_ACTIONS: usize = 16;
/// Longest monitor recording or PerfMon capture.
pub const MAX_CAPTURE: Duration = Duration::from_secs(86_400);

/// One problem in a config, at a key path such as `schedules[0].cron`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub path: String,
    pub message: String,
}

impl Issue {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Issue {
        Issue {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Issue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

/// Joins issues into one message (CLI output, errors).
pub fn describe(issues: &[Issue]) -> String {
    issues.iter().map(Issue::to_string).collect::<Vec<_>>().join("\n")
}

// ---------------------------------------------------------------- file format

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentConfigFile {
    pub version: u32,
    #[serde(default)]
    pub agent: AgentSection,
    #[serde(default)]
    pub retention: RetentionSection,
    #[serde(default)]
    pub schedules: Vec<ScheduleFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSection {
    /// Where run folders are written. Relative paths are relative to the config file.
    #[serde(default)]
    pub output_dir: String,
    /// Lock files and `state.json`; default `<outputDir>/.debrief-agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<String>,
    /// Agent logs (CLEF); default `<stateDir>/logs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_dir: Option<String>,
    /// A run is skipped when the output volume has less free space.
    #[serde(default = "default_min_free_mb")]
    pub min_free_mb: u64,
    /// How often schedules are evaluated.
    #[serde(default = "default_tick_secs")]
    pub tick_secs: u64,
    #[serde(default)]
    pub timezone: TimeZoneChoice,
    /// How long one `logman` call (start, stop, query a Data Collector Set) may take, in seconds.
    #[serde(default = "default_logman_timeout_secs")]
    pub logman_timeout_secs: u64,
    #[serde(default)]
    pub log: LogSection,
}

impl Default for AgentSection {
    fn default() -> Self {
        AgentSection {
            output_dir: String::new(),
            state_dir: None,
            log_dir: None,
            min_free_mb: default_min_free_mb(),
            tick_secs: default_tick_secs(),
            timezone: TimeZoneChoice::Local,
            logman_timeout_secs: default_logman_timeout_secs(),
            log: LogSection::default(),
        }
    }
}

fn default_min_free_mb() -> u64 {
    1024
}
fn default_tick_secs() -> u64 {
    15
}
/// The tools plugin's default `logman` timeout, so the app and the agent agree.
fn default_logman_timeout_secs() -> u64 {
    debrief_plugin_tools::perfmon_control::DEFAULT_TIMEOUT.as_secs()
}
/// Bounds of the `logman` timeout (as the tools plugin's setting).
pub const LOGMAN_TIMEOUT_SECS: (u64, u64) = (5, 600);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    #[default]
    Information,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogSection {
    #[serde(default)]
    pub level: LogLevel,
    #[serde(default = "default_log_mb")]
    pub max_file_mb: u64,
    #[serde(default = "default_keep_files")]
    pub keep_files: u32,
    #[serde(default = "default_log_age")]
    pub max_age_days: u32,
}

impl Default for LogSection {
    fn default() -> Self {
        LogSection {
            level: LogLevel::Information,
            max_file_mb: default_log_mb(),
            keep_files: default_keep_files(),
            max_age_days: default_log_age(),
        }
    }
}

fn default_log_mb() -> u64 {
    10
}
fn default_keep_files() -> u32 {
    5
}
fn default_log_age() -> u32 {
    30
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetentionSection {
    /// Keep at most this many runs (newest first).
    #[serde(default = "default_max_runs", skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<u32>,
    /// Keep runs until together they use this many MB.
    #[serde(default = "default_max_total_mb", skip_serializing_if = "Option::is_none")]
    pub max_total_mb: Option<u64>,
    /// Delete runs older than this.
    #[serde(default = "default_max_age_days", skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
}

impl Default for RetentionSection {
    fn default() -> Self {
        RetentionSection {
            max_runs: default_max_runs(),
            max_total_mb: default_max_total_mb(),
            max_age_days: default_max_age_days(),
        }
    }
}

fn default_max_runs() -> Option<u32> {
    Some(50)
}
fn default_max_total_mb() -> Option<u64> {
    Some(5120)
}
fn default_max_age_days() -> Option<u32> {
    Some(30)
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduleFile {
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Exactly one of `cron`, `every` and `trigger`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<TriggerFile>,
    #[serde(default)]
    pub actions: Vec<ActionFile>,
}

/// What a process threshold looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Metric {
    /// Private bytes, MB.
    PrivateBytes,
    /// Working set, MB.
    WorkingSet,
    /// Virtual bytes, MB.
    VirtualBytes,
    /// CPU, percent of the whole machine.
    Cpu,
    Handles,
    Threads,
}

impl Metric {
    pub fn unit(self) -> &'static str {
        match self {
            Metric::PrivateBytes | Metric::WorkingSet | Metric::VirtualBytes => "MB",
            Metric::Cpu => "%",
            Metric::Handles | Metric::Threads => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum TriggerFile {
    /// Fires when `metric` of a process matching `process` stays above `above`.
    Process {
        process: String,
        metric: Metric,
        above: f64,
        #[serde(default)]
        for_secs: u64,
        #[serde(default = "default_sample_secs")]
        sample_secs: u64,
        #[serde(default = "default_cooldown")]
        cooldown_secs: u64,
    },
    /// Fires when a new event in the watched log matches the query.
    Log {
        path: String,
        ql: String,
        #[serde(default = "default_sample_secs")]
        poll_secs: u64,
        #[serde(default = "default_cooldown")]
        cooldown_secs: u64,
    },
}

fn default_sample_secs() -> u64 {
    5
}
fn default_cooldown() -> u64 {
    900
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum ActionFile {
    /// Runs the diagnostics collector (an `.debriefz` bundle in the run folder).
    Collect {
        /// A collector plan file (TOML or JSON, as `debrief-collect --plan`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan_file: Option<String>,
        /// An inline plan (same keys as a plan file).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<CollectPlan>,
    },
    /// Records the process monitor for a while.
    Monitor {
        /// Process names (globs), executable paths (globs with `/` or `\`) or PIDs.
        targets: Vec<String>,
        duration: String,
        #[serde(default = "default_interval_ms")]
        interval_ms: u64,
        #[serde(default)]
        include_children: bool,
        /// .NET Framework runtime counters (GC, heap, exceptions/s, contention) on the process stats.
        #[serde(default)]
        dotnet: bool,
    },
    /// Copies log files (newest first, within limits).
    CopyLogs {
        from: String,
        #[serde(default)]
        include: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_age_days: Option<u32>,
        #[serde(default = "default_copy_mb")]
        max_total_mb: u64,
    },
    /// Windows: starts an installed PerfMon Data Collector Set, stops it after `duration` and copies its logs.
    Perfmon { collector: String, duration: String },
}

fn default_interval_ms() -> u64 {
    1000
}
fn default_copy_mb() -> u64 {
    200
}

impl AgentConfigFile {
    /// Parses TOML. Syntax and unknown keys are reported as one issue; a
    /// newer version is refused.
    pub fn parse(text: &str) -> Result<AgentConfigFile, Vec<Issue>> {
        let file: AgentConfigFile =
            toml::from_str(text).map_err(|e| vec![Issue::new("", format!("not a valid debrief-agent.toml: {e}"))])?;
        file.check_version()?;
        Ok(file)
    }

    fn check_version(&self) -> Result<(), Vec<Issue>> {
        if self.version == 0 || self.version > CONFIG_VERSION {
            return Err(vec![Issue::new(
                "version",
                format!(
                    "format version {} is not supported by this debrief-agent (supports 1..={CONFIG_VERSION}); update debrief-agent or set version = {CONFIG_VERSION}",
                    self.version
                ),
            )]);
        }
        Ok(())
    }

    /// Reads and parses a config file (bounded size).
    pub fn load(path: &Path) -> Result<AgentConfigFile, Vec<Issue>> {
        let io = |e: std::io::Error| vec![Issue::new("", format!("cannot read {}: {e}", path.display()))];
        let len = std::fs::metadata(path).map_err(io)?.len();
        if len > MAX_CONFIG_BYTES {
            return Err(vec![Issue::new("", format!("{} is larger than 1 MB", path.display()))]);
        }
        AgentConfigFile::parse(&std::fs::read_to_string(path).map_err(io)?)
    }

    /// TOML text (comments of a hand-written file are not kept; see [`AgentConfigFile::to_toml_over`]).
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// TOML text for saving over `existing` (the file's current text): values are updated in
    /// place so the user's comments, blank lines and key order survive.
    /// Keys the file leaves out (defaults) stay out unless their value changes.
    pub fn to_toml_over(&self, existing: &str) -> Result<String, String> {
        // What the file means now; unknown when it is not a valid config (then every value is written).
        let base = match AgentConfigFile::parse(existing) {
            Ok(old) => old.to_toml()?,
            Err(_) => String::new(),
        };
        crate::toml_merge::update_preserving(existing, &base, &self.to_toml()?)
    }

    /// A small working document to start from, writing to `output_dir`.
    pub fn starter(output_dir: &str) -> AgentConfigFile {
        AgentConfigFile {
            version: CONFIG_VERSION,
            agent: AgentSection {
                output_dir: output_dir.into(),
                ..AgentSection::default()
            },
            retention: RetentionSection::default(),
            schedules: vec![ScheduleFile {
                name: "nightly".into(),
                enabled: true,
                cron: Some("0 2 * * *".into()),
                every: None,
                trigger: None,
                actions: vec![ActionFile::Collect {
                    plan_file: None,
                    plan: Some(CollectPlan::default()),
                }],
            }],
        }
    }

    /// Validates and resolves relative paths against `base` (the config's folder).
    pub fn validate(&self, base: &Path) -> Result<AgentConfig, Vec<Issue>> {
        self.check_version()?;
        let mut v = Validator { base, issues: Vec::new() };
        let cfg = v.config(self);
        if v.issues.is_empty() {
            Ok(cfg)
        } else {
            Err(v.issues)
        }
    }
}

// ---------------------------------------------------------------- typed config

/// Limits of the agent's own log.
#[derive(Debug, Clone, PartialEq)]
pub struct LogLimits {
    pub level: LogLevel,
    pub max_file_bytes: u64,
    pub keep_files: usize,
    pub max_age_days: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RetentionPolicy {
    pub max_runs: Option<usize>,
    pub max_total_bytes: Option<u64>,
    pub max_age: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessTrigger {
    pub process: String,
    pub metric: Metric,
    pub above: f64,
    pub sustain: Duration,
    pub sample: Duration,
    pub cooldown: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogTrigger {
    pub path: PathBuf,
    pub ql: String,
    pub poll: Duration,
    pub cooldown: Duration,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TriggerSpec {
    Process(ProcessTrigger),
    Log(LogTrigger),
}

#[derive(Debug, Clone, PartialEq)]
pub enum When {
    Timer(Timer),
    Trigger(TriggerSpec),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Collect(Box<CollectPlan>),
    Monitor {
        /// The process monitor's JSON settings (validated).
        settings: serde_json::Value,
        duration: Duration,
    },
    CopyLogs(LogSpec),
    Perfmon {
        collector: String,
        duration: Duration,
    },
}

impl Action {
    /// `collect`, `monitor`, `copyLogs` or `perfmon` (as in the file).
    pub fn kind(&self) -> &'static str {
        match self {
            Action::Collect(_) => "collect",
            Action::Monitor { .. } => "monitor",
            Action::CopyLogs(_) => "copyLogs",
            Action::Perfmon { .. } => "perfmon",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    pub name: String,
    pub enabled: bool,
    pub when: When,
    pub actions: Vec<Action>,
}

/// A validated configuration: every path absolute, every value in range.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentConfig {
    pub output_dir: PathBuf,
    pub state_dir: PathBuf,
    pub log_dir: PathBuf,
    pub min_free_bytes: u64,
    pub tick: Duration,
    pub timezone: TimeZoneChoice,
    /// Longest `logman` call of a PerfMon capture.
    pub logman_timeout: Duration,
    pub log: LogLimits,
    pub retention: RetentionPolicy,
    pub schedules: Vec<Schedule>,
}

impl AgentConfig {
    /// Loads and validates a config file; relative paths are relative to its folder.
    pub fn load(path: &Path) -> Result<AgentConfig, Vec<Issue>> {
        let file = AgentConfigFile::load(path)?;
        let base = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let base = std::fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
        file.validate(&base)
    }

    pub fn schedule(&self, name: &str) -> Option<&Schedule> {
        self.schedules.iter().find(|s| s.name == name)
    }
}

/// Whether a schedule name is safe as a folder/file segment.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_. ".contains(c)) && !name.starts_with('.')
}

struct Validator<'a> {
    base: &'a Path,
    issues: Vec<Issue>,
}

impl Validator<'_> {
    fn bad(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.issues.push(Issue::new(path, message));
    }

    fn path(&mut self, key: &str, text: &str) -> PathBuf {
        let t = text.trim();
        if t.is_empty() {
            self.bad(key, "a path is required");
            return PathBuf::new();
        }
        if t.contains('\0') {
            self.bad(key, "the path contains a NUL character");
            return PathBuf::new();
        }
        let p = PathBuf::from(t);
        if p.is_absolute() {
            p
        } else {
            self.base.join(p)
        }
    }

    fn range(&mut self, key: &str, value: u64, min: u64, max: u64) -> u64 {
        if value < min || value > max {
            self.bad(key, format!("must be between {min} and {max} (got {value})"));
        }
        value.clamp(min, max)
    }

    fn capture(&mut self, key: &str, text: &str) -> Duration {
        match parse_duration(text) {
            Ok(d) if d >= Duration::from_secs(1) && d <= MAX_CAPTURE => d,
            Ok(_) => {
                self.bad(key, "must be between 1s and 24h");
                Duration::from_secs(1)
            }
            Err(e) => {
                self.bad(key, e);
                Duration::from_secs(1)
            }
        }
    }

    fn config(&mut self, f: &AgentConfigFile) -> AgentConfig {
        let a = &f.agent;
        let output_dir = self.path("agent.outputDir", &a.output_dir);
        let state_dir = match &a.state_dir {
            Some(s) => self.path("agent.stateDir", s),
            None => output_dir.join(".debrief-agent"),
        };
        let log_dir = match &a.log_dir {
            Some(s) => self.path("agent.logDir", s),
            None => state_dir.join("logs"),
        };
        let min_free = self.range("agent.minFreeMb", a.min_free_mb, 0, 10_000_000);
        let tick = self.range("agent.tickSecs", a.tick_secs, 1, 3600);
        let (lo, hi) = LOGMAN_TIMEOUT_SECS;
        let logman_timeout = Duration::from_secs(self.range("agent.logmanTimeoutSecs", a.logman_timeout_secs, lo, hi));
        let log = LogLimits {
            level: a.log.level,
            max_file_bytes: self.range("agent.log.maxFileMb", a.log.max_file_mb, 1, 1024) << 20,
            keep_files: self.range("agent.log.keepFiles", u64::from(a.log.keep_files), 1, 100) as usize,
            max_age_days: self.range("agent.log.maxAgeDays", u64::from(a.log.max_age_days), 1, 3650) as u32,
        };
        let r = &f.retention;
        let retention = RetentionPolicy {
            max_runs: r
                .max_runs
                .map(|n| self.range("retention.maxRuns", u64::from(n), 1, 100_000) as usize),
            max_total_bytes: r.max_total_mb.map(|n| self.range("retention.maxTotalMb", n, 1, 100_000_000) << 20),
            max_age: r
                .max_age_days
                .map(|d| Duration::from_secs(self.range("retention.maxAgeDays", u64::from(d), 1, 36_500) * 86_400)),
        };
        if f.schedules.len() > MAX_SCHEDULES {
            self.bad("schedules", format!("at most {MAX_SCHEDULES} schedules are allowed"));
        }
        let mut names = HashSet::new();
        let schedules = f
            .schedules
            .iter()
            .take(MAX_SCHEDULES)
            .enumerate()
            .filter_map(|(i, s)| {
                let key = format!("schedules[{i}]");
                if !names.insert(s.name.to_ascii_lowercase()) {
                    self.bad(format!("{key}.name"), format!("another schedule is already called \"{}\"", s.name));
                }
                self.schedule(&key, s)
            })
            .collect();
        AgentConfig {
            output_dir,
            state_dir,
            log_dir,
            min_free_bytes: min_free << 20,
            tick: Duration::from_secs(tick),
            timezone: a.timezone,
            logman_timeout,
            log,
            retention,
            schedules,
        }
    }

    fn schedule(&mut self, key: &str, s: &ScheduleFile) -> Option<Schedule> {
        if !valid_name(&s.name) {
            self.bad(
                format!("{key}.name"),
                "use 1-64 letters, digits, spaces, '-', '_' or '.' (not starting with '.')",
            );
        }
        let set = [s.cron.is_some(), s.every.is_some(), s.trigger.is_some()]
            .iter()
            .filter(|b| **b)
            .count();
        let when = match (set, &s.cron, &s.every, &s.trigger) {
            (1, Some(c), _, _) => Timer::cron(c).map(When::Timer).map_err(|e| (format!("{key}.cron"), e)),
            (1, _, Some(e), _) => Timer::every(e).map(When::Timer).map_err(|m| (format!("{key}.every"), m)),
            (1, _, _, Some(t)) => self.trigger(&format!("{key}.trigger"), t).map(When::Trigger),
            _ => Err((key.to_string(), "set exactly one of \"cron\", \"every\" or \"trigger\"".to_string())),
        };
        if s.actions.is_empty() {
            self.bad(format!("{key}.actions"), "add at least one action");
        }
        if s.actions.len() > MAX_ACTIONS {
            self.bad(format!("{key}.actions"), format!("at most {MAX_ACTIONS} actions per schedule"));
        }
        let actions: Vec<Action> = s
            .actions
            .iter()
            .take(MAX_ACTIONS)
            .enumerate()
            .filter_map(|(i, a)| self.action(&format!("{key}.actions[{i}]"), a))
            .collect();
        match when {
            Ok(when) => Some(Schedule {
                name: s.name.clone(),
                enabled: s.enabled,
                when,
                actions,
            }),
            Err((path, message)) => {
                self.bad(path, message);
                None
            }
        }
    }

    fn trigger(&mut self, key: &str, t: &TriggerFile) -> Result<TriggerSpec, (String, String)> {
        match t {
            TriggerFile::Process {
                process,
                metric,
                above,
                for_secs,
                sample_secs,
                cooldown_secs,
            } => {
                if let Err(e) = debrief_monitor::config::Config::from_json(&serde_json::json!({ "targets": [{ "name": process }] })) {
                    return Err((format!("{key}.process"), format!("{e:#}")));
                }
                if !above.is_finite() || *above <= 0.0 {
                    return Err((format!("{key}.above"), "must be a number greater than 0".into()));
                }
                Ok(TriggerSpec::Process(ProcessTrigger {
                    process: process.trim().to_string(),
                    metric: *metric,
                    above: *above,
                    sustain: Duration::from_secs(self.range(&format!("{key}.forSecs"), *for_secs, 0, 86_400)),
                    sample: Duration::from_secs(self.range(&format!("{key}.sampleSecs"), *sample_secs, 1, 3600)),
                    cooldown: Duration::from_secs(self.range(&format!("{key}.cooldownSecs"), *cooldown_secs, 0, 7 * 86_400)),
                }))
            }
            TriggerFile::Log {
                path,
                ql,
                poll_secs,
                cooldown_secs,
            } => {
                let path = self.path(&format!("{key}.path"), path);
                let poll = Duration::from_secs(self.range(&format!("{key}.pollSecs"), *poll_secs, 1, 3600));
                let cooldown = Duration::from_secs(self.range(&format!("{key}.cooldownSecs"), *cooldown_secs, 0, 7 * 86_400));
                if ql.trim().is_empty() {
                    return Err((format!("{key}.ql"), "a query is required, e.g. level >= Error".into()));
                }
                if let Err(e) = debrief_core::ql::parse(ql) {
                    return Err((format!("{key}.ql"), e.message.to_string()));
                }
                Ok(TriggerSpec::Log(LogTrigger {
                    path,
                    ql: ql.trim().to_string(),
                    poll,
                    cooldown,
                }))
            }
        }
    }

    fn action(&mut self, key: &str, a: &ActionFile) -> Option<Action> {
        match a {
            ActionFile::Collect { plan_file, plan } => {
                let plan = match (plan_file, plan) {
                    (Some(_), Some(_)) => {
                        self.bad(key, "set either \"planFile\" or \"plan\", not both");
                        return None;
                    }
                    (Some(file), None) => {
                        let path = self.path(&format!("{key}.planFile"), file);
                        match read_plan(&path) {
                            Ok(p) => p,
                            Err(e) => {
                                self.bad(format!("{key}.planFile"), e);
                                return None;
                            }
                        }
                    }
                    (None, Some(p)) => p.clone(),
                    (None, None) => CollectPlan::default(),
                };
                if let Err(e) = plan.validate() {
                    self.bad(format!("{key}.plan"), e.to_string());
                    return None;
                }
                Some(Action::Collect(Box::new(plan)))
            }
            ActionFile::Monitor {
                targets,
                duration,
                interval_ms,
                include_children,
                dotnet,
            } => {
                let duration = self.capture(&format!("{key}.duration"), duration);
                let settings = serde_json::json!({
                    "targets": targets.iter().map(|t| monitor_target(t)).collect::<Vec<_>>(),
                    "intervalMs": interval_ms,
                    "includeChildren": include_children,
                    "dotnet": dotnet,
                });
                if let Err(e) = debrief_monitor::config::Config::from_json(&settings) {
                    self.bad(format!("{key}.targets"), format!("{e:#}"));
                    return None;
                }
                Some(Action::Monitor { settings, duration })
            }
            ActionFile::CopyLogs {
                from,
                include,
                max_age_days,
                max_total_mb,
            } => {
                let from = self.path(&format!("{key}.from"), from);
                for (i, g) in include.iter().enumerate() {
                    if let Err(e) = globset::Glob::new(g) {
                        self.bad(format!("{key}.include[{i}]"), format!("invalid glob \"{g}\": {e}"));
                    }
                }
                let mb = self.range(&format!("{key}.maxTotalMb"), *max_total_mb, 1, 100_000);
                Some(Action::CopyLogs(LogSpec {
                    path: from.to_string_lossy().into_owned(),
                    include: include.clone(),
                    max_age_days: *max_age_days,
                    max_total_mb: Some(mb),
                }))
            }
            ActionFile::Perfmon { collector, duration } => {
                let name = collector.trim();
                if name.is_empty() || name.starts_with('-') || name.chars().any(|c| c.is_control() || "\"/:*?<>|".contains(c)) {
                    self.bad(format!("{key}.collector"), "enter the name of an installed Data Collector Set");
                    return None;
                }
                let duration = self.capture(&format!("{key}.duration"), duration);
                Some(Action::Perfmon {
                    collector: name.to_string(),
                    duration,
                })
            }
        }
    }
}

/// A monitor target string: digits = PID, containing a path separator = executable path, else a name.
fn monitor_target(t: &str) -> serde_json::Value {
    let t = t.trim();
    if let Ok(pid) = t.parse::<u32>() {
        serde_json::json!({ "pid": pid })
    } else if t.contains('/') || t.contains('\\') {
        serde_json::json!({ "path": t })
    } else {
        serde_json::json!({ "name": t })
    }
}

fn read_plan(path: &Path) -> Result<CollectPlan, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .len();
    if len > MAX_CONFIG_BYTES {
        return Err(format!("{} is larger than 1 MB", path.display()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    CollectPlan::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests;
