// Generated from the Rust wire types by `cargo test -p debrief-app --lib generated_types` (ts-rs; ADR 0016).
// Do not edit: change the Rust type and run the test with DEBRIEF_WRITE_TYPES=1.

export type DocKind = "dump" | "umdhSnapshot" | "umdhDiff";

export type DumpSniffKind = "none" | "dump" | "umdh";

export type DumpSniffResult = { path: string, kind: DumpSniffKind, };

export type DumpSeverity = "info" | "warning" | "critical";

export type DumpHeadline = { "type": "exception", name: string, location: string | null, } | { "type": "noException", threads: number, } | { "type": "umdhSnapshot", stacks: number, bytes: number, } | { "type": "umdhDiff", stacks: number, deltaBytes: number, };

export type DumpDocSummary = { id: number, 
/**
 * File name.
 */
name: string, 
/**
 * The file it was opened from (for projects and reopening).
 */
path: string | null, kind: DocKind, 
/**
 * When the dump was written or the snapshot taken (Unix ms).
 */
timeMs: number | null, 
/**
 * What the documents list shows in one line; the UI words it and formats the sizes.
 */
headline: DumpHeadline, 
/**
 * The process it is about.
 */
process: string | null, 
/**
 * Severity of its most severe finding.
 */
severity: DumpSeverity, };

export type ExceptionCard = { 
/**
 * `0xC0000005`.
 */
code: string, 
/**
 * `ACCESS_VIOLATION`.
 */
name: string, 
/**
 * What the code usually means.
 */
meaning: string | null, 
/**
 * `writing 0x10`, for access violations.
 */
access: string | null, 
/**
 * The faulting instruction.
 */
address: string, 
/**
 * `module!function+0x1d` or `module+0x1234`.
 */
location: string | null, threadId: number, 
/**
 * The decoded fast-fail code of a `STACK_BUFFER_OVERRUN`.
 */
fastFail: string | null, };

export type HandleTypeDto = { name: string, count: number, };

export type HandlesDto = { total: number, byType: Array<HandleTypeDto>, };

export type MemoryDto = { regions: number, committed: number, reserved: number, free: number, image: number, mapped: number, private: number, largestFree: number, };

export type DumpSymbolStatus = { "kind": "notNeeded" } | { "kind": "loaded" } | { "kind": "noIdentity" } | { "kind": "missing" } | { "kind": "serversDisabled" } | { "kind": "notOnServer" } | { "kind": "mismatch", detail: string, } | { "kind": "unreadable", detail: string, } | { "kind": "downloadFailed", detail: string, } | { "kind": "downloading" };

export type DumpSymbolProblems = { missing: number, serversDisabled: number, notOnServer: number, mismatch: number, unreadable: number, downloadFailed: number, 
/**
 * Modules whose PDB is being downloaded now (not a problem yet).
 */
downloading: number, noIdentity: number, };

export type DumpOverview = { summary: DumpDocSummary, 
/**
 * `mini`, `withHeap` or `full`.
 */
dumpKind: string, 
/**
 * `Windows NT 10.0.22631`.
 */
os: string | null, 
/**
 * `x64`.
 */
arch: string | null, processors: number | null, cpu: string | null, processName: string | null, processPath: string | null, pid: number | null, startedMs: number | null, uptimeS: number | null, exception: ExceptionCard | null, threads: number, modules: number, 
/**
 * Modules with symbols.
 */
modulesWithSymbols: number, 
/**
 * Modules without symbols, by reason.
 */
symbolProblems: DumpSymbolProblems, 
/**
 * Frames named by symbols, and all frames.
 */
framesSymbolised: number, frames: number, handles: HandlesDto | null, memory: MemoryDto | null, 
/**
 * `.NET 8.0.1 (coreclr.dll)`.
 */
dotnet: string | null, unloadedModules: Array<string>, notes: Array<string>, };

export type ThreadState = "faulted" | "lockWait" | "waiting" | "running";

export type ThreadRow = { id: number, name: string | null, state: ThreadState, frames: number, 
/**
 * The top frame.
 */
top: string | null, 
/**
 * The first frame in user code (outside system and runtime modules).
 */
userFrame: string | null, };

export type ThreadsRequest = { id: number, offset: number, limit: number, 
/**
 * Case-insensitive text in the thread's id, name or frames.
 */
filter: string | null, };

export type ThreadsPage = { rows: Array<ThreadRow>, total: number, };

export type FrameTrust = "context" | "cfi" | "framePointer" | "scan" | "recorded";

export type FrameRow = { index: number, address: string, module: string | null, function: string | null, 
/**
 * `module!function+0x1d`, `module+0x1234` or the address.
 */
label: string, 
/**
 * `file:line`.
 */
source: string | null, trust: FrameTrust, 
/**
 * In a system or runtime module.
 */
framework: boolean, 
/**
 * The likely culprit of the crash.
 */
culprit: boolean, };

export type StackRequest = { id: number, threadId: number, };

export type StackDto = { threadId: number, frames: Array<FrameRow>, truncated: boolean, };

export type ModuleRow = { name: string, path: string, base: string, size: number, version: string | null, debugFile: string | null, debugId: string | null, symbols: DumpSymbolStatus, framework: boolean, };

export type ModulesRequest = { id: number, offset: number, limit: number, filter: string | null, };

export type ModulesPage = { rows: Array<ModuleRow>, total: number, };

export type DumpFindingKind = "crashCause" | "culprit" | "stackOverflow" | "heapCorruption" | "badAddress" | "lockWait" | "noException" | "dotnet" | "missingSymbols" | "handles" | "addressSpace" | "leakGrowth" | "steadyGrowth" | "largeAllocation" | "heapTotal";

export type FindingEvidence = { docId: number, threadId: number | null, frame: number | null, module: string | null, 
/**
 * The UMDH stack key (for `dumps_umdh_stack`).
 */
stack: string | null, 
/**
 * Threads the finding is about (lock waits).
 */
threads: Array<number>, };

export type DumpFinding = { kind: DumpFindingKind, severity: DumpSeverity, title: string, detail: string, evidence: FindingEvidence, };

export type DumpFindingsRequest = { ids: Array<number>, };

export type UmdhGroup = "stack" | "module" | "function";

export type UmdhDiffRequest = { base: number | null, target: number, group: UmdhGroup, offset: number, limit: number, filter: string | null, };

export type UmdhRow = { 
/**
 * Stack key, module or function (for `dumps_umdh_stack` with stacks).
 */
key: string, 
/**
 * What the row is: the stack's culprit frame, the module or the function.
 */
label: string, module: string | null, deltaBytes: number, deltaCount: number, bytes: number, count: number, 
/**
 * Stacks in the row (1 for stacks).
 */
stacks: number, };

export type UmdhDiffPage = { rows: Array<UmdhRow>, total: number, deltaBytes: number, deltaCount: number, bytes: number, };

export type UmdhStackRequest = { base: number | null, target: number, key: string, };

export type UmdhStackDto = { key: string, deltaBytes: number, deltaCount: number, 
/**
 * Back trace ids (UMDH's), in the newer log.
 */
ids: number[], frames: Array<FrameRow>, };

export type UmdhTreeRequest = { base: number | null, target: number, path: Array<string>, limit: number, };

export type UmdhTreeNode = { key: string, label: string, deltaBytes: number, deltaCount: number, hasChildren: boolean, framework: boolean, };

export type DumpMarker = { id: number, name: string, kind: DocKind, timeMs: number, headline: DumpHeadline, severity: DumpSeverity, };
