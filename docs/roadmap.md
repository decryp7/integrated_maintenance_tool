# IMT roadmap

IMT aims to cover the whole maintenance loop: **collect** diagnostics from a machine,
**analyse** them across instances and sources, **explain** what went wrong, and **share** the
result. Phases are ordered by value and by what later features build on. Engineering
follow-ups (fuzzing, signing, etc.) live in [backlog.md](backlog.md).

## Phase 1: make analyses reusable *(done)*

- **Projects:** `.imtproj` save/load with sources, profile, filters, time range, zoom history,
  columns and tabs; recent projects; file association. *(Done.)*
- **Bookmarks and notes** on events and time ranges, listed in a side panel and kept in the
  project. *(Done.)*
- **Export:** filtered events (CSV, CLEF), charts and timeline (PNG/SVG), incident report
  (HTML/PDF), shareable `.imtz` bundle (project + profile + logs). *(Done.)*
- **Saved queries and filter presets**, plus **highlight rules** (colour rows matching a
  condition, e.g. `ElapsedMs > 5000`). *(Done.)*
- **Context view:** jump from an event to the surrounding lines of its source file, or ±N
  events of the same instance. *(Done.)*
- **Compressed and archived input:** read `.gz`, `.zip` and rotated log sets transparently. *(Done.)*

## Phase 2: find the problem faster *(done)*

- **Query language** with autocomplete and syntax help (fields, operators, functions), e.g.
  `level >= Warning and RequestPath ~ "/capture" and ElapsedMs > 5000`, alongside the
  point-and-click filters.
- **Correlation:** follow a `CorrelationId` / `TraceId` / `RequestId` across instances and
  sources; trace waterfall for spans (OpenTelemetry), request duration view (Gantt).
- **Exception grouping:** fingerprint by type and normalised stack; counts, first/last seen,
  affected instances; jump to occurrences.
- **Pattern mining:** cluster similar messages into templates (Drain algorithm) so 1M lines
  become 200 patterns; show patterns that are new or spiking in a time window.
- **Anomaly highlights:** error-rate spikes, sudden silence (instance stopped logging),
  crash/restart summary, slow-request outliers, marked on the timeline.
- **Compare windows:** "before vs. after deploy" or "good vs. bad instance": what patterns,
  errors and metrics changed.
- **Field statistics:** histograms and percentiles for numeric fields, top values, value
  over time; calculated fields from expressions. *(Done: the `calc` pipeline step.)*
- **Clock-skew correction:** shift a source's timestamps by an offset (or align on a
  shared event) so multi-machine timelines line up. *(Done: manual, aligned on an event, or
  suggested from shared correlation ids; ADR 0006.)*
- **Live tail:** follow growing files (rotation-aware) with the timeline scrolling live. *(Done.)*

## Phase 3: more sources (importer plugins)

- Windows: **Event Logs** (`.evtx` and live), **registry exports** (`.reg`, tree view and
  diff), **msinfo32** (`.nfo`, tree view), Process Monitor CSV/PML, ETW `.etl`, IIS / W3C
  logs, SQL Server ERRORLOG, minidump summaries. *(Done: `.evtx` files and the live Event Log
  follower, `.reg`, `.nfo`, IIS / W3C, Process Monitor CSV, SQL Server ERRORLOG, minidump summaries.
  Planned with profiling: ETW `.etl`, Process Monitor PML.)*
- Cross-platform: syslog / journald, Apache/Nginx access logs, NLog/log4j XML layouts,
  OpenTelemetry (OTLP JSON), Docker/Kubernetes logs, HAR files. *(Done.)*
- ~~Remote sources (SSH/SFTP, SMB, Seq, Elasticsearch, Loki, Application Insights)~~ *(dropped
  2026-10-18: out of scope; copy logs locally or use the diagnostics collector.)*
- **Diff view** for configuration-like data (registry exports, JSON/XML configs, msinfo of
  two machines). *(Done: JSON, XML, INI, TOML, YAML, `.reg`, `.nfo`, and folder snapshots.)*
- **Format gaps found with real-world samples** (October 2026, still open):
  - `.tar` / `.tar.gz` archives.
  - journald export format and `json-pretty` output.
  - CSV files whose first row is data, not a header (e.g. HealthApp).
  - MongoDB 4.4+ JSON logs with `{"$date": …}` timestamps.
  - Logstash events whose `message` is itself nested JSON.
  - Templates for PostgreSQL, MongoDB and Redis logs.
  - W3C `s-computername` mapped to the machine name.
  - SQL Server ERRORLOG time zones in the core text importer.

## Phase 4: collect, not just read

- **Process monitor** with system-tray mode (see backlog): CPU, memory, handles and I/O per
  process; Windows performance counters; .NET runtime counters (GC, exceptions/sec, thread
  pool); process start/exit and crash detection; elevated helper only when needed. *(Done: process
  monitor with tray mode, live charts, Windows counters via PDH, and .NET Framework runtime counters.
  .NET Core runtime counters need EventPipe and come with profiling.)*
