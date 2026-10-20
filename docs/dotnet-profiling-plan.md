# .NET profiling — implementation plan

Status: implemented (2026-10-20) as the plugin `crates/imt-plugin-dotnet`; ADR 0019 records the decisions and supersedes
this brief where they differ (plugin tool instead of `dotnet_*` shell commands, .etl import, ETW waits). Priority order was: **E** (ETW, .NET Framework 4.6.2+ on Windows) → **C** (CLR event
decoding for allocations and heap snapshots) → **D** (app + UI) → **B** (NetTrace V6) → **A** (EventPipe attach, .NET Core/5+).
Work on E and the real-attach checks happens on a Windows machine; everything else also runs on macOS/Linux.
The roadmap entry is in [roadmap.md](roadmap.md) ("Profiling"); this file is the working brief.

Read AGENTS.md first: Rust-only first-party code, pure core / thin edges, parse-don't-validate, no panics on input,
limits on everything, typed errors, every robustness behaviour tested, Conventional style, doc comments on public items,
rustfmt max_width 140 (format only your own files with `rustfmt <files>`; never `cargo fmt --all`).
Use a private CARGO_TARGET_DIR (e.g. target/agent-<letter>) so parallel builds don't clobber each other.
Node for Angular: ≥ 24.15 (Angular CLI 22).
Do NOT commit; the integrator commits. Do NOT edit files outside your ownership list; shared files
(workspace Cargo.toml, app/src-tauri/src/lib.rs, app/src/app/core/api.types.ts, activity/ipc.rs) are edited only where your brief
says so ("single declared line").

.NET 10 writes NetTrace V6; .NET 7 writes V5. Real-runtime tests must be skipped (not failed) when no dotnet is found
(look at $IMT_DOTNET, then `dotnet` on PATH, then ~/.dotnet/dotnet).

## Goal (user-facing)

"Profile .NET process…": pick a running .NET process, choose
- **CPU** — EventPipe sample profiler + runtime events (GC, exceptions, contention, JIT, loader) + rundown → `.nettrace`
  → opens in the existing profiling views (hotspots, trees, flame, timeline).
- **Allocations** — as CPU plus GC keyword at verbose level so `GCAllocationTick` (sampled ~100 KB) arrives with stacks →
  allocation metrics (`allocBytes`, `allocCount`) per stack, with the allocated type as a synthetic leaf frame.
- **Heap snapshot** — GC heap dump keywords (as dotnet-gcdump: GC | GCHeapDump | Type | GCHeapAndTypeNames …): the runtime
  performs an induced GC and walks the heap (BulkType, GCBulkNode, GCBulkEdge, GCBulkRootEdge, static/CCW/weak-table roots)
  → a `HeapSnapshot` (types + object graph + roots) → existing heap views (types, dominators, paths to roots, diff).
Recording runs until stopped or a duration elapses; size-capped; one session per process at a time.

## Contract (owned by the integrator; mirrored in Rust DTOs by stream D)

```ts
// api.types.ts — ".NET profiling" section
export interface DotnetProcess { pid: number; name: string; commandLine: string | null; runtimeVersion: string | null; arch: string | null; }
export type DotnetCaptureKind = 'cpu' | 'allocations' | 'heap';
export interface DotnetCaptureRequest { pid: number; kind: DotnetCaptureKind; /** null = until stopped (ignored for heap) */ durationSec: number | null; }
export type DotnetCaptureState = 'recording' | 'stopping' | 'done' | 'failed' | 'cancelled';
export interface DotnetCaptureStatus { id: string; pid: number; processName: string; kind: DotnetCaptureKind; state: DotnetCaptureState;
  elapsedMs: number; bytes: number; path: string | null; error: string | null; }
```
Commands: `dotnet_processes() -> DotnetProcess[]`, `dotnet_capture_start(request) -> DotnetCaptureStatus`,
`dotnet_capture_stop(id) -> DotnetCaptureStatus`, `dotnet_capture_cancel(id) -> DotnetCaptureStatus`,
`dotnet_captures() -> DotnetCaptureStatus[]` (polled by the UI while recording; quiet in the activity log).
When a capture is `done`, the UI opens `path` with the existing `profiling_open`.

## Crate API: `crates/imt-dotnet` (stream A)

