# Profiling engine

The profiler analysis views (roadmap phase 4b) run on one engine, `crates/imt-profile`, a pure
library with no I/O. Importers are in the `crates/imt-plugin-profiling` plugin. The app exposes
the engine per window through `app/src-tauri/src/profiling.rs`. The wire contract is the
"Profiling" section of `app/src/app/core/api.types.ts`.

```
capture file ──▶ importer (plugin, `profiling.importers` point, guarded)
             ──▶ Profile | HeapSnapshot   (ProfileBuilder / HeapBuilder enforce Limits)
             ──▶ Captures (ProfileStore port, per window)
             ──▶ CallTree under a view (range, threads, just my code, recursion; cached)
             ──▶ hotspots · trees · butterfly · flame/icicle · diff · timeline · findings · moment
heap snapshot ─▶ HeapAnalysis (Lengauer–Tarjan dominators, retained sizes)
             ──▶ types · diff · growth · dominator tree · instances · paths to roots
```

## Model

A `Profile` has the following parts:

- **Frames:** interned by function, module, file, line and kind (`managed`, `native`, `kernel`,
  `unknown`), each with a framework flag.
- **Stacks:** a prefix tree in which each node is a frame plus its parent, so a stack is one id.
- **Threads.**
- **Metrics:** `samples`, `timeNs`, `calls`, `allocCount`, `allocBytes`, `exceptions`, `waitNs`,
  `maxCallNs` and `liveBytes` (native heap memory allocated during a capture and not freed by its end). Each one is tagged `sampled` (an estimate) or `exact`. The UI shows this tag next to
  the values.
- **Samples:** stored in columns and sorted by time. A sample is a weighted stack observation,
  with one value per metric.
- **Timeline events:** GC, JIT, exception, lock wait and marker.
- **Wall-clock start:** optional. When present, the capture can be lined up with the logs.

`calls` and `maxCallNs` count for the node itself and are not added up into callers. `maxCallNs` is
combined by taking the maximum.

A `HeapSnapshot` always has per-type count and bytes. When the format records it, it also has an
object graph in compressed sparse row form: objects, references with labels, and GC roots.

## Views

Every view is a pure function of the profile, its `CallTree` under the request's `ProfileView`, and
the request. The store keeps the six most recently built trees, so paging and zooming do not
rebuild them.

The view transforms are:

- **"Just my code":** a run of framework frames collapses into its first (outermost) frame, which
  keeps the time of the whole run. Frames are matched by the importer's flag, by user patterns, or
  both.
- **Recursion folding:** `direct` folds `a → a` into `a`. `all` folds a frame back into the
  ancestor with the same frame.
- **Range and thread filters.**

The rules for totals:

- A hotspot's total counts each sample once, even when the function appears several times on the
  stack.
- The inverted (bottom-up) tree is never built in memory. Its children are found on demand by
  grouping the callers of the call-tree nodes that end in the requested path.
- Comparisons normalise each profile by its own total. Functions are matched by name and module.

Findings are emitted only when the data supports them:

| Finding | Evidence used |
|---|---|
| Top self time | Hottest functions by self time |
| GC pressure | GC event durations, or GC frames |
| Lock contention | Lock-wait events, or lock frames |
| Exception hot path | The exact exception metric, exception events, or throw frames |
| Thread-pool starvation | Only with timestamps, ≥ 4 named pool threads, and a blocked share above the threshold. Thread injection and sync-over-async are reported when seen. |
| Regression vs a baseline | Self share of a function grows by more than the threshold |

Each finding links to its evidence: a view plus a selection.

Navigation and layout details the UI relies on (it never recomputes them):

- **Frame paths** (`profiling_frame_paths`): the hottest top-down paths from a root to a function
  (all frames with its name and module), outermost occurrence only. "Tree" in the hotspots table
  reveals the first path in the call tree; "Focus" (and "Zoom flame graph to it" in the butterfly)
  makes it the flame graph's focus.
- **Flame rectangles** carry `parent`, the index of the enclosing rectangle (`null` at depth 0), so
  keyboard navigation and zoom chains never guess from geometry. The graph also returns
  `focusFrames`, the frames of the requested focus path, which name the zoom breadcrumbs.
- **Sorting by average per call:** `sort: { metric, of: "avgPerCall" }` orders hotspots by the
  metric's total divided by calls; rows without calls come last in both directions. A profile
  without call counts answers `unsupported`.

## Errors

Every profiling command rejects with the one command error `{ code, message, retryable }` (`CmdError`,
ADR 0016; the engine's `ProfilingErrorDto` codes below are part of its `ErrorCode`). The UI turns it into a `ProfilingFailure` (an `Error` with
the code) at the backend edge and decides by code, never by message text:

