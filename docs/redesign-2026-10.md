# Redesign plan before v1 (2026-10-20)

Four read-only architecture reviews (engine, Tauri shell, Angular UI, all other crates and CI) found about
95 structural issues beyond ADR 0014. IMT is pre-v1, so we redesign instead of patching (no compatibility code).
This page groups them into phases, in order. Each phase is a batch of agent work; every merge is checked again
for needed redesign. Detailed findings: see the phase sections (file:line evidence is in the review notes).

## Status (2026-10-22)

| Phase | Status |
|---|---|
| R0 security, deadlocks, sluggishness | Done for S1-S3 (paths from Rust, `PathGrants`, cleared worker environments), D1-D3 and U1 (see ADR 0016 and docs/architecture.md). U2-U4 and the R row (caches, event size cap) are not verified; kept as open items. |
| R1 hygiene | Done (workspace dependencies, build profiles, dead code removed, the store spike harvested into the store crates, Windows CI job). A macOS CI job waits for a runner. |
| R2 engine | Done by ADR 0014 (6c): the app runs on the DuckDB store worker. |
| R3 IPC and plugins | Done except the four items ADR 0017 lists as deferred. |
| R4 shell | Done (ADR 0016; `ts-rs` generated types). |
| R5 UI | Open: ports per capability, `engineResource()`, the engine "view" operation, splitting the god stores, domain rules into the engine. |
| R6 themes and timeline | Done (ADR 0018; Calm, Console and Precision). |
| 6b Problems pane | Open. |

The open items moved to docs/roadmap.md ("Open items from the 2026-10 redesign plan").

## R0: security, deadlocks, sluggishness (first, small, about 1 day)

Fixes that are safe on today's architecture and matter before anyone else runs IMT.

| # | Issue | Fix |
|---|---|---|
| S1 | `restart_app` / `restart_elevated` pass webview arguments to a new, possibly elevated process (`--export-diagnostics <path> --force` gives an elevated arbitrary write) | reopen paths come from Rust session state; paths only, canonicalised, `--` before them |
| S2 | Webview-supplied paths are read and written without proof the user chose them (`import_profile`, `diff_configs`, `export_profile`, exports, `save_project`); five different validators | `PathGrants` filled only by dialogs, drag-and-drop, CLI and second instance; a `GrantedPath` extractor; one `paths` module |
| S3 | AI worker inherits the app environment | `clear_env` for every worker |
| D1 | Live tail teardown joins its own thread or blocks the main thread | cancel token plus a reaper; the worker never holds the last window `Arc` |
| D2 | Jobs mirror holds a mutex while calling tray/menu APIs that wait on the main thread | one coalescing UI dispatcher (`run_on_main_thread`, no wait), never under a lock |
| D3 | Six sync commands create windows (a documented deadlock on Windows); window state is built under the registry write lock | make them async; build state outside the lock |
| U1 | Every panel ever opened is re-checked after every render (`afterEveryRender(detectChanges)`), hidden tabs included | attach panel views to `ApplicationRef` (dirty tracking) and skip hidden panels |
| U2 | Pointer moves cause app-wide checks and cross-window IPC | throttle hover to one per animation frame; cursor drawn imperatively; throttled pop-out sync |
| U3 | Splitter drag or resize re-queries the timeline and all charts | debounce and round bucket counts; charts size their own buckets |
| U4 | Root stores keep querying after their view closes; grid and inspector fetch on every scroll or arrow key; failed pages retried on scroll | scoped stores; idle-debounced fetches; Retry only on click |
| R | `anomaly_cache.lock().unwrap()`; caches keyed by pointer or JSON text; unbounded multi-line events; silent 2M-record cap | recover locks; generation-plus-hash keys; an event size cap with a diagnostic; report coverage |

## R1: cheap hygiene (about half a day, can run beside R0)

- `[workspace.dependencies]` and `[workspace.package]`; one sysinfo version; `deny.toml` `multiple-versions = "warn"`.
- Build profiles: release `lto = "thin"`, `codegen-units = 1`, `strip`; dev dependencies at `opt-level = 2`.
- Delete dead code: `imt-clr-format`, the `imt-api-perf` service (no consumer), `MonitorPlugin`. Harvest the store
  spike (libduckdb fetch, Tantivy schema, corpus generator, RSS guard) into the new store crates, then delete it.
- CI: a Windows clippy plus nextest job (pipe ACLs, job objects, PDH, event log are untested today) and a macOS
  check. Pin the fuzz nightly; one fuzz cache key per target; tools via `install-action`.
- Mocks are lazy-imported everywhere, so none ship in the Tauri build.
- One sample generator (`xtask gen-samples`).

