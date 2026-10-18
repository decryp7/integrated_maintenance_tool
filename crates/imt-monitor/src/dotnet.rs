//! .NET runtime counters for monitored processes (opt-in with `"dotnet": true`).
//!
//! .NET Framework processes (`clr.dll` loaded) publish the classic `.NET CLR *` performance
//! counters; IMT reads them through PDH (`dotnet_win`, Windows only) and adds them as fields to
//! the process's `Process stats` events. .NET Core / .NET 5+ (`coreclr.dll`) only offers its
//! runtime counters through EventPipe, which the monitor does not read: such processes get
//! `DotnetRuntime = "core"` and a one-time notice instead.
//!
//! Everything here is pure (instance→PID mapping, field mapping, rate math, notices); the
//! platform reads values through the [`DotnetSource`] port.
//!
//! Fields added to `Process stats` (only for .NET processes):
//!
//! | Field | Meaning |
//! |---|---|
//! | `DotnetRuntime` | `"framework"` or `"core"` |
//! | `DotnetGen0Collections`, `DotnetGen1Collections`, `DotnetGen2Collections` | GCs per generation since start |
//! | `DotnetGen0HeapBytes`, `DotnetGen1HeapBytes`, `DotnetGen2HeapBytes` | bytes per generation (gen 0: allocation budget) |
//! | `DotnetLohBytes` | Large Object Heap bytes |
//! | `DotnetHeapBytes` | bytes in all heaps |
//! | `DotnetTimeInGcPercent` | % time in GC since the last GC |
//! | `DotnetPinnedObjects` | pinned objects seen in the last GC |
//! | `DotnetExceptionsPerSec` | exceptions thrown per second (since the previous sample) |
//! | `DotnetContentionPerSec` | failed managed lock acquisitions per second |
//! | `DotnetQueueLengthPerSec` | threads per second that waited for a managed lock |
//! | `DotnetThreads`, `DotnetPhysicalThreads` | current logical / physical managed threads |
//! | `DotnetAssemblies` | assemblies currently loaded |
//!
//! Rates are computed here from the raw cumulative values per PID (not by PDH per instance
//! name), so they stay correct when instance names (`app`, `app#1`) shift as processes exit.

use crate::source::ProcSample;
use crate::tracker::{base_fields, stamp, STATS_TEMPLATE};
use anyhow::Result;
use imt_core::model::{Fields, Level, Record};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Which .NET runtime a process has loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    /// .NET Framework (`clr.dll`, or `mscorwks.dll` for 2.0-3.5): PDH counters available.
    Framework,
    /// .NET Core / .NET 5+ (`coreclr.dll`): counters need EventPipe.
    Core,
}

impl Runtime {
    pub fn as_str(self) -> &'static str {
        match self {
            Runtime::Framework => "framework",
            Runtime::Core => "core",
        }
    }
}

/// Classifies a process by its loaded module names (file names, any case). Framework wins
/// when both are present (a Framework host that also loaded CoreCLR still has PDH counters).
pub fn classify_modules<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<Runtime> {
    let mut found = None;
    for n in names {
        let n = n.rsplit(['\\', '/']).next().unwrap_or(n);
        if n.eq_ignore_ascii_case("clr.dll") || n.eq_ignore_ascii_case("mscorwks.dll") {
            return Some(Runtime::Framework);
        }
        if n.eq_ignore_ascii_case("coreclr.dll") {
            found = Some(Runtime::Core);
        }
    }
    found
}

/// How a counter's value is turned into a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A current value (count or bytes), emitted as an integer.
    Count,
    /// A percentage, rounded to one decimal.
    Percent,
    /// A cumulative raw count; the field is its rate per second between two samples.
    Rate,
}

/// The classic `.NET CLR *` counters the monitor reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClrCounter {
    ProcessId,
    Gen0Collections,
    Gen1Collections,
    Gen2Collections,
    Gen0HeapBytes,
    Gen1HeapBytes,
    Gen2HeapBytes,
    LohBytes,
    HeapBytes,
    TimeInGc,
    PinnedObjects,
    Exceptions,
    Contention,
    LogicalThreads,
    PhysicalThreads,
    QueueLength,
    Assemblies,
}

