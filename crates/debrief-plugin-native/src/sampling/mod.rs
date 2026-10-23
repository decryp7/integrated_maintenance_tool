//! Suspend-and-sample (ADR 0022 §2): the samples of one capture ([`SampleSet`], pure) and their NetTrace form
//! ([`write_nettrace`]). The Windows sampler that fills a set is in `live`.
//!
//! A sample is a thread's stack at one tick, marked [`State::Running`] when the thread used CPU since its previous
//! sample and [`State::Waiting`] otherwise. Running samples become CPU samples; waiting ones become `ThreadWait`
//! events of one interval at their stack, with a wait reason read from the leaf frames' names ([`wait_reason`]).

#[cfg(windows)]
pub mod live;

use debrief_etw::convert::{ext_name, ext_type, SAMPLE};
use debrief_etw::nettrace::{system_time, NetTraceWriter, TraceHeader};
use debrief_plugin_profiling::nettrace::ext::{self, ImageLoad, NativeSymbol};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};

/// Deepest stack kept per sample.
pub const MAX_DEPTH: usize = 256;
/// Most distinct stacks kept (further new stacks are dropped with their samples).
pub const MAX_STACKS: usize = 500_000;
/// Wait reason of a wait the leaf frames do not explain ("Blocked (other wait)").
pub const UNKNOWN_WAIT: u8 = 255;

/// What a thread was doing at a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Waiting,
}

/// One sample: a thread's stack (an index into the set's stacks) at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleRec {
    pub thread: u32,
    pub ts: i64,
    pub stack: u32,
    pub state: State,
}

/// The samples of one capture, with what the NetTrace header and the symbolisation need.
#[derive(Debug, Clone)]
pub struct SampleSet {
    pub process_id: u32,
    /// The target's pointer size (4 or 8).
    pub pointer_size: u32,
    pub start_unix_ms: i64,
    /// The clock at the start, and its frequency (QPC on Windows).
    pub start_ticks: i64,
    pub ticks_per_second: i64,
    /// The sampling interval (per thread).
    pub interval_ns: u64,
    stacks: Vec<Vec<u64>>,
    index: HashMap<Vec<u64>, u32>,
    pub samples: Vec<SampleRec>,
    pub thread_names: BTreeMap<u32, String>,
    pub images: Vec<ImageLoad>,
    max_samples: usize,
    /// Samples not kept (limits).
    pub dropped: u64,
}

impl SampleSet {
    pub fn new(
        process_id: u32,
        pointer_size: u32,
        start_unix_ms: i64,
        start_ticks: i64,
        ticks_per_second: i64,
        interval_ns: u64,
        max_samples: usize,
    ) -> SampleSet {
        SampleSet {
            process_id,
            pointer_size,
            start_unix_ms,
            start_ticks,
            ticks_per_second: ticks_per_second.max(1),
            interval_ns,
            stacks: Vec::new(),
            index: HashMap::new(),
            samples: Vec::new(),
            thread_names: BTreeMap::new(),
            images: Vec::new(),
            max_samples,
            dropped: 0,
        }
    }

    /// Adds a sample (frames leaf first, cut to [`MAX_DEPTH`]); returns `false` when a limit dropped it.
    pub fn push(&mut self, thread: u32, ts: i64, frames: &[u64], state: State) -> bool {
        if self.samples.len() >= self.max_samples {
            self.dropped += 1;
            return false;
        }
        let frames = &frames[..frames.len().min(MAX_DEPTH)];
        let stack = match self.index.get(frames) {
            Some(&i) => i,
            None if self.stacks.len() < MAX_STACKS => {
                let i = self.stacks.len() as u32;
                self.stacks.push(frames.to_vec());
                self.index.insert(frames.to_vec(), i);
                i
            }
            None => {
                self.dropped += 1;
                return false;
            }
        };
        self.samples.push(SampleRec { thread, ts, stack, state });
        true
    }

    /// Whether no more samples fit.
    pub fn is_full(&self) -> bool {
        self.samples.len() >= self.max_samples
    }

    /// The stack of a sample, leaf first.
    pub fn stack(&self, i: u32) -> &[u64] {
        self.stacks.get(i as usize).map(Vec::as_slice).unwrap_or_default()
    }

