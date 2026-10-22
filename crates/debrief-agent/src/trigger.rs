//! Triggers: a process metric staying above a threshold (sampled with the
//! process monitor's [`ProcessSource`]), or a new event matching a Debrief query
//! in a watched log (read incrementally with the engine's live tail).
//!
//! The decision logic ([`Threshold`]) is pure; sampling and file reading are
//! behind small ports so tests drive them with fakes.

use crate::config::{LogTrigger, Metric, ProcessTrigger};
use anyhow::Result;
use debrief_core::query::Query;
use debrief_core::workspace::OpenOptions;
use debrief_core::{Profile, Workspace};
use debrief_monitor::config::Targets;
use debrief_monitor::source::{ProcSample, ProcessSource, SysinfoSource};
use std::path::{Path, PathBuf};

/// Port: samples the processes matching `targets`.
pub trait Sampler: Send {
    fn sample(&mut self, targets: &Targets) -> Vec<ProcSample>;
    /// Logical CPUs, to scale CPU to the whole machine.
    fn cpu_count(&self) -> usize;
}

/// The real sampler (the process monitor's `sysinfo` adapter).
pub struct SystemSampler(SysinfoSource);

impl SystemSampler {
    pub fn new() -> Self {
        SystemSampler(SysinfoSource::new())
    }
}

impl Default for SystemSampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler for SystemSampler {
    fn sample(&mut self, targets: &Targets) -> Vec<ProcSample> {
        self.0.sample(targets, false)
    }
    fn cpu_count(&self) -> usize {
        self.0.cpu_count()
    }
}

/// A metric of one sample, in the unit the config uses (MB, % of the machine, count).
pub fn metric_value(s: &ProcSample, metric: Metric, cpus: usize) -> Option<f64> {
    const MB: f64 = 1024.0 * 1024.0;
    match metric {
        Metric::PrivateBytes => s.private_bytes.map(|b| b as f64 / MB),
        Metric::WorkingSet => Some(s.working_set as f64 / MB),
        Metric::VirtualBytes => Some(s.virtual_bytes as f64 / MB),
        Metric::Cpu => s.cpu_core_percent.map(|c| f64::from(c) / cpus.max(1) as f64),
        Metric::Handles => s.handles.map(f64::from),
        Metric::Threads => s.threads.map(f64::from),
    }
}

/// Threshold state machine: fires when the value stays above `above` for
/// `sustain`, then stays quiet until the value drops below the threshold
/// again (re-arm) and `cooldown` has passed.
#[derive(Debug, Clone)]
pub struct Threshold {
    above: f64,
    sustain_ms: i64,
    cooldown_ms: i64,
    since: Option<i64>,
    armed: bool,
    last_fired: Option<i64>,
}

impl Threshold {
    pub fn new(above: f64, sustain_ms: i64, cooldown_ms: i64) -> Threshold {
        Threshold {
            above,
            sustain_ms,
            cooldown_ms,
            since: None,
            armed: true,
            last_fired: None,
        }
    }

    /// Feeds one observation (`None` = nothing measured); true when it fires.
    pub fn observe(&mut self, now_ms: i64, value: Option<f64>) -> bool {
        let high = value.is_some_and(|v| v > self.above);
        if !high {
            self.since = None;
            self.armed = true;
            return false;
        }
        let since = *self.since.get_or_insert(now_ms);
        let cooled = self.last_fired.is_none_or(|t| now_ms - t >= self.cooldown_ms);
        if self.armed && cooled && now_ms - since >= self.sustain_ms {
            self.armed = false;
            self.last_fired = Some(now_ms);
            return true;
        }
        false
    }
}

/// A process threshold trigger with its sampler.
pub struct ProcessWatch {
    spec: ProcessTrigger,
    targets: Targets,
    sampler: Box<dyn Sampler>,
    state: Threshold,
    next_ms: i64,
}