const MEMORY: &str = ".NET CLR Memory";
const EXCEPTIONS: &str = ".NET CLR Exceptions";
const LOCKS: &str = ".NET CLR LocksAndThreads";
const LOADING: &str = ".NET CLR Loading";

impl ClrCounter {
    /// Every counter; `ProcessId` first (it maps instances to PIDs).
    pub const ALL: [ClrCounter; 17] = [
        ClrCounter::ProcessId,
        ClrCounter::Gen0Collections,
        ClrCounter::Gen1Collections,
        ClrCounter::Gen2Collections,
        ClrCounter::Gen0HeapBytes,
        ClrCounter::Gen1HeapBytes,
        ClrCounter::Gen2HeapBytes,
        ClrCounter::LohBytes,
        ClrCounter::HeapBytes,
        ClrCounter::TimeInGc,
        ClrCounter::PinnedObjects,
        ClrCounter::Exceptions,
        ClrCounter::Contention,
        ClrCounter::LogicalThreads,
        ClrCounter::PhysicalThreads,
        ClrCounter::QueueLength,
        ClrCounter::Assemblies,
    ];

    /// (English performance object, English counter name).
    pub fn names(self) -> (&'static str, &'static str) {
        use ClrCounter::*;
        match self {
            ProcessId => (MEMORY, "Process ID"),
            Gen0Collections => (MEMORY, "# Gen 0 Collections"),
            Gen1Collections => (MEMORY, "# Gen 1 Collections"),
            Gen2Collections => (MEMORY, "# Gen 2 Collections"),
            Gen0HeapBytes => (MEMORY, "Gen 0 heap size"),
            Gen1HeapBytes => (MEMORY, "Gen 1 heap size"),
            Gen2HeapBytes => (MEMORY, "Gen 2 heap size"),
            LohBytes => (MEMORY, "Large Object Heap size"),
            HeapBytes => (MEMORY, "# Bytes in all Heaps"),
            TimeInGc => (MEMORY, "% Time in GC"),
            PinnedObjects => (MEMORY, "# of Pinned Objects"),
            Exceptions => (EXCEPTIONS, "# of Exceps Thrown / sec"),
            Contention => (LOCKS, "Contention Rate / sec"),
            LogicalThreads => (LOCKS, "# of current logical Threads"),
            PhysicalThreads => (LOCKS, "# of current physical Threads"),
            QueueLength => (LOCKS, "Queue Length / sec"),
            Assemblies => (LOADING, "Current Assemblies"),
        }
    }

    /// The English wildcard path, e.g. `\.NET CLR Memory(*)\Process ID`.
    pub fn wildcard_path(self) -> String {
        let (object, counter) = self.names();
        format!("\\{object}(*)\\{counter}")
    }

    pub fn kind(self) -> Kind {
        use ClrCounter::*;
        match self {
            TimeInGc => Kind::Percent,
            Exceptions | Contention | QueueLength => Kind::Rate,
            _ => Kind::Count,
        }
    }

    /// The stable field name on `Process stats` events (`None` for the PID helper).
    pub fn field(self) -> Option<&'static str> {
        use ClrCounter::*;
        Some(match self {
            ProcessId => return None,
            Gen0Collections => "DotnetGen0Collections",
            Gen1Collections => "DotnetGen1Collections",
            Gen2Collections => "DotnetGen2Collections",
            Gen0HeapBytes => "DotnetGen0HeapBytes",
            Gen1HeapBytes => "DotnetGen1HeapBytes",
            Gen2HeapBytes => "DotnetGen2HeapBytes",
            LohBytes => "DotnetLohBytes",
            HeapBytes => "DotnetHeapBytes",
            TimeInGc => "DotnetTimeInGcPercent",
            PinnedObjects => "DotnetPinnedObjects",
            Exceptions => "DotnetExceptionsPerSec",
            Contention => "DotnetContentionPerSec",
            LogicalThreads => "DotnetThreads",
            PhysicalThreads => "DotnetPhysicalThreads",
            QueueLength => "DotnetQueueLengthPerSec",
            Assemblies => "DotnetAssemblies",
        })
    }
}

