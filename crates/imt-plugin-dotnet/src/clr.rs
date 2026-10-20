//! The CLR's event providers and what each capture preset enables (shared by
//! ETW and EventPipe, so both record the same events).

use serde::{Deserialize, Serialize};

/// `Microsoft-Windows-DotNETRuntime`.
pub const RUNTIME: &str = "Microsoft-Windows-DotNETRuntime";
/// {E13C0D23-CCBC-4E12-931B-D9CC2EEE27E4}
pub const RUNTIME_GUID: u128 = 0xe13c0d23_ccbc_4e12_931b_d9cc2eee27e4;
/// `Microsoft-Windows-DotNETRuntimeRundown`.
pub const RUNDOWN: &str = "Microsoft-Windows-DotNETRuntimeRundown";
/// {A669021C-C450-4609-A035-5AF59AF4DF18}
pub const RUNDOWN_GUID: u128 = 0xa669021c_c450_4609_a035_5af59af4df18;
/// `Microsoft-DotNETCore-SampleProfiler` (EventPipe only).
pub const SAMPLE_PROFILER: &str = "Microsoft-DotNETCore-SampleProfiler";

/// Runtime provider keywords.
pub mod keywords {
    pub const GC: u64 = 0x1;
    pub const LOADER: u64 = 0x8;
    pub const JIT: u64 = 0x10;
    pub const NGEN: u64 = 0x20;
    pub const CONTENTION: u64 = 0x4000;
    pub const EXCEPTION: u64 = 0x8000;
    pub const THREADING: u64 = 0x10000;
    pub const TYPE: u64 = 0x80000;
    pub const GC_HEAP_DUMP: u64 = 0x100000;
    pub const GC_HEAP_COLLECT: u64 = 0x800000;
    pub const GC_HEAP_AND_TYPE_NAMES: u64 = 0x1000000;
    pub const STACK: u64 = 0x4000_0000;
    /// Rundown provider: Loader | Jit | NGen | End rundown | (method) Jitted-method IL map off.
    pub const RUNDOWN_END: u64 = 0x8 | 0x10 | 0x20 | 0x100;
}

/// ETW / EventPipe levels.
pub mod level {
    pub const INFORMATIONAL: u8 = 4;
    pub const VERBOSE: u8 = 5;
}

/// What to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preset {
    /// CPU samples, waits and runtime events (GC, exceptions, contention, JIT, loader).
    Cpu,
    /// As CPU plus `GCAllocationTick` (about one sample per 100 KB allocated) with stacks.
    Allocations,
    /// An induced GC with a heap walk: types, objects, references, roots.
    #[serde(rename = "heap")]
    HeapSnapshot,
}

impl Preset {
    /// Runtime provider keywords and level.
    pub fn runtime(self) -> (u64, u8) {
        use keywords::*;
        let cpu = GC | LOADER | JIT | CONTENTION | EXCEPTION | THREADING;
        match self {
            Preset::Cpu => (cpu, level::INFORMATIONAL),
            Preset::Allocations => (cpu, level::VERBOSE),
            // No GC keyword: allocation ticks and later GCs would keep the stream busy after the walk
            // (EventPipe detects its end by silence; ETW adds GC to see the induced GC end).
            Preset::HeapSnapshot => (TYPE | GC_HEAP_DUMP | GC_HEAP_COLLECT | GC_HEAP_AND_TYPE_NAMES, level::VERBOSE),
        }
    }

    /// Whether CPU samples (and waits) are recorded.
    pub fn samples(self) -> bool {
        !matches!(self, Preset::HeapSnapshot)
    }

    /// Whether method names are collected at the end (rundown).
    pub fn rundown(self) -> bool {
        self.samples()
    }

    /// Words for the UI and the logs.
    pub fn label(self) -> &'static str {
        match self {
            Preset::Cpu => "CPU",
            Preset::Allocations => "allocations",
            Preset::HeapSnapshot => "heap snapshot",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_enable_what_they_promise() {
        assert_eq!(Preset::Cpu.runtime().1, level::INFORMATIONAL);
        assert_eq!(Preset::Allocations.runtime().1, level::VERBOSE, "allocation ticks are verbose");
        let (heap, _) = Preset::HeapSnapshot.runtime();
        assert_ne!(heap & keywords::GC_HEAP_COLLECT, 0, "triggers the induced GC");
        assert_ne!(heap & keywords::GC_HEAP_DUMP, 0);
        assert!(!Preset::HeapSnapshot.samples());
        assert!(Preset::Cpu.rundown());
        assert_eq!(serde_json::to_string(&Preset::HeapSnapshot).unwrap(), "\"heap\"");
        assert_eq!(serde_json::from_str::<Preset>("\"allocations\"").unwrap(), Preset::Allocations);
        assert_eq!(Preset::Cpu.label(), "CPU");
    }
}
