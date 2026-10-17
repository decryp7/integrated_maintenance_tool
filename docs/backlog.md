# Engineering backlog

Follow-ups from the 2026-10-17 practices review, by priority. P0 items are done
(user-regex limits, line-size cap, cargo-deny, npm audit, fmt check in CI, action digest
pinning via Renovate, least-privilege capabilities and strict CSP).

## Next features (agreed order)

1. **Projects, export and import**
   - `.imtproj` (versioned JSON): opened paths relative to the project file, removed files,
     embedded or referenced profile, filters/search/group-by/time range/zoom history,
     columns, tabs, selection, bookmarks and notes on events. New / Open / Save / Save As /
     Recent, unsaved-changes prompt, file association.
   - Exports via an `Exporter` port: filtered events to CSV and CLEF (re-importable), charts
     and timeline to PNG/SVG, incident report (HTML/PDF), shareable `.imtz` bundle (project +
     profile + logs, optionally only the filtered range). Streamed with progress and cancel.
   - Import: open project, open bundle, import/export profile, drag-and-drop.
   - Persistence behind a `ProjectStore` port.
2. **Process monitor plugin (system tray)**
   - `imt-monitor` crate behind a `Collector` port; tracks processes by name pattern, path
     or PID; writes CLEF "Process stats" (CpuPercent, WorkingSetMb, PrivateMb, Threads,
     Handles, IO) and "Process started/exited" events to rotating files that IMT opens
     directly (no extractors needed; start/stop markers show crashes and restarts).
   - Sources: Win32 process APIs / macOS libproc (default, via `sysinfo` or `windows`);
     optional PDH performance counters listed in config (use `PdhAddEnglishCounter`, map
     `name#n` instances to PIDs); later .NET EventPipe runtime counters (GC, exceptions/sec,
     thread-pool); optional ETW for exact process start/exit when elevated.
   - Tauri tray icon (start/pause recording, choose processes, interval, open IMT, quit),
     close-to-tray, optional autostart. CPU% normalised to the whole machine by default.
   - Later: headless CLI / Windows service mode for servers.
   - **Elevation, least privilege:** never require admin for the whole app (parsers handle
     untrusted input; UIPI blocks drag-and-drop into elevated windows). The monitor runs as a
     separate helper that is started elevated only when tracking protected processes or
     using ETW (UAC `runas` on Windows, administrator prompt on macOS). Windows menu item
     "Restart as administrator" for admin-only log folders, with an "Administrator" badge and
     a drag-and-drop hint while elevated. "Access denied" states offer "Retry as administrator".

## P1: soon

- **Fuzzing:** `cargo-fuzz` target per importer (json, text, csv) and for `Profile::from_toml`;
  run on a nightly schedule with a short time budget; commit the corpus.
- **Property tests:** `proptest` "never panics" for importers and timestamp parsing;
  invariants for query paging (pages partition the result) and chart bucketing.
- **Snapshot tests:** `insta` for importer output on the samples.
- **Mutation testing:** `cargo-mutants` on `imt-core` (scheduled, `--in-diff` on PRs).
- **Benchmarks:** `criterion` on the samples scaled up (10M events); store results as CI
  artifacts and compare.
- **Non-blocking startup:** paths passed on the command line are opened before the window
  shows; open them in the background after startup, emit a `workspace-changed` event the UI
  listens to, and show progress.
- **Streaming and cancellation:** import progress and cancellable long queries via Tauri
  `Channel` + cancel token.
- **Typed errors:** `thiserror` error enums in `imt-core`; serializable `{ code, message }`
  errors at the command boundary.
- **Logging:** `tracing` with rotating files; no file contents in logs by default.
- **UI end-to-end:** Playwright flows against the mock backend; WebdriverIO + `tauri-driver`
  smoke test on Windows/Linux CI.
- **Lint:** angular-eslint flat config, TypeScript `strict` + `noUncheckedIndexedAccess`.
- **Process:** Conventional Commits check, `git-cliff` changelog, lefthook pre-commit
  (fmt, clippy, eslint), `.editorconfig`, Gitea PR template.
- **Signing:** Windows Authenticode, macOS Developer ID + notarization (needs a self-hosted
  Mac runner with credentials).
- **Updates:** `tauri-plugin-updater` with signed manifests; keep the signing key backed up.
  Check current Tauri security advisories for the updater before enabling it.

## P2: later

- Persistent store (DuckDB) behind `LogStore`; SQL console tab.
- Importer plugins: registry exports, msinfo `.nfo`, Windows event logs.
- SBOMs (`cargo-cyclonedx`, `@cyclonedx/cyclonedx-npm`) attached to releases.
- Opt-in crash reports and an "export diagnostics" action.
- WCAG 2.2 AA review (grid keyboard model, chart table alternatives), i18n readiness.