/// One value of one counter for one PDH instance. For [`Kind::Rate`] counters `value` is
/// the raw cumulative count; otherwise it is the displayed value.
#[derive(Debug, Clone, PartialEq)]
pub struct ClrValue {
    pub counter: ClrCounter,
    pub instance: String,
    pub value: f64,
}

/// Port: where CLR counter values and runtime detection come from (PDH and the module list
/// on Windows; fakes in tests).
pub trait DotnetSource: Send {
    /// Reads every available counter for every CLR instance once.
    fn collect(&mut self) -> Result<Vec<ClrValue>>;
    /// The runtime `pid` has loaded; `None` when it has none or it cannot be read.
    fn runtime(&mut self, pid: u32) -> Option<Runtime>;
}

/// An opened source plus problems found while opening (e.g. a missing counter category),
/// reported once as warnings.
pub struct Opened {
    pub source: Box<dyn DotnetSource>,
    pub problems: Vec<String>,
}

/// Opens the platform's .NET counter source.
#[cfg(windows)]
pub fn open() -> Result<Opened> {
    crate::dotnet_win::open()
}

/// Opens the platform's .NET counter source.
#[cfg(not(windows))]
pub fn open() -> Result<Opened> {
    anyhow::bail!(".NET runtime counters are only available on Windows")
}

/// Gives unsuffixed duplicate instance names their PDH index (`app`, `app` → `app`, `app#1`).
/// Names that already carry a `#n` suffix are kept as they are.
pub fn disambiguate(names: &mut [String]) {
    let mut seen: HashMap<String, u32> = HashMap::new();
    for n in names.iter_mut() {
        let count = seen.entry(n.clone()).or_insert(0);
        if *count > 0 {
            *n = format!("{n}#{count}");
        }
        *count += 1;
    }
}

/// Maps CLR instance names to PIDs using the `Process ID` counter (skips `_Global_` / 0).
pub fn instance_pids(values: &[ClrValue]) -> HashMap<&str, u32> {
    values
        .iter()
        .filter(|v| v.counter == ClrCounter::ProcessId && v.value >= 1.0 && v.value <= f64::from(u32::MAX))
        .map(|v| (v.instance.as_str(), v.value as u32))
        .collect()
}

/// Groups values by PID; values of unmapped instances are dropped.
pub fn by_pid(values: &[ClrValue]) -> HashMap<u32, HashMap<ClrCounter, f64>> {
    let pids = instance_pids(values);
    let mut out: HashMap<u32, HashMap<ClrCounter, f64>> = HashMap::new();
    for v in values {
        if let Some(pid) = pids.get(v.instance.as_str()) {
            out.entry(*pid).or_default().insert(v.counter, v.value);
        }
    }
    out
}

/// Per-second rate between two cumulative samples (`(value, epoch ms)`); `None` without a
/// baseline, when time did not advance or when the count went down (counter reset).
pub fn rate(prev: Option<(f64, i64)>, value: f64, now: i64) -> Option<f64> {
    let (p, t) = prev?;
    let dt = now - t;
    (dt > 0 && value >= p).then(|| (value - p) * 1000.0 / dt as f64)
}

fn round(x: f64, digits: i32) -> Value {
    let f = 10f64.powi(digits);
    Value::from((x * f).round() / f)
}

/// Turns one counter value into a field value (`None` for rates without a baseline).
fn field_value(kind: Kind, value: f64, prev: Option<(f64, i64)>, now: i64) -> Option<Value> {
    match kind {
        Kind::Count => Some(Value::from(value.max(0.0).round() as u64)),
        Kind::Percent => Some(round(value.clamp(0.0, 100.0), 1)),
        Kind::Rate => rate(prev, value, now).map(|r| round(r, 2)),
    }
}

/// How often a process is probed for a runtime before it is considered not .NET.
const MAX_PROBES: u8 = 3;
/// Samples a Framework process may lack PDH counters before that is reported.
const MISSING_GRACE: u8 = 2;