| Code | Meaning | Engine errors |
|---|---|---|
| `notACapture` | No importer recognised the file | `Unrecognised` |
| `notFound` | The file or the capture does not exist (any more) | missing path, `UnknownCapture` |
| `io` | The file could not be read | other I/O errors |
| `limit` | A size, count or time bound was hit | `LimitExceeded`, too large, too many paths |
| `unsupported` | A feature or data the capture does not have | `Unsupported`, `NotAvailable` |
| `invalid` | The file content or the request is invalid | `Malformed`, `Importer`, `WrongCapture`, `InvalidRequest` |

## Drag and drop

Dropped files are routed once: the workspace offers them to the `DROPPED_FILES_HANDLERS` (multi
token, `app/src/app/core/dropped-files.ts`) and opens only the unclaimed ones as logs. Profiling's
handler calls `profiling_sniff` (first 64 KiB of each file, nothing is opened), opens the captures
and heap snapshots, and shows their real errors (missing file, I/O, damaged capture) in the
Profiles view with Retry. Folders and unreadable paths sniff as `none` and go to the log import.

## Projects and sessions

Profiling saves its state under the `profiling` key of the project's (and the autosaved
session's) `ui` object: a version, the paths of the open captures, the active profile, heap and
baseline by path, and the view settings (just my code and its patterns, frame flags, recursion,
metric). Every field is optional; a missing or malformed field takes its default, and a value that
is not an object counts as no state. Reopening re-imports the captures through the engine;
captures already open from the same file are kept. Files that no longer exist are listed in the
Profiles view's notice instead of failing the restore. A new project closes every capture.

## Limits

`imt_profile::Limits` sets these defaults:

| Limit | Default |
|---|---|
| Input size (after decompression) | 1 GiB |
| Distinct frames | 1M |
| Call-tree nodes | 4M |
| Samples | 20M |
| Threads | 64k |
| Events | 2M (beyond this they are dropped and noted) |
| Stack depth | 4096 frames (deeper stacks keep their outer frames and are noted) |
| Name length | 1024 characters |
| Heap objects | 20M |
| Heap references | 100M |
| JSON nesting | 128 |

Queries are paged with at most 10,000 rows per page. A flame graph has at most 200,000 rectangles
and a timeline at most 4,000 buckets. Searches and patterns have capped length and regex size.
Paths to roots stop after 2M visited objects. At most 32 captures can be open per window.

## Importers

| Id | Format | Notes |
|---|---|---|
| `speedscope` | speedscope JSON | Sampled → `timeNs`/`samples` (or `allocBytes`). Evented → exact `timeNs`, `calls`, `maxCallNs`. Each profile becomes a thread. |
| `folded` | collapsed stacks (`stackcollapse-*`) | `_[k]`/`_[j]`/`_[i]`/`_[w]` annotations are honoured. No timestamps. |
| `cpuprofile` | Chrome/Node `.cpuprofile` | Time is attributed per sample as DevTools does. `(garbage collector)` is kept for findings. |
| `chrome-trace` | Chrome trace event JSON | `Profile`/`ProfileChunk` CPU samples. Without those, `X` and `B`/`E` duration events become an exact wall-clock profile. |
| `pprof` | `profile.proto` (gzip or raw) | prost messages written from google/pprof's `profile.proto` (Apache-2.0), no protoc. Sample types map to metrics. Inlined frames are expanded. Threads come from labels. |
| `perf-script` | `perf script` text | Kernel, `[unknown]` and JIT-map frames are recognised. Period → `timeNs` for cpu-clock and task-clock. |
| `nettrace` | .NET EventPipe and IMT ETW captures (NetTrace V4/V5/V6) | Details below the table. |
| `heapsnapshot` | V8 `.heapsnapshot` | Field layout comes from `meta`. Weak and shortcut edges are skipped. Only V8's synthetic root (node 0) is a GC root. |

`nettrace` decodes the following:

- SampleProfiler thread samples.
- Method and module load and rundown events, used to name the frames.
- GC start/end, exceptions, contention start/stop, and JIT.
- `GCAllocationTick` → allocation samples (`allocBytes`, `allocCount`) with the allocated type as the leaf frame.
- Thread-pool worker starts (thread names) and starvation adjustments (markers).
- GC heap walks (`GCBulkType`/`Node`/`Edge` and root events): a trace with one imports as a heap snapshot
  (roots grouped as `[Stack roots]`, `[Finalizer queue]`, `[Handles]`, `[Static variables]`, ...).
