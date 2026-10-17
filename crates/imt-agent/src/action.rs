//! Running one action into a run folder. [`ActionExecutor`] is the port the
//! job runner uses; [`SystemExecutor`] is the real adapter built on the
//! collector, the process monitor and the PerfMon controller. External tools
//! always run with timeouts (the collector plan's `commandTimeoutSecs`,
//! `logman`'s own timeout); recordings end at their duration or on shutdown.

use crate::config::Action;
use crate::run::{dir_size, ActionRecord, ActionStatus};
use imt_collect::{select_logs, Collector, NoProgress};
use imt_core::export::ClefFileSink;
use imt_core::plugin::Collector as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Contract: runs `action` writing only inside `dir`, honours `stop`, never
/// panics on bad input and reports problems in the returned record.
pub trait ActionExecutor: Send + Sync {
    fn execute(&self, action: &Action, dir: &Path, stop: &AtomicBool) -> ActionRecord;
}

/// Monitor CLEF files roll at this size; this many are kept.
const MONITOR_FILE_BYTES: u64 = 10 << 20;
const MONITOR_FILES: usize = 10;

/// The real executor.
pub struct SystemExecutor {
    /// Whether PerfMon (`logman`) is available; false off Windows.
    pub perfmon: bool,
    /// Longest `logman` call (the config's `agent.logmanTimeoutSecs`).
    pub logman_timeout: Duration,
}

impl Default for SystemExecutor {
    fn default() -> Self {
        SystemExecutor {
            perfmon: perfmon_available(),
            logman_timeout: imt_plugin_tools::perfmon_control::DEFAULT_TIMEOUT,
        }
    }
}

impl SystemExecutor {
    /// The real executor with the configured `logman` timeout.
    pub fn with_logman_timeout(logman_timeout: Duration) -> SystemExecutor {
        SystemExecutor {
            logman_timeout,
            ..SystemExecutor::default()
        }
    }

    /// The PerfMon controller over `runner` (`windows`: whether logman exists), with the configured timeout.
    fn perfmon_controller(&self, runner: imt_plugin_tools::Runner, windows: bool) -> imt_plugin_tools::perfmon_control::PerfmonController {
        imt_plugin_tools::perfmon_control::PerfmonController::with_platform(runner, windows).timeout(self.logman_timeout)
    }
}

fn perfmon_available() -> bool {
    cfg!(windows)
}

impl ActionExecutor for SystemExecutor {
    fn execute(&self, action: &Action, dir: &Path, stop: &AtomicBool) -> ActionRecord {
        let started = Instant::now();
        let mut rec = match action {
            Action::Collect(plan) => collect(plan, dir),
            Action::Monitor { settings, duration } => monitor(settings, *duration, dir, stop),
            Action::CopyLogs(spec) => copy_logs(spec, dir, stop),
            Action::Perfmon { collector, duration } if self.perfmon => {
                let c = self.perfmon_controller(std::sync::Arc::new(imt_collect::SystemRunner), cfg!(windows));
                perfmon_with(
                    &c,
                    collector,
                    *duration,
                    dir,
                    stop,
                    &imt_plugin_tools::perfmon_control::output_files,
                )
            }
            Action::Perfmon { .. } => ActionRecord::new(
                "perfmon",
                ActionStatus::Skipped,
                Some("Performance Monitor capture is only available on Windows".into()),
            ),
        };
        rec.ms = started.elapsed().as_millis() as u64;
        rec
    }
}

