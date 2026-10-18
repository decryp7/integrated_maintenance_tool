//! Turns samples into events: stats, process lifecycle, restart detection, access denied.
//! Pure and clock-free (time is passed in), so scenarios are deterministic to test.

use crate::config::CpuMode;
use crate::source::ProcSample;
use imt_core::model::{Fields, Level, Record};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const SOURCE_CONTEXT: &str = "IMT.Monitor";
/// Message template of the periodic per-process `Process stats` event.
pub const STATS_TEMPLATE: &str = "Process stats {ProcessName} CPU {CpuPercent}% · WorkingSet {WorkingSetMb} MB";
/// An exit followed by a start of the same name within this window counts as a restart.
const RESTART_WINDOW_MS: i64 = 10_000;
/// Processes that die younger than this are reported as warnings.
const SHORT_LIVED_SEC: u64 = 60;
const MB: f64 = 1024.0 * 1024.0;

pub(crate) fn stamp(mut r: Record, ts: i64) -> Record {
    r.ts = Some(ts);
    r
}

pub(crate) fn base_fields(host: &str) -> Fields {
    let mut f = Fields::new();
    f.insert("MachineName".into(), host.into());
    f.insert("SourceContext".into(), SOURCE_CONTEXT.into());
    f
}

fn round1(x: f64) -> Value {
    Value::from((x * 10.0).round() / 10.0)
}

struct Live {
    name: String,
    uptime_sec: u64,
}

struct PendingExit {
    pid: u32,
    name: String,
    uptime_sec: u64,
    ts: i64,
    restarted_as: Option<u32>,
}

/// Tracks monitored processes between samples.
pub struct Tracker {
    cpu: CpuMode,
    cpu_count: f64,
    host: String,
    live: HashMap<u32, Live>,
    denied: HashSet<u32>,
    pending: Vec<PendingExit>,
    last_tick: Option<i64>,
}

impl Tracker {
    pub fn new(cpu: CpuMode, cpu_count: usize, host: &str) -> Self {
        Tracker {
            cpu,
            cpu_count: cpu_count.max(1) as f64,
            host: host.to_string(),
            live: HashMap::new(),
            denied: HashSet::new(),
            pending: Vec::new(),
            last_tick: None,
        }
    }

    /// Whether `pid` is currently monitored (used to filter per-process counters).
    pub fn is_tracked(&self, pid: u32) -> bool {
        self.live.contains_key(&pid)
    }

    fn proc_record(&self, level: Level, ts: i64, name: &str, pid: u32, message: String, template: &str) -> Record {
        let mut r = stamp(Record::new(0), ts);
        r.level = level;
        r.message = message;
        r.template = Some(template.into());
        r.fields.insert("ProcessName".into(), name.into());
        r.fields.insert("ProcessId".into(), pid.into());
        r.fields.extend(base_fields(&self.host));
        r
    }

    /// Processes one set of samples taken at `now` (epoch ms) and returns the events to write.
    pub fn tick(&mut self, now: i64, samples: &[ProcSample]) -> Vec<Record> {
        let mut out = Vec::new();
        let elapsed_sec = self.last_tick.map(|t| ((now - t) as f64 / 1000.0).max(0.001));
        let first_tick = self.last_tick.is_none();
        self.last_tick = Some(now);
        let present: HashSet<u32> = samples.iter().map(|s| s.pid).collect();

        self.denied.retain(|p| present.contains(p));
        let gone: Vec<u32> = self.live.keys().filter(|p| !present.contains(p)).copied().collect();
        for pid in gone {
            if let Some(l) = self.live.remove(&pid) {
                self.pending.push(PendingExit {
                    pid,
                    name: l.name,
                    uptime_sec: l.uptime_sec,
                    ts: now,
                    restarted_as: None,
                });
            }
        }

        for s in samples {
            if s.access_denied {
                if self.denied.insert(s.pid) {
                    let msg = format!("Access denied reading {} ({}); run the monitor elevated", s.name, s.pid);
                    let mut r = self.proc_record(
                        Level::Warning,
                        now,
                        &s.name,
                        s.pid,
                        msg,
                        "Access denied reading {ProcessName} ({ProcessId}); run the monitor elevated",
                    );
                    r.fields.insert("AccessDenied".into(), true.into());
                    out.push(r);
                }
                continue;
            }
            if !self.live.contains_key(&s.pid) {
                let restarted = !first_tick && self.match_restart(now, s);
                out.push(self.start_record(now, s, first_tick, restarted));
                self.live.insert(
                    s.pid,
                    Live {
                        name: s.name.clone(),
                        uptime_sec: s.uptime_sec,
                    },
                );
            }
            if let Some(l) = self.live.get_mut(&s.pid) {
                l.uptime_sec = s.uptime_sec;
            }
            if let Some(r) = self.stats_record(now, s, elapsed_sec) {
                out.push(r);
            }
        }
        self.flush_exits(now, false, &mut out);
        out
    }

