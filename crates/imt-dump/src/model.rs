//! The document model: what the dumps plugin extracts from a crash/memory dump or a UMDH log, and what every
//! analysis reads. Plain values that cross the plugin boundary as postcard ([`crate::wire`]), so no serde
//! attributes postcard cannot carry.

use serde::{Deserialize, Serialize};

/// What a file opened as a dump document holds.
// One per open file: the variants' size difference does not matter, and boxing would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Document {
    Dump(Dump),
    Umdh(UmdhLog),
}

/// How much of the process a dump holds (the `MiniDumpWriteDump` type it was written with).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DumpKind {
    /// Threads, stacks and modules only.
    Mini,
    /// A minidump with the heap (private read/write memory).
    WithHeap,
    /// All of the process' memory (`Memory64List`).
    Full,
}

/// A crash or memory dump of one process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dump {
    pub kind: DumpKind,
    /// When the dump was written (Unix ms).
    pub time_ms: Option<i64>,
    pub system: Option<SystemInfo>,
    pub process: Option<ProcessInfo>,
    pub exception: Option<ExceptionInfo>,
    pub threads: Vec<Thread>,
    pub modules: Vec<Module>,
    /// Modules unloaded before the dump was written (name only; a crash in one is a known pattern).
    pub unloaded_modules: Vec<String>,
    pub handles: Option<HandleSummary>,
    pub memory: Option<MemorySummary>,
    /// The .NET runtime the process had loaded, when it had one.
    pub dotnet: Option<DotnetRuntime>,
    /// What the reader could not do (damaged streams, limits hit, threads left unwalked).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemInfo {
    /// `Windows NT`, `Linux`, ...
    pub os: String,
    /// `10.0.22631`.
    pub os_version: String,
    /// Service pack or build string.
    pub os_build: Option<String>,
    pub arch: Arch,
    pub processors: u32,
    pub cpu_info: Option<String>,
}

/// The dumped process' CPU architecture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arch {
    X86,
    Amd64,
    Arm64,
    Arm,
    Other(String),
}

impl Arch {
    /// Bytes per pointer.
    pub fn pointer_size(&self) -> u8 {
        match self {
            Arch::X86 | Arch::Arm => 4,
            _ => 8,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Arch::X86 => "x86",
            Arch::Amd64 => "x64",
            Arch::Arm64 => "ARM64",
            Arch::Arm => "ARM",
            Arch::Other(s) => s,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: Option<u32>,
    /// The executable's file name.
    pub name: String,
    /// Its full path, when the dump has it.
    pub path: Option<String>,
    /// Unix ms.
    pub started_ms: Option<i64>,
    pub uptime_s: Option<u64>,
}

/// The exception record of a crash dump. Names and meanings are decoded by the engine
/// ([`crate::exceptions`]), so a plugin cannot word them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExceptionInfo {
    pub code: u32,
    pub flags: u32,
    /// The faulting instruction.
    pub address: u64,
    pub thread_id: u32,
    /// `ExceptionInformation` (at most 15 values).
    pub parameters: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    pub id: u32,
    pub name: Option<String>,
    /// Innermost frame first.
    pub frames: Vec<Frame>,
    /// More frames were found than kept.
    pub truncated: bool,
    pub suspend_count: u32,
    pub priority: i32,
}

/// How a frame was found (as the stack walker reports it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trust {
    /// The CPU context's instruction pointer.
    Context,
    /// Call frame information.
    Cfi,
    /// The previous frame's frame pointer.
    FramePointer,
    /// A stack word that points into a module: a candidate, not a verified return address.
    Scan,
    /// Recorded by another stack walker (UMDH, the runtime).
    Recorded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub address: u64,
    /// Index into [`Dump::modules`].
    pub module: Option<u32>,
    pub function: Option<String>,
    /// Bytes from the function's start.
    pub function_offset: Option<u64>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub trust: Trust,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    pub base: u64,
    pub size: u64,
    /// Full path as the dump records it.
    pub path: String,
    pub version: Option<String>,
    /// The PDB name (`ntdll.pdb`).
    pub debug_file: Option<String>,
    /// The PDB's identifier (GUID and age, as symbol servers spell it).
    pub debug_id: Option<String>,
    /// Whether symbols were found for it.
    pub symbols: bool,
}