- IMT's ETW extension events (`IMT-Windows-Kernel`, ADR 0019 and ADR 0022): image loads (native frames as
  `module+0xoffset`), context-switch waits (`wait` events, `waitNs`), thread names, `NativeSymbol` (native
  frames named by function, with module, source file and the function's first line) and the native heap events
  `HeapAlloc`/`HeapFree`/`HeapReAlloc` (paired by address: `allocBytes`/`allocCount` at the allocating stack,
  exact; what is not freed by the end becomes `liveBytes`). Addresses resolve in this order: CLR methods,
  native symbols, images, `0x…`.

It follows PerfView's `EventPipeFormat.md` / `NetTraceFormat.md` and the runtime's `ep-file.c` and
`ep-block.c`. It reads NetTrace V4/V5 (FastSerialization objects) and V6 (.NET 10 blocks, threads by
index). Pointer-sized payload fields follow the trace's pointer size (the target's bitness).

Every importer meets these requirements:

- **Format detection:** it sniffs the file content, never only the file extension (exception: `.etl`, whose
  buffer header has no signature, is recognised by its extension and checked when read).
- **No panics:** it never panics on any input.
- **Bounds:** it checks every count against the bytes that remain and against `Limits` before
  allocating.
- **Tests:** it has a proptest "never panics" test (`crates/imt-plugin-profiling/tests/properties_*.rs`)
  and a cargo-fuzz target (`fuzz_prof_*`, see [fuzzing](fuzzing.md)).

Importers run as guarded contributions. A panic becomes an error that names the importer and
counts towards quarantining its plugin. Sniffing runs under the ordinary service budget (10 s);
an import runs on a helper thread under its own budget of 600 s
(`Service::call_detached_within`, `IMPORT_BUDGET` in `app/src-tauri/src/profiling.rs`). Past it
the import is abandoned, the plugin is marked hung until the call returns, and the user gets an
error instead of a frozen window.

An importer declares what it produces with `CaptureImporter::produces` (profiles by default; the
heap snapshot importer says `heap`). `CaptureCatalog::sniff` uses it to tell captures, heap
snapshots and other files apart without importing anything: the best importer's score must reach
`SNIFF_THRESHOLD` (50), so a weak guess stays a log file.

### Deferred formats

- **`.gcdump`** (dotnet-gcdump / PerfView heap snapshots): not read. The format is TraceEvent's
  FastSerialization of a `GCHeapDump` object graph and is documented only by the TraceEvent source. IMT takes .NET
  heap snapshots itself instead: *Profile .NET process…* with the *Heap snapshot* kind records the runtime's heap-walk
  events and builds a `HeapSnapshot` from them ([ADR 0019](adr/0019-dotnet-profiling.md); a NetTrace with heap-walk
  events imports as a heap snapshot). Plugins are Rust only ([ADR 0017](adr/0017-one-ipc-out-of-process-plugins.md)),
  so there is no .NET helper process.
