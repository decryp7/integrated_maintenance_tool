//! .NET EventPipe `.nettrace` importer (format V4 and V5: .NET Core 3.0 to .NET 9,
//! as written by `dotnet-trace`, `dotnet-monitor` and `DiagnosticsClient`).
//!
//! What it reads:
//!
//! - `Microsoft-DotNETCore-SampleProfiler/ThreadSample` (event 0): one CPU sample per
//!   event on its thread with its interned stack. Metrics: Samples and TimeNs, both
//!   sampled; TimeNs per sample is the sampling interval the runtime records in the
//!   Trace object's `ExpectedCPUSamplingRate` field (nanoseconds; 1 ms by default and
//!   when the field is outside 1 µs..1 s).
//! - Symbols: `MethodLoadVerbose` (runtime 143), `MethodDCStartVerbose` /
//!   `MethodDCEndVerbose` (rundown 143/144) give code ranges → `Namespace.Name` frames
//!   (kind Managed); `ModuleLoad_V2` (152) / `ModuleDCStart_V2` / `ModuleDCEnd_V2`
//!   (rundown 153/154) give module file names. Addresses no range covers become `0x…`
//!   Native frames, counted in the profile notes.
//! - Runtime events: `GCStart`/`GCEnd` (1/2, paired by count) → GC timeline events;
//!   `ExceptionThrown_V1` (80) → Exception events and an Exceptions metric at the throw
//!   stack; `ContentionStart_V1`/`ContentionStop` (81/91, paired per thread) → LockWait
//!   events and WaitNs at the waiting stack; `MethodJittingStarted` (145) paired with
//!   `MethodLoadVerbose` → JIT events. Metrics are declared only when present.
//!
//! Timestamps are QPC ticks; nanoseconds since the trace's sync time are
//! `(ts - syncTimeQPC) * 1e9 / qpcFrequency` in 128-bit arithmetic. The start time is
//! the Trace object's UTC SYSTEMTIME. NetTrace V6 (.NET 10) is reported as
//! unsupported; gzip-wrapped files are inflated within [`Limits::max_input_bytes`].
//!
//! Sources (format and payload layouts):
//! - PerfView `src/TraceEvent/EventPipe/NetTraceFormat_v5.md` and `NetTraceFormat.md` (V6),
//!   <https://github.com/microsoft/perfview/tree/main/src/TraceEvent/EventPipe>
//! - PerfView `EventPipeEventSource.cs`, `EventCache.cs`, `StackCache.cs` (reader behaviour:
//!   header compression, stack cache flushed at sequence points, wrapping sequence numbers)
//! - dotnet/runtime `src/native/eventpipe/ep-file.c`, `ep-block.c` (writer: alignment,
//!   block sizes, `sampling_rate_in_ns` in the Trace object, stack ids restart after an SPBlock)
//! - CLR event manifest (`ClrEtwAll.man`) for the runtime/rundown payloads.

mod cursor;
mod format;
mod runtime;
#[doc(hidden)]
pub mod writer;

#[cfg(test)]
mod tests;

use crate::util;
use format::{Parsed, Rt, TraceInfo};
use imt_profile::{
    Capture, CaptureImporter, EventKind, FrameKind, FrameSpec, Limits, Measurement, MetricId, MetricInfo, Profile, ProfileBuilder,
    ProfileError, ProfileEvent, ProfileKind, Result, NONE,
};
use std::collections::HashMap;

/// Importer id and error prefix.
pub(crate) const FORMAT: &str = "nettrace";

/// Sampling interval assumed when the trace does not record a plausible one.
const DEFAULT_INTERVAL_NS: u64 = 1_000_000;

/// .NET EventPipe `.nettrace` (V4/V5) importer.
pub struct NettraceImporter;

impl CaptureImporter for NettraceImporter {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        ".NET nettrace (EventPipe)"
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        if head.starts_with(format::MAGIC) {
            100
        } else if util::is_gzip(head) && file_name.to_ascii_lowercase().contains(".nettrace") {
            90
        } else {
            0
        }
    }

    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture> {
        if bytes.len() as u64 > limits.max_input_bytes {
            return Err(ProfileError::LimitExceeded {
                what: "file size",
                limit: limits.max_input_bytes,
                hint: "open a smaller capture",
            });
        }
        let bytes = util::maybe_gunzip(bytes, limits, FORMAT)?;
        let parsed = format::parse(&bytes, limits)?;
        Ok(Capture::Profile(build(file_name, parsed, limits)?))
    }
}