impl Module {
    /// The file name of [`Module::path`].
    pub fn name(&self) -> &str {
        crate::modules::file_name(&self.path)
    }

    pub fn contains(&self, address: u64) -> bool {
        address >= self.base && address - self.base < self.size
    }
}

/// Open handles by object type (from the handle data stream).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandleSummary {
    pub total: u64,
    /// `(type name, count)`, most first.
    pub by_type: Vec<(String, u64)>,
}

/// The address space (from the memory info list), in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MemorySummary {
    pub regions: u64,
    pub committed: u64,
    pub reserved: u64,
    pub free: u64,
    /// Committed memory by type.
    pub image: u64,
    pub mapped: u64,
    pub private: u64,
    pub largest_free: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DotnetFlavor {
    /// .NET Framework (`clr.dll`, `mscorwks.dll`).
    Framework,
    /// .NET Core / .NET 5+ (`coreclr.dll`, `libcoreclr.so`).
    Core,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DotnetRuntime {
    pub flavor: DotnetFlavor,
    /// The runtime module's file version.
    pub version: Option<String>,
    /// The runtime module's path.
    pub module: String,
}

// ---- UMDH ----

/// Which UMDH output a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UmdhKind {
    /// `umdh -p:<pid> -f:<file>`: the allocations of one moment.
    Snapshot,
    /// `umdh <old> <new>`: the difference of two snapshots, computed by UMDH.
    Diff,
}

/// A UMDH log: allocation stacks with their counts and bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UmdhLog {
    pub kind: UmdhKind,
    pub process: Option<UmdhProcess>,
    /// The loaded modules the snapshot lists (to place raw addresses).
    pub modules: Vec<UmdhModule>,
    pub stacks: Vec<UmdhStack>,
    /// When it was taken (Unix ms): the file's own time, else its modification time.
    pub time_ms: Option<i64>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UmdhProcess {
    pub pid: Option<u32>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UmdhModule {
    pub base: u64,
    pub size: u64,
    pub name: String,
}

/// One allocation stack (a UMDH `BackTrace`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UmdhStack {
    /// The back trace id of UMDH's stack database.
    pub id: u64,
    /// Bytes requested by the allocations alive on this stack (the newer side of a diff).
    pub bytes: u64,
    pub count: u64,
    /// The older side of a diff.
    pub old_bytes: Option<u64>,
    pub old_count: Option<u64>,
    /// The largest single allocation seen on this stack (snapshots only).
    pub largest: u64,
    /// Innermost (the allocator) first.
    pub frames: Vec<UmdhFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UmdhFrame {
    pub module: Option<String>,
    pub function: Option<String>,
    /// From the function's start (`fn+0x1d`) or, without a function, from the module's base.
    pub offset: Option<u64>,
    /// The raw address, when UMDH printed one.
    pub address: Option<u64>,
    pub file: Option<String>,
    pub line: Option<u32>,
}

impl UmdhFrame {
    /// `module!function+0x1d`, `module+0x1234` or the raw address.
    pub fn label(&self) -> String {
        let off = |o: Option<u64>| o.filter(|o| *o != 0).map(|o| format!("+{o:#x}")).unwrap_or_default();
        match (&self.module, &self.function) {
            (Some(m), Some(f)) => format!("{m}!{f}{}", off(self.offset)),
            (None, Some(f)) => format!("{f}{}", off(self.offset)),
            (Some(m), None) => format!("{m}{}", off(self.offset)),
            (None, None) => self.address.map(|a| format!("{a:#x}")).unwrap_or_else(|| "?".into()),
        }
    }

    /// What identifies the frame across snapshots: `module!function`, or the label without symbols.
    pub fn key(&self) -> String {
        match (&self.module, &self.function) {
            (Some(m), Some(f)) => format!("{m}!{f}"),
            (None, Some(f)) => f.clone(),
            _ => self.label(),
        }
    }
}