    /// Emits exits still waiting for a possible restart (call when stopping).
    pub fn finish(&mut self, now: i64) -> Vec<Record> {
        let mut out = Vec::new();
        self.flush_exits(now, true, &mut out);
        out
    }

    /// Links a newly started process to a recent exit of the same name; true when it is a restart.
    fn match_restart(&mut self, now: i64, s: &ProcSample) -> bool {
        let found = self
            .pending
            .iter_mut()
            .find(|p| p.restarted_as.is_none() && now - p.ts <= RESTART_WINDOW_MS && p.name.eq_ignore_ascii_case(&s.name));
        found.map(|p| p.restarted_as = Some(s.pid)).is_some()
    }

    fn start_record(&self, now: i64, s: &ProcSample, first_tick: bool, restarted: bool) -> Record {
        let (msg, tpl) = if first_tick {
            (
                format!("Now monitoring {} ({}), running for {}s", s.name, s.pid, s.uptime_sec),
                "Now monitoring {ProcessName} ({ProcessId}), running for {UptimeSec}s",
            )
        } else {
            // Wording matches the default profile's start marker ("application ... started").
            (
                format!("Application started: {} ({})", s.name, s.pid),
                "Application started: {ProcessName} ({ProcessId})",
            )
        };
        let mut r = self.proc_record(Level::Information, now, &s.name, s.pid, msg, tpl);
        if let Some(e) = &s.exe {
            r.fields.insert("Exe".into(), e.as_str().into());
        }
        if first_tick {
            r.fields.insert("UptimeSec".into(), s.uptime_sec.into());
            r.fields.insert("AlreadyRunning".into(), true.into());
        }
        if restarted {
            r.fields.insert("Restarted".into(), true.into());
        }
        r
    }

    fn flush_exits(&mut self, now: i64, all: bool, out: &mut Vec<Record>) {
        let pending = std::mem::take(&mut self.pending);
        for p in pending {
            if !(all || p.restarted_as.is_some() || now - p.ts >= RESTART_WINDOW_MS) {
                self.pending.push(p);
                continue;
            }
            let short = p.uptime_sec < SHORT_LIVED_SEC;
            let level = if short || p.restarted_as.is_some() {
                Level::Warning
            } else {
                Level::Information
            };
            let msg = format!("Application exited: {} ({}) after {}s", p.name, p.pid, p.uptime_sec);
            let mut r = self.proc_record(
                level,
                p.ts,
                &p.name,
                p.pid,
                msg,
                "Application exited: {ProcessName} ({ProcessId}) after {UptimeSec}s",
            );
            r.fields.insert("UptimeSec".into(), p.uptime_sec.into());
            if let Some(n) = p.restarted_as {
                r.fields.insert("RestartedAs".into(), n.into());
            }
            out.push(r);
        }
    }

    fn stats_record(&self, now: i64, s: &ProcSample, elapsed_sec: Option<f64>) -> Option<Record> {
        let raw = f64::from(s.cpu_core_percent?);
        let cpu = match self.cpu {
            CpuMode::Core => raw,
            CpuMode::Machine => (raw / self.cpu_count).min(100.0),
        };
        let cpu = (cpu * 10.0).round() / 10.0;
        let ws = (s.working_set as f64 / MB * 10.0).round() / 10.0;
        let msg = format!("Process stats {} CPU {cpu:.1}% · WorkingSet {ws:.1} MB", s.name);
        let mut r = self.proc_record(Level::Information, now, &s.name, s.pid, msg, STATS_TEMPLATE);
        if let Some(e) = &s.exe {
            r.fields.insert("Exe".into(), e.as_str().into());
        }
        r.fields.insert("CpuPercent".into(), cpu.into());
        r.fields.insert("WorkingSetMb".into(), ws.into());
        r.fields.insert("VirtualMb".into(), round1(s.virtual_bytes as f64 / MB));
        if let Some(p) = s.private_bytes {
            r.fields.insert("PrivateMb".into(), round1(p as f64 / MB));
        }
        if let Some(t) = s.threads {
            r.fields.insert("Threads".into(), t.into());
        }
        if let Some(h) = s.handles {
            r.fields.insert("Handles".into(), h.into());
        }
        if let Some(sec) = elapsed_sec {
            if let Some(b) = s.read_bytes {
                r.fields.insert("ReadBytesPerSec".into(), ((b as f64 / sec).round() as u64).into());
            }
            if let Some(b) = s.write_bytes {
                r.fields.insert("WriteBytesPerSec".into(), ((b as f64 / sec).round() as u64).into());
            }
        }
        r.fields.insert("UptimeSec".into(), s.uptime_sec.into());
        Some(r)
    }
}