## R2: the engine (ADR 0014, amended; batch 6, about 3 days)

ADR 0014 stands. The engine review adds what it must also decide:

- **Stable identity:** `EventId(u64)` assigned once, separate from time-order position (about 20 `records[id]`
  sites, reports, tail, paging); `SourceId` persisted with path, archive entry and fingerprint; marker kinds
  persisted per generation instead of positional bits.
- **One row interface:** a crate-level `RowRef` (from `ql::Row`, interned field ids) implemented for Arrow rows;
  conditions, grouping, extractors, transforms (`apply_batch`), redaction and all analyses use it.
- **One predicate language:** `Condition`, `FieldFilter`, `Query.text` and free text are lowered to the QL `Expr`
  at the edge, giving one evaluator and one SQL compiler; one free-text meaning (Tantivy). `Condition` becomes an enum.
- **Chunking needs a file plan:** a sequential sniff (encoding, format, day order, header) and a newline scan
  that gives each chunk its start line, offset and date hint; time-only formats and JSON arrays stay single-chunk.
- **Profile changes are classified:** presentation (no data work), derivation (from `raw`), parsing (stored raw
  lines, or files whose fingerprint matches, else refuse with a message). Raw lines are kept per source, not only
  per event, plus a byte offset per record for source context.
- **Pure timestamp parsing:** a parse environment (reference time, zone) stored in the project; the winning
  format is remembered per source.
- **Typed profile settings** (serde enums, unknown values refused); pipeline steps as a tagged enum; `dedupe`
  per source after sorting with key expiry; lookup tables stored in the project.
- **Bounded:** fields per source, top-K for compare and facets, a capped Drain miner, an event size cap.
- **Owned replies:** no `Page<'_>` borrows across the port.
- **Live tail** becomes a pure follower state machine that emits "replace source S from line L" batches; alerts
  consume appended batches; transforms run once.
- **Plugin guard per batch** with lock-free admission; no per-file file copies or profile recompiles.
- **One archive module** for projects, share bundles and collector bundles (the `.imtz` name is already used),
  told apart by manifest `format`; the database entry streams to disk under its own limits.
- The registry splits into **ingest extensions** (in the store worker) and **host extensions** (in the app).

## R3: one IPC stack, plugins and workers (ADR 0017; batch 7 brought forward, about 2 days)

- **`imt-plugin-api`** crate with no `imt-core` dependency: plugin traits, the ADR 0014 `RecordBatch` schema,
  manifest, frame types, owned ids (`Arc<str>`; no more `Box::leak`). Core depends on it; plugins depend only on it.
- **One frame codec** in `imt-ipc` (length-prefixed postcard: request id, cancel, stream item, typed error) for
  control, plugins, the AI worker and the store worker. Delete gRPC (tonic, prost, protox, three `build.rs`
  files), `proto/`, `sdk/dotnet`, the .NET CI and the gRPC conformance suite; update AGENTS.md (Languages) and
  supersede ADR 0004 and the transport part of ADR 0002.
- **Out-of-process only:** delete in-process plugin linking and the dual isolation mode. Importer plugins are
  supervised by the store worker and stream batches to it directly, with credits and backpressure (no 4 GiB
  buffers, no app hop).
- **One supervisor** (restart hook to restore state), **one child `serve()` loop**, **one launcher** with a
  `SandboxSpec`: job memory and CPU limits and rlimits, a low-integrity or AppContainer token on Windows,
  landlock plus seccomp (no network) on Linux, a sandbox profile on macOS, `clear_env`, and host-opened file
  handles instead of paths.
- **Split `imt-ai`** into `imt-ai-proto` (worker messages), `imt-ai-runtime` (downloads and installs through one
  `SafeHttp`), `imt-ai` (assistant, chat, tools) and `imt-browser-agent`. The worker links only proto and ipc.
  The assistant takes the `Query` and `Analysis` ports.
- **One redactor** (`imt_core::redact`) composed by AI privacy and child-log scrubbing.
- **Libraries, not plugin-to-plugin dependencies:** `imt-perfmon` and the event-log follower become libraries
  used by plugins and the agent. In-app collectors push batches through `Ingest` (CLEF files only for the
  headless agent).
- `imt-profile` and its importers move into a worker, capped by bytes.
- Fuzz targets for every frame decoder and the worker HTTP parser; mutants and coverage for `imt-ipc`, the host
  and the store worker.

Status (2026-10-21, branch `r3/ipc-plugins`): done except the `imt-ai` split, the redactor, the `imt-perfmon` and
`imt-eventlog` libraries, importer plugins under the store worker (with ADR 0014 6c) and postcard store DTOs; see
ADR 0017, "Implementation notes". `imt-plugin-dotnet` is migrated, its ETW recorder the first elevated plugin process.

