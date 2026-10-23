//! The query DTOs the app returns to the UI (camelCase JSON). With the `ts` feature they derive ts-rs, and the
//! app's `generated_types` test writes them to `app/src/app/core/generated/dumps.ts` (ADR 0016), so the wire
//! contract has one definition.
//!
//! Addresses are hex strings (`0x7ffa12340000`): JSON numbers lose 64-bit precision.

use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

/// What an opened document is.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DocKind {
    Dump,
    UmdhSnapshot,
    UmdhDiff,
}

/// What a file is, from its first bytes (drag-and-drop routing).
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DumpSniffKind {
    None,
    Dump,
    Umdh,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpSniffResult {
    pub path: String,
    pub kind: DumpSniffKind,
}

/// An open document, as the documents list shows it.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpDocSummary {
    pub id: u32,
    /// File name.
    pub name: String,
    /// The file it was opened from (for projects and reopening).
    pub path: Option<String>,
    pub kind: DocKind,
    /// When the dump was written or the snapshot taken (Unix ms).
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub time_ms: Option<i64>,
    /// What the documents list shows in one line; the UI words it and formats the sizes.
    pub headline: DumpHeadline,
    /// The process it is about.
    pub process: Option<String>,
    /// Severity of its most severe finding.
    pub severity: Severity,
}

/// A document's one-line headline as typed values: the UI turns it into words and formats sizes with the shared
/// units rules, so no size or count travels as text.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DumpHeadline {
    /// The dump's exception: `ACCESS_VIOLATION`, at `ntdll.dll!RtlFreeHeap+0x1d` when the faulting frame is known.
    Exception { name: String, location: Option<String> },
    /// A dump without an exception, with its thread count.
    NoException { threads: u32 },
    /// A UMDH snapshot: its allocation stacks and the bytes they hold.
    UmdhSnapshot {
        stacks: u32,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        bytes: u64,
    },
    /// A UMDH comparison: the growth (negative: shrink) across its allocation stacks.
    UmdhDiff {
        stacks: u32,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        delta_bytes: i64,
    },
}

#[cfg_attr(feature = "ts", derive(TS), ts(rename = "DumpSeverity"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