fn rel(dir: &Path, p: &Path) -> String {
    p.strip_prefix(dir).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

fn failed(kind: &str, e: impl std::fmt::Display) -> ActionRecord {
    ActionRecord::new(kind, ActionStatus::Failed, Some(e.to_string()))
}

fn collect(plan: &imt_collect::CollectPlan, dir: &Path) -> ActionRecord {
    let collector = Collector::new(plan.clone(), &imt_collect::SystemRunner, &NoProgress);
    match collector.run(&dir.join("collect")) {
        Ok(result) => {
            let mut rec = ActionRecord::new("collect", ActionStatus::Ok, None);
            if !result.manifest.errors.is_empty() {
                rec.message = Some(format!(
                    "{} item(s) failed; see the bundle's manifest",
                    result.manifest.errors.len()
                ));
            }
            rec.bytes = std::fs::metadata(&result.bundle).map(|m| m.len()).unwrap_or(0);
            rec.outputs.push(rel(dir, &result.bundle));
            rec
        }
        Err(e) => failed("collect", format!("{e:#}")),
    }
}

/// Sets `local` after `duration` or when `stop` is set; returns whether `stop` ended it.
fn wait(duration: Duration, stop: &AtomicBool, local: &AtomicBool) -> bool {
    let end = Instant::now() + duration;
    while !stop.load(Ordering::Relaxed) && !local.load(Ordering::Relaxed) {
        if Instant::now() >= end {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    local.store(true, Ordering::Relaxed);
    stop.load(Ordering::Relaxed)
}

fn monitor(settings: &serde_json::Value, duration: Duration, dir: &Path, stop: &AtomicBool) -> ActionRecord {
    let out = dir.join("monitor");
    let sink = match ClefFileSink::new(&out, "process-monitor", MONITOR_FILE_BYTES, MONITOR_FILES) {
        Ok(s) => s,
        Err(e) => return failed("monitor", format!("{e:#}")),
    };
    let local = AtomicBool::new(false);
    let (result, stopped) = std::thread::scope(|s| {
        let timer = s.spawn(|| wait(duration, stop, &local));
        let result = imt_monitor::ProcessMonitor.run(settings, &sink, &local);
        local.store(true, Ordering::Relaxed);
        (result, timer.join().unwrap_or(false))
    });
    match result {
        Ok(()) => {
            let status = if stopped { ActionStatus::Stopped } else { ActionStatus::Ok };
            let mut rec = ActionRecord::new("monitor", status, None);
            rec.bytes = dir_size(&out);
            rec.outputs.push("monitor".into());
            rec
        }
        Err(e) => failed("monitor", format!("{e:#}")),
    }
}

/// A relative path that stays inside the destination.
fn safe_rel(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    p.components().all(|c| matches!(c, Component::Normal(_))).then(|| p.to_path_buf())
}

fn copy_logs(spec: &imt_collect::LogSpec, dir: &Path, stop: &AtomicBool) -> ActionRecord {
    let (files, rejected) = select_logs(spec, SystemTime::now());
    let out = dir.join("logs");
    let mut rec = ActionRecord::new("copyLogs", ActionStatus::Ok, None);
    let mut problems: Vec<String> = rejected
        .iter()
        .filter(|r| !r.reason.starts_with("older than"))
        .map(|r| format!("{}: {}", r.path, r.reason))
        .collect();
    let mut copied = 0usize;
    for f in &files {
        if stop.load(Ordering::Relaxed) {
            rec.status = ActionStatus::Stopped;
            break;
        }
        let Some(rel_path) = safe_rel(&f.rel) else { continue };
        let target = out.join(rel_path);
        let made = target.parent().map_or(Ok(()), std::fs::create_dir_all);
        match made.and_then(|_| std::fs::copy(&f.path, &target)) {
            Ok(n) => {
                rec.bytes += n;
                copied += 1;
            }
            Err(e) => problems.push(format!("{}: {e}", f.path.display())),
        }
    }
    if copied > 0 {
        rec.outputs.push("logs".into());
    }
    rec.message = Some(format!(
        "{copied} file(s) copied, {} skipped",
        rejected.len() + (files.len() - copied)
    ));
    if copied == 0 && rec.status == ActionStatus::Ok {
        rec.status = if problems.is_empty() {
            ActionStatus::Skipped
        } else {
            ActionStatus::Failed
        };
    }
    if !problems.is_empty() {
        let first: Vec<String> = problems.into_iter().take(5).collect();
        rec.message = Some(format!("{}; {}", rec.message.unwrap_or_default(), first.join("; ")));
    }
    rec
}

/// The PerfMon capture with the controller injected (tests use a fake `logman`).
pub(crate) fn perfmon_with(
    c: &imt_plugin_tools::perfmon_control::PerfmonController,
    name: &str,
    duration: Duration,
    dir: &Path,
    stop: &AtomicBool,
    outputs: &dyn Fn(&imt_plugin_tools::perfmon_control::CollectorStatus) -> Vec<PathBuf>,
) -> ActionRecord {
    let since = SystemTime::now();
    if let Err(e) = c.start(name) {
        return failed("perfmon", e);
    }
    let stopped = wait(duration, stop, &AtomicBool::new(false));
    if let Err(e) = c.stop(name) {
        return failed("perfmon", format!("could not stop {name}: {e}"));
    }
    let status = match c.query(name) {
        Ok(s) => s,
        Err(e) => return failed("perfmon", e),
    };
    let out = dir.join("perfmon");
    let mut rec = ActionRecord::new("perfmon", if stopped { ActionStatus::Stopped } else { ActionStatus::Ok }, None);
    let fresh: Vec<PathBuf> = outputs(&status)
        .into_iter()
        .filter(|p| std::fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|t| t >= since))
        .collect();
    for p in fresh {
        let Some(file) = p.file_name() else { continue };
        let copied = std::fs::create_dir_all(&out).and_then(|_| std::fs::copy(&p, out.join(file)));
        match copied {
            Ok(n) => {
                rec.bytes += n;
                rec.outputs.push(format!("perfmon/{}", file.to_string_lossy()));
            }
            Err(e) => rec.message = Some(format!("cannot copy {}: {e}", p.display())),
        }
    }
    if rec.outputs.is_empty() && rec.message.is_none() {
        rec.message = Some("the collector set produced no new log file".into());
    }
    rec
}

#[cfg(test)]
mod tests;