/// Converts QPC timestamps to nanoseconds since the trace start.
#[derive(Debug, Clone, Copy)]
struct Clock {
    origin: i64,
    freq: i64,
}

impl Clock {
    fn ns(&self, ts: i64) -> u64 {
        if ts <= self.origin || self.freq <= 0 {
            return 0;
        }
        let ticks = (i128::from(ts) - i128::from(self.origin)) as u128;
        u64::try_from(ticks * 1_000_000_000 / self.freq as u128).unwrap_or(u64::MAX)
    }
}

/// Unix milliseconds of a UTC SYSTEMTIME, when it is a valid date.
fn system_time_ms(t: [u16; 8]) -> Option<i64> {
    let [year, month, _dow, day, hour, minute, second, ms] = t.map(i64::from);
    if !(1601..=30827).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 || ms > 999 {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + ms)
}

/// A timeline event before it is added to the builder.
struct Pending {
    kind: EventKind,
    thread: Option<u64>,
    ts: i64,
    dur_ns: u64,
    label: String,
    stack: Option<u32>,
    /// A metric value recorded as a sample at the event's stack.
    value: Option<(MetricId, u64)>,
}

/// Pairs start/stop events into timeline events (input sorted by time).
fn timeline(events: &[format::RtRec], clock: &Clock) -> Vec<Pending> {
    let mut out = Vec::new();
    let mut gcs: HashMap<u32, (i64, u32, u32)> = HashMap::new();
    let mut waits: HashMap<u64, (i64, Option<u32>, bool)> = HashMap::new();
    let mut jits: HashMap<(u64, u64), (i64, &str)> = HashMap::new();
    let dur = |from: i64, to: i64| clock.ns(to).saturating_sub(clock.ns(from));
    for e in events {
        match &e.rt {
            Rt::GcStart { count, generation, reason } => {
                gcs.insert(*count, (e.ts, *generation, *reason));
            }
            Rt::GcEnd { count } => {
                if let Some((ts, generation, reason)) = gcs.remove(count) {
                    out.push(gc_event(*count, ts, generation, reason, dur(ts, e.ts)));
                }
            }
            Rt::Exception { type_name } => out.push(Pending {
                kind: EventKind::Exception,
                thread: Some(e.thread),
                ts: e.ts,
                dur_ns: 0,
                label: type_name.clone(),
                stack: e.stack,
                value: Some((MetricId::Exceptions, 1)),
            }),
            Rt::ContentionStart { native } => {
                waits.insert(e.thread, (e.ts, e.stack, *native));
            }
            Rt::ContentionStop => {
                if let Some((ts, stack, native)) = waits.remove(&e.thread) {
                    let d = dur(ts, e.ts);
                    out.push(Pending {
                        kind: EventKind::LockWait,
                        thread: Some(e.thread),
                        ts,
                        dur_ns: d,
                        label: if native {
                            "Native lock contention"
                        } else {
                            "Monitor lock contention"
                        }
                        .to_string(),
                        stack,
                        value: Some((MetricId::WaitNs, d)),
                    });
                }
            }
            Rt::JitStart { method_id, name } => {
                jits.insert((e.thread, *method_id), (e.ts, name));
            }
            Rt::JitDone { method_id, name } => {
                let (ts, d) = match jits.remove(&(e.thread, *method_id)) {
                    Some((ts, _)) => (ts, dur(ts, e.ts)),
                    None => (e.ts, 0),
                };
                out.push(jit_event(e.thread, ts, d, name));
            }
        }
    }
    // Unfinished GCs and JITs (the trace ended first) are kept as instants.
    let mut open_gcs: Vec<_> = gcs.into_iter().collect();
    open_gcs.sort_unstable_by_key(|(count, _)| *count);
    out.extend(open_gcs.into_iter().map(|(count, (ts, g, r))| gc_event(count, ts, g, r, 0)));
    let mut open_jits: Vec<_> = jits.into_iter().collect();
    open_jits.sort_unstable_by_key(|(k, (ts, _))| (*ts, *k));
    out.extend(
        open_jits
            .into_iter()
            .map(|((thread, _), (ts, name))| jit_event(thread, ts, 0, name)),
    );
    out
}