    /// The distinct code addresses of all stacks, sorted.
    pub fn addresses(&self) -> Vec<u64> {
        let mut v: Vec<u64> = self.stacks.iter().flatten().copied().collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// The `KWAIT_REASON` a waiting stack most likely means, from its leaf-first function names (the first few
/// frames are looked at): sleeps, locks, I/O completion queues, object waits; [`UNKNOWN_WAIT`] otherwise.
pub fn wait_reason(names: &[&str]) -> u8 {
    const RULES: &[(&[&str], u8)] = &[
        (&["ntdelayexecution", "sleepex"], 4),
        (
            &[
                "rtlacquiresrwlock",
                "rtlpwaitoncriticalsection",
                "rtlpentercriticalsection",
                "rtlentercriticalsection",
                "rtlpwaitonaddress",
                "waitonaddress",
                "ntwaitforalertbythreadid",
                "ntwaitforkeyedevent",
            ],
            21,
        ),
        (
            &["ntremoveiocompletion", "getqueuedcompletionstatus", "ntwaitforworkviaworkerfactory"],
            15,
        ),
        (
            &[
                "ntwaitforsingleobject",
                "ntwaitformultipleobjects",
                "waitforsingleobject",
                "waitformultipleobjects",
                "ntusergetmessage",
                "ntusermsgwaitformultipleobjects",
                "ntreadfile",
                "ntwritefile",
                "ntdeviceiocontrolfile",
            ],
            6,
        ),
    ];
    let lower: Vec<String> = names.iter().take(4).map(|n| n.to_ascii_lowercase()).collect();
    RULES
        .iter()
        .find(|(keys, _)| lower.iter().any(|n| keys.iter().any(|k| n.contains(k))))
        .map_or(UNKNOWN_WAIT, |(_, r)| *r)
}

/// What [`write_nettrace`] wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WriteStats {
    pub samples: u64,
    pub waits: u64,
    pub symbols: u64,
}

/// Writes `set` with `symbols` as NetTrace: images, thread names and symbols, then CPU samples and waits.
pub fn write_nettrace<W: Write>(set: &SampleSet, symbols: &[NativeSymbol], out: W) -> io::Result<(W, WriteStats)> {
    let header = TraceHeader {
        sync_time: system_time(set.start_unix_ms).unwrap_or([1970, 1, 4, 1, 0, 0, 0, 0]),
        sync_ticks: set.start_ticks,
        ticks_per_second: set.ticks_per_second,
        pointer_size: set.pointer_size,
        process_id: set.process_id,
        processors: std::thread::available_parallelism().map_or(1, |n| n.get() as u32),
        sampling_ns: u32::try_from(set.interval_ns).unwrap_or(u32::MAX),
    };
    let mut w = NetTraceWriter::new(out, &header)?;
    let mut stats = WriteStats::default();
    let at = set.start_ticks;
    for i in &set.images {
        w.event(ext_type(ext::ids::IMAGE_LOAD, "ImageLoad"), 0, at, &[], &ext::image_load(i))?;
    }
    for (tid, name) in &set.thread_names {
        w.event(
            ext_type(ext::ids::THREAD_NAME, "ThreadName"),
            u64::from(*tid),
            at,
            &[],
            &ext::thread_name(name),
        )?;
    }
    let names: HashMap<u64, &str> = symbols.iter().map(|s| (s.address, s.function.as_str())).collect();
    for s in symbols {
        w.event(
            ext_type(ext::ids::NATIVE_SYMBOL, ext_name(ext::ids::NATIVE_SYMBOL)),
            0,
            at,
            &[],
            &ext::native_symbol(s),
        )?;
        stats.symbols += 1;
    }
    let interval_ticks = u64::try_from(i128::from(set.ticks_per_second) * i128::from(set.interval_ns) / 1_000_000_000).unwrap_or(0);
    let mut reasons: HashMap<u32, u8> = HashMap::new();
    for s in &set.samples {
        let stack = set.stack(s.stack);
        match s.state {
            State::Running => {
                w.event(SAMPLE, u64::from(s.thread), s.ts, stack, &1u32.to_le_bytes())?;
                stats.samples += 1;
            }
            State::Waiting => {
                let reason = *reasons.entry(s.stack).or_insert_with(|| {
                    let leaf: Vec<&str> = stack.iter().take(4).filter_map(|a| names.get(a).copied()).collect();
                    wait_reason(&leaf)
                });
                let payload = ext::thread_wait(&ext::ThreadWait {
                    duration: interval_ticks,
                    reason,
                    readied_by: 0,
                });
                w.event(
                    ext_type(ext::ids::THREAD_WAIT, "ThreadWait"),
                    u64::from(s.thread),
                    s.ts,
                    stack,
                    &payload,
                )?;
                stats.waits += 1;
            }
        }
    }
    Ok((w.finish()?, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_plugin_profiling::nettrace::NettraceImporter;
    use debrief_profile::{Capture, CaptureImporter, EventKind, Limits, MetricId};

    const EXE: u64 = 0x7ff6_0000_0000;

    fn set() -> SampleSet {
        let mut s = SampleSet::new(42, 8, 1_792_000_000_000, 1_000, 10_000_000, 10_000_000, 100);
        s.images.push(ImageLoad {
            base: EXE,
            size: 0x10_000,
            path: r"C:\app\busy.exe".into(),
        });
        s.thread_names.insert(7, "worker".into());
        for i in 0..30 {
            s.push(7, 1_000 + i * 100_000, &[EXE + 0x10, EXE + 0x20], State::Running);
        }
        for i in 0..10 {
            s.push(8, 1_000 + i * 100_000, &[EXE + 0x900, EXE + 0x30, EXE + 0x20], State::Waiting);
        }
        s
    }

    fn sym(address: u64, function: &str) -> NativeSymbol {
        NativeSymbol {
            address,
            start: address & !0xff,
            line: 9,
            function: function.into(),
            file: "busy.rs".into(),
        }
    }

    #[test]
    fn writes_cpu_samples_waits_and_symbols_the_importer_reads() {
        let s = set();
        assert_eq!(s.addresses(), [EXE + 0x10, EXE + 0x20, EXE + 0x30, EXE + 0x900]);
        let symbols = [
            sym(EXE + 0x10, "busy::spin"),
            sym(EXE + 0x20, "busy::main"),
            sym(EXE + 0x900, "RtlAcquireSRWLockExclusive"),
        ];
        let (bytes, stats) = write_nettrace(&s, &symbols, Vec::new()).unwrap();
        assert_eq!(
            stats,
            WriteStats {
                samples: 30,
                waits: 10,
                symbols: 3
            }
        );
        let Capture::Profile(p) = NettraceImporter.import("busy.nettrace", &bytes, &Limits::default()).unwrap() else {
            panic!("a profile");
        };
        assert!(p.frames.iter().any(|f| f.function == "busy::spin"));
        let waits: Vec<_> = p.events.iter().filter(|e| e.kind == EventKind::Wait).collect();
        assert_eq!(waits.len(), 10);
        assert_eq!(waits[0].dur_ns, 10_000_000);
        assert!(waits[0].label.contains("lock"), "{}", waits[0].label);
        assert!(p.metrics.iter().any(|m| m.id == MetricId::WaitNs));
        assert!(p.threads.iter().any(|t| t.name == "worker"));
    }

    #[test]
    fn limits_drop_samples_and_cut_stacks() {
        let mut s = SampleSet::new(1, 4, 0, 0, 1, 1_000_000, 2);
        let deep: Vec<u64> = (1..=1000).collect();
        assert!(s.push(1, 1, &deep, State::Running));
        assert_eq!(s.stack(0).len(), MAX_DEPTH);
        assert!(s.push(1, 2, &[1], State::Waiting));
        assert!(s.is_full());
        assert!(!s.push(1, 3, &[1], State::Running));
        assert_eq!(s.dropped, 1);
        assert!(s.stack(99).is_empty());
        assert_eq!(s.ticks_per_second, 1);
    }

    #[test]
    fn wait_reasons_follow_the_leaf_frames() {
        assert_eq!(wait_reason(&["NtDelayExecution", "SleepEx"]), 4);
        assert_eq!(wait_reason(&["NtWaitForAlertByThreadId", "RtlAcquireSRWLockExclusive"]), 21);
        assert_eq!(wait_reason(&["NtRemoveIoCompletion"]), 15);
        assert_eq!(wait_reason(&["NtWaitForSingleObject", "WaitForSingleObjectEx"]), 6);
        assert_eq!(wait_reason(&["busy::work"]), UNKNOWN_WAIT);
        assert_eq!(wait_reason(&[]), UNKNOWN_WAIT);
        assert_eq!(
            wait_reason(&["a", "b", "c", "d", "NtDelayExecution"]),
            UNKNOWN_WAIT,
            "only the leaf frames"
        );
    }
}
