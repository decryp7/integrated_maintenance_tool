# IMT roadmap

IMT aims to cover the whole maintenance loop: **collect** diagnostics from a machine,
**analyse** them across instances and sources, **explain** what went wrong, and **share** the
result. Phases are ordered by value and by what later features build on. Engineering
follow-ups (signing, accessibility review, i18n of engine text) and the open items of the 2026-10 redesign plan are
at the end of this page.

## Phase 1: make analyses reusable *(done)*

- **Projects:** `.imtproj` save/load with sources, profile, filters, time range, zoom history,
  columns and tabs; recent projects; file association. *(Done.)*
- **Notes** anchored to events, time ranges, sources, the project and regions of images, listed in a Notes panel and
  kept in the project ([ADR 0021](adr/0021-notes.md)). Bookmarks became **Saved queries**. *(Done.)*
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

- **Process monitor** with system-tray mode: CPU, memory, handles and I/O per
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

**Status.** Done: the profiling engine and views over imported captures, event-based **.NET capture** with ETW and
EventPipe ([ADR 0019](adr/0019-dotnet-profiling.md); no CLR Profiling API agent), **native (C/C++) capture** by ETW
or suspend-and-sample with PDB symbols ([ADR 0022](adr/0022-native-profiling.md)), `.etl` import, and **crash dumps
and UMDH heap logs** with problem-first findings and a dump-capture tool ([ADR 0023](adr/0023-dumps-and-umdh.md)).
The deep-mode agent, `.gcdump`, WebView2 and Linux/macOS native collectors described below remain planned or dropped
as marked.

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
  the engine validates and previews before the user applies them. The AI never searches the web
  itself: the SearXNG and Brave web search it used to offer was removed in favour of the internal
  browser below.
