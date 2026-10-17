//! Monitor settings: parsed once from JSON into typed values (parse, don't validate).

use crate::counters::CounterPath;
use anyhow::{anyhow, bail, Result};
use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

/// Shortest allowed sampling interval.
pub const MIN_INTERVAL_MS: u64 = 250;
const DEFAULT_INTERVAL_MS: u64 = 2000;

/// How per-process CPU is scaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CpuMode {
    /// 0-100 % of the whole machine (all cores).
    #[default]
    Machine,
    /// Per core, like `top`: a process using two cores shows 200 %.
    Core,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTarget {
    name: Option<String>,
    path: Option<String>,
    pid: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    targets: Vec<RawTarget>,
    interval_ms: Option<u64>,
    #[serde(default)]
    cpu: CpuMode,
    #[serde(default)]
    counters: Vec<String>,
    #[serde(default)]
    include_children: bool,
}

/// One thing to watch.
#[derive(Debug, Clone)]
pub enum Target {
    /// Case-insensitive glob on the process name (with or without `.exe`).
    Name(GlobMatcher),
    /// Case-insensitive glob on the full executable path (`/` and `\` are equivalent).
    Path(GlobMatcher),
    Pid(u32),
}

/// The set of targets; a process is monitored when any target matches.
#[derive(Debug, Clone, Default)]
pub struct Targets(Vec<Target>);

fn glob(pattern: &str) -> Result<GlobMatcher> {
    Ok(GlobBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .map_err(|e| anyhow!("{e}"))?
        .compile_matcher())
}

fn normalise_path(p: &str) -> String {
    p.replace('\\', "/")
}

impl Targets {
    pub fn new(items: Vec<Target>) -> Self {
        Targets(items)
    }

    pub fn matches(&self, pid: u32, name: &str, exe: Option<&str>) -> bool {
        let bare = name.strip_suffix(".exe").or_else(|| name.strip_suffix(".EXE")).unwrap_or(name);
        let exe = exe.map(normalise_path);
        self.0.iter().any(|t| match t {
            Target::Name(g) => g.is_match(name) || g.is_match(bare),
            Target::Path(g) => exe.as_deref().is_some_and(|e| g.is_match(e)),
            Target::Pid(p) => *p == pid,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The PIDs when every target is a PID: no need to enumerate all processes.
    pub fn only_pids(&self) -> Option<Vec<u32>> {
        self.0
            .iter()
            .map(|t| if let Target::Pid(p) = t { Some(*p) } else { None })
            .collect()
    }
}

/// Validated monitor settings.
#[derive(Debug, Clone)]
pub struct Config {
    pub targets: Targets,
    pub interval_ms: u64,
    pub cpu: CpuMode,
    pub counters: Vec<CounterPath>,
    pub include_children: bool,
}

impl Config {
    /// Parses and validates the collector's JSON settings; errors say what to fix.
    pub fn from_json(value: &Value) -> Result<Config> {
        let raw: RawConfig = serde_json::from_value(value.clone()).map_err(|e| anyhow!("invalid monitor config: {e}"))?;
        let mut items = Vec::new();
        for (i, t) in raw.targets.iter().enumerate() {
            let set = [t.name.is_some(), t.path.is_some(), t.pid.is_some()].iter().filter(|b| **b).count();
            if set != 1 {
                bail!("targets[{i}]: specify exactly one of \"name\", \"path\" or \"pid\"");
            }
            if let Some(n) = &t.name {
                if n.trim().is_empty() {
                    bail!("targets[{i}]: \"name\" must not be empty");
                }
                items.push(Target::Name(
                    glob(n.trim()).map_err(|e| anyhow!("targets[{i}]: bad name pattern \"{n}\": {e}"))?,
                ));
            } else if let Some(p) = &t.path {
                if p.trim().is_empty() {
                    bail!("targets[{i}]: \"path\" must not be empty");
                }
                let pattern = normalise_path(p.trim());
                items.push(Target::Path(
                    glob(&pattern).map_err(|e| anyhow!("targets[{i}]: bad path pattern \"{p}\": {e}"))?,
                ));
            } else if let Some(pid) = t.pid {
                if pid == 0 {
                    bail!("targets[{i}]: \"pid\" must be greater than 0");
                }
                items.push(Target::Pid(pid));
            }
        }
        let counters = raw
            .counters
            .iter()
            .enumerate()
            .map(|(i, c)| CounterPath::parse(c).map_err(|e| anyhow!("counters[{i}]: {e}")))
            .collect::<Result<Vec<_>>>()?;
        if items.is_empty() && counters.is_empty() {
            bail!("config needs at least one entry in \"targets\" (or \"counters\")");
        }
        let interval_ms = raw.interval_ms.unwrap_or(DEFAULT_INTERVAL_MS);
        if interval_ms < MIN_INTERVAL_MS {
            bail!("intervalMs must be at least {MIN_INTERVAL_MS} (got {interval_ms})");
        }
        Ok(Config {
            targets: Targets(items),
            interval_ms,
            cpu: raw.cpu,
            counters,
            include_children: raw.include_children,
        })
    }

    pub fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms)
    }
}
