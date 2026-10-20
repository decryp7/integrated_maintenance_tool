# 14. Database-first engine: DuckDB is the only store, imports stream in parallel, projects are self-contained

- Status: accepted (2026-10-20)
- Date: 2026-10-20
- Amends: [ADR 0010](0010-persistent-store.md) (DuckDB stays the engine, Tantivy the full-text index, a worker
  process hosts both; the in-memory `Workspace` is no longer kept as the default for normal sizes)
- Related: ADR 0002 (worker processes), ADR 0004 (plugins), ADR 0012 (jobs)
- Extended by the redesign plan [redesign-2026-10.md](../redesign-2026-10.md), phase R2 (stable ids, one row
  interface, one predicate language, file plans for chunking, classified profile changes)

## Context

Users import folders of several GB. Today:

- **Memory.** `Workspace` keeps every event as a `Record` (`String` message, optional template and exception,
  a JSON field map) in a `Vec<Record>`. An 8 GB folder took 31 GB of RAM and the UI was sluggish afterwards.
- **Whole files in memory.** `Importer::import(ctx, content: &str)` receives the entire decoded file, so a
  2 GB file is a 2 GB string before parsing starts.
- **One lock per window.** Each window holds `RwLock<Box<dyn LogStore>>`. Imports, reloads, live tail and profile
  changes take the write lock; every read (grid page, timeline, validation) waits. Sync Tauri commands run on the
  main thread, so a read that waits for the lock freezes every window.
- **Features read records directly.** Patterns, exceptions, anomalies, compare, correlate, traces, requests,
  spans, AI context and exports iterate `&[Record]`, so they need all records in RAM.
- **A wide port.** `LogStore` has about 50 methods covering ingest, queries, analysis, profiles, plugins, alerts and
  live tail. An adapter has to implement all of them and every consumer depends on all of them.
- **Projects point at files.** `ProjectFile` stores paths, the profile and clock offsets. Sharing a project means
  sharing the original files at the same paths, and reopening re-imports everything.
- **Untimed lines** have no first-class treatment; they mix into the timeline or silently disappear, and parse
  problems are reported only as notifications (batch 5 added `TimestampIssue`).

The user decided (2026-10-20): go straight to importing into a database for the lowest memory and easy project
export and sharing; Tantivy for search (case-insensitive word search across punctuation, e.g. "graphic builder"
finds `GRAPHIC.BUILDER`); multi-threaded import; every UI operation off the UI thread; a Problems pane fed by
import diagnostics with engine-verified quick fixes (including an AI-proposed fix the engine validates).

## Decision

### 1. One store: DuckDB in a store worker, behind small ports

- `imt-store-worker` (new binary, the ADR 0002 worker pattern) hosts DuckDB and Tantivy for **all windows**; one
  worker per app instance, one database per project/window. The app talks to it through `imt-ipc` with the same
  supervisor, restart and timeout rules as the AI worker. A crashed worker is restarted and the project reopened
  (the database is durable); in-flight requests fail with a retryable error.
- The in-memory `Workspace` is removed. Tests that need a store use the real DuckDB adapter on a temp folder
  (DuckDB opens in milliseconds); pure functions keep their own unit tests.
- **The port is split by responsibility** (interface segregation). Consumers depend only on what they use:
  - `Ingest`: open, reload, refresh, remove source, clear, restore, live tail append. Returns an `ImportSession`
    that streams progress and can be cancelled; never blocks readers.
  - `Query`: summary, page, timeline, facet, chart, field stats, source context, query language (run, check,
    complete), export.
  - `Analysis`: patterns, exception groups, anomalies, compare, correlate, traces, requests, moment, overview,
    runs/spans.
  - `Profiles`: active profile, set/validate profile, previews (parse, extractor, spans).
  - `Diagnostics`: import problems (see 5) and quick-fix evaluation.
  - `Search`: Tantivy full-text queries returning record ids.
  Every method takes `&self`; implementations are `Send + Sync` and synchronise internally. **The window-level
  `RwLock<Box<dyn LogStore>>` goes away**: reads run concurrently with an import (DuckDB MVCC), and a write that
  must be exclusive (profile change) is a job that swaps in a new generation atomically when it completes.
- **Every Tauri command is `async`** and runs blocking work on the blocking pool. A test fails the build if a
  `#[tauri::command]` is not `async`.

### 2. Streaming, parallel import

- Importers stop receiving whole files. The text importer contract becomes
  `fn import(&self, ctx: &ImportCtx, input: &mut dyn ChunkSource, sink: &mut dyn BatchSink) -> Result<()>`:
  a chunk source yields decoded text in bounded chunks that always end on a line boundary; the importer pushes
  `RecordBatch`es (see 3) of at most `BATCH_ROWS` (8,192) rows. Binary importers get a bounded reader instead of a
  byte slice. `MAX_LINE_LEN` and the archive expansion limits stay.
- **Pipeline** (all stages bounded, so memory is flat whatever the folder size):
  1. discovery: walks roots, sniffs formats, estimates sizes (for progress);
  2. parse: a rayon pool, cores − 1 by default; many files in parallel, and large uncompressed text files split into
     ~64 MB chunks that start at the first line that begins an event, so continuation lines (stack traces) never
     separate from their event; compressed files are parsed one per worker thread;
  3. write: one DuckDB writer per database using the **Arrow appender** (the spike's row-at-a-time appender was the
     ingest bottleneck); records get their final ids here;
  4. index: Tantivy on its own thread, from the same batches; the grid works before search is ready.
- Limits: worker threads, queue depth and DuckDB `memory_limit` are capped (defaults: cores − 1, 4 batches per
  stage, 25 % of RAM) and configurable in Settings. Import progress, per-file errors and cancel go through the
  Jobs pane (ADR 0012); one bad file is reported and the rest imports (`guard::catch` as today).
