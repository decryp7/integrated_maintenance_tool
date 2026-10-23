# 5. Headless collection agent (`debrief-agent`)

- Status: accepted
- Date: 2026-10-17

## Context

Servers need diagnostics collected on a schedule or when something goes wrong (a memory spike,
an error in a log), without anyone running the Debrief app. The collector (`debrief-collect`) and the
process monitor (`debrief-monitor`) already exist as libraries, and the engine can tail logs.

## Decision

- **A separate crate and binary, `debrief-agent`**, built on the `debrief-collect`, `debrief-monitor` and
  `debrief-core` libraries (and the tools plugin's `logman` controller for PerfMon). The Debrief app only
  edits its config and browses its output through the same library; it never hosts the agent.
- **One versioned config file, `debrief-agent.toml`** (camelCase keys, `version = 1`, newer versions
  refused). It is parsed once into typed values; every problem is reported with its key path
  (`schedules[0].cron`), which the UI shows next to the field. The UI form is driven by a schema
  that the agent library generates. The documented example is `crates/debrief-agent/debrief-agent.example.toml`.
- **Schedules** are cron expressions (`croner`, MIT: POSIX/Vixie five or six fields, aliases,
  `L`/`W`/`#`, local time or UTC), fixed intervals, or triggers (a process metric above a
  threshold for a while, with re-arm and cooldown; a new event matching a query in a tailed log).
  Missed timer slots are caught up once, not once per slot.
- **One run never overlaps itself.** Each schedule takes a lock file (exclusive create) holding the
  owner's PID and process start time; a lock whose owner is gone or whose PID was reused is stale
  and taken over. This also covers a manual `debrief-agent once` racing the service.
- **Every run gets a folder** (`yyyyMMdd-HHmmss-<schedule>`) with a versioned `manifest.json`,
  written at start (`running`) and at the end, atomically. Retention after each run deletes
  only run folders, by count, age and total size, never the current or a running run.
- **Robustness:** free disk space is checked first (sysinfo's disk list, already a dependency) and
  a run is skipped with a log entry when below `minFreeMb`; external tools always run with
  timeouts; a panicking action fails only itself; corrupt `state.json` is set aside and defaults
  are used; the agent logs CLEF to rolling files with size, count and age limits.
- **Services share one loop** (`service::run_service`). OS-specific code is thin: Windows Service
  Control Manager integration via `windows-service` (MIT OR Apache-2.0) with `service install` /
  `uninstall` (administrator rights required, explained when missing); generated systemd units
  (with `PrivateNetwork=yes`) and launchd plists that administrators install with the usual tools.
- **No network.** The agent opens no listener and makes no connections; it reads local files and
  runs local tools, and writes only under its output, state and log folders.

## Consequences

- Servers can collect unattended, and the output opens in Debrief like any folder.
- Windows service start/stop has only been type-checked off Windows (`cargo check --target
  x86_64-pc-windows-msvc -p debrief-agent`); its runtime is verified on the Windows CI runner or by hand.
- Config comments are kept when the UI saves the file: `toml_merge` (with `toml_edit`, MIT OR
  Apache-2.0) rewrites only the values whose meaning changed, in place; an unreadable file is replaced
  by a fresh document. Saving over a file other than the one loaded asks first (no silent overwrite).
- The `logman` timeout is `agent.logmanTimeoutSecs` (default and range as the tools plugin's setting).
