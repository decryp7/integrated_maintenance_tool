//! Debrief process monitor: samples processes (and, on Windows, performance counters) and
//! writes CLEF events that Debrief opens directly.
//!
//! * [`ProcessMonitor`] is a [`Collector`] (`"process-monitor"`); the tools plugin (`debrief-plugin-tools`) runs it.
//! * [`config::Config`] is the validated JSON settings, [`source::ProcessSource`] the sampling port,
//!   [`tracker::Tracker`] the pure event logic and [`cli`] the headless `debrief-monitor` tool.
//!
//! Event wording is chosen so the default profile's lifecycle markers fire:
//! `Application started: {ProcessName} ({ProcessId})` (start marker) and
//! `Application exited: {ProcessName} ({ProcessId}) after {UptimeSec}s` (stop marker).
//!
//! With `"dotnet": true`, [`dotnet`] adds .NET runtime fields (`Dotnet*`) to `Process stats`.

pub mod cli;
pub mod config;
pub mod counters;
pub mod dotnet;
#[cfg(windows)]
mod dotnet_win;
#[cfg(windows)]
mod pdh;
pub mod source;
pub mod tracker;

use anyhow::Result;
use config::Config;
use counters::{counter_records, CounterSource};
use debrief_core::export::EventSink;
use debrief_core::model::{Level, Record};
use debrief_core::plugin::Collector;
use dotnet::{DotnetSource, DotnetTracker};
use source::{ProcessSource, SysinfoSource};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracker::{base_fields, stamp, Tracker};

/// Samples the configured processes until stopped. See [`Config`] for the settings.
pub struct ProcessMonitor;

impl Collector for ProcessMonitor {
    fn id(&self) -> &str {
        "process-monitor"
    }
    fn name(&self) -> &str {
        "Process monitor"
    }
    fn run(&self, config: &serde_json::Value, sink: &dyn EventSink, stop: &AtomicBool) -> Result<()> {
        let cfg = Config::from_json(config)?;
        let mut source = SysinfoSource::new();
        let probes = Probes {
            counters: (!cfg.counters.is_empty()).then(|| counters::open(&cfg.counters)),
            dotnet: cfg.dotnet.then(dotnet::open),
        };
        run_loop(&cfg, &mut source, probes, cfg.interval(), sink, stop, &now_ms)
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn warning(host: &str, ts: i64, message: String) -> Record {
    let mut r = stamp(Record::new(0), ts);
    r.level = Level::Warning;
    r.message = message;
    r.fields = base_fields(host);
    r
}

/// Sleeps `d`, waking early when `stop` is set.
fn sleep_unless_stopped(d: Duration, stop: &AtomicBool) {
    let end = Instant::now() + d;
    while !stop.load(Ordering::Relaxed) {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::sleep(left.min(Duration::from_millis(20)));
    }
}

/// Optional measurement sources. Each is `None` when not configured, `Some(Err)` when it
/// could not be opened (reported once as a warning).
#[derive(Default)]
pub(crate) struct Probes {
    pub counters: Option<Result<Box<dyn CounterSource>>>,
    pub dotnet: Option<Result<dotnet::Opened>>,
}

/// The sampling loop, with every dependency injected.
pub(crate) fn run_loop(
    cfg: &Config,
    source: &mut dyn ProcessSource,
    probes: Probes,
    interval: Duration,
    sink: &dyn EventSink,
    stop: &AtomicBool,
    clock: &dyn Fn() -> i64,
) -> Result<()> {
    let host = source.host_name();
    let mut tracker = Tracker::new(cfg.cpu, source.cpu_count(), &host);
    let mut counter_source = match probes.counters {
        Some(Ok(c)) => Some(c),
        Some(Err(e)) => {
            sink.write(&[warning(&host, clock(), format!("Performance counters unavailable: {e}"))])?;
            None
        }
        None => None,
    };
    let mut dotnet_source: Option<Box<dyn DotnetSource>> = match probes.dotnet {
        Some(Ok(o)) => {
            let notes: Vec<Record> = o.problems.into_iter().map(|p| warning(&host, clock(), p)).collect();
            if !notes.is_empty() {
                sink.write(&notes)?;
            }
            Some(o.source)
        }
        Some(Err(e)) => {
            sink.write(&[warning(&host, clock(), format!(".NET runtime counters unavailable: {e}"))])?;
            None
        }
        None => None,
    };
    let mut dotnet_tracker = DotnetTracker::new();
    let filter_processes = !cfg.targets.is_empty();
    while !stop.load(Ordering::Relaxed) {
        let now = clock();
        let samples = source.sample(&cfg.targets, cfg.include_children);
        let mut batch = tracker.tick(now, &samples);
        if let Some(d) = dotnet_source.as_mut() {
            match d.collect() {
                Ok(values) => {
                    let tick = dotnet_tracker.tick(now, &host, &samples, &values, &mut |pid| d.runtime(pid));
                    dotnet::enrich(&mut batch, &tick.fields);
                    batch.extend(tick.notices);
                }
                Err(e) => {
                    batch.push(warning(&host, now, format!(".NET runtime counter collection failed: {e}")));
                    dotnet_source = None;
                }
            }
        }
        if let Some(c) = counter_source.as_mut() {
            match c.collect() {
                Ok(values) => batch.extend(counter_records(
                    &values,
                    &cfg.counters,
                    filter_processes,
                    &|p| tracker.is_tracked(p),
                    now,
                    &host,
                )),
                Err(e) => {
                    batch.push(warning(&host, now, format!("Performance counter collection failed: {e}")));
                    counter_source = None;
                }
            }
        }
        if !batch.is_empty() {
            sink.write(&batch)?;
        }
        sleep_unless_stopped(interval, stop);
    }
    let rest = tracker.finish(clock());
    if !rest.is_empty() {
        sink.write(&rest)?;
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod dotnet_win_tests;
#[cfg(test)]
mod tests;