/// The exception card of a crash dump.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionCard {
    /// `0xC0000005`.
    pub code: String,
    /// `ACCESS_VIOLATION`.
    pub name: String,
    /// What the code usually means.
    pub meaning: Option<String>,
    /// `writing 0x10`, for access violations.
    pub access: Option<String>,
    /// The faulting instruction.
    pub address: String,
    /// `module!function+0x1d` or `module+0x1234`.
    pub location: Option<String>,
    pub thread_id: u32,
    /// The decoded fast-fail code of a `STACK_BUFFER_OVERRUN`.
    pub fast_fail: Option<String>,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpOverview {
    pub summary: DumpDocSummary,
    /// `mini`, `withHeap` or `full`.
    pub dump_kind: String,
    /// `Windows NT 10.0.22631`.
    pub os: Option<String>,
    /// `x64`.
    pub arch: Option<String>,
    pub processors: Option<u32>,
    pub cpu: Option<String>,
    pub process_name: Option<String>,
    pub process_path: Option<String>,
    pub pid: Option<u32>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub started_ms: Option<i64>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub uptime_s: Option<u64>,
    pub exception: Option<ExceptionCard>,
    pub threads: u32,
    pub modules: u32,
    /// Modules with symbols.
    pub modules_with_symbols: u32,
    /// Modules without symbols, by reason.
    pub symbol_problems: SymbolProblems,
    /// Frames named by symbols, and all frames.
    pub frames_symbolised: u32,
    pub frames: u32,
    pub handles: Option<HandlesDto>,
    pub memory: Option<MemoryDto>,
    /// `.NET 8.0.1 (coreclr.dll)`.
    pub dotnet: Option<String>,
    pub unloaded_modules: Vec<String>,
    pub notes: Vec<String>,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandlesDto {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub total: u64,
    pub by_type: Vec<HandleTypeDto>,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandleTypeDto {
    pub name: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub count: u64,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDto {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub regions: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub committed: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub reserved: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub free: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub image: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub mapped: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub private: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub largest_free: u64,
}

/// What a thread is doing, from its top frames.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadState {
    /// It raised the dump's exception.
    Faulted,
    /// Blocked acquiring a lock.
    LockWait,
    /// Waiting on a kernel object or sleeping.
    Waiting,
    /// Anything else (or unknown without symbols).
    Running,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRow {
    pub id: u32,
    pub name: Option<String>,
    pub state: ThreadState,
    pub frames: u32,
    /// The top frame.
    pub top: Option<String>,
    /// The first frame in user code (outside system and runtime modules).
    pub user_frame: Option<String>,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsRequest {
    pub id: u32,
    pub offset: u32,
    pub limit: u32,
    /// Case-insensitive text in the thread's id, name or frames.
    pub filter: Option<String>,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsPage {
    pub rows: Vec<ThreadRow>,
    pub total: u32,
}

/// How a frame was found.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FrameTrust {
    Context,
    Cfi,
    FramePointer,
    Scan,
    Recorded,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameRow {
    pub index: u32,
    pub address: String,
    pub module: Option<String>,
    pub function: Option<String>,
    /// `module!function+0x1d`, `module+0x1234` or the address.
    pub label: String,
    /// `file:line`.
    pub source: Option<String>,
    pub trust: FrameTrust,
    /// In a system or runtime module.
    pub framework: bool,
    /// The likely culprit of the crash.
    pub culprit: bool,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackRequest {
    pub id: u32,
    pub thread_id: u32,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackDto {
    pub thread_id: u32,
    pub frames: Vec<FrameRow>,
    pub truncated: bool,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleRow {
    pub name: String,
    pub path: String,
    pub base: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub size: u64,
    pub version: Option<String>,
    pub debug_file: Option<String>,
    pub debug_id: Option<String>,
    pub symbols: SymbolStatusDto,
    pub framework: bool,
}

/// What symbols a module has (ADR 0036): the UI shows each with its next step (enable symbol servers, add a
/// folder, check the network).
#[cfg_attr(feature = "ts", derive(TS), ts(rename = "DumpSymbolStatus"))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SymbolStatusDto {
    /// No frame needed a name from it.
    NotNeeded,
    /// A PDB with its GUID and age named its frames.
    Loaded,
    /// It records no PDB identity (name, GUID, age), so no PDB can be matched.
    NoIdentity,
    /// No PDB with its identity in the symbol folders or the cache.
    Missing,
    /// Missing, and downloads from symbol servers are off.
    ServersDisabled,
    /// Missing, and no configured symbol server has it.
    NotOnServer,
    /// A PDB of its name is another build (GUID or age differ).
    Mismatch { detail: String },
    /// A PDB was found but could not be read.
    Unreadable { detail: String },
    /// The download failed.
    DownloadFailed { detail: String },
    /// Its PDB is being downloaded; the view updates when it arrives.
    Downloading,
}

impl From<&crate::model::SymbolStatus> for SymbolStatusDto {
    fn from(s: &crate::model::SymbolStatus) -> SymbolStatusDto {
        use crate::model::SymbolStatus as S;
        match s {
            S::NotNeeded => SymbolStatusDto::NotNeeded,
            S::Loaded => SymbolStatusDto::Loaded,
            S::NoIdentity => SymbolStatusDto::NoIdentity,
            S::Missing => SymbolStatusDto::Missing,
            S::ServersDisabled => SymbolStatusDto::ServersDisabled,
            S::NotOnServer => SymbolStatusDto::NotOnServer,
            S::Mismatch { detail } => SymbolStatusDto::Mismatch { detail: detail.clone() },
            S::Unreadable { detail } => SymbolStatusDto::Unreadable { detail: detail.clone() },
            S::DownloadFailed { detail } => SymbolStatusDto::DownloadFailed { detail: detail.clone() },
            S::Downloading => SymbolStatusDto::Downloading,
        }
    }
}

/// How many modules a frame needed a name from have no symbols, by reason (the overview's hints).
#[cfg_attr(feature = "ts", derive(TS), ts(rename = "DumpSymbolProblems"))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolProblems {
    pub missing: u32,
    pub servers_disabled: u32,
    pub not_on_server: u32,
    pub mismatch: u32,
    pub unreadable: u32,
    pub download_failed: u32,
    /// Modules whose PDB is being downloaded now (not a problem yet).
    pub downloading: u32,
    pub no_identity: u32,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModulesRequest {
    pub id: u32,
    pub offset: u32,
    pub limit: u32,
    pub filter: Option<String>,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModulesPage {
    pub rows: Vec<ModuleRow>,
    pub total: u32,
}

/// What a finding is about.
#[cfg_attr(feature = "ts", derive(TS), ts(rename = "DumpFindingKind"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindingKind {
    CrashCause,
    Culprit,
    StackOverflow,
    HeapCorruption,
    BadAddress,
    LockWait,
    NoException,
    Dotnet,
    MissingSymbols,
    Handles,
    AddressSpace,
    LeakGrowth,
    SteadyGrowth,
    LargeAllocation,
    HeapTotal,
}

/// Where the evidence of a finding is: a document, and in it a thread and frame or an allocation stack.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingEvidence {
    pub doc_id: u32,
    pub thread_id: Option<u32>,
    pub frame: Option<u32>,
    pub module: Option<String>,
    /// The UMDH stack key (for `dumps_umdh_stack`).
    pub stack: Option<String>,
    /// Threads the finding is about (lock waits).
    pub threads: Vec<u32>,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpFinding {
    pub kind: FindingKind,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub evidence: FindingEvidence,
}

/// Findings of one dump, or of a series of UMDH logs of one process (oldest first).
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpFindingsRequest {
    pub ids: Vec<u32>,
}

/// How UMDH growth is grouped.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UmdhGroup {
    Stack,
    Module,
    Function,
}

/// The growth from `base` (an older snapshot; none for a UMDH diff log or "everything alive") to `target`.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UmdhDiffRequest {
    pub base: Option<u32>,
    pub target: u32,
    pub group: UmdhGroup,
    pub offset: u32,
    pub limit: u32,
    pub filter: Option<String>,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UmdhRow {
    /// Stack key, module or function (for `dumps_umdh_stack` with stacks).
    pub key: String,
    /// What the row is: the stack's culprit frame, the module or the function.
    pub label: String,
    pub module: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub delta_bytes: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub delta_count: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub count: u64,
    /// Stacks in the row (1 for stacks).
    pub stacks: u32,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UmdhDiffPage {
    pub rows: Vec<UmdhRow>,
    pub total: u32,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub delta_bytes: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub delta_count: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UmdhStackRequest {
    pub base: Option<u32>,
    pub target: u32,
    pub key: String,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UmdhStackDto {
    pub key: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub delta_bytes: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub delta_count: i64,
    /// Back trace ids (UMDH's), in the newer log.
    #[cfg_attr(feature = "ts", ts(type = "number[]"))]
    pub ids: Vec<u64>,
    pub frames: Vec<FrameRow>,
}

/// The children of `path` (frame keys from the outermost caller) in the top-down tree of growing stacks.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UmdhTreeRequest {
    pub base: Option<u32>,
    pub target: u32,
    pub path: Vec<String>,
    pub limit: u32,
}

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UmdhTreeNode {
    pub key: String,
    pub label: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub delta_bytes: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub delta_count: i64,
    pub has_children: bool,
    pub framework: bool,
}

/// A document on the log timeline: a marker at the dump's (or snapshot's) time.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpMarker {
    pub id: u32,
    pub name: String,
    pub kind: DocKind,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub time_ms: i64,
    pub headline: DumpHeadline,
    pub severity: Severity,
}
