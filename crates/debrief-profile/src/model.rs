//! The common profile model every importer produces and every view reads.
//!
//! A [`Profile`] holds interned [`Frame`]s, a prefix tree of stacks (each
//! node is a frame plus its parent, so a stack is one id), threads, the
//! metrics the capture measured, weighted samples (columnar, sorted by time)
//! and timeline events. Build one with [`crate::ProfileBuilder`].

use serde::{Deserialize, Serialize};

/// Sentinel for "no parent" / "no node".
pub const NONE: u32 = u32::MAX;

/// What a profile mostly measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProfileKind {
    /// CPU samples.
    Cpu,
    /// Allocations (count, bytes).
    Alloc,
    /// Wall-clock time (instrumented spans, traces).
    Wall,
    /// Several of the above.
    Mixed,
}

/// Where a frame's code runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FrameKind {
    /// JIT-compiled or interpreted code of a managed runtime (.NET, JVM, JavaScript).
    Managed,
    /// Native user-mode code.
    Native,
    /// Operating system kernel.
    Kernel,
    #[default]
    Unknown,
}

/// One function (interned): identity is function + module + file + line + kind.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub function: String,
    pub module: Option<String>,
    pub file: Option<String>,
    /// The function's line (start line or the sampled line, as the format gives it).
    pub line: Option<u32>,
    pub kind: FrameKind,
    /// Framework / system code (collapsed by "just my code").
    pub is_framework: bool,
}

/// What a metric counts. The JSON name is camelCase (`timeNs`, `allocBytes`, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetricId {
    /// Number of samples (or observations).
    Samples,
    /// CPU or wall time in nanoseconds.
    TimeNs,
    /// Number of calls (entries into the function); not inclusive.
    Calls,
    /// Allocated objects.
    AllocCount,
    /// Allocated bytes.
    AllocBytes,
    /// Exceptions thrown.
    Exceptions,
    /// Time spent waiting (locks, I/O, sleeps) in nanoseconds.
    WaitNs,
    /// Longest single call in nanoseconds; not inclusive, aggregated by maximum.
    MaxCallNs,
    /// Bytes allocated during the capture and not freed by its end (native heap growth by stack).
    LiveBytes,
}

/// Unit of a metric's values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Unit {
    Count,
    Nanoseconds,
    Bytes,
}

impl MetricId {
    pub const ALL: [MetricId; 9] = [
        MetricId::Samples,
        MetricId::TimeNs,
        MetricId::Calls,
        MetricId::AllocCount,
        MetricId::AllocBytes,
        MetricId::Exceptions,
        MetricId::WaitNs,
        MetricId::MaxCallNs,
        MetricId::LiveBytes,
    ];

    /// A caller's total includes its callees' values (time, samples, allocations).
    /// Calls and the longest call are per node only.
    pub fn inclusive(self) -> bool {
        !matches!(self, MetricId::Calls | MetricId::MaxCallNs)
    }

    /// Values are combined by maximum instead of sum.
    pub fn is_max(self) -> bool {
        self == MetricId::MaxCallNs
    }

    pub fn unit(self) -> Unit {
        match self {
            MetricId::TimeNs | MetricId::WaitNs | MetricId::MaxCallNs => Unit::Nanoseconds,
            MetricId::AllocBytes | MetricId::LiveBytes => Unit::Bytes,
            _ => Unit::Count,
        }
    }

    /// Column label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            MetricId::Samples => "Samples",
            MetricId::TimeNs => "Time",
            MetricId::Calls => "Calls",
            MetricId::AllocCount => "Allocations",
            MetricId::AllocBytes => "Allocated bytes",
            MetricId::Exceptions => "Exceptions",
            MetricId::WaitNs => "Wait time",
            MetricId::MaxCallNs => "Max per call",
            MetricId::LiveBytes => "Unfreed bytes",
        }
    }

    /// Combines `v` into `acc` according to the metric's fold.
    #[inline]
    pub fn fold(self, acc: &mut u64, v: u64) {
        if self.is_max() {
            *acc = (*acc).max(v);
        } else {
            *acc = acc.saturating_add(v);
        }
    }
}

/// How a metric's values were obtained, so the UI can label estimates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Measurement {
    /// Statistical: values are estimates from periodic samples.
    Sampled,
    /// Exact: every event was recorded (instrumentation, tracing).
    Exact,
}

/// A metric present in a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricInfo {
    pub id: MetricId,
    pub measurement: Measurement,
    pub unit: Unit,
    pub label: &'static str,
}

impl MetricInfo {
    pub fn new(id: MetricId, measurement: Measurement) -> MetricInfo {
        MetricInfo {
            id,
            measurement,
            unit: id.unit(),
            label: id.label(),
        }
    }
}

/// A thread (or other sample lane: a goroutine, a speedscope profile).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    /// Index in the profile.
    pub id: u32,
    /// The operating system's id, when known.
    pub os_id: Option<u64>,
    pub name: String,
}