```rust
pub struct ProcessInfo { pub pid: u32, pub name: String, pub command_line: Option<String>, pub runtime_version: Option<String>, pub arch: Option<String> }
pub fn list_processes() -> Vec<ProcessInfo>;                   // discovery + ProcessInfo per port, timeouts, never panics
pub enum Preset { Cpu, Allocations, HeapSnapshot }
pub struct Limits { pub max_bytes: u64, pub max_duration: Duration, pub connect_timeout: Duration }
pub struct Recording { .. }                                      // background thread copying the stream to a file
impl Recording {
    pub fn start(pid: u32, preset: Preset, out: &Path, limits: Limits) -> Result<Recording, DotnetError>;
    pub fn bytes(&self) -> u64; pub fn elapsed(&self) -> Duration; pub fn is_finished(&self) -> bool;
    pub fn stop(self) -> Result<Finished, DotnetError>;          // StopTracing (rundown) and drain; idempotent-safe
    pub fn cancel(self);                                           // stop and delete the file
}
pub struct Finished { pub path: PathBuf, pub bytes: u64, pub truncated: bool }
#[non_exhaustive] pub enum DotnetError { NotFound{pid}, Unsupported{why}, Denied, Timeout, Protocol(String), Io(io::Error), Limit(String) }
```
Heap snapshot preset: stop automatically once the heap walk is over (stream quiet for ~1.5 s after data, or a GCEnd
for the induced GC if detected cheaply; hard cap 120 s).
Pure parts (message encoding/decoding, provider configs, port-name parsing) are plain functions with unit tests; I/O is thin.
Tests: a fake diagnostic server (Unix socket on macOS/Linux; named pipe on Windows behind cfg) answering ProcessInfo /
CollectTracing / StopTracing and streaming bytes; error paths (refused, garbage reply, error code, peer closing mid-stream,
size limit, timeout). A real-runtime test that starts a tiny .NET app (fixture project under crates/imt-dotnet/tests/fixtures,
not shipped) and records CPU / allocations / heap, asserting the file is a nettrace the existing importer accepts
(once B/C land) — skipped when dotnet is missing.

Protocol reference: https://github.com/dotnet/diagnostics/blob/main/documentation/design-docs/ipc-protocol.md
(magic "DOTNET_IPC_V1\0", header {magic[14], size u16, commandset u8, commandid u8, reserved u16}; EventPipe commandset
0x02: StopTracing 0x01, CollectTracing 0x02, CollectTracing2 0x03 (requestRundown), CollectTracing3/4/5 for newer runtimes;
Process commandset 0x04: ProcessInfo 0x00 / ProcessInfo2 0x04 / ProcessInfo3; Server 0xFF: OK 0x00, Error 0xFF).
Unix port: $TMPDIR/dotnet-diagnostic-{pid}-{disambiguation}-socket; Windows: \\.\pipe\dotnet-diagnostic-{pid}.
Check the runtime source / docs for exact payload layouts and versions; be precise.

## Streams and file ownership

- **A (attach)**: new `crates/imt-dotnet/**` (+ one line in workspace Cargo.toml members). Windows named-pipe code must type-check
  (`cargo check -p imt-dotnet --target x86_64-pc-windows-msvc` if it builds without the Windows SDK; otherwise say so).
- **B (NetTrace V6)**: `crates/imt-plugin-profiling/src/nettrace/{format.rs,cursor.rs,mod.rs (only the version dispatch),tests.rs,writer.rs}`
  + fuzz/proptest updates for nettrace. Spec: PerfView `NetTraceFormat.md` (V6 section) and runtime `ep-file.c`/`ep-block.c` (.NET 10).
  Keep V4/V5. Writer gains V6 output for tests. docs/profiling.md: remove "V6 unsupported".
- **C (memory)**: `crates/imt-plugin-profiling/src/nettrace/runtime.rs` + new `nettrace/alloc.rs`, `nettrace/heap.rs`
  (heap importer `nettrace-heap`, `produces` heap, wins the sniff when GC bulk heap events are present), registration line in
  `plugin.rs`, tests, fuzz target `fuzz_prof_nettrace_heap`. Coordinate with B through the decoder's event iterator only
  (do not edit format.rs/cursor.rs; if the iterator API must change, write down what you need in your report).
- **D (app + UI)**: `app/src-tauri/src/dotnet.rs` (commands, state: running recordings per app, output dir under app data
  `captures/`, quiet poll), registration lines in lib.rs + activity/ipc.rs classification; `app/src/app/profiling/dotnet/**`
  (dialog: process list with refresh + filter, kind choice with plain explanations of overhead/what you get, duration,
  record/stop/cancel with elapsed + size, error states with next step, auto-open on done), backend abstraction + Tauri impl +
  mock impl (plausible data), command "Profile .NET process…" (Tools menu / Profiling), unit tests + Playwright e2e (mock).

## PRIORITY CHANGE: .NET Framework 4.6.2+ (Windows) is the primary target