- Performance budget, measured by a CI benchmark on the 10M-event corpus: at least 3× the spike's 72.7k events/s on
  the same machine class, peak RSS of the worker under 2 GB, app process RSS unaffected by data size.

### 3. One batch format end to end

- `imt_core::batch::RecordBatch` wraps an Arrow `RecordBatch` with IMT's fixed columns: `id u64`, `ts i64?`
  (UTC ms), `ts_state u8` (parsed, inherited from the event above, untimed), `level u8`, `source u32`,
  `line u32`, `message string`, `template string?`, `exception string?`, `markers u64`, `raw string` (the original
  event text), plus dynamic fields.
- **Dynamic fields** are columns when the profile declares them (extractors with a type) and a `MAP(VARCHAR,
  VARCHAR)` column `extra` otherwise; queries on `extra` keys work, declared fields are fast.
- Importers, the store, Tantivy, exports and **the plugin protocol (batch 7)** all use this type: a plugin importer
  sends Arrow IPC frames of the same schema, so plugin imports need no second conversion path. Record ids widen to
  `u64`.

### 4. Analysis runs where the data is

- Counting, bucketing, facets, field stats, compare and timeline are **SQL** (window functions, `time_bucket`),
  generated by the engine from the parsed query language; no row leaves the worker.
- Algorithms that need code (patterns/Drain, exception fingerprints, anomalies, correlate, spans, AI context)
  read **streams of record batches** (`Query::scan(filter, columns) -> impl Iterator<Item = RecordBatch>`), with
  their results cached per generation in the database, not in RAM.
- The query language keeps its grammar and semantics; its evaluator compiles to SQL where it can and falls back to
  a batch scan for operators SQL cannot express. Full-text terms go through Tantivy and join by id.

### 5. Time and diagnostics are data

- During import, a line without a timestamp that continues an event (multiline importers, indentation, stack
  frames) joins the event above it. An event that still has no valid timestamp is stored with
  `ts_state = untimed`: it **stays in the grid** (marked) and is **left out of the timeline, time filters and
  charts**.
- Import problems are rows in a `diagnostics` table: kind (timestamp unparsed, field invalid, line too long,
  encoding, unknown format, truncated archive...), severity, source, affected count, up to 20 sample line numbers,
  and an optional engine-computed fix (e.g. the batch 5 `TimestampSuggestion`). Counts are aggregated; never one
  row per bad line. The notification and the Explorer warning read from it; the Problems pane (batch 6b) lists it.
- A **quick fix** is a profile override proposed by the engine or the AI. The engine evaluates it on the samples
  ("fixes 9,812 of 9,820 lines") before the user applies it; applying stores it in the project's profile
  overrides (undoable) and re-derives only the affected sources (from `raw`, see 6). The AI never applies a fix.

### 6. Projects are self-contained and shareable

- A project is a folder `<name>.imt/` (and, for sharing, the same folder zipped as `<name>.imtz`):
  - `project.toml`: format version, name, profile overrides, clock offsets, layout, opened roots (for refresh);
  - `data.duckdb`: records (including `raw`), sources, diagnostics, annotations, cached analysis;
  - `snapshots/`;
  - `index/` (Tantivy) is a cache: never shared, rebuilt in the background when missing or stale.
- Because records keep their `raw` text, a profile change re-derives fields **from the database** in parallel,
  without re-reading files, and a shared project opens with no access to the original files. "Open file / Open at
  line / Show in folder" (batch 5) degrade with a message when the original path is missing.
- **Export project** writes the `.imtz` (zip, no index; optional redaction through the existing redactor);
  **Open project** accepts both forms; a newer format version is refused with a message saying which IMT is
  needed. IMT is pre-v1, so there is no migration from the old `ProjectFile` (it is removed).

## Consequences

- Memory no longer grows with the data in the app process; the worker is bounded by its caps. Reopen is instant.
- Concurrent reads during imports remove the lock-induced UI freezes; with async-only commands, nothing runs on the
  main thread.
- Executable grows by the DuckDB library (~37 MB on Windows, measured in the spike); Tantivy is already a dependency
  of the spike. Both are MIT; `deny.toml` is checked.
- A large refactor of `imt-core`: importers, the store, analysis, project and tests. The UI contract
  (`api.types.ts`) changes little: paging, buckets and jobs already fit; new are `tsState` on rows, the
  diagnostics list, and project export/open. The mock backend follows.
- The IPC hop adds latency per request (sub-millisecond on a local pipe); requests stay coarse (a page, a bucket
  set), never per row.
- Risks: SQL compilation of the query language (mitigated by the batch-scan fallback and differential tests
  against the current evaluator on the samples before it is removed), DuckDB's single-writer model (one writer per
  database is by design here), and worker crashes (supervised restart; the database survives).

## Implementation plan

1. **Batch 6a, foundations** (one agent): `RecordBatch` and schema; `ChunkSource`/`BatchSink`; the text and JSON
   importers streaming; the split ports; `imt-store-duck` adapter in-process for tests; `ingest → page/timeline`
   working on the samples; differential tests against the current `Workspace`.
2. **Batch 6b, in parallel once 6a's ports are merged** (two agents): (i) the parallel pipeline, Arrow appender,
   Tantivy index, store worker and supervisor, caps, benchmark; (ii) analysis and query language on SQL and batch
   scans, untimed lines, diagnostics table, project folder/`.imtz` export and open.
3. **Batch 6c**: switch the app to the new ports, remove `Workspace`, the window lock and the old `ProjectFile`;
   all commands async with the guard test; the remaining importers and binary importers.
4. **Batch 6d**: the Problems pane with quick fixes and the AI fix (UI on top of the diagnostics port).
