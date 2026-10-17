//! The scheduler shared by the CLI and every service flavour: evaluates
//! timers and triggers each tick, starts due schedules on worker threads (a
//! schedule never overlaps itself), records the last runs in `state.json`
//! and waits for running work on shutdown.

use crate::clock::Clock;
use crate::config::{AgentConfig, TriggerSpec, When};
use crate::job::{run_schedule, JobDeps, Outcome};
use crate::run::{RunReason, RunStatus};
use crate::state::{AgentState, Loaded};
use crate::trigger::{LogWatch, ProcessWatch, Sampler};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Makes a process sampler for each process trigger.
pub type SamplerFactory = Arc<dyn Fn() -> Box<dyn Sampler> + Send + Sync>;

#[derive(Clone)]
pub struct Deps {
    pub job: JobDeps,
    pub sampler: SamplerFactory,
}

impl Deps {
    /// The real dependencies for `cfg`, logging to `log`.
    pub fn system(log: Arc<crate::log::AgentLog>, cfg: &AgentConfig) -> Deps {
        Deps {
            job: JobDeps {
                executor: Arc::new(crate::action::SystemExecutor::with_logman_timeout(cfg.logman_timeout)),
                probe: Arc::new(crate::lock::SystemProbe),
                disk: Arc::new(crate::disk::SystemDisks),
                clock: Arc::new(crate::clock::SystemClock),
                log,
            },
            sampler: Arc::new(|| Box::new(crate::trigger::SystemSampler::new())),
        }
    }
}

enum Watch {
    Process(Box<ProcessWatch>),
    Log(Box<LogWatch>),
}

pub struct Agent {
    cfg: Arc<AgentConfig>,
    deps: Deps,
    state: AgentState,
    /// Timers are due relative to this (the last run, or when the agent started).
    anchors: HashMap<String, DateTime<Utc>>,
    watches: HashMap<String, Watch>,
    running: HashMap<String, JoinHandle<Outcome>>,
    stop: Arc<AtomicBool>,
}

impl Agent {
    pub fn new(cfg: AgentConfig, deps: Deps, stop: Arc<AtomicBool>) -> Agent {
        let log = deps.job.log.clone();
        let (state, loaded) = AgentState::load(&cfg.state_dir);
        if let Loaded::Recovered(why) = loaded {
            log.warn("Agent state was unusable; starting from defaults", &[("Reason", Value::from(why))]);
        }
        let now = deps.job.clock.now();
        let mut anchors = HashMap::new();
        let mut watches = HashMap::new();
        for s in cfg.schedules.iter().filter(|s| s.enabled) {
            match &s.when {
                When::Timer(_) => {
                    let last = state.get(&s.name).last_run_ms.and_then(DateTime::from_timestamp_millis);
                    anchors.insert(s.name.clone(), last.unwrap_or(now));
                }
                When::Trigger(TriggerSpec::Process(p)) => match ProcessWatch::new(p.clone(), (deps.sampler)()) {
                    Ok(w) => {
                        watches.insert(s.name.clone(), Watch::Process(Box::new(w)));
                    }
                    Err(e) => log.error(
                        "Trigger disabled",
                        &[("Schedule", Value::from(s.name.as_str())), ("Error", Value::from(format!("{e:#}")))],
                    ),
                },
                When::Trigger(TriggerSpec::Log(l)) => {
                    watches.insert(s.name.clone(), Watch::Log(Box::new(LogWatch::new(l.clone()))));
                }
            }
        }
        Agent {
            cfg: Arc::new(cfg),
            deps,
            state,
            anchors,
            watches,
            running: HashMap::new(),
            stop,
        }
    }

    fn clock(&self) -> &dyn Clock {
        &*self.deps.job.clock
    }

    /// Schedules due now (index, reason). Polls the triggers.
    pub fn due(&mut self) -> Vec<(usize, RunReason)> {
        let now = self.clock().now();
        let mut out = Vec::new();
        let log = self.deps.job.log.clone();
        for (i, s) in self.cfg.schedules.iter().enumerate() {
            if !s.enabled || self.running.contains_key(&s.name) {
                continue;
            }
            match &s.when {
                When::Timer(t) => {
                    let anchor = self.anchors.get(&s.name).copied().unwrap_or(now);
                    if t.is_due(anchor, now, self.cfg.timezone) {
                        out.push((i, RunReason::Schedule { timer: t.describe() }));
                    }
                }
                When::Trigger(_) => {
                    let fired = match self.watches.get_mut(&s.name) {
                        Some(Watch::Process(w)) => Ok(w.poll(now.timestamp_millis())),
                        Some(Watch::Log(w)) => w.poll(now.timestamp_millis()),
                        None => Ok(None),
                    };
                    match fired {
                        Ok(Some(detail)) => out.push((i, RunReason::Trigger { detail })),
                        Ok(None) => {}
                        Err(e) => log.warn(
                            "Trigger check failed",
                            &[("Schedule", Value::from(s.name.as_str())), ("Error", Value::from(format!("{e:#}")))],
                        ),
                    }
                }
            }
        }
        out
    }

