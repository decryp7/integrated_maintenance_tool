# 19. .NET profiling: event-based capture through ETW and EventPipe, one NetTrace path

- Status: accepted (2026-10-20)
- Related: docs/dotnet-profiling-plan.md (the brief and the wire contract), ADR 0012 (jobs), ADR 0004 (languages),
  docs/profiling.md (the profile model and views), AGENTS.md (pure core / thin edges, robustness)

## Context

IMT opens profiles and heap snapshots of many runtimes, but cannot record a .NET process itself. The primary targets
are .NET Framework 4.6.2+ services and desktop apps on Windows (no EventPipe, no diagnostics port) and, second, .NET
Core 3+/5+ processes on every platform. Users must be able to record CPU (with wait/blocking time), allocations and
heap snapshots of a process that is already running, including 32-bit processes, and then be led to the problems
(problem-first views), not only to raw call trees. Users also have `.etl` traces recorded by PerfView, WPR/xperf or
`logman` and want to open them in the same views.

Classic .NET profilers load a profiler DLL into the target (`ICorProfiler` attach or launch with
`COR_ENABLE_PROFILING`). That changes the target, needs a native DLL per bitness, can crash the target, and is
disallowed in many production environments.

## Decision

1. **Event-based only. Nothing is ever loaded into the target process.** Capture uses only what the runtime and the
   OS already publish: **ETW** on Windows (.NET Framework 4.6.2+ and .NET Core) and **EventPipe** over the .NET
   diagnostics IPC (.NET Core 3+/5+, every platform). In-process profiling (ICorProfiler attach/launch, injection,
   IL rewriting) is **out of scope**; any future need for it requires a new ADR.