#[derive(Default)]
struct ProcState {
    runtime: Option<Runtime>,
    probes: u8,
    missing: u8,
    notified: bool,
    prev: HashMap<ClrCounter, (f64, i64)>,
}

/// What one sample produced: fields per PID and one-time notices.
#[derive(Debug, Default)]
pub struct DotnetTick {
    pub fields: HashMap<u32, Fields>,
    pub notices: Vec<Record>,
}

/// Remembers runtimes, rate baselines and notices per monitored process. Pure: values,
/// runtime probing and time are passed in.
#[derive(Default)]
pub struct DotnetTracker {
    procs: HashMap<u32, ProcState>,
}

impl DotnetTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Processes one collection taken at `now` for the monitored `samples`.
    pub fn tick(
        &mut self,
        now: i64,
        host: &str,
        samples: &[ProcSample],
        values: &[ClrValue],
        probe: &mut dyn FnMut(u32) -> Option<Runtime>,
    ) -> DotnetTick {
        let live: HashSet<u32> = samples.iter().filter(|s| !s.access_denied).map(|s| s.pid).collect();
        self.procs.retain(|pid, _| live.contains(pid));
        let mut grouped = by_pid(values);
        let mut out = DotnetTick::default();
        for s in samples.iter().filter(|s| !s.access_denied) {
            let st = self.procs.entry(s.pid).or_default();
            if let Some(counters) = grouped.remove(&s.pid) {
                st.runtime = Some(Runtime::Framework);
                st.missing = 0;
                out.fields.insert(s.pid, framework_fields(st, &counters, now));
                continue;
            }
            if st.runtime.is_none() && st.probes < MAX_PROBES {
                st.probes += 1;
                st.runtime = probe(s.pid);
            }
            let Some(runtime) = st.runtime else { continue };
            let mut f = Fields::new();
            f.insert("DotnetRuntime".into(), runtime.as_str().into());
            out.fields.insert(s.pid, f);
            let notice = match runtime {
                Runtime::Core => Some(core_notice(now, host, s)),
                Runtime::Framework => {
                    st.missing = st.missing.saturating_add(1);
                    (st.missing >= MISSING_GRACE).then(|| framework_missing_notice(now, host, s))
                }
            };
            if let Some(n) = notice.filter(|_| !st.notified) {
                st.notified = true;
                out.notices.push(n);
            }
        }
        out
    }
}

fn framework_fields(st: &mut ProcState, counters: &HashMap<ClrCounter, f64>, now: i64) -> Fields {
    let mut f = Fields::new();
    f.insert("DotnetRuntime".into(), Runtime::Framework.as_str().into());
    for c in ClrCounter::ALL {
        let (Some(name), Some(&v)) = (c.field(), counters.get(&c)) else {
            continue;
        };
        if let Some(value) = field_value(c.kind(), v, st.prev.get(&c).copied(), now) {
            f.insert(name.into(), value);
        }
        if c.kind() == Kind::Rate {
            st.prev.insert(c, (v, now));
        }
    }
    f
}

fn notice(level: Level, now: i64, host: &str, s: &ProcSample, runtime: Runtime, message: String, template: &str) -> Record {
    let mut r = stamp(Record::new(0), now);
    r.level = level;
    r.message = message;
    r.template = Some(template.into());
    r.fields = base_fields(host);
    r.fields.insert("ProcessName".into(), s.name.as_str().into());
    r.fields.insert("ProcessId".into(), s.pid.into());
    r.fields.insert("DotnetRuntime".into(), runtime.as_str().into());
    r
}

fn core_notice(now: i64, host: &str, s: &ProcSample) -> Record {
    notice(
        Level::Information,
        now,
        host,
        s,
        Runtime::Core,
        format!(
            "Runtime counters for {} ({}) are not collected: .NET Core / .NET 5+ publishes them through EventPipe, not performance counters",
            s.name, s.pid
        ),
        "Runtime counters for {ProcessName} ({ProcessId}) are not collected: .NET Core / .NET 5+ publishes them through EventPipe, not performance counters",
    )
}