- **Diagnostics collector:** one action (or a small standalone/CLI collector to run on a
  customer machine) that gathers app logs, event logs, msinfo, selected registry keys, a
  performance-counter snapshot and environment info into an `.imtz` bundle, with redaction. *(Done.)*
- **Scheduled collection** and a headless/service mode for servers. *(Done: `imt-agent`, ADR 0005;
  the Windows service is type-checked here and exercised on the Windows CI runner only.)*

## Phase 4b: profiling

Profiler plugins = a collector (start/stop capture with the platform's official tools) plus an
importer; IMT adds two generic views shared by all of them: a **flame graph** (CPU stacks, from
speedscope / collapsed-stack formats) and an **allocation table with snapshot diff**.

- **.NET Framework 4.6.2+ (primary target)**, two levels feeding the same views:
  1. *ETW (default, production-safe, no restart):* CLR provider events via a session started by
     IMT — sampled CPU stacks, GC start/end and heap sizes, AllocationTick (sampled ~100 KB),
     exceptions, contention, JIT; heap snapshots (`.gcdump`, as PerfView takes them). A small
     Windows-only helper using TraceEvent converts traces to the flame-graph/allocation formats.
  2. *CLR Profiling API agent ("deep mode", dev/test):* a native profiler DLL
     (`ICorProfilerCallback`, loaded via `COR_ENABLE_PROFILING`/`COR_PROFILER`/
     `COR_PROFILER_PATH`; attach to running processes with reduced callbacks) streaming exact
     allocation stacks, method enter/leave timings, GC heap-graph snapshots (leak paths) and
     exceptions to IMT. Caveats shown in the UI: one profiler per process (conflicts with APM
     agents), noticeable overhead, a profiler bug can crash the app.
     - *Attach to a running process* (CLR 4+, `ICLRProfiling::AttachProfiler`,
       `InitializeForAttach`, clean `RequestProfilerDetach`): GC events and heap walks (object
       graph → leak paths), CPU sampling via `DoStackSnapshot`, exceptions, JIT/thread/module
       events. Not available after attach (startup-only flags): per-allocation callbacks and
       enter/leave hooks → offer *Launch with profiler* for those. x86 and x64 profiler builds
       (match target bitness); same user or elevated for services/IIS. Verify the
       allowed-after-attach flag set against Microsoft docs during implementation.
  No .NET performance counters in the PerfMon presets (kept generic).
- **.NET (Core / 5+)**: `dotnet-counters` (System.Runtime) onto the timeline; `dotnet-trace`
  CPU sampling → flame graph; `dotnet-gcdump` heap snapshots → top types and diffs.
- **C++ on Windows**: UMDH heap snapshots with allocation stacks (gflags +ust) diffed to find
  leaks; Windows Performance Recorder (`wpr`) CPU traces exported for flame graphs; symbols from
  PDBs / symbol server.
- **C++ on Linux/macOS**: `perf` (CPU) and `heaptrack` (memory) on Linux; `sample`/`xctrace` on
  macOS.
- **Memory snapshots (all profilers)**: take snapshot (on demand, scheduled, or automatically
  when e.g. Private Bytes grows by N MB); compare two snapshots (per type or allocation stack:
  count/bytes before → after, sorted by growth, new types highlighted); compare a series of 3+
  to find steady growth; retention paths to GC roots ("why is this alive"); retained size via
  dominators.
- **WebView2 apps** (WinForms/WPF hosts, incl. .NET Framework 4.6.2): track the
  `msedgewebview2.exe` process family (browser/renderer/GPU by `--type`) as children of the host;
  enable remote debugging via `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` and attach over the Chrome
  DevTools Protocol for console messages, JS exceptions, network requests, periodic
  `Performance.getMetrics` (JS heap, DOM nodes, listeners, documents), JS heap snapshots
  (take/compare) and CPU profiles (flame graph); import runtime logs (`--enable-logging`),
  Crashpad crash reports and the runtime version/user-data-folder health.
- Captures line up with logs and counters on the same timeline (Moment view includes them).

### Profiler analysis UI (shared by every profiler plugin)

*(Done: the engine (`imt-profile`) and every view below except source/line heat, over imported
captures; see [profiling.md](profiling.md). The collectors above are still planned.)*

The engine owns a common profile model (samples or exact events → call tree with per-frame
self/total time, samples, calls, allocations, exceptions, lock waits) and does all aggregation;
the UI shows virtualised, sortable views over it:

- **Hotspots:** functions sorted by any column — self time, total time, self % / total %,
  samples, call count, average and max per call, allocations (count/bytes), exceptions, wait
  time — with group-by (function, type, namespace, module, thread) and search.
- **Call tree** (top-down) and **inverted tree** (bottom-up), with expand-hottest-path,
  recursion folding and "just my code" (collapse framework/system frames into one).
- **Caller/callee (butterfly)** view for a selected function.
- **Flame graph and icicle**, zoomable, with search highlighting and a **differential flame graph**.
- **Thread timeline:** per-thread CPU, GC pauses, JIT, exceptions and lock waits; brushing a range
  re-aggregates every view to that range. The range can also come from the log timeline or a
  Moment ("why was 10:42 slow?").
- **Source/line heat** when symbols and sources are available (PDB/symbol server, dSYM, DWARF).
- **Compare two profiles** (baseline vs regression): per-function deltas, sorted by change.
- **Memory:** top types by count/bytes/retained size, allocation call stacks, snapshot diff,
  growth across 3+ snapshots, dominator tree and paths to GC roots.
- **Automatic findings:** top self-time functions, GC time > threshold, lock contention, thread
  pool starvation, sync-over-async, exceptions in hot paths, JIT storms, regressions vs baseline,
  each linking to the evidence.

Measurement honesty: *sampling* (ETW, EventPipe, perf, `sample`) gives low overhead and
statistical time with estimated rather than exact call counts; *instrumentation* (CLR Profiling
API enter/leave in launch mode) gives exact call counts and timings at higher overhead. The UI
labels which kind each column is.

Import of existing captures as well as IMT's own: `.nettrace` (dotnet-trace/EventPipe), `.etl`
(WPR/PerfView via the Windows helper), `perf script`/collapsed stacks, pprof, Chrome trace /
`.cpuprofile`, speedscope JSON, `.gcdump`. Native collectors run as supervised worker processes
(ADR 0002) and expose the shared profile model through plugin services (ADR 0003). *(Done:
`.nettrace`, `perf script`, collapsed stacks, pprof, Chrome trace / `.cpuprofile`, speedscope and
V8 `.heapsnapshot` in `imt-plugin-profiling`; `.etl` and `.gcdump` need the Windows helper and are
planned.)*

## Phase 5: explain and share

- **AI assistant (local only):** *(Done.)* the official prebuilt llama.cpp `llama-server`
  (downloaded on demand like a model, ADR 0009), driven by a supervised worker process, runs a GGUF model chosen for the machine's hardware (one-click download from Hugging
  Face with checksum verification). It chats about the workspace with read-only engine tools and
  cited evidence, and proposes regexes, profiles, pipelines, markers, extractors and queries that
  the engine validates and previews before the user applies them. Optional web search for error
  codes is off by default, uses engine-built redacted queries and needs approval per request.