    /// Reaps finished runs and starts due ones.
    pub fn tick(&mut self) {
        self.reap(false);
        for (i, reason) in self.due() {
            self.start(i, reason);
        }
    }

    fn start(&mut self, i: usize, reason: RunReason) {
        let now = self.clock().now();
        let name = self.cfg.schedules[i].name.clone();
        self.anchors.insert(name.clone(), now);
        self.state.entry(&name).last_run_ms = Some(now.timestamp_millis());
        self.save_state();
        let (cfg, deps, stop) = (self.cfg.clone(), self.deps.job.clone(), self.stop.clone());
        let handle = std::thread::spawn(move || run_schedule(&cfg, &cfg.schedules[i], reason, &deps, &stop));
        self.running.insert(name, handle);
    }

    /// Records finished runs; with `all`, waits for every running one.
    fn reap(&mut self, all: bool) {
        let done: Vec<String> = self
            .running
            .iter()
            .filter(|(_, h)| all || h.is_finished())
            .map(|(n, _)| n.clone())
            .collect();
        for name in done {
            let Some(h) = self.running.remove(&name) else { continue };
            let outcome = h.join().unwrap_or_else(|_| Outcome::Skipped("the run crashed".into()));
            self.record(&name, &outcome);
        }
    }

    fn record(&mut self, name: &str, outcome: &Outcome) {
        if let Outcome::Ran { id, status } = outcome {
            let e = self.state.entry(name);
            e.last_status = Some(*status);
            e.last_run_id = Some(id.clone());
        }
        self.save_state();
    }

    fn save_state(&self) {
        if let Err(e) = self.state.save(&self.cfg.state_dir) {
            self.deps
                .job
                .log
                .warn("Cannot save the agent state", &[("Error", Value::from(e.to_string()))]);
        }
    }

    /// Runs until `stop` is set, then waits for running work.
    pub fn run(&mut self) {
        let log = self.deps.job.log.clone();
        log.info(
            "Agent started",
            &[
                ("Schedules", Value::from(self.cfg.schedules.iter().filter(|s| s.enabled).count())),
                ("OutputDir", Value::from(self.cfg.output_dir.to_string_lossy().into_owned())),
            ],
        );
        while !self.stop.load(Ordering::Relaxed) {
            self.tick();
            self.clock().sleep(self.cfg.tick, &self.stop);
        }
        self.reap(true);
        log.info("Agent stopped", &[]);
    }

    /// Runs the named schedule (even if disabled) or every enabled one, now,
    /// one after another. Unknown names are an error.
    pub fn run_once(&mut self, name: Option<&str>) -> Result<Vec<(String, Outcome)>, String> {
        let picked: Vec<usize> = match name {
            Some(n) => vec![self
                .cfg
                .schedules
                .iter()
                .position(|s| s.name == n)
                .ok_or_else(|| format!("there is no schedule called \"{n}\""))?],
            None => (0..self.cfg.schedules.len()).filter(|i| self.cfg.schedules[*i].enabled).collect(),
        };
        let mut out = Vec::new();
        for i in picked {
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            let s = &self.cfg.schedules[i];
            self.state.entry(&s.name).last_run_ms = Some(self.clock().now().timestamp_millis());
            let outcome = run_schedule(&self.cfg, s, RunReason::Manual, &self.deps.job, &self.stop);
            let name = s.name.clone();
            self.record(&name, &outcome);
            out.push((name, outcome));
        }
        Ok(out)
    }

    pub fn state(&self) -> &AgentState {
        &self.state
    }

    /// Exit code for `once`: 0 when every run succeeded, 1 otherwise.
    pub fn exit_code(results: &[(String, Outcome)]) -> i32 {
        let ok = results.iter().all(|(_, o)| matches!(o, Outcome::Ran { status: RunStatus::Ok, .. }));
        i32::from(!ok)
    }
}

#[cfg(test)]
mod tests;