fn gc_event(count: u32, ts: i64, generation: u32, reason: u32, dur_ns: u64) -> Pending {
    Pending {
        kind: EventKind::Gc,
        thread: None,
        ts,
        dur_ns,
        label: format!("GC #{count} gen {generation} ({})", runtime::gc_reason(reason)),
        stack: None,
        value: None,
    }
}

fn jit_event(thread: u64, ts: i64, dur_ns: u64, name: &str) -> Pending {
    Pending {
        kind: EventKind::Jit,
        thread: Some(thread),
        ts,
        dur_ns,
        label: name.to_string(),
        stack: None,
        value: None,
    }
}

/// Method code ranges, sorted by start, for address lookup.
struct Symbols {
    /// (start, end exclusive, method index).
    ranges: Vec<(u64, u64, usize)>,
}

impl Symbols {
    fn new(methods: &[runtime::MethodInfo]) -> Symbols {
        let mut ranges: Vec<(u64, u64, usize)> = methods
            .iter()
            .enumerate()
            .map(|(i, m)| (m.start, m.start.saturating_add(u64::from(m.size.max(1))), i))
            .collect();
        ranges.sort_by_key(|r| (r.0, r.2));
        ranges.dedup_by_key(|r| r.0);
        Symbols { ranges }
    }

    fn lookup(&self, addr: u64) -> Option<usize> {
        let i = self.ranges.partition_point(|r| r.0 <= addr);
        let r = self.ranges.get(i.checked_sub(1)?)?;
        (addr < r.1).then_some(r.2)
    }
}

/// Turns address stacks into builder stacks, interning frames.
struct Resolver<'p> {
    parsed: &'p Parsed,
    symbols: Symbols,
    method_frames: Vec<u32>,
    address_frames: HashMap<u64, u32>,
    stacks: Vec<u32>,
}