fn framework_missing_notice(now: i64, host: &str, s: &ProcSample) -> Record {
    notice(
        Level::Warning,
        now,
        host,
        s,
        Runtime::Framework,
        format!(
            "No .NET CLR performance counters for {} ({}); they may be disabled for this process or machine",
            s.name, s.pid
        ),
        "No .NET CLR performance counters for {ProcessName} ({ProcessId}); they may be disabled for this process or machine",
    )
}

/// Adds the per-PID fields to the matching `Process stats` events in `batch`.
pub fn enrich(batch: &mut [Record], fields: &HashMap<u32, Fields>) {
    for r in batch.iter_mut().filter(|r| r.template.as_deref() == Some(STATS_TEMPLATE)) {
        let pid = r
            .fields
            .get("ProcessId")
            .and_then(Value::as_u64)
            .and_then(|p| u32::try_from(p).ok());
        if let Some(extra) = pid.and_then(|p| fields.get(&p)) {
            r.fields.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(counter: ClrCounter, instance: &str, value: f64) -> ClrValue {
        ClrValue {
            counter,
            instance: instance.into(),
            value,
        }
    }

    fn proc(pid: u32, name: &str) -> ProcSample {
        ProcSample {
            pid,
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn classifies_runtimes_from_modules() {
        assert_eq!(classify_modules(["ntdll.dll", "CLR.DLL"]), Some(Runtime::Framework));
        assert_eq!(classify_modules(["C:\\x\\mscorwks.dll"]), Some(Runtime::Framework));
        assert_eq!(classify_modules(["kernel32.dll", "coreclr.dll"]), Some(Runtime::Core));
        assert_eq!(classify_modules(["coreclr.dll", "clr.dll"]), Some(Runtime::Framework));
        assert_eq!(classify_modules(["notclr.dll", "kernel32.dll"]), None);
        assert_eq!(classify_modules([]), None);
        assert_eq!((Runtime::Framework.as_str(), Runtime::Core.as_str()), ("framework", "core"));
    }

    #[test]
    fn counter_table_is_complete_and_stable() {
        let fields: Vec<_> = ClrCounter::ALL.iter().filter_map(|c| c.field()).collect();
        assert_eq!(fields.len(), 16);
        assert_eq!(fields.iter().collect::<HashSet<_>>().len(), 16, "unique field names");
        assert!(fields.iter().all(|f| f.starts_with("Dotnet")));
        assert_eq!(ClrCounter::ALL[0], ClrCounter::ProcessId);
        assert_eq!(ClrCounter::ProcessId.wildcard_path(), "\\.NET CLR Memory(*)\\Process ID");
        assert_eq!(
            ClrCounter::Exceptions.wildcard_path(),
            "\\.NET CLR Exceptions(*)\\# of Exceps Thrown / sec"
        );
        assert_eq!(ClrCounter::Assemblies.names(), (".NET CLR Loading", "Current Assemblies"));
        assert_eq!(ClrCounter::Contention.names().0, ".NET CLR LocksAndThreads");
        assert_eq!(ClrCounter::TimeInGc.kind(), Kind::Percent);
        assert_eq!(ClrCounter::QueueLength.kind(), Kind::Rate);
        assert_eq!(ClrCounter::HeapBytes.kind(), Kind::Count);
        // Every path parses as a counter path.
        for c in ClrCounter::ALL {
            assert!(crate::counters::CounterPath::parse(&c.wildcard_path()).unwrap().is_wildcard());
        }
    }

    #[test]
    fn disambiguates_duplicate_instances() {
        let mut n: Vec<String> = ["app", "svc", "app", "app", "x#1"].map(String::from).to_vec();
        disambiguate(&mut n);
        assert_eq!(n, ["app", "svc", "app#1", "app#2", "x#1"]);
    }

    #[test]
    fn maps_instances_to_pids() {
        let values = vec![
            v(ClrCounter::ProcessId, "_Global_", 0.0),
            v(ClrCounter::ProcessId, "app", 10.0),
            v(ClrCounter::ProcessId, "app#1", 11.0),
            v(ClrCounter::ProcessId, "bogus", -1.0),
            v(ClrCounter::HeapBytes, "app", 100.0),
            v(ClrCounter::HeapBytes, "app#1", 200.0),
            v(ClrCounter::HeapBytes, "_Global_", 300.0),
            v(ClrCounter::HeapBytes, "gone", 1.0),
        ];
        let map = instance_pids(&values);
        assert_eq!((map.len(), map["app"], map["app#1"]), (2, 10, 11));
        let g = by_pid(&values);
        assert_eq!(g.len(), 2);
        assert_eq!(g[&10][&ClrCounter::HeapBytes], 100.0);
        assert_eq!(g[&11][&ClrCounter::HeapBytes], 200.0);
    }

    #[test]
    fn rate_math() {
        assert_eq!(rate(None, 5.0, 1000), None);
        assert_eq!(rate(Some((5.0, 1000)), 15.0, 3000), Some(5.0));
        assert_eq!(rate(Some((5.0, 1000)), 5.0, 1500), Some(0.0));
        assert_eq!(rate(Some((5.0, 1000)), 7.0, 1000), None, "no time passed");
        assert_eq!(rate(Some((5.0, 1000)), 2.0, 2000), None, "counter reset");
        assert_eq!(field_value(Kind::Rate, 1.0, Some((0.0, 0)), 3000), Some(Value::from(0.33)));
        assert_eq!(field_value(Kind::Percent, 123.456, None, 0), Some(Value::from(100.0)));
        assert_eq!(field_value(Kind::Percent, 12.345, None, 0), Some(Value::from(12.3)));
        assert_eq!(field_value(Kind::Count, -3.0, None, 0), Some(Value::from(0u64)));
        assert_eq!(field_value(Kind::Count, 41.6, None, 0), Some(Value::from(42u64)));
    }

    fn framework_values(pid: f64, instance: &str, exceptions: f64) -> Vec<ClrValue> {
        vec![
            v(ClrCounter::ProcessId, instance, pid),
            v(ClrCounter::Gen0Collections, instance, 7.0),
            v(ClrCounter::HeapBytes, instance, 4096.0),
            v(ClrCounter::TimeInGc, instance, 2.25),
            v(ClrCounter::Exceptions, instance, exceptions),
            v(ClrCounter::LogicalThreads, instance, 4.0),
        ]
    }

    #[test]
    fn framework_fields_and_rates_follow_the_pid_when_instances_shift() {
        let mut t = DotnetTracker::new();
        let samples = [proc(10, "app"), proc(11, "app")];
        let mut never = |_| -> Option<Runtime> { panic!("Framework processes are not probed") };
        let mut values = framework_values(10.0, "app", 3.0);
        values.extend(framework_values(11.0, "app#1", 100.0));
        let first = t.tick(1000, "H", &samples, &values, &mut never);
        let f = &first.fields[&10];
        assert_eq!(f["DotnetRuntime"], "framework");
        assert_eq!(f["DotnetGen0Collections"], 7);
        assert_eq!(f["DotnetHeapBytes"], 4096);
        assert_eq!(f["DotnetTimeInGcPercent"], 2.3);
        assert_eq!(f["DotnetThreads"], 4);
        assert!(f.get("DotnetExceptionsPerSec").is_none(), "no baseline yet");
        assert!(first.notices.is_empty());
        // pid 10 exits: pid 11 becomes instance `app`; its rate still uses its own baseline.
        let second = t.tick(3000, "H", &samples[1..], &framework_values(11.0, "app", 110.0), &mut never);
        assert_eq!(second.fields.len(), 1);
        assert_eq!(second.fields[&11]["DotnetExceptionsPerSec"], 5.0);
        // pid 10 is forgotten; when the pid is reused its baseline starts fresh.
        let third = t.tick(5000, "H", &samples[..1], &framework_values(10.0, "app", 50.0), &mut never);
        assert!(third.fields[&10].get("DotnetExceptionsPerSec").is_none());
    }

    #[test]
    fn core_processes_get_a_runtime_field_and_one_notice() {
        let mut t = DotnetTracker::new();
        let samples = [proc(20, "web")];
        let mut probes = 0;
        let mut probe = |_| {
            probes += 1;
            Some(Runtime::Core)
        };
        let a = t.tick(1000, "H", &samples, &[], &mut probe);
        let b = t.tick(2000, "H", &samples, &[], &mut probe);
        assert_eq!(probes, 1, "the runtime is cached");
        assert_eq!(a.fields[&20]["DotnetRuntime"], "core");
        assert_eq!(b.fields[&20]["DotnetRuntime"], "core");
        assert_eq!(a.notices.len(), 1);
        assert!(b.notices.is_empty(), "reported once");
        let n = &a.notices[0];
        assert_eq!(n.level, Level::Information);
        assert!(n.message.contains("web (20)") && n.message.contains("EventPipe"), "{}", n.message);
        assert_eq!(
            (n.fields["ProcessId"].clone(), n.fields["DotnetRuntime"].clone()),
            (20.into(), "core".into())
        );
    }

    #[test]
    fn non_dotnet_processes_are_probed_a_few_times_then_left_alone() {
        let mut t = DotnetTracker::new();
        let samples = [proc(30, "native")];
        let mut probes = 0;
        let mut probe = |_| {
            probes += 1;
            None
        };
        for i in 0..6 {
            let r = t.tick(i * 1000, "H", &samples, &[], &mut probe);
            assert!(r.fields.is_empty() && r.notices.is_empty());
        }
        assert_eq!(probes, u32::from(MAX_PROBES));
        // Access-denied samples are never probed.
        let mut t = DotnetTracker::new();
        let denied = ProcSample {
            access_denied: true,
            ..proc(31, "x")
        };
        let mut never = |_| -> Option<Runtime> { panic!("not probed") };
        assert!(t.tick(0, "H", &[denied], &[], &mut never).fields.is_empty());
    }

    #[test]
    fn framework_without_counters_is_reported_after_a_grace_sample() {
        let mut t = DotnetTracker::new();
        let samples = [proc(40, "legacy")];
        let mut probe = |_| Some(Runtime::Framework);
        let a = t.tick(1000, "H", &samples, &[], &mut probe);
        assert_eq!(a.fields[&40]["DotnetRuntime"], "framework");
        assert!(a.notices.is_empty(), "counters may appear shortly after start");
        let b = t.tick(2000, "H", &samples, &[], &mut probe);
        assert_eq!(b.notices.len(), 1);
        assert_eq!(b.notices[0].level, Level::Warning);
        assert!(b.notices[0].message.contains("may be disabled"));
        assert!(t.tick(3000, "H", &samples, &[], &mut probe).notices.is_empty());
        // Counters showing up later are used.
        let d = t.tick(4000, "H", &samples, &framework_values(40.0, "legacy", 1.0), &mut probe);
        assert_eq!(d.fields[&40]["DotnetHeapBytes"], 4096);
    }

    #[test]
    fn enrich_adds_fields_to_stats_events_only() {
        let mut stats = Record::new(0);
        stats.template = Some(STATS_TEMPLATE.into());
        stats.fields.insert("ProcessId".into(), 5.into());
        let mut other = Record::new(0);
        other.template = Some("Application started: {ProcessName} ({ProcessId})".into());
        other.fields.insert("ProcessId".into(), 5.into());
        let mut unknown = stats.clone();
        unknown.fields.insert("ProcessId".into(), 6.into());
        let mut batch = vec![stats, other, unknown];
        let mut f = Fields::new();
        f.insert("DotnetThreads".into(), 3.into());
        enrich(&mut batch, &HashMap::from([(5, f)]));
        assert_eq!(batch[0].fields["DotnetThreads"], 3);
        assert!(batch[1].fields.get("DotnetThreads").is_none());
        assert!(batch[2].fields.get("DotnetThreads").is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_off_windows() {
        assert!(open().err().unwrap().to_string().contains("only available on Windows"));
    }
}