- **`.etl`** (WPR/PerfView/logman): read on Windows by the native profiling plugin (`imt-plugin-native`,
  ADR 0022), which converts one process (.NET or native) to NetTrace with symbols; see
  [Native (C/C++) captures](#native-cc-captures).

Crash dumps and UMDH heap logs are not profiles; they have their own engine (`imt-dump`) and views
([ADR 0023](adr/0023-dumps-and-umdh.md), [features](features.md#crash-dumps-and-umdh-heap-logs)).

## Samples

`samples/profiling/` holds small synthetic captures of a fictional "OrderService": profiler
captures, not logs (`crates/imt-plugin-profiling/tests/samples.rs` checks that every file there
sniffs as a capture or heap snapshot and imports non-empty). They were generated by deterministic
tests, and no third-party captures are included. Never edit them by hand: change the generator,
regenerate with `IMT_REGENERATE_SAMPLES=1` and `--ignored`, then commit the result. Tests fail if
a committed sample no longer matches its generator.

`orders.perf.txt` is kept to about 100 KB (`PERF_SAMPLES` = 120 samples in `gen_pprof.rs`; each
sample is ~800 bytes of text). To change its size, edit `PERF_SAMPLES` and run:

```sh
IMT_REGENERATE_SAMPLES=1 cargo test -p imt-plugin-profiling --no-default-features \
    --test gen_pprof -- --ignored regenerate_samples
```

| Files | Generator |
|---|---|
| `orders.nettrace` | `cargo test -p imt-plugin-profiling --no-default-features --test gen_nettrace -- --ignored` (spec-following writer in `nettrace::writer`) |
| `orders.pb.gz`, `orders-alloc.pb.gz`, `orders.perf.txt`, `orders.folded`, `orders-regression.folded` | `... --test gen_pprof -- --ignored regenerate_samples` (pprof written with the prost messages) |
| `orders*.speedscope.json`, `orders.cpuprofile`, `orders.trace.json`, `heap-{1,2,3}.heapsnapshot` | `... --test gen_json_samples -- --ignored` |

The pairs `orders.folded` / `orders-regression.folded` and `orders.speedscope.json` /
`orders-regression.speedscope.json` show a regression: serialisation grows from about 10% to
about 30%, and `Shop.Orders.AuditLog.Write` is new. In the heap snapshots, `Order` grows steadily
from 50 to 100 to 150 instances, and `AuditLog` first appears in the second snapshot.

## .NET captures

`crates/imt-plugin-dotnet` ([ADR 0019](adr/0019-dotnet-profiling.md)) records a running .NET process without loading
anything into it: ETW for .NET Framework and Core (administrator rights; the recorder runs in an elevated plugin
process), EventPipe over the runtime's diagnostics IPC for .NET Core (no elevation). Presets are *CPU and waits*,
*Allocations* (an allocation sample about every 100 KB, with its type and stack) and *Heap snapshot* (the runtime
induces one GC and describes the heap). The trace is written as NetTrace, so the existing importer and every view
serve it; ETW-only data (waits, native frames as `module+0xoffset`) travels in IMT's `IMT-Windows-Kernel` events.
`imt-etw` holds the ETW decoders, the converter and the `.etl` reader that both this plugin and the native plugin use.
Captures are cancellable jobs with size and duration caps.

## Native (C/C++) captures

*Tools ▸ Profile native process…* records a running native process with the native profiling plugin
(`crates/imt-plugin-native`, ADR 0022). Nothing is loaded into the target. Two modes:

| Mode | Rights | How | What it gives |
|---|---|---|---|
| Sampling | none (your own processes) | the plugin suspends one target thread at a time, copies its registers and the top 64 KiB of its stack, resumes it (an RAII guard resumes on every path) and walks the copy with DbgHelp `StackWalkEx`; 100 Hz per thread by default, 1 kHz at most | CPU samples where a thread's CPU time advanced since its previous sample, `wait` events otherwise (one interval each; the reason is read from the leaf frames: sleep, lock, I/O completion, object wait). Wall clock, approximate |
| ETW | administrator (the plugin's elevated process, started through the administrator prompt) | a system-logger session: kernel `PROFILE` (1 ms) and `CSWITCH`/`DISPATCHER` with stack walks, image loads; optionally the heap provider for the target (allocations with stacks); the CLR providers when the target runs .NET (mixed stacks) | every CPU sample, waits with the thread that ended them, heap allocations and unfreed memory by stack |

Both write NetTrace through `imt-etw`'s writer; after recording, `imt-symbols` names the distinct code addresses
(symbol path from the dialog or *Settings ▸ Native profiling*, the cache `<app data>/symbols` in symstore layout,
and, only when enabled, the Microsoft symbol server) and the capture ends with `NativeSymbol` events. Kernel image
paths (`\Device\HarddiskVolumeN\…`) are mapped to drive paths first. A finished capture opens on *Profile
findings* (problem first).

Native findings: **hot native function** (the user-code function with most self time, with its source location;
standard-library code it calls counts as its own), **wait hot spot** (the user-code function above the longest
waits, locks recognised), **native heap growth** (the call path holding most `liveBytes`). OS and runtime DLLs
(`ntdll.dll`, `kernelbase.dll`, `ucrtbase.dll`, `vcruntime*`, ...) are never reported as the place of a fix.

`.etl` traces (WPR/xperf: `wpr -start CPU -start Heap`, PerfView, `logman`) open through this plugin's importer,
IMT's one `.etl` importer for .NET and native processes: dropping one opens its busiest .NET process (else its
busiest process), *File ▸ Open process from .etl…* lists the processes (.NET first) to choose from. The same
converter and symbolisation run: managed frames from the CLR events, native frames from symbols.

Limits: x64 and x86 (WOW64) targets from the 64-bit IMT; ARM64 targets are not sampled. Sampled stacks stop at
JIT-compiled frames whose unwind data only the runtime knows (managed parts of sampled stacks are truncated; use
ETW for mixed stacks). Captures stop at 1 hour, 1 GiB, or 4M samples; at most 200,000 distinct addresses are
symbolised. Heap tracing slows allocation-heavy targets while it runs.

## Performance

`crates/imt-profile/tests/perf.rs` builds 1M samples over 50k functions, with about 1.19M call-tree
nodes, 16 threads, and stacks 8–38 frames deep. Release build, Apple silicon:

| Operation | Time |
|---|---|
| Build the profile (interning; parsing excluded) | 91 ms |
| Hotspots, cold (builds the call tree) | 162 ms |
| Hotspots, warm | 71 ms |
| Hotspots by type | 91 ms |
| Flame graph layout at 1600 px with search (4,090 rectangles) | 40 ms |
| Inverted flame graph | 21 ms |
| Hot path | 0.7 ms |
| Rebuild with range + just my code | 158 ms |
| Thread timeline | 4 ms |
| Function diff | 237 ms |
| Findings with a baseline | 365 ms |

Debug builds run a fifth of the samples, with budgets 20× looser.
