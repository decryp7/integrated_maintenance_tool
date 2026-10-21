//! The CLR's ETW providers, as far as the ETW code needs them (names, GUIDs, keywords, levels). Capture presets
//! stay with each profiler.

/// `Microsoft-Windows-DotNETRuntime`.
pub const RUNTIME: &str = "Microsoft-Windows-DotNETRuntime";
/// {E13C0D23-CCBC-4E12-931B-D9CC2EEE27E4}
pub const RUNTIME_GUID: u128 = 0xe13c0d23_ccbc_4e12_931b_d9cc2eee27e4;
/// `Microsoft-Windows-DotNETRuntimeRundown`.
pub const RUNDOWN: &str = "Microsoft-Windows-DotNETRuntimeRundown";
/// {A669021C-C450-4609-A035-5AF59AF4DF18}
pub const RUNDOWN_GUID: u128 = 0xa669021c_c450_4609_a035_5af59af4df18;
/// `Microsoft-DotNETCore-SampleProfiler` (the provider of NetTrace CPU samples).
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
    pub const STACK: u64 = 0x4000_0000;
    /// Rundown provider: Loader | Jit | NGen | End rundown.
    pub const RUNDOWN_END: u64 = 0x8 | 0x10 | 0x20 | 0x100;
}

/// ETW levels.
pub mod level {
    pub const INFORMATIONAL: u8 = 4;
    pub const VERBOSE: u8 = 5;
}

/// Rundown `DCEndComplete`.
pub const DC_END_COMPLETE: u32 = 146;
/// Runtime `GCEnd` and `GCBulkNode`.
pub const GC_END: u32 = 2;
pub const GC_BULK_NODE: u32 = 18;