## R4: the shell (new ADR 0016, about 2 days)

- **One command table** (attribute macro): kind (query or mutation), window class allowed, payload limit, async
  only. It generates the handler list, the activity classification, the Tauri `AppManifest` ACL (pop-outs get a
  smaller set) and a size guard. Logging uses a task-local span with outcome and duration.
- **Typed wire:** request and response DTOs everywhere (no `Value`), TypeScript generated with ts-rs or specta,
  a CI diff check, and golden JSON fixtures emitted by Rust tests.
- **One error type:** `CmdError { code, message, retryable, field, line }`; no substring classification; typed
  reasons from `imt-core`.
- **Services, not commands calling commands:** Tauri-free `ProjectService`, `ProfileService`, `ShareService`
  and others (the `ai::core` pattern). Commands are one-line adapters.
- **`WindowScoped` features** register create and destroy hooks; teardown is one loop (fixes the `OpenedReports`
  leak).
- **One `ConfigStore`:** versioned, atomic, cached; one place for plugin enablement.
- **Jobs are the only cancel authority**; events send versions or deltas, not full snapshots; persistence off
  locks and debounced.
- Engine health comes from engine events, not summary diffs; the profile has one source of truth (the store
  generation; persisted after the swap).
- Capabilities: list `core:*` permissions explicitly; harden the CSP (`object-src 'none'`, `base-uri`,
  `form-action`, `frame-ancestors`).
- An `AppContext` built in `setup` replaces lazy services and process statics; split `settings.rs` and
  `lifecycle.rs`.

## R5: the UI (about 2 days)

- **Ports mirror ADR 0014** (`QueryPort`, `IngestPort`, `AnalysisPort`, `ProfilePort`, ...), one DI token each.
- **`engineResource()`** on Angular `resource()` with an `AbortSignal` mapped to a request id and a Rust
  `cancel(requestId)` that interrupts DuckDB; it replaces 63 hand-copied stale-response guards in 28 files.
- **An engine "view" operation:** timeline, annotations and runs for one query and generation in one reply;
  pop-outs reuse host results; lazy query effects.
- **Split the god stores and components:** `WorkspaceStore` into query, data, timeline, preferences and theme
  stores, with side effects in app initialisers; `PagedRowsStore` out of the grid; the timeline and dock service
  split by concern; typed `busy` state; the import UI driven by its job (no polling, no label matching).
- **Domain rules move to the engine:** highlight rules (an index per row in `Page`), profile edits through
  `toml_edit` (keeps the user's comments and order), regex validity and mask preview, lane values instead of
  parsing lane keys.
- **Accessibility:** CDK key managers everywhere; the grid announces the active row; the timeline has one tab
  stop per lane with arrow keys between markers.
- **Mocks become fixtures** validated against the golden JSON from R4, not a second engine.

## R6: themes and the timeline (ADR 0018, about 2 days)

- **Theme engine:** token sets plus traits, density as its own setting, Settings ▸ Appearance.
- **Calm (default) and Console** themes, light and dark, with bundled fonts; Precision follows after the engine
  switch (6c), which provides per-lane error counts and source line numbers.
- **No hard-coded styles:** component styles use tokens only, enforced by lint.
- **Timeline redesign** from tokens and traits; overlapping spans as stacked, labelled sub-tracks.
- **Checks per theme and scheme:** colour-vision and contrast checks, axe, and screenshot tests.

## Order and estimate (2 agents at a time)

| Phase | Depends on | Estimate | Visible result |
|---|---|---|---|
| R0 + R1 | – | about 1 day | no known security holes or deadlocks; UI noticeably smoother |
| R2 (batch 6) | R1 | about 3 days | flat memory, fast import, shareable projects, word search |
| R3 (batch 7) | R2's batch type | about 2 days | sandboxed plugins and workers, one IPC stack, smaller builds |
| R4 | R3 | about 2 days | typed contract, safe commands, one config and error model |
| R5 | R2, R4 | about 2 days | cancellable, coalesced requests; engine-owned rules |
| R6 themes + timeline | R5 | about 2 days | three switchable themes (two first); a polished timeline with stacked spans |
| Notes (ADR 0021) | 6c | about 1 day | a note on anything (a log line, a time range, a lane, a finding, a rule, a profiler frame), stored in the project |
| 6b Problems pane | R2, R5 | about 1 day | diagnostics with quick fixes and AI fix |

About 11 working days in total, plus CI rounds. R0 starts as soon as batch 5 is merged.