impl ProcessWatch {
    pub fn new(spec: ProcessTrigger, sampler: Box<dyn Sampler>) -> Result<ProcessWatch> {
        let cfg = debrief_monitor::config::Config::from_json(&serde_json::json!({ "targets": [{ "name": spec.process }] }))?;
        let state = Threshold::new(spec.above, spec.sustain.as_millis() as i64, spec.cooldown.as_millis() as i64);
        Ok(ProcessWatch {
            targets: cfg.targets,
            spec,
            sampler,
            state,
            next_ms: i64::MIN,
        })
    }

    /// Samples when the sample interval has elapsed; returns a description when it fires.
    pub fn poll(&mut self, now_ms: i64) -> Option<String> {
        if now_ms < self.next_ms {
            return None;
        }
        self.next_ms = now_ms + self.spec.sample.as_millis() as i64;
        let cpus = self.sampler.cpu_count();
        let samples = self.sampler.sample(&self.targets);
        let worst = samples
            .iter()
            .filter_map(|s| metric_value(s, self.spec.metric, cpus).map(|v| (v, s)))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if self.state.observe(now_ms, worst.map(|w| w.0)) {
            let (v, s) = worst?;
            return Some(format!(
                "{} ({}) {:?} = {:.1}{} > {}{}",
                s.name,
                s.pid,
                self.spec.metric,
                v,
                self.spec.metric.unit(),
                self.spec.above,
                self.spec.metric.unit()
            ));
        }
        None
    }
}

/// Most new records examined per poll (a burst beyond that is still detected by count).
const MAX_FILES: usize = 512;

/// A log trigger: tails the file (or the files of a folder) from its end and
/// fires when the number of records matching the query grows.
pub struct LogWatch {
    spec: LogTrigger,
    query: Query,
    ws: Option<Workspace>,
    seen: usize,
    next_ms: i64,
    last_fired: Option<i64>,
}

impl LogWatch {
    pub fn new(spec: LogTrigger) -> LogWatch {
        let query = Query {
            ql: spec.ql.clone(),
            ..Query::default()
        };
        LogWatch {
            spec,
            query,
            ws: None,
            seen: 0,
            next_ms: i64::MIN,
            last_fired: None,
        }
    }

    fn files(path: &Path) -> Vec<PathBuf> {
        if path.is_file() {
            return vec![path.to_path_buf()];
        }
        let Ok(entries) = std::fs::read_dir(path) else { return Vec::new() };
        let mut v: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .take(MAX_FILES)
            .collect();
        v.sort();
        v
    }

    fn open(&mut self) -> Result<()> {
        let mut ws = Workspace::new(Profile::default_profile())?;
        ws.open_with(std::slice::from_ref(&self.spec.path), &OpenOptions { start_at_end: true })?;
        self.seen = ws.matching(&self.query)?.len();
        self.ws = Some(ws);
        Ok(())
    }

    /// Reads what was appended; returns the newest matching message when it fires.
    /// A missing file is not an error: it is opened once it appears.
    pub fn poll(&mut self, now_ms: i64) -> Result<Option<String>> {
        if now_ms < self.next_ms {
            return Ok(None);
        }
        self.next_ms = now_ms + self.spec.poll.as_millis() as i64;
        if self.ws.is_none() {
            if !self.spec.path.exists() {
                return Ok(None);
            }
            self.open()?;
            return Ok(None);
        }
        let Some(ws) = self.ws.as_mut() else { return Ok(None) };
        let mut changed = Self::files(&self.spec.path);
        changed.push(self.spec.path.clone());
        ws.refresh_tail(&changed)?;
        let ids = ws.matching(&self.query)?;
        let count = ids.len();
        let grew = count > self.seen;
        self.seen = count;
        if !grew {
            return Ok(None);
        }
        let cooled = self.last_fired.is_none_or(|t| now_ms - t >= self.spec.cooldown.as_millis() as i64);
        if !cooled {
            return Ok(None);
        }
        self.last_fired = Some(now_ms);
        let message = ids
            .last()
            .and_then(|id| ws.records().get(*id as usize))
            .map(|r| r.message.chars().take(200).collect::<String>())
            .unwrap_or_default();
        Ok(Some(format!("log event in {}: {message}", self.spec.path.display())))
    }
}

#[cfg(test)]
mod tests;
