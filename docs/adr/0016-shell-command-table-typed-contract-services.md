# 16. Shell: command table, typed contract, services

- Status: accepted
- Date: 2026-10-20

## Context

The shell review of 2026-10-20 (redesign phase R4 of the 2026-10 plan) found that the Tauri shell had
grown by accretion: 225 commands, half of them sync (run on the main thread), listed in one macro and again,
by name, in three classification tables of the activity log; three error contracts (plain text, the AI's
`{code, message}`, the updater's `{kind, message, nextStep}`) told apart by substring matching; several
registries that could each cancel some work; paths from the webview checked in five places; and no runtime
rule about which window may call what (a pop-out panel or the "AI is browsing" banner could call all 225).

Debrief is pre-v1, so the shell is redesigned rather than patched; nothing below keeps compatibility code.

## Decision

1. **Path grants everywhere** (follow-up of R0). Every command that reads or writes a path the webview names
   checks it against the calling window's grants (`paths.rs`): files opened, projects, bundles, profiling
   captures, local AI models and runtime archives, the agent configuration, profile versions (a plain file
   name, `paths::child`). Grants come only from Rust: native dialogs, drag and drop, the command line and
   second launches (pending opens are granted as the window takes them), the window's project file, Debrief's
   own outputs (an unpacked bundle, a tool's reported `bundle`/`dir`, the demo samples, the autosaved
   session, the captures an opened project reopens) and **the recent lists, which Rust now owns**
   (`recent.rs`, `recent.json`): an entry is recorded only after a granted open or save, and is granted
   again when it is reopened from the welcome page, the Recent projects dialog or the tray.

2. **Jobs are the one cancel authority.** Imports, streamed queries, research, AI set-ups and runtime
   installs and the update download are jobs, and the wire has one way to stop work: `jobs_cancel(id)`.
   `cancel_query`, `cancel_import`, `ai_setup_cancel`, `ai_runtime_cancel`, `browser_research_stop` and
   `updates_cancel` are gone. Commands that start work return its job (or, for a streamed query, send it
   first on the channel); `updates_discard` drops a *downloaded* update, which is not a cancel. A job's stop
   action runs on its own thread and the caller waits at most two seconds; shutdown stops all at once.
   Window state (visible, focused) is read on the main thread (`ui_dispatch::run`), never from a worker.

3. **One error type.** Every command rejects with `CmdError { code, message, retryable, field?, line?,
   hint?, detail? }` (`error.rs`), one `ErrorCode` list for all commands (general codes plus the AI's,
   profiling's and the updater's). Domain errors keep their types and convert once at the command edge.
   Reasons are typed where they arise instead of being recovered from text: path refusals are `notGranted`,
   overwrite refusals `fileExists`, tool failures carry `ToolFailure` (`denied`, `missing`) from the tool,
   profile compile errors carry `RuleContext` (which rule, hence which line). The UI's `invoke` turns every
   rejection into a `CmdError` (`core/cmd-error.ts`); views show `message` and `hint` and offer Retry only
   when `retryable`.

4. **One command table.** `command_table!` in `lib.rs` declares each command once: its kind (query, mutation,
   quiet), the windows that may call it (`MAIN`: `main` and `w-*`; `POPOUT`: `p-*`; `BANNER`: `aib-*`) and
   its largest request. It generates the invoke handler and the activity log's classification; `commands::guard`
   enforces the window classes and the request size before arguments are parsed (a pop-out cannot restart
   Debrief or answer its host's prompts; the banner can only read what drives its browser and stop it). Every
   command is async; a test fails on a sync one. The CSP adds `object-src 'none'`, `base-uri 'self'`,
   `form-action 'none'` and `frame-ancestors 'none'`, and the capabilities list the `core:*` permissions
   the UI uses instead of `core:default`.

5. **Quitting Debrief** is one command (`quit_app`) behind File ▸ Exit and Debrief ▸ Quit Debrief (⌘Q, replacing the
   system Quit so the unsaved-changes prompt is never skipped; ADR 0008 amended).

6. **Typed wire, generated.** The shell's wire types derive ts-rs (in tests) and are generated into
   `app/src/app/core/generated/`; `api.types.ts` re-exports them. `cargo test -p debrief-app --lib generated_types`
   fails when a generated file is stale (`DEBRIEF_WRITE_TYPES=1` rewrites them). Replies the shell builds are typed
   (e.g. `WorkspaceSummary`); replies that borrow the engine's data under its lock stay JSON until ADR 0014's
   owned replies exist.

7. **Services, not commands calling commands.** Tauri-free services (`services/profile.rs`, `services/project.rs`)
   hold the logic; commands are adapters, and the shell's own callers use services and helpers. The profile has
   one source of truth: it is written to disk and becomes the window's profile only after the window's store
   swapped to it.

8. **Per-window features and configuration.** Features with per-window state register a `WindowScoped` scope
   (`window_scoped.rs`); teardown is one bounded loop (the opened diagnostics reports no longer leak). One
   `ConfigStore` (`config_store.rs`) keeps the shell's files: versioned documents, atomic writes by one writer,
   damaged files kept aside, a cached `Config<T>`, and a `Debounced` writer for frequent writes (notifications,
   never under the centre's lock). `debrief://jobs` sends a versioned delta, and nothing when a window's jobs did not
   change.

9. **An `AppContext` built in `setup`** (folders, run flags; path grants, recent lists, window scopes and the UI
   dispatcher created up front), one set of lock and blocking helpers (`sync.rs`), one way to open a web
   address; `lifecycle.rs` and `settings.rs` lose plugin enablement (`plugin_config.rs`) and the app's own sections
   (`builtin_sections.rs`).

## Consequences

- A new command is one table line; forgetting its kind or window class does not compile, and a sync
  command fails the tests.
- The UI cannot name a file the user did not choose; everything Debrief hands the UI that it may open later is
  granted when Rust hands it out.
- The wire contract changed for errors and for every cancel; the mock backends keep their shapes, the Tauri
  adapters cancel through jobs.

## Not done yet

Recorded so the next phase picks them up: TypeScript generation and golden JSON fixtures for the engine's
types (they need the derive in `debrief-core` and ADR 0014's owned replies); a `ShareService`; `ai.json` and
`local-models.json` on the `ConfigStore` (owned by the AI crate); engine health from engine events
(review #12; `engine_errors.rs` still reads a crash marker in the import reason until R2's events exist).

Done since: plugin tools' path arguments are checked against the window's grants (`tools.rs` passes `WindowGrants`
as the tool context's path policy), and a domain event of an async command keeps its window: the IPC wrapper
remembers the window of the latest 256 operation ids, which `domain_event` looks up when it runs off the dispatch
thread (instead of a task-local, which Tauri's spawned command futures cannot carry).

## Amendment (2026-10-21, ADR 0024)

The `BANNER` window class (`aib-*`) is gone with the banner window; the table's classes are `MAIN` and `POPOUT`.
The internal browser's page webview (`browser-*`) is no window class: nothing of the table is callable from it.
Capabilities are scoped by webview label (`webviews`), since the page is a child webview of a Debrief window.
