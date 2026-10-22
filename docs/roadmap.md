# IMT roadmap

IMT aims to cover the whole maintenance loop: **collect** diagnostics from a machine, **analyse** them across instances and
sources, **explain** what went wrong, and **share** the result. This page lists what is still open; what shipped is in
[features.md](features.md), [CHANGELOG.md](../CHANGELOG.md) and the [ADRs](architecture.md#architecture-decisions).

## Shipped

Kept short on purpose; each line links to where it is described.

| Area | Done |
|---|---|
| Reuse and share | Projects (`.imtz`, [ADR 0014](adr/0014-database-first-engine.md)), notes ([0021](adr/0021-notes.md)), saved queries and highlight rules, context view, archives, export (CSV, CLEF, PNG/SVG, HTML/PDF reports, bundles), redaction ([0007](adr/0007-reports-pdf-and-redaction.md)), a redacted project copy ([0041](adr/0041-redacted-project-copy.md)), dashboards |
| Finding the problem | Query language, correlation and traces, exception grouping, pattern mining, anomalies, comparisons, field statistics and calculated fields, clock-skew correction and live tail on the store ([0006](adr/0006-clock-skew-applied-on-load.md), [0037](adr/0037-store-clock-skew-and-live-tail.md)), runs, Moment, known issues, the incident summary and playbooks ([0039](adr/0039-known-issues-incident-summary-playbooks.md)) |
| Sources | Windows (EVTX, registry, msinfo32, Process Monitor, IIS, ERRORLOG, minidump), syslog/journald, web server logs, XML layouts, OpenTelemetry, Docker/Kubernetes, HAR, configuration diff, per-line mixed formats ([0020](adr/0020-mixed-sources.md)), the parser library and AI-generated parsers ([0026](adr/0026-parser-library.md), [0032](adr/0032-ai-parser-generation.md)), text encodings ([0040](adr/0040-text-encodings.md)), paste as log, video sources ([0033](adr/0033-video-sources-ffmpeg-runtime.md)) |
| Collecting | Process monitor, Event Log follower, Performance Monitor sets, the diagnostics collector, the headless agent (`imt-agent`, [0005](adr/0005-headless-agent.md)) |
| Profiling and dumps | The profiling engine and views, .NET capture by ETW and EventPipe ([0019](adr/0019-dotnet-profiling.md)), native capture ([0022](adr/0022-native-profiling.md)), `.etl` import, crash dumps and UMDH ([0023](adr/0023-dumps-and-umdh.md)) with PDB symbols by identity and symbol servers ([0036](adr/0036-dump-symbols-by-pdb-identity.md)) |
| AI | Local assistant, vision, knowledge files ([0029](adr/0029-knowledge-files.md)), the AI's Chromium browser and Research ([0011](adr/0011-internal-browser.md), [0043](adr/0043-chromium-renderer.md)), jobs and notifications ([0012](adr/0012-jobs.md)) |
| Platform | DuckDB store with Tantivy search ([0014](adr/0014-database-first-engine.md)), the Store pane and SQL console ([0030](adr/0030-store-pane-and-sql-console.md)), performance settings ([0031](adr/0031-performance-settings.md)), the store port table ([0042](adr/0042-store-port-table.md)), out-of-process Rust plugins ([0017](adr/0017-one-ipc-out-of-process-plugins.md)) and the signed plugin SDK ([0027](adr/0027-plugin-sdk.md)), Rhai scripting, auto-update, settings import/export, i18n readiness |
| Workbench | Docking, multiple timelines and windows, command palette, themes including high contrast, the shortcut editor, view areas and perspectives ([0035](adr/0035-view-areas-and-perspectives.md)), Home and the timeline minimap, search everything, native error snapshots ([0034](adr/0034-native-window-capture.md)), What's new |

Dropped on purpose: remote sources (SSH, SMB, Seq, Elasticsearch, Loki, Application Insights; 2026-10-18), WASM plugins and a
plugin catalogue (2026-10-18), the CLR Profiling API "deep mode" agent, and the AI's own web search (replaced by the browser).

## Open: more sources

- **Format gaps found with real-world samples** (October 2026):
  - `.tar` / `.tar.gz` archives.
  - journald export format and `json-pretty` output.
  - CSV files whose first row is data, not a header (e.g. HealthApp).
  - MongoDB 4.4+ JSON logs with `{"$date": …}` timestamps.
  - Logstash events whose `message` is itself nested JSON.
  - Templates for PostgreSQL, MongoDB and Redis logs.
  - W3C `s-computername` mapped to the machine name.
  - SQL Server ERRORLOG time zones in the core text importer.
- **Process Monitor PML** files.
- **macOS FFmpeg**: no publisher offers LGPL macOS builds with stable URLs and checksums, so on macOS the user provides `ffmpeg`
  ([ADR 0033](adr/0033-video-sources-ffmpeg-runtime.md)); pin one when a trustworthy source exists.

## Open: profiling

- `.gcdump` import and capture (`dotnet-gcdump`), and `dotnet-counters` (System.Runtime) onto the timeline.
- **Source/line heat** in the profiler views when symbols and sources are available (PDB, dSYM, DWARF).
- **Linux and macOS native collectors**: `perf` and `heaptrack` on Linux, `sample` / `xctrace` on macOS.
- **WebView2 apps**: track the `msedgewebview2.exe` family under the host and attach over the Chrome DevTools Protocol for
  console messages, network requests, `Performance.getMetrics`, JS heap snapshots and CPU profiles.
- **Snapshot automation**: take heap snapshots on a schedule or when Private Bytes grows by N MB.
- **Vision for agent mode**: send a screenshot of the AI's browser window alongside the DOM snapshot
  ([ADR 0025](adr/0025-vision-one-model.md) already reads images in the chat). Local only; text-only models keep working.

## Open: platform

- **Store**: moving to DuckDB 2.0's server mode once it is released and supported.
- **Signing and notarisation**: needs certificates and a self-hosted Mac runner (see [releasing.md](releasing.md)).

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
  CI runners only. The Mac development machine also serves as the `macos-latest` runner, so macOS jobs run there.
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
  - domain rules move to the engine: highlight rules (an index per row in `Page`), profile edits through `toml_edit`
    (keeps the user's comments), regex validity and mask preview, lane values instead of parsing lane keys;
  - accessibility: CDK key managers in the lists and trees that still handle arrow keys by hand;
  - mocks become fixtures validated against golden JSON emitted by Rust tests.

  Done since the plan: the workspace stores are split (query, data, timeline, selection, preferences, theme,
  status), `PagedRowsStore` is out of the grid, the busy state is typed (`WorkspaceBusy`), the import UI follows its
  job, the grid names its active row (`aria-activedescendant`), timeline lanes have one tab stop each, and the
  Problems pane (6b) is built: import diagnostics by file and kind with counts (tab title, status-bar badge), jumps
  to the lines, engine-checked quick fixes (`evaluate_fix`, applied as a `[format]` change of the source's parser
  through `parser_draft`), and "Ask the AI to fix" through the existing parser proposal.
- **Store performance and promotion (ADR 0014, "Deviations and open points"):** at 10M events several operations are
  still over the 0.5 s target (filtered and word-search pages, timelines with instance lanes, charts, field stats,
  correlate, Moment, runs); field promotion is decided per segment (sticky promotion and re-promotion on compaction
  are open).
- **Format gaps** listed under Phase 3.