2. **A built-in plugin, one interface.** All of it is the plugin crate `crates/imt-plugin-dotnet`
   (`net.decryptology.dotnet`), not shell code. Its pure core (no imt-core, no Tauri; what the fuzz targets build)
   holds `nettrace` (the streaming writer), `etw` (pure kernel/CLR event decoding and the ETW → NetTrace converter,
   plus the cfg(windows) real-time session and `.etl` reader glue) and `eventpipe` (pure IPC protocol plus a thin
   named-pipe/Unix-socket transport). Both backends sit behind one `Collector` / `Recording` interface (start,
   bytes, elapsed, stop, cancel); the facade picks the backend per process: .NET Framework → ETW, .NET Core →
   EventPipe (detected from the loaded runtime module, `clr.dll` vs `coreclr.dll`, or the diagnostics port). The
   plugin adapter (feature `plugin`) contributes through the existing registry only:
   - the tool **`dotnet-profiler`** (`processes`, `start`, `status`, `stop`, `cancel`, `etl-processes`,
     `etl-convert`), reached through the generic `tool_invoke`; a running capture or conversion is reported as a
     tool task, which the shell already turns into a cancellable collection Job (Cancel = the tool's `stop`);
   - the **`.etl` importer** on `profiling.importers`, so dropping or opening an `.etl` file goes through the
     existing file-open path (it picks the .NET process with the most CPU samples and notes the others; the
     tool's `etl-processes` / `etl-convert` let the user pick another).
   Findings and views are computed in Rust in `imt-profile`; the CLR payload decoders live with the nettrace
   importer in `imt-plugin-profiling`, independent of the container. When plugins move out of process (redesign
   R3) the plugin migrates unchanged; the ETW collector then runs elevated in its own plugin process instead of
   IMT being restarted as administrator.
3. **One file format, one importer.** Every capture is written as a **NetTrace V5** file. EventPipe streams it
   natively; the ETW collector and the `.etl` reader convert their events through a production **streaming NetTrace
   writer** (`imt_plugin_dotnet::nettrace`): CLR events keep their manifest payloads under their provider names and event
   ids/versions; kernel CPU samples become `Microsoft-DotNETCore-SampleProfiler/ThreadSample` events with the sampled
   stack. What NetTrace has no event for is written under the provider **`IMT-Windows-Kernel`** (documented in
   `imt_plugin_profiling::nettrace::ext`): `ImageLoad` (base, size, file name: module + offset for addresses no
   method covers) and `ThreadWait` (a context-switch wait: duration, wait reason, readying thread, with the stack at
   which the thread blocked). The existing nettrace importer therefore opens ETW, `.etl` and EventPipe captures the
   same way, and the decoders work on provider + id + version + payload + stack alone, independent of the container.
4. **Bitness is recorded, not assumed.** Pointer-sized payload fields (`win:Pointer` in the CLR manifest: heap
   addresses, type handles, root ids) follow the target's bitness, which the trace header's pointer size records:
   EventPipe writes it; the ETW converter takes it from the event header flags (32/64-bit). The 64-bit IMT records
   **x86 and x64 targets** through ETW (WOW64 stacks keep the 32-bit user frames). Stacks keep user-mode frames only
   (kernel and WOW64 thunk frames are dropped) so they fit the target's pointer size.
5. **CPU, waits and runtime events (ETW).** One real-time session per capture ("IMT-dotnet-<pid>-<n>", system-logger
   mode, Windows 8+) enables the kernel profile, context-switch, ready-thread, process/thread and image-load flags
   with stack walks on samples and context switches, and the CLR runtime provider filtered to the target PID with
   keywords per preset (Loader, Jit, Exception, Contention, GC, Threading; GC verbose for allocations; the heap-dump
   keywords for snapshots, which make the CLR induce a GC and walk the heap). At stop, the CLR rundown provider is
   enabled for method/module names and the session ends after `DCEndComplete` (or a timeout). Orphaned IMT sessions
   (a crash) are stopped at the next start; a session is always stopped on drop.
6. **Rights.** ETW needs an administrator (or a member of Performance Log Users): `ERROR_ACCESS_DENIED` maps to
   `DotnetError::Denied`, the UI shows the existing "Restart as administrator" flow (`elevation-hint.ts`,
   `system::restart_elevated`). EventPipe needs no elevation (same user as the target).
7. **Heap snapshots** stop by themselves once the induced GC's `GCEnd` arrives after heap-walk events (ETW), or when
   the stream has been quiet for ~1.5 s after data (EventPipe); hard cap 120 s. The heap-walk events (`GCBulkType`,
   `GCBulkNode`, `GCBulkEdge`, `GCBulkRootEdge`, static, CCW and conditional-weak-table roots) build a
   `HeapSnapshot` through `HeapBuilder` (types, objects, references, roots); retained sizes and dominators come from
   the existing heap analysis. A nettrace with heap-walk events imports as a heap snapshot.
8. **Allocations**: `GCAllocationTick` (sampled about every 100 KB) with its stack → `allocBytes`/`allocCount`
   samples, the allocated type as a synthetic leaf frame.
9. **`.etl` import** (Windows only; elsewhere "ETL import needs Windows"): the ETW consumer API reads the file
   (`OpenTraceW` in file mode + `ProcessTrace`), a first pass lists the processes it contains (with sample counts),
   the user picks one, and the same converter writes a NetTrace that opens as usual. Managed frames resolve from CLR
   method/rundown events, native frames as `module+0xoffset` from image loads; PDB symbolisation is a later,
   separate resolver step over the same image-load records (planned with the native profiler).
10. **Shell and UI.** Captures and `.etl` conversions run as cancellable Jobs with a size cap and a duration cap; the
    UI ("Profile .NET process…") is problem-first: engine-computed findings link into the existing views.
    With ADR 0016: a capture is a Job of kind `capture` (several run at once, beside a diagnostics collection);
    cancelling it is `jobs_cancel` on its job (found with `tool_task_job`), which discards the file, while the
    tool's `stop` operation keeps it. The dialog polls the tool's `status` through the quiet `tool_poll` (only
    operations a tool declares pollable). Missing ETW rights are a typed `ToolFailure::denied`, so the command fails
    with the `denied` code and the "Restart as administrator" hint; finished files are tool outputs the window may
    open.
11. **Robustness.** Decoders never panic (fuzz targets for every new decoder), every table and file is bounded,
    connect/read/rundown steps have timeouts, every failure path (access denied, refused connection, garbage reply,
    peer closing mid-stream, size limit, timeout, truncated `.etl`) has a test. The Windows glue type-checks on CI
    through `cfg(windows)`; real ETW recordings need an elevated machine and run as an ignored test.

## Consequences

- No target is ever modified; captures are safe for production processes, at the price of sampled CPU data
  (1 ms default ETW / EventPipe interval) and allocation ticks instead of exact allocation counts.
- One importer and one set of views serve every .NET capture; ETW-only data (waits, native frames as module+offset)
  appears through the documented `IMT-Windows-Kernel` events.
- ETW recording requires elevation; EventPipe does not. Real-machine ETW verification is manual (an ignored test
  with instructions), CI covers the pure parts on every platform.
- NetTrace V6 (.NET 10) is read as well as V4/V5; IMT itself writes V5.
- The test-only nettrace writer in `imt-plugin-profiling` stays for malformed-input tests; production writing lives in
  `imt-plugin-dotnet` (different jobs: one builds arbitrary, also broken, files; the other streams valid ones within limits).

## Implementation notes (R3, 2026-10-21)

The plugin moved onto `imt-plugin-api` and is served by `imt-plugins` in its own process (ADR 0017), with the
`WriteFolders` permission (normal integrity: EventPipe connects to the target's diagnostics channel, captures go to
the data folder) and a 3 GiB memory cap. Its tool and the `.etl` importer cross the contract (the importer as a
profile importer whose capture crosses encoded with `imt_profile::wire`; `.etl` paths are typed path arguments
checked against the window's grants).

ETW is the first **elevated plugin process**: the tool's `start` keeps the capture list in the normal process, and
for a .NET Framework target asks the host to run the tool's elevated operations `etw-start` / `etw-poll` /
`etw-stop` / `etw-cancel` in the plugin's elevated process (`HostRequest::InvokeElevated`; the host accepts only
operations the manifest declares elevated). The host starts that process through the administrator prompt on first
use instead of restarting IMT; a declined prompt is the typed `denied` failure. The elevated process writes only into
`<data folder>/captures/dotnet` (the data folder comes from the host, the file name is checked), records only .NET
processes and exits with IMT. EventPipe, process listing and `.etl` conversion stay unelevated.

The manual ETW test is unchanged (it exercises the recorder directly):
`IMT_ETW_TESTS=1 cargo test -p imt-plugin-dotnet --test real_runtime -- --ignored --nocapture` from an elevated prompt.
The elevated route itself is tested with the elevated role started as an ordinary process
(`imt-plugin-host`'s `the_normal_process_runs_only_declared_elevated_operations_through_the_host`, and the
`etw-*` operations against a fake recorder in `imt-plugin-dotnet`); the prompt is a manual check.

Amendment (2026-10-21, ADR 0022): the ETW decoders, the ETW → NetTrace converter, the NetTrace writer and the `.etl`
reader are the shared crate `imt-etw`; this plugin keeps only its recording control. `.etl` traces open through the
native profiling plugin's importer and its *Open process from .etl…* picker (one importer for .NET and native
processes, .NET processes first); this plugin's `.etl` importer and its `etl-processes` / `etl-convert` operations are
deleted.
