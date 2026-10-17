# 2. Small core, everything else a plugin; native code out of process

- Status: accepted
- Date: 2026-10-17

## Context

IMT keeps growing: Windows tools (PerfMon, process monitor, diagnostics collector), extra
file formats, analysis views, error snapshots, a local LLM assistant, and soon profilers.
Each feature added to the core makes it harder to keep the core stable, and a bug in any of
them can take the whole app down. Some features run native code (llama.cpp, PDH, ETW, the CLR
profiling API) whose crashes (segfault, abort, out-of-memory) cannot be caught by Rust's
`catch_unwind`.

## Decision

**Microkernel.** The core is what every session needs. Everything else is a plugin that can be
disabled, quarantined and left out in safe mode:

| Core (`imt-core`, app shell) | Plugins |
|---|---|
| Workspace/`LogStore`, record model, built-in text/JSON/CLEF/CSV importers, archives | Extra formats: registry, msinfo, EVTX, BLG, PerfMon CSV, W3C/HAR, OTel, syslog/journald |
| Profiles, pipelines, markers, query language, timeline/grid/facets/charts | Tools: PerfMon control, process monitor, diagnostics collector |
| Plugin host, guard/quarantine, safe mode, session recovery, settings host | AI assistant (local LLM), web search for it |
| IMT's own logging, error snapshots, crash detection | Profilers (.NET, C++), WebView2 diagnostics |
| | Analysis extras where they are optional (traces/requests); exporters beyond CSV/CLEF |

The engine `Plugin` contract grows contribution points only by addition (`API_VERSION` stays
compatible): importers, transforms, exporters, hooks, collectors, settings sections, and
**tools/commands** (typed request/response operations the shell dispatches generically and
guards). The UI mirrors it: every feature is a `provide…()` bundle of views and commands,
gated by its engine plugin's status; safe mode loads only the core UI.

**Process isolation by risk.**

- *Safe Rust plugins* run in-process behind the guarded adapters (panics become errors;
  repeated crashes quarantine the plugin).
- *Native or crash-prone code* runs in a **supervised worker process** (Tauri sidecar)
  speaking versioned JSON-RPC over stdio: the first is `imt-ai-worker` (llama.cpp); profilers
  and other native integrations follow the same pattern. The supervisor applies timeouts,
  restarts with backoff, quarantines after repeated crashes, kills workers on exit (Windows
  Job objects; stdin close on Unix) and runs them at reduced priority.
- External tools (`relog`, `logman`, `wevtutil`) are already separate processes and always
  run with timeouts.
- *Third-party plugins* run sandboxed in WebAssembly (wasmtime: memory limit, epoch/fuel
  interruption), so they can neither crash nor hang IMT, or in the plugin host process below.
- *Plugin host process* (`imt-plugin-host`, like VS Code's extension host): any Rust plugin can be
  switched to run out of process (per-plugin setting, automatic after a quarantine, and for every
  plugin in "isolated mode"). Calls and record batches cross the boundary as length-prefixed
  binary messages; the host is supervised like a worker. First-party Rust plugins default to
  in-process for speed, because in-process guarding catches panics but not everything: stack
  overflow and out-of-memory abort the process, and a hung call cannot be pre-empted (only
  abandoned). Built-in parsers therefore also carry recursion/size limits, fuzzing and a
  watchdog that abandons and quarantines calls exceeding their time budget.

### Wire protocol for out-of-process plugins

All workers and the plugin host share one transport:

- **Control plane: gRPC** (`tonic` + `prost`) over **local transports only**: Unix domain
  sockets in a per-user 0700 directory, Windows named pipes with a current-user ACL. There is
  no TCP port, so no network exposure and no firewall prompts. The supervisor generates a
  one-time token, passes it to the child on startup, and every call must present it.
  gRPC gives streaming, deadlines, cancellation, health checks, schema evolution, and plugins
  in any language (C#, C++, Python), which matters for the .NET tooling (TraceEvent) and C++
  profilers. Protobuf schemas are compiled with pure-Rust tooling, so no `protoc` is needed to
  build.
- **Data plane: Apache Arrow IPC record batches** for bulk tabular data (imported records,
  samples, allocations). Arrow is columnar and language-neutral, and it can be zero-copy. Its
  IPC format already uses FlatBuffers for metadata, so IMT doesn't need a separate FlatBuffers
  schema. Batches stream inside gRPC messages, the Arrow Flight pattern. The same batches feed
  the planned DataFusion/Parquet store.
- **Shared memory only where measured necessary:** for very large transfers (huge imports,
  heap snapshots), a memory-mapped region is handed to the child by handle (not by a guessable
  name), with ownership hand-off and sequence numbers, so a worker crash mid-write is
  detected and the batch discarded. `iceoryx2` is the candidate if a ring buffer is needed.
- Small, latency-insensitive traffic (LLM tokens, status) uses plain protobuf messages.
- **Decision (2026-10-17): gRPC is the one transport** for all out-of-process plugins, workers
  and data. Bulk data is batched (Arrow batches, never per-record calls), which keeps gRPC's
  per-call overhead negligible. Raw sockets, Cap'n Proto and shared memory were considered and are
  kept out unless a benchmark shows a batched gRPC path is more than ~10% slower than in-process
  for a real workload.

## Consequences

- A crash in a plugin or worker costs that feature, never the session; the user sees which
  feature failed and can retry, reset or disable it.
- Features can be developed, tested and released independently; safe mode is genuinely
  minimal.
- Some cost: IPC serialisation for workers, an extra binary per worker to bundle and sign,
  and generic dispatch for plugin commands (kept typed with serde at both ends).
- Migration is incremental: AI is built this way from the start; the Windows tools move from
  direct shell calls to a tools plugin next; profilers start as worker plugins.
