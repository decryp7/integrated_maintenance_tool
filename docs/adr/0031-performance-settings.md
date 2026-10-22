# 31. Performance settings: memory caps, CPU, DuckDB, warnings and suggested values

- Status: accepted (2026-10-22)
- Related: ADR 0014 (DuckDB store, memory budgets), ADR 0017 (out-of-process plugins and workers, sandbox),
  ADR 0012 (jobs, notification centre), ADR 0030 (Store pane, SQL console), AGENTS.md (engine is the source of truth, validate in Rust, versioned
  persistent state, bounded resources)

## Context

Debrief runs its work in child processes: the store worker (DuckDB, the parse threads, the full-text index writer), one
process per plugin, and the AI worker. Each has a memory cap the host enforces with its memory watcher
(`debrief_ipc::process`, the same on every platform): the store worker 2 GiB (`DEFAULT_MEMORY_CAP`), plugins what their
manifest asks for (1 GiB otherwise), the AI worker none. The store's knobs (DuckDB memory limit and threads, parse
threads, index writer) were constants tuned for the budgets of ADR 0014 (768 MB importing, 300 MB querying), and the
only user setting was *Low memory mode* (Settings ▸ Advanced ▸ Performance, `performance.json`, the worker's
`--low-memory`). Users on small machines (8 GB with a local model loaded) and on large ones (32 GB, 16 cores) need
other values, and when a cap stops a process the user saw only an error in the panel that failed.

## Decision

1. **One section, Settings ▸ Performance** (its own navigation group), served by the app from a Rust schema like every
   section (`app/src-tauri/src/performance/`). It replaces Advanced ▸ Performance. `performance.json` moves to format 2;
   an older, damaged or newer file falls back to the defaults (no migration before v1). Fields, all validated in Rust
   (ranges by the schema, relations by the section's check, see 6):
   - **Memory**: the store worker's hard cap (2048 MB), its import budget (768 MB) and query budget (300 MB), the cap of
     plugins that declare no need of their own (1024 MB; a plugin that declares more keeps its own; a per-plugin
     override, set in the Plugins panel, wins over both), the AI worker's own process (1024 MB; llama-server, which
     holds the model, is a separate process and not counted), and *Give freed memory back at once* (the allocator
     behaviour of the old Low memory mode).
   - **CPU**: parse threads (0 = automatic), DuckDB query threads (4), index writer threads (2), the plugins' CPU share
     (100%) and *leave N cores free* (1). The CPU share is exact on Windows (job object `cpu_rate_percent`, a hard
     cap); on Linux and macOS a share below 100% only lowers the plugins' priority further (nice 19 instead of 10).
     The help text says so.
   - **DuckDB (advanced)**: memory limit (0 = from the query budget), spill folder (empty = the store's `tmp` folder),
     max spill size (`max_temp_directory_size`, 0 = DuckDB's default), allocator flush threshold (32 MB) and
     allocator background threads (on). Each field has a plain-language explanation.
2. **Budgets are inputs, knobs are derived, by one pure function** (`StoreTuning::from_budgets`): DuckDB's limit is
   the query budget minus the worker's base (108 MB; 300 → 192 MB), parse threads are `cores − free` limited to one
   per 96 MB of the import budget and to 8 (768 → 8), the index writer heap is a twelfth of the import budget
   (768 → 64 MB). The defaults therefore reproduce today's tested values exactly, so the memory tests of ADR 0014
   keep guarding the defaults. Explicit Advanced values win over derived ones.
3. **Low memory mode becomes a preset**, next to *Standard*: buttons in the section that fill the fields (import
   384 MB, query 236 MB → DuckDB 128 MB, 4 parse threads, 2 query threads, 1 index thread, memory given back at once)
   and are applied like any edit. A preset is data computed in Rust (`Preset::values`), not a mode with its own code
   path, so there is no state where the preset and the fields disagree. The worker still accepts `--low-memory`
   (tests, and the allocator switch); the values travel as `--tuning=<json>` (`protocol::StoreTuning`), parsed once at
   the worker's edge, invalid parts ignored with a log line.
4. **Suggest settings** is a pure Rust function of the environment (`performance::suggest`): total and free memory,
   cores, free disk space and its kind when known, and the size of the active AI model, which is subtracted first.
   It keeps a reserve for the system (a quarter of the memory, at least 2 GB), gives the store worker a quarter of what
   is left (1–8 GB), budgets in the tested ratios of the cap, plugins a quarter (0.5–4 GB), turns on *give memory
   back at once* below 3 GB of headroom, keeps cores free (1 up to 8 cores, 2 above), fewer parse threads on a
   spinning disk, and a spill cap of a quarter of the free disk when it is short. It is table-tested for 8 GB/4 cores
   with a model, 16 GB/8 and 32 GB/16. The *Suggest settings* button shows the current and suggested values side by
   side and applies them on request; the first run (no `performance.json`) applies them silently and saves them.
5. **The watcher reports to the host.** `debrief_ipc::process` gets a pure `MemoryWatch` (peak, an 80% warning with
   hysteresis: re-armed below 70%) and a `MemoryListener` in `SpawnSpec` that hears `Warning`, `Stopped` and `Ended`
   events with the use, the peak and the cap. The store client, the plugin host and the AI launcher pass the app's
   listener through. The app turns them into notification-centre messages:
   - at 80% a **quiet** entry (no toast) — "Plugin 'X' is using 420 of 512 MB" — deduplicated per process;
   - when the watcher stops a process, a warning **toast** naming it and why, with *Raise the cap…* (opens
     Settings ▸ Performance or the plugin's override with a suggested value: the peak plus 50%, rounded up to 256 MB)
     and *Retry* (resets the plugin, the AI engine, or lets the store worker start again on next use);
   - the last peak and cap of each plugin process are shown with its health in the Plugins panel.
6. **Relations are checked in Rust** before anything is saved: the import budget and the query budget must fit in the
   cap, an explicit DuckDB limit must fit in the query budget, the spill folder must be an existing folder. Errors are
   field errors, like range errors.
7. **When changes apply.** The section is marked *needs restart*: caps, budgets and threads are read when a process
   starts. The values are handed to the store worker's supervisor at once, so the worker's next start (after Debrief
   restarts, a crash or a cap stop) uses them; Debrief does not restart a worker that may hold open stores and running
   imports on its own.

## Consequences

- The memory tests of ADR 0014 keep guarding the defaults; new tests cover the schema and defaults, damaged files,
  cross-field validation, the derivation, the suggestion table, the worker applying `--tuning`, DuckDB's settings
  after `open_db`, the watcher's thresholds and events, and the notification flow in the UI.
- Settings that give a process more memory than the machine has are the user's choice; Suggest settings is the guard
  rail, and the warning before the stop gives time to act.
- The plugin processes the store worker starts for its imports get the same caps and CPU share (through
  `--tuning`), but their watcher events stay in the worker's log: forwarding them over the worker's IPC is left for
  later.
- The Store pane's SQL console (ADR 0030) keeps its own connection and its locked limits; these settings tune the
  stores' main connection only.
- Opening an exported project applies the memory limit and threads at once (`DuckStore::configure`); the spill
  folder and the index writer apply when a store is opened or created.