- **Internal web browser** *(next; replaces the AI's web search: SearXNG and Brave are removed)*:
  - A sandboxed browser window that the user drives: *Search the web for this error* opens the
    user's chosen engine.
  - *Add page to AI context* hands the visible page's text to the model as cited, untrusted
    evidence. The AI can also read along (opt-in per chat) and suggest links that the user opens.
  - **Agent mode:** the AI drives the visible browser (navigate, click, type, read) at human pace,
    with a Stop button. Approval is needed for new sites, typed text and form submissions, and
    workspace data headed into a URL or form is highlighted. CAPTCHAs and logins go to the user,
    and every action is logged.
  - No stealth or bot-detection evasion. Remote pages get no IPC except a one-shot capture that a
    user click arms.
- **Redaction / PII masking** rules (emails, tokens, IPs, custom regexes) applied to views,
  exports and bundles. *(Done: `[redact]`, redacted view, redacted exports, reports and bundles.)*
- **Incident report builder:** pick bookmarks, charts and notes into a narrative report.
  *(Done: HTML and PDF export, saved in the project.)*
- **Dashboards:** arrange charts, counters and tables per profile (e.g. "OrderService
  health"). *(Done: `[[dashboard]]`, edit and wall modes.)*

## Phase 6: scale and platform

- **Persistent store** behind `LogStore`: data larger than RAM, instant reopen, background
  indexing with progress and cancel, SQL console tab for power users; the in-memory store stays
  for normal sizes. *(Spike done: DuckDB vs. DataFusion + Parquet, each with Tantivy, on 10M events —
  see [store-spike.md](store-spike.md). ADR 0010 (accepted): DuckDB in a supervised worker process with a
  Tantivy index, moving to DuckDB 2.0's server mode once it is released and supported;
  implementation not started.)*
- **Very large UIs**: scaled grid scrolling (50M rows), virtualised facet lists and trees. *(Done.)*
- **Plugin SDK:** out-of-process plugins over gRPC (`imt.plugin.v1`) with the Rust and C# SDKs.
  *(Done.)* ~~WASM components and a plugin catalogue~~ *(dropped 2026-10-18).*
- **Scripting** for custom transforms where profiles aren't enough. *(Done: Rhai `script` step,
  sandboxed and bounded per record.)*
- **Workbench UX:** dockable/split panes, multiple timelines, command palette with every action,
  keyboard shortcut map, layouts. *(Done.)*
- **Distribution:**
  - Auto-update with signed releases. *(Done; needs the signing key pair and CI secrets, see
    [releasing.md](releasing.md).)*
  - Code signing / notarisation. *(Needs certificates and a macOS runner.)*
  - Settings import/export. *(Done.)*
  - i18n. *(Readiness done: every UI string marked, runtime locale loading, pseudo-locale, CI
    check; engine-produced text still English.)*
  - WCAG 2.2 AA review and opt-in crash reports: see [backlog.md](backlog.md).
