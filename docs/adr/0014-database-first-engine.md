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
- **Memory is cut to the minimum** (user decision, 2026-10-20). Budgets, enforced by CI tests on the 10M-event corpus
  and by the worker's job-object limit:
  - **App process:** flat, independent of data size: target under 250 MB with one window and a 10M-event project. It
    holds no records: only the page on screen (a few hundred rows), timeline buckets and counts.
  - **Store worker, idle or querying:** under 300 MB. DuckDB `memory_limit` defaults to 256 MB (spilling to its temp
    folder), its buffer pool is not a cache of the whole data, and per-generation analysis results live in the
    database, never in RAM.
  - **Store worker, importing:** under 768 MB whatever the folder size (600 MB until 2026-10-21; raised by user decision
    when mimalloc v3 left little margin, see the notes below; the worker's hard cap is 2 GB). A "Low memory" setting
    (planned) trades 20-40% of the import speed for about 400 MB (freed memory returned at once, fewer batches in flight): batches of at most 8,192 rows, at most 4 batches
    in flight per stage, file chunks streamed (never a whole file), Tantivy's writer budget 64 MB, DuckDB 256 MB.
  - Every cache is bounded by bytes (not entries) and dropped under memory pressure; nothing grows with the data.
  - Tests measure peak RSS (the spike's `rss.rs` sampler) and fail above the budget; Settings can raise the limits for
    users with large machines who want faster imports.
- Performance budget, measured by the same CI benchmark: at least 3× the spike's 72.7k events/s on the same machine
  class, within the memory budget above.

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

## Implementation notes (R2a, 2026-10-20)

Phase R2a ("foundations") adds the new engine next to `Workspace`; the app keeps using `LogStore` until 6c.

### What exists

- **`crates/imt-batch`** (Arrow 58, no `imt-core` dependency; re-exported as `imt_core::batch`): `EventId(u64)`,
  `SourceId(u32)`, `TsState {Parsed, Inherited, Untimed}` and `EventTime` (a timestamp and its state cannot
  disagree), `LevelCode`, `EventSchema`, `EventBatchBuilder` → `PendingBatch` (no ids) → `assign_ids` →
  `EventBatch` (validated: at most 8,192 rows, required columns non-null, level and state codes valid, `ts` null
  exactly when untimed), `BatchRow` accessors.
- **Schema as built**: `id u64, ts i64?, ts_state u8, level u8, source u32, line u32, byte_offset u64, message,
  template?, exception?, markers u64, raw`, then one column per extractor field declared with a type
  (`f_<name>`: `string` → VARCHAR, `integer` → BIGINT, `number`/`bytes`/`duration` → DOUBLE, `bool` → BOOLEAN;
  case-insensitive duplicates and more than 64 go to `extra`), then `extra MAP(VARCHAR, VARCHAR)`. A value that
  does not fit its declared column's type is kept in `extra` as text. `byte_offset` is new (for source context).
- **`imt_core::row`**: `RowRef` with interned keys (`FieldKeys`/`FieldKey`; `BatchSlots` resolves a key to its
  column or `extra` once per batch) for `RecordRow` and `ArrowRow`; `to_record(&BatchRow) -> Record`.
- **`imt_core::ingest`**: `ReaderChunks` (8 MiB reads, whole lines, exact byte offsets for UTF-8 and UTF-16, lines
  over `MAX_LINE_LEN` skipped without being buffered), `LineReader` (look-ahead that can be given back),
  `FilePlan` (1 MiB sniff: encoding, importer, text format, day order, time-only, JSON shape, splittable) and
  `scan_chunk_starts`, streaming text and JSON importers, a `legacy` adapter (CSV, plugin text and binary
  importers), `RecordBatcher`, `BatchSink`, per-source `Problems` (line too long, event too large, line not parsed,
  timestamp unparsed: counts and at most 20 sample lines), `MAX_EVENT_BYTES` = 8 MiB, and
  `import_path`/`import_files`/`files_to_read`. Text before a file's first event is one untimed event.
- **`imt_core::ports`** (every method `&self`, `Send + Sync`, owned DTOs, `StoreError {Unsupported {operation,
  phase}, Invalid, NotFound, Busy, Cancelled, Io, Engine}`):
  - `Ingest`: `open(&[PathBuf]) -> ImportSession`, `reload`, `refresh(&[PathBuf])`, `remove_source(SourceId)`,
    `clear`, `restore(&WorkspaceState)`, `append_tail(&[PathBuf]) -> usize`. `ImportSession`: `progress()`,
    `cancel()`, `is_finished()`, `wait() -> ImportReport` (a crashed job is an error, not a panic).
  - `Query`: `summary() -> SummaryReply`, `page(&PageQuery) -> PageReply` (`EventView` rows; `anchor:
    Option<EventId>`), `timeline(&TimelineRequest) -> Timeline`, `facet(&FacetRequest) -> Facet`,
    `scan(&Filter, &mut dyn FnMut(&EventBatch) -> ControlFlow<()>)`, `chart`, `field_stats`, `source_context`,
    `run_query`, `check_query`, `complete_query`, `export`.
  - `Analysis`: `patterns`, `exception_groups`, `anomalies`, `compare`, `correlate -> Vec<EventId>`, `traces`,
    `requests`, `moment`, `overview`, `runs`.
  - `Profiles`: `active_profile`, `validate_profile`, `set_profile -> ImportSession`, `preview_parse`,
    `preview_extractor`.
  - `Diagnostics`: `problems() -> Vec<DiagnosticRow>`, `evaluate_fix(&QuickFix) -> FixEvaluation`.
  - `Search`: `search(&SearchRequest) -> SearchReply` (ids), `index_status()`.
- **`crates/imt-store-duck`** (not a workspace member: its own `[workspace]` and `Cargo.lock`; the prebuilt
  libduckdb 1.5.6 comes from `scripts/fetch-libduckdb.sh` with pinned SHA-256s): tables `meta` (format version
  1, declared columns, next ids, so ids are never reused), `sources`, `diagnostics`, `marker_kinds` (bits per
  generation) and `events`; one writer per import with the Arrow appender; `memory_limit` 256 MB with a `tmp`
  spill folder. Implemented: `Ingest::open` (sequential), `remove_source`, `clear`; `Query::{summary, page,
  timeline, facet, scan}`; `Profiles::{active_profile, validate_profile}`; `Diagnostics::problems`;
  `DuckStore::open` refuses a newer format with a message. Everything else returns `Unsupported` naming R2b, R2c
  or 6c.
- **Queries**: time range, levels, plain text (`ILIKE`/`LIKE`), sources and markers compile to SQL (values as
  escaped literals; text containing NUL never enters SQL). Regex text, field filters, instance selection and the
  query language scan the SQL-prefiltered events in time order through the existing `CompiledQuery`. Timeline
  (lanes, density, histogram, first sources, marker clusters) and facets are SQL aggregates on the SQL path and the
  in-memory algorithm on the scan path. Time order is `coalesce(ts, i64::MAX), source, line, id`.
- **Tests**: differential tests against `Workspace` on `/samples` with the default and the sample profile (page
  totals and rows in both directions and at deep offsets, anchors, the source filter, timelines at 10 and 400
  buckets, facets including `@group`, the field list, the instance key, unrecognised files); a peak-RSS test of
  importing the samples (`IMT_BENCH_COPIES=n` for larger runs); unit tests for chunking, plans, batching,
  validation and the writer.

### Deviations from the decision (keep or revisit)

- **Importers push records, not Arrow batches.** They emit `ParsedEvent` (record, raw text, byte offset) into the
  `RecordBatcher`, because `profile.ingest` (extractors, pipeline, markers) still runs on `Record`s. Once these are
  ported to `RowRef` (R2c) the batcher can derive fields on the batch.
- **Pipeline steps run per batch** (at most 8,192 events), not per file: `dedupe` collapses repeats within a batch.
- **Paging uses `ORDER BY … LIMIT/OFFSET`** (the scan fallback collects matching ids); R2b adds a time-order `pos`
  column so deep pages and anchors become range lookups.
- **Arrow 58 here, Arrow 60 in the plugin SDK**: duckdb-rs 1.10506 links Arrow 58; R3 must pick one (upgrade
  duckdb-rs when it moves, or convert at the plugin edge). Resolved by ADR 0017: Arrow does not cross the plugin
  boundary (plugins send postcard wire events) and the plugin SDK is gone, so only Arrow 58 remains.
- **`extra` is text**: JSON numbers and booleans in undeclared fields come back as strings (`value_number` still
  reads numbers; a JSON `true` is no longer numeric). In SQL, floats of declared columns are rendered by DuckDB
  (`1e+20` where serde writes `1e20`), and plain-text search folds case with `ILIKE`, not with Unicode regex folding.
- **Writes are grouped**: a commit costs 45–580 ms on the test VM, so the writer commits every 32 batches or 1 s,
  across source boundaries (readers see an import's progress about once a second); an aborted source's rows are
  deleted. `checkpoint_threshold` is 1 GB while importing and an import ends with `CHECKPOINT` (frequent automatic
  checkpoints rewrote the table and made imports 3× slower).
- **Kept from the old import for now**: archives are expanded in memory (within the archive caps), JSON arrays and
  single objects are read whole, legacy importers get the whole decoded file (raw text is cut by line numbers;
  binary events have no raw text). Not yet in the store: clock offsets, `timestamp_issue`, live-tail seeds, the
  redacted view, plugin annotators. `FilePlan::chunks` is computed by `scan_chunk_starts` but not used yet.
- **Ids in old DTOs**: `Record.id`, `MarkerCluster.id` and `PageRequest.anchor` are `u32`; conversions from
  `EventId` saturate until 6c widens them.

### Measured (R2a: sequential, release build, Windows VM shared with other builds)

- `/samples` (31 sources, 28,161 events, 9.8 MB): parsing alone about 97k events/s; the full import into DuckDB
  3.2 s (8.8k events/s) including the final checkpoint; peak RSS 79 MB.
- 20 copies (563,220 events, 195 MB): parsing alone 84–125k events/s; the full import 18–30k events/s (31 s with the
  final checkpoint, 19 s without); peak RSS 334 MB (budget 600 MB). Commits and the checkpoint take most of the
  time: the writer, not parsing, is the bottleneck.

### Next

- **R2b**: the parallel parse pipeline (rayon, bounded queues of 4 batches, chunk starts moved to event starts),
  the store worker over `imt-ipc` with supervisor and caps, the Tantivy index fed from the batches, `refresh`, the
  `pos` column, and a faster writer (transactions large enough for DuckDB to write row groups directly instead of
  the WAL, checkpoints in the background, `raw` stored only when it differs from `message`) to reach the 3× target;
  the 10M-event benchmark and RSS test in CI (needs libduckdb in CI; then `imt-store-duck` becomes a member).
- **R2c**: analysis, charts, field stats and the query language on SQL and batch scans (port `Condition`, extractors,
  transforms and analyses to `RowRef`), classified profile changes re-derived from `raw`, diagnostics with
  quick-fix evaluation and `timestamp_issue`, untimed events in the UI (`tsState` in `api.types.ts`), projects as
  `.imt` folders and `.imtz` archives (one archive module; the database streamed under its own limits), clock
  offsets, and source context by `byte_offset`.

## Implementation notes (R2b, 2026-10-20)

Phase R2b ("write side") changes the storage layout decided above: **events are zstd Parquet segments written by the
parse threads; DuckDB is the query engine and keeps a small native catalog.** This supersedes "DuckDB native
`events` table written by one Arrow appender" (§2 step 3) and the R2a writer. The ports are unchanged for readers
(`SELECT e.* FROM events e` still works); `events` is now a read-only view.

### Storage bake-off (decision)

Measured on the same parsed batches (563,220 events = 20 copies of `/samples`, 195 MB of logs; release; DuckDB
256 MB, 4 threads; Windows VM shared with other builds, so ±30 %):

| Layout / writer | events/s | peak RSS | on disk |
|---|---|---|---|
| R2a: appender per batch, commit every 32 batches, final checkpoint | 7.6k end to end (writer 49k + checkpoint) | 340 MB | – |
| A: native table, one Arrow appender, one transaction, checkpoint once | 36.5k | +429 MB | 318 MB |
| A with `raw` NULL when equal to `message` | 60k | +429 MB | 318 MB (`raw == message` for 0 of 563k events) |
| A forcing dictionary / zstd / uncompressed | 135k / 127k / 101k | | 318 / 405 / 429 MB |
| A without `raw` at all | 190k | | 115 MB |
| A': Arrow-scan insert (duckdb-rs `ArrowVTab`) | 21–25k | leaks every batch (process-global arena, documented) | – |
| A'': 2/4/8 appenders on own connections | 61–75k (no scaling) | | |
| B: zstd Parquet segments, 1 writer / 8 writers | 261–409k / 0.97–1.7M | +50 / +78 MB | **36.7 MB** |

Queries on the same data (cold/warm ms): native vs Parquet within 1–3× (page 64–99 vs 158–180, timeline 27 vs 75,
facet 61 vs 104, cancel 39 vs 6 ms after the interrupt). `raw` is 190 of the 215 MB of text and DuckDB's string
compression of it is the native writer's bottleneck; zstd Parquet is 8.7× smaller and parallel. **Decision (approved
2026-10-20): Parquet segments + native catalog.** Rejected: native table (single-writer bottleneck, 9× larger),
`raw`-if-different (no effect on real data), Arrow-scan inserts (memory leak), parallel appenders (no gain).

### What exists

- **Parallel import** (`imt_core::ingest::import_files_parallel`, `ParallelOptions`, `SharedSink`): files on a rayon
  pool of `min(cores − 1, 8)` threads (configurable; the cap keeps the worker in its memory budget), largest first.
  Large uncompressed text and JSON-lines files are split at `FilePlan` chunk starts (one newline scan), each start
  moved forward to the next line that begins an event (the text format's regex; `{` in the first column for JSON),
  each chunk read with its first line number and byte offset, so chunked reading equals reading whole (tested on
  stack traces, BOMs, UTF-16 offsets). Time-only formats, JSON arrays/single objects, other importers and archives
  stay one chunk. A chunk's failure or crash fails its source only. Source ids are reserved per file before reading
  (zip archives reserve one per entry), so they follow file order whatever thread starts first.
- **Segments** (`imt_store_duck::segments`): one open segment per parse thread, ~131k rows (16 batches), row groups
  of 65,536, zstd level 1, written as `.tmp` and renamed when complete. Each segment reserves a contiguous **id
  block** when it opens (the next free id is kept in `meta`), so ids are unique, never reused, dense per segment and a
  lookup by id prunes to one file. A segment is registered (manifest row + catalog rows + view refresh, one
  transaction) when full, when 2 s old, or at the end of the import; catalog writes are batched because a DuckDB
  commit costs tens of milliseconds on this VM.
- **Catalog tables**: `segments(file, first_id, end_id, rows, live, indexed, promoted)`, `segment_fields(file, key,
  rows, numeric_rows, distinct_est, top_values)` (exact up to 256 distinct values per field, then HyperLogLog; ten top
  values as JSON), `histogram(file, minute, level, source, n)` (minute = `ts div 60000`, NULL for untimed),
  `ord(pos, id)`, `trash(path)`, plus R2a's `meta`, `sources`, `diagnostics`, `marker_kinds`.
- **Views**: `events` = the event schema columns (`SELECT e.*` unchanged) over `read_parquet([segments],
  union_by_name = true)` filtered to ready sources; `events_wide` = every stored column including the promoted
  `p_<key>` columns. Both are recreated on every manifest change and on open (they name files by absolute path, so a
  moved or unpacked project works). **`events` is read-only**: changing events means writing new segments and
  swapping them in the manifest (a new generation).
- **Promoted fields** (`imt_store_duck::promote`): an `extra` key present in ≥ 50 % of a segment's first batch gets
  a typed column `p_<key>` in that segment (BIGINT/BOOLEAN only when the text round-trips exactly, else VARCHAR;
  profile-declared fields keep their `f_` columns; at most 32, case-insensitively unique). The column is a **copy**:
  `extra` stays complete. (Moving the values out and rebuilding `extra` in the view with `map_concat` made every
  read of whole rows 10–100× slower at 10M events: summary 49 s, timeline 255 s.) The promoted set is in the
  manifest. Read side: `p_<key>` from `events_wide` where the segment promotes the key, `extra['key']` elsewhere.
- **Visibility**: a source is listed (ready) when it is finished and all segments holding its rows are registered;
  aborted and removed sources disappear from the views at once. Compaction rewrites any segment with rows of
  non-ready sources (so field catalogs and histograms stay exact) and merges small segments once there are more
  than 32; it reads through the canonical projection and writes through the same segment writer (new id range =
  union of the merged blocks). Old files a reader still holds go to `trash` and are deleted later; crash leftovers
  (`.tmp`, unlisted files) are swept on open; a listed segment that is missing is refused with a message.
- **Positions** (`Positions` port: `ids_at`, `position_of`, `positioned`): `ord` is rebuilt after each import,
  removal and refresh (0.1–0.3 s per 563k events). `DuckStore::rows(ids)` fetches a page's rows from the segments
  whose id blocks hold them (the second phase of every page).
- **Full-text** (`Search` port): Tantivy in `index/`, a rebuildable cache: a background thread indexes registered
  segments of ready sources one at a time under the store's write lock (an import waits for at most one segment),
  so import speed does not depend on indexing; writer heap 64 MB, freed when idle. Text = raw text, plus message and
  exception where raw lacks them; Tantivy's default tokenizer (split on non-alphanumerics, lower-case), so
  `graphic builder` finds `GRAPHIC.BUILDER`. No positions: phrases are an AND of their words in the index, then
  checked as case-insensitive substrings of message/raw/exception on up to 1,000 candidates (5,000 took 0.9 s at 10M) (`SearchReply.exact`
  says whether the total is exact). Re-indexing a segment deletes its documents by segment key first, so a crash
  never duplicates; removed sources are deleted by term; an index of another epoch or layout is rebuilt.
- **`refresh(changed)`**: drops the sources of changed or deleted files (and their archive members), forgets them
  as unrecognised, and imports the opened roots again, which reads the changed and the new files.
- **Store worker** (`crates/imt-store-worker`): a binary hosting all stores of the app (handles per process) behind
  one gRPC call `Call(method, store, JSON body)` (replaced in R3, ADR 0017: `StoreRequest` frames over `imt_ipc`,
  `Call { store, op: StoreOp, body }` with the body still JSON until 6c);
  `StoreWorker` supervises it with `imt_ipc::Supervisor` (lazy start, backoff, quarantine, killed with IMT) and a
  1 GiB memory cap (since R3 the launcher's `SandboxSpec::memory_limit`: a job object on Windows, `RLIMIT_AS` on Unix);
  `RemoteStore` implements Ingest, Query (summary, page, timeline, facet), Positions, Profiles (active, validate),
  Diagnostics (problems) and Search; it reopens its store after a restart, and a call that saw a crash fails with a
  retryable `StoreError::Engine`. `scan` and the other Query/Analysis operations answer `Unsupported` over the worker
  until 6c (batches need R3's Arrow IPC frames).
- **Limits**: `StoreOptions { parse_threads, duck_threads (4), memory_limit ("192MB") }`, `DuckStore::configure`;
  DuckDB caches Parquet footers (`parquet_metadata_cache`).

### Port changes (all small)

- `Positions` (new port; ISP: not added to `Query`).
- `ImportSession::remote(snapshot, cancel, wait)`; `ProgressSnapshot::stopped()`.
- `StoreError::Unsupported { operation, phase }` hold `Cow<'static, str>` (so it crosses the process boundary);
  `StoreError::unsupported(..)` is unchanged.
- `SearchReply { total, ids, exact }`.
- Serde: port replies and the API types they carry derive `Deserialize` (`SummaryReply`, `PageReply`, `EventView`,
  `ImportReport`, `DiagnosticRow`, `SearchReply`, `IndexStatus`, `FixEvaluation`, `Timeline`, `Lane`, `MarkerCluster`,
  `Facet`, `FacetValue`, `SourceInfo`, `TimestampIssue`, `TimestampSuggestion`, `FieldInfo`, `UnrecognizedFile`,
  `Severity`, `FileKind`, `ProblemKind`, `ProgressSnapshot`); `TimelineRequest`, `FacetRequest`, `SearchRequest`,
  `QuickFix` derive `Serialize`. No JSON shape changed.

### Measured (R2b, release, the same VM)

- **Import throughput**, `/samples` × 20 (563,220 events): **126–173k events/s** in process and 172k/s through the
  worker (R2a: 7.6–30k/s); peak RSS 291–318 MB (budget 600). Parse phase alone ~390k events/s with 8 threads;
  finishing segments 0.35 s; positions 0.12–0.3 s. `/samples` alone: 1.0 s (27k/s; fixed costs dominate).
- **Worker budgets** (20 copies, through the real worker): importing 318 MB, indexing 177 MB, idle 132 MB, querying
  206 MB; the app process grew 11 → 13 MB.
- **10M events** (3.7 GB of CLEF, `IMT_BENCH_EVENTS=10000000`): import 81 s = **123k events/s**, peak RSS 415 MB;
  background indexing 38–118k docs/s (runs differ with the VM's load), peak 407–455 MB; store 787 MB (segments
  409 MB, index 339 MB). Reopen 0.1 s, 78 MB idle. Cold/warm ms, peak RSS of the process ≤ 228 MB unless noted:
  - write-side paths (what R2c switches to): page first/middle/last by position incl. rows 249/234/71; field list
    from the catalog 4; 1000-bucket timeline from `histogram` 41; level × source counts 16; facet on a promoted
    column 115; field statistics on a promoted column 245; errors total 88; errors page ids (two-phase) 211; text
    total + 100 ids (Tantivy) 21; phrase (Tantivy + check) 245; 100 search hits fetched 140.
  - the R2a read SQL still in the `Query` port (R2c replaces it): summary 4,086 (field list over the `extra` map),
    page first by `ORDER BY` 2,715 (by `OFFSET` in the middle: over 60 s), errors page with total 2,286, timeline
    17,712 (peak 347 MB), facet on `extra` 2,646, field stats on `extra` 1,287, `ILIKE` text 3,060.
  - every write-side path is under the 0.5 s budget; the query budget of 300 MB holds except for the R2a timeline.
- The 3× target (≥ 218k events/s on the spike's machine class) is met by the parse phase but not end to end on this
  shared VM; field statistics and promotion cost about a third of the write path (they replace full scans at query
  time). Further gains: more parse threads where memory allows (16 threads: 165k/s at 428 MB on 20 copies), and
  moving the per-segment statistics off the parse threads.

### Deviations and open points

- Tantivy is fed from registered segments after the import, not from the batches in flight (one code path for
  import, rebuild and repair; import speed independent of indexing). Search is ready shortly after the grid.
- `raw` is always stored (it never equals `message` on real data; zstd handles it).
- Paging: R2c switches to two-phase pages (ids by key or `ord`, then `rows(ids)`), timelines from `histogram` at
  ≥ 1-minute resolution (GROUP BY on events only below a minute or with filters), field lists and facet suggestions
  from `segment_fields`, promoted columns from `events_wide`, and text through `Search`.
- Memory: DuckDB's 192 MB limit, Parquet metadata and Tantivy's mapped files leave little room under 300 MB while
  querying 10M events; queries that sort whole rows (R2a's `ORDER BY … OFFSET`) exceed it and must go.
- Not yet: Linux/macOS memory caps (R3's sandbox), a `StoreWorker` child-log kind (6c adds
  `ChildKind::StoreWorker`), live tail and restore over the worker (6c), an `.imtz` writer (R2c).

### CI (to do by the integrator; `.gitea` untouched)

The store crates link the official prebuilt libduckdb, so they cannot become workspace members until every job that
builds the workspace fetches it (a skipped test still has to link). Steps:

1. In the `rust format + clippy`, `rust tests`, `bench` and Windows smoke jobs, before cargo:
   ```yaml
   - name: libduckdb (prebuilt, pinned SHA-256)
     shell: bash
     run: |
       eval "$(crates/imt-store-duck/scripts/fetch-libduckdb.sh "$HOME/.cache/imt-libduckdb")"
       echo "DUCKDB_LIB_DIR=$DUCKDB_LIB_DIR" >> "$GITHUB_ENV"
       echo "DUCKDB_INCLUDE_DIR=$DUCKDB_INCLUDE_DIR" >> "$GITHUB_ENV"
       echo "LD_LIBRARY_PATH=$DUCKDB_LIB_DIR:${LD_LIBRARY_PATH:-}" >> "$GITHUB_ENV"   # Linux
       # Windows: echo "$DUCKDB_LIB_DIR" >> "$GITHUB_PATH" (duckdb.dll next to the test binaries)
   ```
   with an `actions/cache` (v3-compatible) of `~/.cache/imt-libduckdb` keyed on the script's hash.
2. Workspace `Cargo.toml`: add `crates/imt-store-duck` and `crates/imt-store-worker` to `members`, remove them from
   `exclude`; delete their `[workspace]` tables and their own `Cargo.lock` files; use `x.workspace = true` for deps in
   `[workspace.dependencies]` (add `duckdb = "=1.10506.1"`, `parquet = "58"`, `tantivy = "0.27"` there).
3. Bench job: `IMT_BENCH_EVENTS=10000000 cargo test --release -p imt-store-duck --test bench -- --nocapture` (about
   4 minutes of import and indexing plus the corpus, generated once into a cached folder via `IMT_BENCH_DIR`) and
   `IMT_BENCH_COPIES=20 cargo test --release -p imt-store-worker --test worker -- --nocapture` (budgets asserted).
4. Bundling (6c): ship `duckdb.dll` / `libduckdb.so` / `libduckdb.dylib` and `imt-store-worker` with the app.

## Implementation notes (R2c, 2026-10-20)

Phase R2c ("read side") gives the DuckDB store the rest of the read ports; the app still uses `LogStore` until 6c.

### What exists

- **One predicate language** (review finding 7). `condition::Check` is one variant per operator (operands typed,
  a list value only for `in`; the `in` list is no longer joined with `|` and split again); a `Condition` is its
  `Check` lowered into `ql::Expr`. `CompiledQuery` lowers every criterion (sources, time range, field filters,
  instance selection; levels, markers, the search box, the QL filter and leading `where` stages) into `Expr`
  conjuncts evaluated by the one evaluator over `&dyn RowRef` (records and Arrow rows alike; field names interned
  as `FieldKey`, `BatchSlots` check the name so keys of other `FieldKeys` stay correct). New `Expr` variants carry
  the condition semantics exactly: `TextEq` (level names ignore ASCII case), `TextIn`, `NumCmp` (strictly
  numeric), `Search`, `Levels`, `Markers`, `Sources`, `Time`, `Group`, `Const`. One intended change: `ne` is the
  negation of `eq`, so `@level ne Warning` now also ignores case.
- **SQL compiler** (`imt-store-duck/src/sql.rs`): `expr_sql` compiles each conjunct to a never-NULL predicate or
  `None` (regex, NUL text); the compiled conjuncts prefilter, the evaluator decides the rest on the scan (so the
  prefilter is never approximate). Numbers follow `value_number` (`TRY_CAST(trim(..))`, finite only), QL `Cmp`
  compares by severity, numerically when both sides are numbers, else as text; contains folds with `lower()`.
- **`RowRef`** gains `id`, `named_value`, `for_each_field`, `text_of`/`number_of`; `Record` (source name empty),
  `RecordRow`, `OwnedRow` (an owned event streamed out of a store) and `ArrowRow` implement it.
- **Analyses are pure over `RowRef`** (finding 1): patterns, exception groups, anomalies (and memory growth),
  compare, field stats, correlate, spans/waterfall, requests; new pure modules for the Moment
  (`MomentBuilder`, `MetricCollector`), the health check (`findings`), trace summaries, chart series
  (`chart::Buckets`, `SeriesBuilder`, `chart_defs`) and runs (`spans::book`). The Workspace and the store call the
  same code.
- **Bounded** (finding 13): the Drain miner keeps `MAX_CLUSTERS` (10,000) patterns and counts the rest in one
  overflow pattern; compare tracks at most `max_distinct + 1` values per field and side (identical results: a
  field with more is not compared anyway) and 1,000 fields; the waterfall is linear (finding 19: the
  `roots.contains` and root walk were quadratic).
- **Coverage** (finding 16): the `Analysis` port returns `Analyzed<T> { result, coverage }`; `Coverage { matched,
  analysed, limits }` says when `MAX_ANALYSED` (2M) or a cap bounded the result. The query language fails with a
  message instead of truncating silently.
- **The store's `Analysis` and `Query` ports** are complete except `export` (6c): events stream out of the
  database in time order (`EventStream`, a pull iterator over the DuckDB statement, the residual filter on Arrow
  rows, never the `raw` column); compare uses two connections; correlate and chart bounds are SQL.
- **Per-generation cache** (finding 18): `analysis_cache(version, kind, hash, query, value)` holds patterns,
  exception groups and anomalies, keyed by the persisted `meta.data_version` (bumped by every import, removal,
  clear and re-import; older versions are deleted) and the FNV-1a hash of the canonical query (set-like lists
  sorted; the text is stored and compared too). Nothing is cached while an import runs.
- **Query language on the store**: the program's filter part compiles to SQL with the query; `ql::run_rows`
  streams the leading `where`s and a first `stats` (nothing is kept per event), `sort`/`head`/`fields` keep
  what they need. `Query::run_query_with(req, &QueryControl)`; `QueryControl::on_cancel` hooks interrupt the
  running DuckDB statement (`InterruptHandle`), and an interrupted statement is `StoreError::Cancelled`.
  `StoreError::Query(QlError)` keeps the error span.
- **Profiles**: `profile_change::classify` sorts a change into presentation, derivation or parsing (finding 4)
  with the changed sections. The store swaps the profile for a presentation change; for derivation and parsing
  it re-reads the sources as a job when every file is still there and unchanged (plain files: same size), and
  refuses with a message naming the missing or changed files otherwise. `Ingest::reload` works.
  `Profile::with_override` merges TOML over a profile (project overrides, quick fixes). Extractor field types are
  the `FieldType` enum; an unknown type is refused with its line (finding 20). `preview_parse` and the extractor
  preview (`ExtractPreviewer`) are pure and served by the store.
- **Diagnostics**: `ProblemKind` adds `fieldInvalid`, `encoding`, `unknownFormat`, `truncatedArchive`,
  `eventSplit`. `DiagnosticRow` has `source: Option<SourceId>`, `path`, `detail` and `fix: Option<DiagnosticFix>`;
  files no importer reads are `unknownFormat` rows; unreadable timestamps come with the batch 5 suggestion
  verified against the stored untimed events (the override keeps the current formats). `evaluate_fix` merges the
  override, then re-reads up to 10,000 stored untimed events of the affected sources with the candidate
  profile's importer ("fixes N of M"); an event that cannot be read alone (a CSV line without its header) counts
  when the candidate's timestamp formats read a timestamp in it that the active ones do not (said in `notes`).
- **Projects** (`imt_core::package`): `<name>.imt/` with `project.toml` (`format = "imt-project"`, version 1,
  name, profile overrides, clock offsets, layout, roots; missing keys default, a newer version is refused), the
  database, `snapshots/`, and the caches `index/` and `tmp/`. `ProjectDir::export_with` writes the `.imtz` without
  caches or WAL (Parquet stored, the rest deflated, zip64); the store exports a `COPY FROM DATABASE` copy of its
  open database (Windows does not let another handle read it). `open_package` is the one archive entry point: a
  root `project.toml` of format `imt-project` is a project (extracted under `MAX_PROJECT_ENTRY` 256 GiB per file
  and 1 TiB in all, streamed to disk), a share manifest a share bundle, anything else a collector bundle
  (finding 14). `DuckStore::{create_project, open_project, save_project, export_project}`; a project opens
  without its log files.
- **Source context** (finding 12): read from the file at the event's `byte_offset` when the line there is the
  event's first stored line (plain UTF-8 files, a 4 MiB window either side); otherwise the stored events of the
  source around the line, with `SourceContext.note` saying why (missing, changed, compressed or archived).
- **Tests**: differential against the old condition evaluator for every operator on the samples, records vs
  Arrow rows for every criterion, and the DuckDB store vs the Workspace for every operator and QL form (pages),
  every analysis (ids compared as source path and line), charts (also the profile's charts with `when`), field
  stats, runs, the query language (events and tables), check and completion; diagnostics, quick fixes,
  classified profile changes, projects opened without their files, and cancellation.

### Deviations and open points

- **Writes stay R2b's.** With events as read-only Parquet segments (R2b), re-deriving fields and markers from
  `raw` without the files means writing a new segment generation; R2c re-reads the files instead and refuses
  when they are gone. A change that declares other typed columns is refused (the schema is per store until
  segments carry their own). `forget_events` (clear, re-import) still deletes from `events`; R2b's manifest swap
  replaces it.
- **Not done here**: dedupe per source after sorting with key expiry and `StepCfg` as a tagged enum (finding 11),
  lookup tables stored in the project, the pure timestamp parse environment and the per-source winning format
  (finding 15), marker kinds written per generation on a profile change (finding 5), a stored source fingerprint
  (finding 8; the size check stands in), redaction of an exported project, clock offsets applied by the store
  (they are kept in `project.toml`; the `events` view of R2b can apply them), producers of the new problem kinds
  (`fieldInvalid` in the batcher, `encoding`, `truncatedArchive`, `eventSplit` in the R2b pipeline), and
  `Query::export`.
- **Ids**: QL event ids, sample ids, `ContextRequest.record_id` and run events are still `u32` (saturating).
- **Analyses that keep events**: anomaly detection and spans keep the window's events (as owned rows in the
  store); a projection of the few columns they read is the next step for the memory budget.
- **Compare reads numeric text as numbers** (stores keep undeclared fields as text), so JSON strings such as
  `"500"` count as numbers too.

### Read performance (R2c, after the R2b measurement at 10M events)

- **Pages are two-phase**: the page's ids by an id-only sort of the matching events (`ORDER BY` time key, `LIMIT`
  over ids, never wide rows), then `rows_by_id`. The R2b primitives (`ord`/positions for unfiltered pages and
  anchors, `rows(ids)`) slot in behind `Inner::page` without a port change.
- **Narrow reads**: `sql::Columns` says which wide columns a read needs (the message, which fields; others read
  as empty, `extra` filtered to the named keys with `list_filter`); every analysis states its columns, reads in
  time order only when its result depends on order (charts, field stats, compare and the Moment's previous
  window read unordered), and anomaly detection reads the message only of errors and stop messages. Analyses no
  longer keep whole events: spans stream (earliest event wins, as if sorted) and anomalies keep a few dozen bytes
  per event (`Slim`).
- **Timeline**: no window functions: one GROUP BY (lane, bucket, level, matched, source) with `min(struct)`
  for the first events, and one GROUP BY over marked events for the clusters (`arg_min` for the message).
- **Facets** are one scan (`sum`/`count` over the groups). **Charts** of `count()`, `min()` and `max()` are one
  GROUP BY (sums and averages stay in Rust so they add up as the engine does). The field list and instance
  key are cached in the database per data version and profile (a reopened project does not rescan `extra`).
- **For R2b to plug in** (shapes to agree in the ports): the per-segment field catalog replaces `Inner::derived`
  (fields with count, distinct (capped), numeric); per-segment minute × level × source histograms answer
  `timeline_sql` for buckets of a minute or more without content filters other than levels (the GROUP BY stays
  for the rest); the Search port answers plain text (`Expr::Search` with a literal, `Expr::Text`) with ids,
  `ILIKE` only verifying phrases.
- **Benchmark**: `tests/bench_reads.rs` (`IMT_BENCH_READS=<copies>` or `IMT_BENCH_STORE=<folder>`,
  `IMT_BENCH_STRICT=1` to enforce) times every `Query` and `Analysis` method and the RSS per operation. On this
  machine (release, R2a's native-table store, 563,220 events = 20 copies of the samples, a fresh process): summary
  6 ms, pages 85–145 ms (text 240 ms), timeline 200 ms, facets 7–40 ms, chart 60 ms, correlate 63 ms, field stats
  540 ms, QL stats 810 ms, Moment 1.1 s, patterns 1.0 s, anomalies 2.3 s, compare 2.0 s, traces 1.2 s, runs 1.0 s
  (cached analyses 1–2 ms); peak RSS 486 MB (anomalies; patterns, traces, runs and QL stats 320–360 MB). At 10M
  the time-ordered analyses (patterns, anomalies, traces, runs, Moment, QL sorts) and field stats/QL `stats`
  (still in Rust) will miss the 0.5 s budget; they need SQL pre-aggregation (field stats, `stats`), the
  position order of R2b instead of a sort, or to run as jobs with cached results.

### Reads on R2b's structures (R2c, after merging R2b)

- **Fields**: statements read `events_wide`; a key is read from its promoted `p_<key>` column when every
  segment that has the key promotes it as text (`sql::Fields`, `FieldSource`), else from `extra`. The field
  list comes from `segment_fields` (counts exact; distinct exact when every segment lists all its values,
  counted in one SQL statement below `PREAGGREGATE_ABOVE` events, else the segments' estimates summed).
- **Pages**: unfiltered pages and anchors use `ord` positions; filtered pages stay two-phase (ids by the time
  key, then `rows_by_id`, which prunes to the segment files holding the ids).
- **Timeline**: from 1M events (`PREAGGREGATE_ABOVE`, `set_preaggregate_above` for tests) a timeline of
  minute-or-longer buckets that filters only sources, time and levels, where each source belongs to one lane,
  comes from the minute histogram: counts per whole minute (a bucket edge inside a minute takes the whole
  minute), lane bounds from the sources' first and last events (exact without a time range), markers from
  the marked events. Otherwise one GROUP BY over the events, as before.
- **Field stats and QL `stats`** aggregate in SQL when the filter is exact: field stats in one statement
  (`quantile_cont`, top values, histogram bins by `analysis::field_stats::bin_layout`); a program's first
  `stats` as one GROUP BY (keys as text, groups in first-seen order by `min(struct(time key))`, nearest-rank
  percentiles from a sorted `list`, the engine's limit on values per percentile), the remaining stages by
  `ql::run_table`. Floats can differ from the engine in the last digits (summation order).
- **Time-ordered analyses** (patterns, anomalies, traces, runs, Moment, QL without `stats`, previews) no longer
  sort the wide rows of every matching event: joining `ord` and sorting all ids both exceeded the 192 MB
  limit or took minutes at 10M events, so a stream (`EventStream::ordered`) reads windows of time with about
  100,000 matching events each (from the minute histogram, scaled by the share of events that match; a busy
  minute split evenly; untimed events last), each sorted on its own; a stream of fewer events sorts at once.
  An exact stream stops at its limit and counts the rest in SQL.
- **Not done: plain text through the full-text index.** Tantivy's hits are words, the search box and `Text`
  match substrings (`timeout` finds `ReadTimeout`, `timeouts`): on `/samples` the index finds 194 of the 431
  events. Narrowing by the index changes what users find, so it needs a decision first: word semantics for
  the search box (matching the Search port), or an n-gram index that is a superset of substring matches.
- **Promotion is per segment**: at 10M, 9 of 93 segments with `MachineName` did not promote it (it is decided
  on each segment's first batch), so the facet reads `extra` (0.8–1.3 s instead of 0.12–0.25 s from the column).
  Sticky promotion (a key promoted once stays promoted in later segments, or compaction re-promotes) would let
  every read use the column.
- **Measured** (release, 10M events, `IMT_BENCH_STORE`, cached analyses dropped first; the machine was shared
  with other builds, so times vary by up to 2x between runs): pages 50–360 ms (were 0.5–2.5 s), field stats
  2.3 s (15 s), QL stats 0.3 s (refused before), Moment 0.5 s (1.0 s), exception groups 3 s, requests 5.6 s
  (30 s); patterns, anomalies, traces and runs now finish (they failed before: refused or out of memory); peak
  RSS 971 MB in anomaly detection (was 1,047 MB), under 270 MB elsewhere. Still over budget: timelines with
  instance lanes at 10M (3–6 s; sources with several instances need the events), facets of keys not promoted
  everywhere, charts (1.3 s), correlate.

### Span rule inference (R2c)

- `Profiles::propose_span_rule(SpanRuleRequest { examples: [{ start, end }], hints: { name?, keyField? } })`
  reads the example events and returns `SpanProposal { candidates, notes }`; `imt_core::spans::infer::propose` is
  pure. What tells starts from ends: the words only every start line (or only every end line) has, wherever they
  are ("open X" / "close X", "X started" / "X ended", "View X loaded" / "View X closed"), else a field whose value
  differs (`Phase = begin` / `end`). The key: a token both lines of each pair share, captured by a bounded regex
  (`user_regex`) anchored on the verbs or on the neighbouring token (`[Tab:42]` gives `Tab:(?P<SpanKey>…)`),
  or a field both events carry with the same value (JSON logs); a token that does not vary across several
  examples is not a key; up to three candidates, alternatives when several tokens qualify.
- A candidate is a profile override (the profile's lists repeated, since lists replace): an extractor for
  `SpanKey` (untyped, so no new column), `<kind>-start` / `<kind>-stop` markers (whole-word, case-insensitive
  message conditions plus `SpanKey` exists) and a `[[span]]` with `per = "field:SpanKey"`. Applying it is
  `set_profile(active.with_override(..))` (a derivation change, undoable by setting the old profile back).
- `Profiles::evaluate_span_rule(SpanRuleCheck { profileOverride, kind })` evaluates any rule, the engine's or
  an AI's: the candidate profile's extractors and markers run on each event as an import would and its runs
  are paired by the same `Pairer` as the Runs table. `SpanEvaluation`: spans, distinct keys, open, unmatched
  starts and ends, longest and shortest, the most runs open at once in a lane, `dropped` (beyond a rule's
  per-lane and key limits), up to 20 sample pairs, events looked at. The store reads only events that can be a
  start or stop (the markers' conditions on fields no extractor makes, compiled to SQL), in time order; the pure
  evaluator skips other events with the same test (10M events in 0.8 s, release). As in every `[[span]]`, a key
  started twice before it stops is one run.

### For 6c (`api.types.ts`)

- `Analysis` results: `{ result, coverage: { matched, analysed, limits } }`.
- `DiagnosticRow`: `source` nullable, new `path`, `detail?`, `fix?: { title, profileOverride, timestamp? }`; new
  kinds; `FixEvaluation`: `{ fixed, checked, unchecked, notes? }`.
- `SourceContext.note?`; `StoreError::Query` carries the `QlError` (message, offset, len, suggestions).
- Project open/export use `ProjectManifest` (`project.toml`) and `.imtz`.
- Span rule inference: `SpanRuleRequest`, `SpanProposal { candidates: [{ description, key, exampleKeys, start, stop,
  kind, profileOverride, evaluation }], notes }`, `SpanRuleCheck`, `SpanEvaluation`.

## Implementation notes (6c, 2026-10-21)

Batch 6c switches the app to the new engine: every window runs on the DuckDB store in the store worker; the
in-memory `Workspace` stays in `imt-core` only as the test oracle (differential tests) and for what is listed under
"Not on the store yet".

### What switched

- **One store per window, out of process.** A window's engine slot holds a `RemoteStore` (crate
  `imt-store-client`, no DuckDB link) on a supervised `imt-store-worker` child (`ChildKind::StoreWorker`, its own
  child log, a 2 GB memory cap, restart and reopen of its stores after a crash). The store folder of a window is a
  scratch folder (`<data>/stores/<label>-<pid>-<n>`, swept at start unless its process is alive) until the user
  saves it as a project. Tests use the same store in process (`LocalHost`), so command tests run without the
  worker binary.
- **Every command through typed port calls.** The worker protocol is a typed operation table (`StoreOp`, wire
  order = declaration order, new operations at the end) with JSON bodies of the port DTOs; scans stream Arrow IPC
  items; calls are cancellable (a `Cancel` frame interrupts DuckDB through the `QueryControl` scope of the call).
  No command passes `serde_json::Value` through any more; the parity test asks every operation in process and
  over the worker and compares the JSON (export file bytes included).
- **Ports.** `Store` = `Ingest + Query + Positions + Analysis + Profiles + Diagnostics + Search + Projects`.
  Added: `Query::events` (by id: bookmarks, reports, AI citations), `Projects` (save, export, close),
  `Profiles::preview_spans`, `Profiles::{parsers, classify_parser, assign_parser}` (ADR 0026, below),
  `Search::partial_matches`, `Diagnostics` reached from new commands (`import_problems`, `evaluate_fix`,
  `index_status`). `StoreError` gained `Damaged` (a segment missing or unreadable: not retryable, the hint says to
  open the logs or the `.imtz` again) and `Locked` (the catalog is open in another process: retryable after
  closing it there).
- **Projects.** A saved project is the store folder: `.imtz` (zip of `project.toml`, catalog and segments, no
  index; the index is rebuilt in the background) and opens without the original logs; the UI layout goes in
  `ui.json` beside it. `.imt`/`.imtproj` files of the old format still open (their logs are imported again).
- **Live tail** is append ingest: the watcher's changed files are read from their last offset into new segments
  (`append_tail`; truncation reads the file again, new files under an opened folder are added) and the index
  refreshes. Live windows (`window: { kind: "live" }`) resolve into `from` in the store ("now" = the newest event,
  or the wall clock with `[live] clock = "wall"`), shared with the oracle (`query::resolve_live_window`).
- **Import progress** comes from the import job (`JobProgress` with the current file and bytes); opening returns
  the job id and the UI follows the job instead of polling `import_progress`.
- **Smart search.** The full-text index tokenizes with `imt_core::words` (one pure tokenizer for the index and the
  oracle): runs of letters and digits, lower-cased, plus their `camelCase`/`HTTPServer`/digit parts; a search asks
  for every word, the last one as a prefix (unless the text ends in a space or quote); phrases are checked as
  written. The search box (`Query.textMode: "words"`) filters by the index's hits (handed to SQL as a small
  Parquet id list, cached per search and store generation, at most 2M ids); while the index is catching up, SQL
  narrows by substrings of the words and the scan checks them exactly. Then a cancellable background job
  (`search_partial`, superseded by the next search) counts the events that hold the text only inside words; when
  there are some the UI switches to `textMode: "wordsOrSubstring"` and shows "+N" ("N more partial matches
  added"), and pages mark each row `searchMatch: "words" | "partial"`. The query language's `contains` keeps its
  substring meaning.
- **Bundling and CI.** `imt-store-duck`, `imt-store-client` and `imt-store-worker` are workspace members (their own
  `[workspace]` and lock files are gone; duckdb, parquet, arrow and tantivy are workspace dependencies). Every Rust
  CI job fetches the pinned libduckdb (by SHA-256, cached) and puts it on the library path; installers ship
  `imt-store-worker` as a sidecar and libduckdb under `lib/` (`build-sidecars.mjs`). A missing or unloadable
  libduckdb is a clear message naming the file and where it was looked for. The nightly `bench` workflow runs the
  10M-event store benchmark.

### Prepared for ADR 0026 (parsers as individual files)

The `sources` table records each source's parser (`parser_id`, `parser_version`, `parser_hash`, the hash being
the SHA-256 of the parser's canonical TOML) and the `parsers` table keeps a copy of every parser's TOML, so a
project names and carries what read it (`project.toml` lists the parser refs; the `.imtz` embeds the TOML through
the catalog). Until ADR 0026 lands, the workspace's profile is the one parser of every source (id `profile`).
Changing the profile re-reads only the sources whose parser hash changed (and re-derives the rest by the
profile-change classes). The port API already has the shape the parser library needs: `parsers()` (which parser
reads which sources), `classify_parser(def)` (the change class and the sources it touches, per parser) and
`assign_parser(source, def)` (refused with `Unsupported` until the library exists), so per-file matching adds
implementations, not port changes. The worker compiles each parser once (today: one).

### Not on the store yet (typed `Unsupported` with a hint, or still on the old path)

- Redacted views and the redaction summary, clock skew correction, live alerts and per-file tail status, plugin
  annotators over the store, starting a tail at the end, run-query progress over the worker.
- Share bundles of the log files (replaced by projects: `.imtz`).
- `imt-agent` (the headless agent) still runs `Workspace` in process.
- The old `ProjectFile` remains for session files (autosave/restore), the activity report sample and opening old
  `.imt` files; projects themselves are `project.toml` + store.
- Golden JSON fixtures and ts-rs generation of the engine DTOs (the TS mirror is kept by hand and checked by the
  parity and command tests).

### Measured (6c, release, the shared Windows VM, 10M events = 360 copies of the samples)

`tests/bench_reads.rs` on a store of 10,137,960 events in 11,160 sources (360 copies of `/samples`), queries in
a process that did not import. The VM was shared with four other agents' builds (load around 80%), so times vary
up to 3x between runs (one run had `page text` at 45 s, the next at 1.1 s); read them as orders of magnitude.

- **Import**: 113 s (90k events/s) to the store, then 73 s for the full-text index in the background.
- **Interactive (budget 0.5 s)**: summary 0.25 s; pages 0.03-0.33 s unfiltered, 0.7 s errors, 1.1-1.5 s text
  and word search; facets of levels and sources 0.12-0.19 s, of `MachineName` 0.8 s (not promoted in every
  segment); timelines 3.4-4.3 s (instance lanes: the events are grouped, the histogram path needs one lane per
  source); chart 1.5 s; field stats 2.8 s; QL `stats` 0.35 s; problems 0.7 s (was 69 s); correlate 1.9 s; Moment
  23 s (it detects anomalies over all events); cached analyses 1 ms.
- **Jobs (no interactive budget; cancellable, cached per query and generation)**: partial-match scan 0.3 s;
  patterns 23 s, exception groups 2.3 s, anomalies 38 s, compare 18 s, traces 29 s, requests 28 s, runs 150 s.
- **Memory (budget 300 MB while querying)**: 75-230 MB for every operation except compare (309 MB), Moment
  (316 MB) and anomalies (424 MB, was 943 MB: the slim events now keep only the fields they have and share
  repeated texts). Timelines with thousands of sources ran out of DuckDB's memory limit (183 MB) before lanes x
  buckets and lanes x sources were grouped apart; now 180 MB.
- **Over budget, open**: timelines with instance lanes, filtered pages and word-search pages above 0.5 s,
  charts, field stats, correlate, Moment (should reuse cached anomalies of the window only), and runs (150 s:
  every timed event in time order through the span pairers).

### Deviations and open points

- Promotion is still decided per segment from its first batch; sticky promotion (keep a key promoted once the store
  promoted it) and re-promotion on compaction are open.
- Slow analyses run as jobs with coverage; results are cached per query and generation in the catalog. Anomalies'
  peak memory and timeline instance lanes at 10M are measured above; see the open points there.
- Mixed sources on the DuckDB path (ADR 0020) wait for the merge of main.

## Performance notes (perf/rust, 2026-10-21)

Measured on the shared Windows VM (24 cores, other builds running: single runs vary by 2-3x, so every number
below is from interleaved A/B runs, at least two each).

- **Segment listing**: closed segments are listed together, at most once a second, by whichever parse thread
  finds the catalog free (`writer::LIST_EVERY`). Listing each segment at once rebound the views over every
  segment file (quadratic in segments) under the catalog lock; it was 53% of the import's on-CPU samples.
  10M import: 72-43k events/s before, 105k events/s after (same period).
- **Allocator** (user decision 2026-10-21: mimalloc v3): the store worker, the app, the plugins process and
  the benches that stand for them use mimalloc v3 as the global allocator through `imt-alloc`, which sets
  its purge timing at start-up (`mi_option_set`; the workers clear their environment): v3's default keeps
  freed memory for 1 s, `imt_alloc::tune` sets v2's 10 ms (arenas 10x).

  | | worker, 20 copies (asserts 600 MB importing) | 10M import, in process | 10M reads, peak |
  |---|---|---|---|
  | system allocator | 49-82k ev/s, peak 283-314 MB | 28-79k ev/s, peak 524-553 MB | 980-986 MB |
  | mimalloc v2 | 50-85k ev/s, peak 454-501 MB | 99-254k ev/s, peak 597-682 MB | 841-872 MB |
  | v3, default (1 s) | 58-98k ev/s, peak 643-690 MB | 107-187k ev/s, peak 727-815 MB | 954 MB |
  | v3, purge 0 ms | 47-68k ev/s, peak 288-349 MB | 52-134k ev/s, peak 410-566 MB | 918-945 MB |
  | **v3, purge 10 ms** (shipped) | 55-65k ev/s, peak 494-577 MB | 151-207k ev/s, peak 691-756 MB | 920-962 MB |

  (v2, v3 at 0 ms and v3 at 10 ms in the same interleaved rounds, three each.) v3 at 10 ms imports as fast
  as v2 and stays under the worker's 600 MB budget, with less margin (577 MB at worst); purging at once
  halves the peak but costs 20-40% of the import speed. Read times are the same with every variant; the
  read peak is anomaly detection's (over the 300 MB query budget with any allocator, see below).
- **The app** (in-memory engine until 6c) uses the same allocator: `benches/engine.rs` import, mimalloc (v2,
  before the switch to v3) vs the system allocator, two rounds: clef +81/+83%, serilog-text +60/+98%, log4net +118/+88%, csv +175/+73%.
- Tried and dropped (no difference beyond the VM's noise): seeded foldhash / FxHash for the per-event maps of
  facets, timelines, charts, patterns, exception groups, QL `stats` and span pairing (criterion: changes of
  -50% to +74% in either direction between rounds, including benchmarks that do not use those maps; the
  store's analyses are bound by DuckDB reads); moving the JSON importer's message and template instead of
  cloning them and reusing the batcher's field-text buffer (worker bench within noise); listing the import's
  files' sizes and archive slots in parallel (one quiet pair 95 s vs 90 s).
- **JSON events keep their parsed map** as their fields (`importers::json::fields_of`) unless keys need
  renaming, flattening or repeat; the old path re-inserted every key into a second map. Engine bench
  import/clef +19% / +16%; 10M CLEF import 269-276k -> 279-291k events/s in quiet pairs (the ADR's 218k
  target is met end to end in process when the VM is quiet; the 20-copy worker run stays at 52-60k, its
  fixed costs dominate). Identical output, field order included (tested against the old path on /samples).
- Not changed, still hot (10M import profile, on-CPU samples of the parse threads): `serde_json::Value` maps
  per event (parsing, `IndexMap` inserts with SipHash, dropping records: ~30%) and Parquet encoding and zstd
  (~20%).
- **To do after 6c lands** (in files 6c rewrites, so left alone here):
  - `fsync` of every closed segment on the parse thread (`segments.rs`, `SegmentWriter::close`; 12% of the
    parse threads' samples): sync in the listing step or a background thread instead;
  - `SegmentStats::add` on the parse threads (`stats.rs`, 3.5%): move it off them, or allow more than 8
    parse threads within the memory budget (R2b measured 16 threads at 428 MB);
  - time-ordered analyses issue one statement per ~100k-event window (`analysis.rs`, `EventStream::ordered`),
    each rebinding the view over every segment and scanning every row group's `ts` (segments overlap in time,
    so pruning skips little): runs 26-80 s, compare 11-33 s, anomalies 8-40 s, patterns 5-25 s at 10M;
  - anomaly detection's peak (840-990 MB at 10M, `Slim` per event).
- The worker benchmark (`IMT_BENCH_COPIES=20 … --test worker`) fails at `summary()` already at 2ba7e00 (DuckDB
  out of memory at its 183 MB limit); it now prints the import line before the queries so the import is still
  measured.