impl<'p> Resolver<'p> {
    fn new(parsed: &'p Parsed) -> Resolver<'p> {
        Resolver {
            parsed,
            symbols: Symbols::new(&parsed.methods),
            method_frames: vec![NONE; parsed.methods.len()],
            address_frames: HashMap::new(),
            stacks: vec![NONE; parsed.stacks.len()],
        }
    }

    fn frame(&mut self, b: &mut ProfileBuilder, addr: u64) -> Result<u32> {
        if let Some(i) = self.symbols.lookup(addr) {
            if self.method_frames[i] == NONE {
                let m = &self.parsed.methods[i];
                let module = self.parsed.modules.get(&m.module_id).map(String::as_str);
                self.method_frames[i] = b.frame(FrameSpec::new(&m.name).module(module).kind(FrameKind::Managed))?;
            }
            return Ok(self.method_frames[i]);
        }
        if let Some(&f) = self.address_frames.get(&addr) {
            return Ok(f);
        }
        let f = b.frame(FrameSpec::new(&format!("0x{addr:x}")).kind(FrameKind::Native))?;
        self.address_frames.insert(addr, f);
        Ok(f)
    }

    fn stack(&mut self, b: &mut ProfileBuilder, stack: Option<u32>) -> Result<u32> {
        let Some(s) = stack.filter(|&s| (s as usize) < self.stacks.len()) else {
            return b.empty_stack();
        };
        if self.stacks[s as usize] == NONE {
            let parsed = self.parsed;
            let mut frames = Vec::with_capacity(parsed.stacks[s as usize].len());
            for &addr in &parsed.stacks[s as usize] {
                frames.push(self.frame(b, addr)?);
            }
            self.stacks[s as usize] = b.stack_leaf_first(&frames)?;
        }
        Ok(self.stacks[s as usize])
    }
}

fn build(file_name: &str, mut parsed: Parsed, limits: &Limits) -> Result<Profile> {
    let info: TraceInfo = parsed
        .info
        .ok_or_else(|| ProfileError::malformed(FORMAT, "the trace has no Trace object"))?;
    if parsed.samples.is_empty() && parsed.events.is_empty() {
        return Err(ProfileError::malformed(
            FORMAT,
            "the trace has no CPU samples or runtime events; record it with `dotnet-trace collect --profile cpu-sampling`",
        ));
    }
    let origin = parsed.min_ts.map_or(info.sync_qpc, |m| m.min(info.sync_qpc));
    let clock = Clock {
        origin,
        freq: info.qpc_freq,
    };
    parsed.events.sort_by_key(|e| e.ts);
    let pending = timeline(&parsed.events, &clock);

    let mut metrics = Vec::new();
    if !parsed.samples.is_empty() {
        metrics.push(MetricInfo::new(MetricId::Samples, Measurement::Sampled));
        metrics.push(MetricInfo::new(MetricId::TimeNs, Measurement::Sampled));
    }
    for id in [MetricId::Exceptions, MetricId::WaitNs] {
        if pending.iter().any(|p| p.value.is_some_and(|(m, _)| m == id)) {
            metrics.push(MetricInfo::new(id, Measurement::Exact));
        }
    }
    if metrics.is_empty() {
        metrics.push(MetricInfo::new(MetricId::Samples, Measurement::Sampled));
    }
    let mut b = ProfileBuilder::new(FORMAT, ProfileKind::Cpu, &metrics, *limits)?;
    let stem = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let stem = stem.split('.').next().unwrap_or(stem);
    if stem.is_empty() {
        b.set_name(&format!("Process {}", info.process_id));
    } else {
        b.set_name(stem);
    }
    b.set_has_time(true);
    // The origin is the sync time, or an earlier first event (offset ≤ 0).
    let offset_ms = (i128::from(origin) - i128::from(info.sync_qpc)) * 1000 / i128::from(info.qpc_freq);
    let offset_ms = i64::try_from(offset_ms).unwrap_or(i64::MIN);
    b.set_started_ms(system_time_ms(info.sync_time).map(|ms| ms.saturating_add(offset_ms)));
    b.set_duration_ns(parsed.max_ts.map_or(0, |t| clock.ns(t)));

    let interval = u64::from(info.sampling_ns);
    let interval = if (1_000..=1_000_000_000).contains(&interval) {
        interval
    } else {
        DEFAULT_INTERVAL_NS
    };
    let mut resolver = Resolver::new(&parsed);
    let mut row = vec![0u64; b.metrics().len()];
    if let (Some(s), Some(t)) = (b.metric_index(MetricId::Samples), b.metric_index(MetricId::TimeNs)) {
        row[s] = 1;
        row[t] = interval;
    }
    for s in &parsed.samples {
        let thread = b.thread(Some(s.thread), "")?;
        let stack = resolver.stack(&mut b, s.stack)?;
        b.sample(thread, clock.ns(s.ts), stack, &row)?;
    }
    for p in pending {
        let thread = p.thread.map(|t| b.thread(Some(t), "")).transpose()?;
        let at = clock.ns(p.ts);
        // Events with a metric (exceptions, lock waits) record it as a sample at their stack.
        let stack = match (p.value, thread) {
            (Some((metric, v)), Some(thread)) => {
                let stack = resolver.stack(&mut b, p.stack)?;
                b.sample_with(thread, at, stack, &[(metric, v)])?;
                Some(stack)
            }
            _ => None,
        };
        b.event(ProfileEvent {
            kind: p.kind,
            thread,
            start_ns: at,
            dur_ns: p.dur_ns,
            label: p.label,
            stack,
        });
    }
    notes(&mut b, &parsed, resolver.address_frames.len());
    b.finish()
}

fn notes(b: &mut ProfileBuilder, p: &Parsed, unresolved: usize) {
    if unresolved > 0 {
        if p.methods.is_empty() {
            b.note(format!(
                "The trace has no method load or rundown events, so {unresolved} code addresses are shown as 0x… frames; \
                 record with dotnet-trace's default rundown enabled to get method names"
            ));
        } else {
            b.note(format!(
                "{unresolved} code addresses are not covered by any method load or rundown event and are shown as 0x… frames"
            ));
        }
    }
    let counts = [
        (
            p.missing_stacks,
            "events referred to stacks missing from the file and are shown without a stack",
        ),
        (p.unknown_metadata, "events with undefined metadata were skipped"),
        (p.damaged_payloads, "runtime events had damaged payloads and were skipped"),
        (p.dropped_events, "runtime events beyond the event limit were skipped"),
        (p.dropped_methods, "method or module records beyond the frame limit were ignored"),
        (p.unknown_objects, "unknown objects in the file were skipped"),
    ];
    for (n, what) in counts {
        if n > 0 {
            b.note(format!("{n} {what}"));
        }
    }
}