/// Kinds of timeline events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventKind {
    /// Garbage collection (a pause when it has a duration).
    Gc,
    /// Just-in-time compilation of a method.
    Jit,
    /// An exception was thrown.
    Exception,
    /// A thread waited for a lock.
    LockWait,
    /// A thread was switched out: blocked (I/O, sleep, wait handle) or ready but preempted (ETW context switches).
    Wait,
    /// Anything else worth a mark (process start, user markers).
    Marker,
}

/// A timeline event (exact, from the runtime or tracer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileEvent {
    pub kind: EventKind,
    pub thread: Option<u32>,
    /// Nanoseconds since the profile's start.
    pub start_ns: u64,
    /// 0 for an instant.
    pub dur_ns: u64,
    pub label: String,
    /// The stack at the event, when recorded.
    #[serde(skip)]
    pub stack: Option<u32>,
}

/// A node of the stack prefix tree: `frame` called from the stack `parent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackNode {
    pub parent: u32,
    pub frame: u32,
}

/// Samples, columnar and sorted by time (when the profile has timestamps).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Samples {
    pub stack: Vec<u32>,
    pub thread: Vec<u32>,
    /// Nanoseconds since the profile's start (0 for formats without time).
    pub time: Vec<u64>,
    /// `metrics.len()` values per sample, in the profile's metric order.
    pub values: Vec<u64>,
}

impl Samples {
    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

/// An imported profile. Immutable once built.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub kind: ProfileKind,
    /// The importer (format) it came from, e.g. `speedscope`.
    pub source: String,
    /// Wall-clock start (Unix ms), when the format records it.
    pub started_ms: Option<i64>,
    pub duration_ns: u64,
    /// Samples carry real timestamps (thread timeline and time ranges work).
    pub has_time: bool,
    pub frames: Vec<Frame>,
    pub stacks: Vec<StackNode>,
    pub threads: Vec<Thread>,
    pub metrics: Vec<MetricInfo>,
    pub samples: Samples,
    pub events: Vec<ProfileEvent>,
    /// Things the user should know (truncated stacks, unresolved addresses, skipped records).
    pub notes: Vec<String>,
}

impl Profile {
    /// Position of a metric in the per-sample values.
    pub fn metric_index(&self, id: MetricId) -> Option<usize> {
        self.metrics.iter().position(|m| m.id == id)
    }

    /// The metric views use when the request names none: time, else samples, else the first.
    pub fn default_metric(&self) -> MetricId {
        [MetricId::TimeNs, MetricId::Samples]
            .into_iter()
            .find(|m| self.metric_index(*m).is_some())
            .or_else(|| self.metrics.first().map(|m| m.id))
            .unwrap_or(MetricId::Samples)
    }

    /// Frames of a stack, root first.
    pub fn stack_frames(&self, mut stack: u32) -> Vec<u32> {
        let mut out = Vec::new();
        while stack != NONE {
            let Some(node) = self.stacks.get(stack as usize) else { break };
            out.push(node.frame);
            stack = node.parent;
        }
        out.reverse();
        out
    }

    /// Sample index range whose time lies in `[from, to]` (all samples without time).
    pub fn sample_range(&self, from: u64, to: u64) -> std::ops::Range<usize> {
        if !self.has_time {
            return 0..self.samples.len();
        }
        let t = &self.samples.time;
        let lo = t.partition_point(|&x| x < from);
        let hi = t.partition_point(|&x| x <= to);
        lo..hi.max(lo)
    }

    /// Wall-clock end (Unix ms), when the start is known.
    pub fn ended_ms(&self) -> Option<i64> {
        self.started_ms
            .map(|s| s.saturating_add(i64::try_from(self.duration_ns / 1_000_000).unwrap_or(i64::MAX)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_properties() {
        assert!(MetricId::TimeNs.inclusive());
        assert!(!MetricId::Calls.inclusive());
        assert!(!MetricId::MaxCallNs.inclusive());
        assert_eq!(MetricId::AllocBytes.unit(), Unit::Bytes);
        assert_eq!(MetricId::LiveBytes.unit(), Unit::Bytes);
        assert!(MetricId::LiveBytes.inclusive());
        assert_eq!(MetricId::WaitNs.unit(), Unit::Nanoseconds);
        assert_eq!(MetricId::Exceptions.unit(), Unit::Count);
        let mut acc = 5;
        MetricId::MaxCallNs.fold(&mut acc, 3);
        assert_eq!(acc, 5);
        MetricId::Samples.fold(&mut acc, u64::MAX);
        assert_eq!(acc, u64::MAX, "saturates");
        for m in MetricId::ALL {
            assert!(!m.label().is_empty());
            assert_eq!(MetricInfo::new(m, Measurement::Exact).unit, m.unit());
        }
        assert_eq!(serde_json::to_string(&MetricId::AllocBytes).unwrap(), "\"allocBytes\"");
    }
}
