# 44. A headless command line over the store worker

- Status: accepted (2026-10-22)
- Date: 2026-10-22
- Related: ADR 0005 (headless agent), ADR 0014 (store worker), ADR 0016 (typed contract, Tauri-free services), ADR 0039 (known issues, incident report)

## Context

Scripts, CI and headless machines need the engine's answers (imports, queries, anomalies, patterns, exceptions, known
issues, the incident report) without the desktop app. The existing command-line tools collect (`debrief-collect`,
`debrief-agent`) or sample (`debrief-monitor`); none analyses. Re-implementing analysis in a second place would break "the
Rust engine is the single source of truth".

## Decision

A new crate `debrief-cli` (library `debrief_cli`, executable `debrief`) drives a `Store` through the ports only:

- The library is generic over a small `StoreHost` port (create or open a store folder). The executable's
  `WorkerHost` starts the same supervised `debrief-store-worker` the app uses (so plugins, memory caps and the DuckDB
  store behave identically); tests use the DuckDB store in process, the adapter the worker wraps.
- Parsing (`args`), rendering (`output`) and time parsing are pure and unit-tested; `run` is the only module that
  talks to the store. Analyses and the report call the same functions as the shell (`match_known_issues`,
  `build_incident_report`, `resolve_in`, the HTML and PDF `ReportRenderer`s).
- The executable's name is the single constant `BIN_NAME`; the product rename changes one line.
- Exit codes are part of the contract: 0 done, 1 failed, 2 usage, 3 findings with `--fail-on-findings`. With `--json`
  every command prints exactly one JSON object (errors too); diagnostics go to standard error.
- Output is bounded (`--limit` capped at 100,000, 500 characters per cell, 4 MB knowledge files). A kept store
  (`--store`) is reusable; without it the scratch store is removed on every path out.
- Reports are written to a temporary sibling and renamed, and never replace a file without `--overwrite`.

## Consequences

- Scripts depend on the JSON shapes, which are the engine's own camelCase DTOs; changing them is a CLI change too.
- The CLI cannot run without the worker binary and libduckdb next to it (or `DEBRIEF_STORE_WORKER`, `DEBRIEF_LIB_DIR`); a
  missing worker is a clear message, not a crash.
- Later commands (error outliers, time-window import, watched folders) are added as subcommands or options of the
  same crate.