.NET Framework has no EventPipe/diagnostic port. It is profiled through **ETW**, in Rust (`windows` crate), no TraceEvent/C#:
- **E (ETW collector, Windows)**: new `crates/imt-etw/**`. Real-time ETW session (StartTraceW / EnableTraceEx2 / OpenTraceW /
  ProcessTrace on a worker thread, CloseTrace/ControlTraceW stop; unique session name "IMT-<pid>-<n>"; always stopped on drop
  and on app exit; orphaned IMT sessions from a crash are stopped at next start). Providers:
  `Microsoft-Windows-DotNETRuntime` {E13C0D23-CCBC-4E12-931B-D9CC2EEE27E4} filtered to the target PID
  (EVENT_FILTER_TYPE_PID via ENABLE_TRACE_PARAMETERS) with keywords per preset (same presets as EventPipe: Cpu → Loader|Jit|
  Exception|Contention|GC(informational)|Stack(0x40000000); Allocations → + GC verbose (AllocationTick); HeapSnapshot →
  GC|GCHeapDump|Type|GCHeapAndTypeNames… which makes the CLR induce a GC and walk the heap) and, at stop,
  `Microsoft-Windows-DotNETRuntimeRundown` {A669021C-C450-4609-A035-5AF59AF4DF18} for method/module names.
  CPU: kernel SampledProfile + stack walk (system trace provider: SystemTraceControlGuid / EVENT_TRACE_FLAG_PROFILE with
  TraceSetInformation(TraceStackTracingInfo) for PerfInfo SampledProfile; or the "NT Kernel Logger" on older Windows), process
  filtered client-side by PID; stacks from the StackWalk event (or extended data EVENT_HEADER_EXT_TYPE_STACK_TRACE64 for CLR
  events with EVENT_ENABLE_PROPERTY_STACK_TRACE).
  The collector converts the live events into a **NetTrace (V5) file** via a production writer (promote/extend the nettrace
  writer in imt-plugin-profiling into a reusable module, or a writer in imt-etw implementing the same format): CLR events keep
  their manifest payloads (provider "Microsoft-Windows-DotNETRuntime", same event ids/versions), kernel samples become
  SampleProfiler ThreadSample events with the sampled stack. Result: the same importer (B/C) opens ETW and EventPipe captures.
  Admin/Performance Log Users needed: map ERROR_ACCESS_DENIED to `Denied` so the UI offers "Restart as administrator".
  Pure parts (provider/keyword configs, event header → nettrace record conversion, stack merging, PID filtering) are tested on
  every platform with recorded event fixtures; the Windows-only glue is cfg(windows) and must type-check for
  x86_64-pc-windows-msvc. Real tests (a tiny .NET Framework 4.6.2 console fixture) are #[ignore] unless on Windows with
  IMT_ETW_TESTS=1 and admin.
- **A** (EventPipe, .NET Core/5+) stays, second priority: same Recording API, so D drives both through one trait
  `Collector { list_processes(); start(pid, preset, out, limits) -> Box<dyn Recording> }` exposed by a small
  `crates/imt-dotnet` facade that picks EventPipe for .NET Core processes and ETW for .NET Framework processes (process
  detection on Windows: clr.dll vs coreclr.dll loaded; on macOS/Linux only EventPipe).
- **C** must decode CLR events from the event payload alone (provider + id + version + payload bytes + stack), independent of
  the container, so ETW-converted and EventPipe captures behave the same.
- DotnetProcess gains `runtime: 'framework' | 'core'` and `needsElevation: boolean`.

## Notes for D
- Missing rights (`Denied`, or `needsElevation` on a process): reuse `app/src/app/tools/elevation-hint.ts` (button
  "Restart as administrator" → LifecycleService.restartElevated, Rust `system::restart_elevated`, Windows only) and
  `tools/tool-errors.ts` (isRightsError). Do not duplicate them.
- Verification on real Windows is MANUAL by the user: add a section to docs/testing-manual.md ("Profile a .NET Framework
  4.6.2+ app": CPU, allocations, heap snapshot, two snapshots diff, non-admin → Restart as administrator, stop/cancel,
  process exit mid-recording, 32-bit target).

## Handoff (2026-10-17): other open work, after the .NET profiler

- Visual QA sweep: `cd app && npx playwright test -c scripts/visual-qa/playwright.config.ts` (screenshots of every view,
  menu and the palette per theme/size in `target/visual-qa`, layout problems in `report.json`). Open findings:
  Metrics/Dashboards end-of-line labels overlap (ECharts `labelLayout: { moveOverlap: 'shiftY' }`), Dashboards CPU legend
  pages instead of wrapping, Alerts tile runs name into time ("errorSpike14 Oct"), Collection schedules sticky save bar
  covers the last rows, Process Monitor shows its validation error before any input, Patterns count bars all the same
  width, Traces "1 spans", Overview time-span card wraps; light-theme and 1024×700 images not reviewed yet; add Settings
  sections and dialogs to the sweep.
- Docking: pinning an auto-hidden panel re-mounts it (the e2e test now waits for it); check whether that flickers.
- Pop-out drag: docking on release uses `pointer_button_down` (macOS/Windows); verify on Windows that dragging a pop-out
  over a guide and holding does not dock until release.
- Then: Angular Material (themed via IMT tokens) + `MatIconRegistry` IMT icon set, macOS native menu only, default layout
  opens core views only, process monitor live charts, plugin manager, wave 4 (persistent store spike, Rhai, auto-update),
  final full pass + signed build, merge `feature/log-analysis-ide` into `main`.