- **Internal web browser** *(Done, [ADR 0011](adr/0011-internal-browser.md); replaces the AI's web search)*:
  - A sandboxed browser window that the user drives: *Search the web for this error* opens the
    user's chosen engine.
  - *Add page to AI context* hands the visible page's text to the model as cited, untrusted
    evidence. The AI can also read along (opt-in per chat) and suggest links that the user opens.
  - **Agent mode:** the AI drives the visible browser (navigate, click, type, read) at human pace,
    with a Stop button. Approval is needed for new sites, typed text and form submissions, and
    workspace data headed into a URL or form is highlighted. CAPTCHAs and logins go to the user,
    and every action is logged.
  - No stealth or bot-detection evasion. Remote pages get no IPC at all: page results come back as the
    return values of scripts IMT evaluates, each checked against a one-shot nonce, the expected origin and
    a size cap. *(Not yet verified on macOS hardware.)*
  - **Vision for agent mode** *(planned, not started)*. The local model can already read images in the chat
    ([ADR 0025](adr/0025-vision-one-model.md): one model plus its `<model>.mmproj.gguf` projector). Sending a
    screenshot of the browser panel alongside the DOM snapshot is the next step; it helps on visually complex pages
    (canvas content, image buttons, layout cues). Text-only models keep working as today. It stays local; no cloud model.
- **Jobs and notifications** *(Done, [ADR 0012](adr/0012-jobs.md))*: every long task (AI requests, research,
  imports, long queries and analyses, exports, downloads, survey, storage clean-up, collections) is a job
  with per-kind queues and limits enforced in Rust, a Jobs pane with progress, Cancel, Retry and View, toasts
  and OS notifications for long jobs, history saved with the project, and a notification centre for what needs
  attention (deduplicated, capped, saved per project and per app).
- **Research mode** *(Done, ADR 0012 amending 0011, ADR 0024)*: the AI researches the web by itself, read-only: it
  picks the results from the engine's results page (Google, DuckDuckGo, Bing or a custom engine), reads 10, 20 or 30
  of them (default 10) over the next results pages, skips CAPTCHA, login and payment pages, and answers with a
  citation per claim; a confirmation when the search text contains workspace data. A live search test checks the real
  engines in the real app (nightly and manual only, [development.md](development.md#live-search-test)). *(The real
  browser flow is not yet verified on every webview, macOS in particular.)*
- **Knowledge files** *(Done, [ADR 0029](adr/0029-knowledge-files.md))*: Markdown notes about the user's systems that
  every AI feature reads, per project and global.
- **Redaction / PII masking** rules (emails, tokens, IPs, custom regexes) applied to views,
  exports and bundles. *(Done: `[redact]`, redacted view, redacted exports, reports and bundles.)*
- **Incident report builder:** pick bookmarks, charts and notes into a narrative report.
  *(Done: HTML and PDF export, saved in the project.)*
- **Dashboards:** arrange charts, counters and tables per profile (e.g. "OrderService
  health"). *(Done: `[[dashboard]]`, edit and wall modes.)*

## Phase 6: scale and platform

- **Persistent store** behind the store ports: data larger than RAM, instant reopen, background
  indexing with progress and cancel, projects that work without the original logs. *(Done: ADR 0014, phase 6c. Every
  window runs on the DuckDB store in the store worker process, with Tantivy word search; the in-memory workspace is no
  longer used by the app. The original spike, DuckDB against DataFusion + Parquet on 10M events, was removed once its
  results were in ADR 0010.)* Still open: a SQL console tab for power users, and moving to DuckDB 2.0's server mode once
  it is released and supported.
- **Very large UIs**: scaled grid scrolling (50M rows), virtualised facet lists and trees. *(Done.)*
- **Plugins:** Rust plugins on `imt-plugin-api`, each in its own sandboxed process over one IPC (ADR 0017; the gRPC `imt.plugin.v1` contract and the C# SDK it replaced are gone).
  *(Done.)* **Plugin SDK and signed packages** *(Done, [ADR 0027](adr/0027-plugin-sdk.md))*: `cargo imt-plugin`, the
  test harness, `.imtplugin` packages with ed25519 signatures, the Plugins panel and trust settings; see
  [plugin-sdk.md](plugin-sdk.md). ~~WASM components and a plugin catalogue~~ *(dropped 2026-10-18).*
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
  - WCAG 2.2 AA review and opt-in crash reports: see "Engineering follow-ups" below.

## Engineering follow-ups

Done in the 2026-10-18 pass, for reference: security and supply chain (user-regex and line-size limits, cargo-deny,
npm audit, least-privilege capabilities, strict CSP, Renovate, SBOMs, generated release notes, signed auto-update),
tests (fuzzing, proptest, insta snapshots, criterion, cargo-mutants, Playwright, WebdriverIO + tauri-driver),
robustness (typed errors, cancellable queries, import progress, rolling logs without file contents) and process
(Conventional Commits check, lefthook, TypeScript `strict`). Still open:

- **Signing:** Windows Authenticode and macOS Developer ID + notarisation (needs certificates and a self-hosted Mac
  runner). The auto-update key pair and secrets: see [releasing.md](releasing.md).
- **WCAG 2.2 AA review:** axe runs WCAG 2.2 A/AA rules in every e2e check and passes, target size (2.5.8) included
  (dense timeline markers share one target; annotation bands are reached through the "Evidence..." menu). Still to do
  by hand: focus-not-obscured (2.4.11) with docked and auto-hidden panels, chart table alternatives everywhere, a
  screen-reader pass (NVDA, VoiceOver).
- **Opt-in crash reports:** local capture exists (crash marker and restore, error snapshots, "Report a problem"
  bundle). Uploading needs a server and a privacy review; not planned.
- **i18n of engine text:** settings schema labels, error messages, analysis sentences and AI text come from Rust in
  English; they need message ids (the UI already parses one of them, the `pipeline step N (kind):` prefix, which first
  needs a structured error code).
- **Linux/macOS checks:** code paths only reviewed on Windows (llama-server child lifetime on Linux/macOS, macOS runtime
  signing, non-Windows stubs of the Windows plugins, the Linux and macOS sandboxes) are covered by the Linux and macOS
  CI runners only. A macOS CI job waits for a runner with the `macos-latest` label.
- **Batch queries:** the monitor live charts (11 charts) and multiple timelines send one request each; a batched
  `query_charts` / `query_timelines` would save round-trips if they show up as slow.

## Open items from the 2026-10 redesign plan

The plan (four architecture reviews, phases R0 to R6) is done except for the items below. IMT is pre-v1, so each is
done by redesigning, not by compatibility code.

- **R0, not verified:** the "R" row (lock recovery in analysis caches, caches keyed by generation plus hash, an event
  size cap with a diagnostic, reporting when a cap drops records) needs a pass; U2 to U4 (pointer moves, resize
  debouncing, stores that stop querying when their view closes) are described in the responsiveness rules of
  [development.md](development.md) but have no regression test named for them.
- **R3 (ADR 0017, "Deferred"):** split `imt-ai` into `imt-ai-proto`, `imt-ai-runtime`, `imt-ai` and
  `imt-browser-agent` with one `SafeHttp` and the assistant on the `Query`/`Analysis` ports; one redactor
  (`imt_core::redact`) shared by AI privacy and child-log scrubbing; `imt-perfmon` and the event-log follower as
  libraries instead of through `imt-collect`/`imt-monitor` (the tools plugin still links `imt-core`); importer plugins
  streaming batches to the store worker directly with credits (needs postcard store DTOs instead of JSON bodies).
- **R5, the UI:**
  - ports mirroring ADR 0014 (`QueryPort`, `IngestPort`, `AnalysisPort`, `ProfilePort`), one DI token each;
  - `engineResource()` on Angular `resource()` with an `AbortSignal` mapped to a request id and a Rust
    `cancel(requestId)`, replacing the hand-copied stale-response guards;
  - an engine "view" operation (timeline, annotations and runs for one query and generation in one reply);
  - split the large stores and components (`WorkspaceStore` into query, data, timeline, preferences and theme
    stores; `PagedRowsStore` out of the grid; typed `busy` state; import UI driven by its job);
  - domain rules move to the engine: highlight rules (an index per row in `Page`), profile edits through `toml_edit`
    (keeps the user's comments), regex validity and mask preview, lane values instead of parsing lane keys;
  - accessibility: CDK key managers everywhere, the grid announces the active row, one tab stop per timeline lane;
  - mocks become fixtures validated against golden JSON emitted by Rust tests.
- **6b Problems pane** (R2, R5): diagnostics from import with engine-verified quick fixes (including an AI-proposed
  fix). The store already has `Diagnostics` and `evaluate_fix`; the pane is not built.
- **Store performance and promotion (ADR 0014, "Deviations and open points"):** at 10M events several operations are
  still over the 0.5 s target (filtered and word-search pages, timelines with instance lanes, charts, field stats,
  correlate, Moment, runs); field promotion is decided per segment (sticky promotion and re-promotion on compaction
  are open).
- **Format gaps** listed under Phase 3.
