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

- **Persistent store (DuckDB)** behind `LogStore`: data larger than RAM, instant reopen,
  background indexing with progress and cancel, SQL console tab for power users.
- **Plugin SDK:** WASM plugins (importers, collectors, views) with a manifest, sandboxed
  and versioned; later a plugin catalogue.
- **Scripting** for custom transforms (e.g. Rhai or JavaScript) where profiles aren't enough.
- **Workbench UX:** dockable/split panes, multiple timelines, command palette with every
  action, keyboard shortcut map, layouts per project.
- **Distribution:** auto-update with signed releases, code signing/notarisation, settings
  import/export, i18n, WCAG 2.2 AA, opt-in crash reports.
