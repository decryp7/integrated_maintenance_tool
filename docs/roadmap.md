# IMT roadmap

IMT aims to cover the whole maintenance loop: **collect** diagnostics from a machine,
**analyse** them across instances and sources, **explain** what went wrong, and **share** the
result. Phases are ordered by value and by what later features build on. Engineering
follow-ups (fuzzing, signing, etc.) live in [backlog.md](backlog.md).

## Phase 1: make analyses reusable (next)

- **Projects:** `.imtproj` save/load with sources, profile, filters, time range, zoom history,
  columns and tabs; recent projects; file association.
- **Bookmarks and notes** on events and time ranges, listed in a side panel and kept in the
  project.
- **Export:** filtered events (CSV, CLEF), charts and timeline (PNG/SVG), incident report
  (HTML/PDF), shareable `.imtz` bundle (project + profile + logs).
- **Saved queries and filter presets**, plus **highlight rules** (colour rows matching a
  condition, e.g. `ElapsedMs > 5000`).
- **Context view:** jump from an event to the surrounding lines of its source file, or ±N
  events of the same instance.
- **Compressed and archived input:** read `.gz`, `.zip` and rotated log sets transparently.

## Phase 2: find the problem faster

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
  over time; calculated fields from expressions.
- **Clock-skew correction:** shift a source's timestamps by an offset (or align on a
  shared event) so multi-machine timelines line up.
- **Live tail:** follow growing files (rotation-aware) with the timeline scrolling live.

## Phase 3: more sources (importer plugins)

- Windows: **Event Logs** (`.evtx` and live), **registry exports** (`.reg`, tree view and
  diff), **msinfo32** (`.nfo`, tree view), Process Monitor CSV/PML, ETW `.etl`, IIS / W3C
  logs, SQL Server ERRORLOG, minidump summaries.
- Cross-platform: syslog / journald, Apache/Nginx access logs, NLog/log4j XML layouts,
  OpenTelemetry (OTLP JSON), Docker/Kubernetes logs, HAR files.
- Remote: fetch from SSH/SFTP and SMB shares; query Seq, Elasticsearch/OpenSearch, Grafana
  Loki and Azure Application Insights and treat results as sources.
- **Diff view** for configuration-like data (registry exports, JSON/XML configs, msinfo of
  two machines).

## Phase 4: collect, not just read

- **Process monitor** with system-tray mode (see backlog): CPU, memory, handles and I/O per
  process; Windows performance counters; .NET runtime counters (GC, exceptions/sec, thread
  pool); process start/exit and crash detection; elevated helper only when needed.
- **Diagnostics collector:** one action (or a small standalone/CLI collector to run on a
  customer machine) that gathers app logs, event logs, msinfo, selected registry keys, a
  performance-counter snapshot and environment info into an `.imtz` bundle, with redaction.
- **Scheduled collection** and a headless/service mode for servers.

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

## Phase 5: explain and share

- **AI assistant (opt-in):** summarise an incident, explain an exception and stack trace,
  suggest likely root causes from correlated events and metrics, turn plain-English
  questions into queries. Runs on the user's own API key; redaction is applied before
  anything leaves the machine, and nothing is sent without consent.
- **Redaction / PII masking** rules (emails, tokens, IPs, custom regexes) applied to views,
  exports and bundles.
- **Incident report builder:** pick bookmarks, charts and notes into a narrative report.
- **Dashboards:** arrange charts, counters and tables per profile (e.g. "OrderService
  health").

## Phase 6: scale and platform

- **Persistent store** behind `LogStore`: data larger than RAM, instant reopen, background
  indexing with progress and cancel, SQL console tab for power users. Spike first: DuckDB vs.
  DataFusion + Parquet (pure Rust), each with Tantivy for full-text search, benchmarked on 10M+
  events; the in-memory store stays for normal sizes. Frequently used fields are promoted to
  typed columns.
- **Very large UIs**: scaled scrolling in the events grid beyond ~1.2M rows (browser element
  height limit), virtualized facet lists and trees.
- **Plugin SDK:** WASM plugins (importers, collectors, views) with a manifest, sandboxed
  and versioned; later a plugin catalogue.
- **Scripting** for custom transforms (e.g. Rhai or JavaScript) where profiles aren't enough.
- **Workbench UX:** dockable/split panes, multiple timelines, command palette with every
  action, keyboard shortcut map, layouts per project.
- **Distribution:** auto-update with signed releases, code signing/notarisation, settings
  import/export, i18n, WCAG 2.2 AA, opt-in crash reports.
