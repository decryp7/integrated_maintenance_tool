# 47. Time-window import

- Status: accepted (2026-10-22)
- Date: 2026-10-22
- Related: ADR 0014 (streaming import), ADR 0042 (port table), ADR 0044 (command line)

## Context

Opening a 20 GB folder to look at one hour is the common case. Everything outside the hour costs memory, disk and
time in the pipeline, the Parquet segments and the full-text index.

## Decision

- `ImportWindow { from, to }` (epoch ms, inclusive, either end open) lives in `debrief-core::ingest`. It is **parsed once**
  by `ImportWindow::new`, which refuses an end before the start.
- It is a **store setting**, `Ingest::set_import_window`, not an argument of `open`: `open`, `reload` and `refresh`
  keep their shape and every caller (CLI, app) sets the window first. It is one new row in the port table (the wire
  moved to version 2 and the snapshot test with it). A store that cannot filter answers `Unsupported`; the in-memory
  `Workspace` is the reference implementation without it. Live tail ignores the window.
- `ImportEnv` carries the window; `RecordBatcher::with_window` drops events outside it **before** the profile's
  pipeline and the writer and counts them (`BatchedSource::outside_window`). Events without a time are outside any
  window (they cannot be placed).
- Skipping what cannot match: a file whose modification time is more than 26 hours before `from` is not opened
  (log times are often local and clocks can be a day off). This is the only skip by file; there is no seeking inside a
  file because timestamps are not guaranteed monotonic across formats, so events are still parsed, then dropped.
- The CLI exposes it as `--since` and `--until`.

## Consequences

- The window applies to files read after it is set; a kept store keeps what it already holds, so changing the window
  means `clear` and reading again.
- The app offers it as *Open a time range…* in the Explorer: after the native picker a small dialog asks for the UTC
  range, the shell command `set_import_window` sets it, the paths are opened and the window is cleared again.
- Seeking by binary search over sorted text files is a possible later optimisation behind the same setting.
