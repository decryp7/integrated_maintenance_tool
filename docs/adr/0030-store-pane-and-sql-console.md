# 30. The Store pane and a sandboxed read-only SQL console (amends ADR 0014)

- Status: accepted (2026-10-22)
- Amends: ADR 0014 (database-first engine: new ports and worker operations, a second DuckDB database per store)
- Related: ADR 0012 (jobs), ADR 0016 (command table, typed errors, ts-rs), ADR 0017 (one IPC), ADR 0021 (notes),
  UX audit 2026-10-22 findings C1 and C2 (`docs/ux-audit/README.md`), AGENTS.md (engine is the source of truth,
  validate in Rust, bounded resources)

## Context

The DuckDB/Parquet store and its Tantivy index (ADR 0014) are invisible. When an import is slow, memory is high, a
field is slow to filter or search seems incomplete, users and support cannot see the segments, field promotion,
index state or the worker's memory against its budgets, and power users have no way to ask what the query language
does not cover. The audit asked for a read-only Store pane (C1) and a sandboxed SQL console (C2).

## Decision

### 1. New ports, computed in the store

Three small ports join `imt_core::ports::Store` (interface segregation; their default methods answer
`StoreError::Unsupported`, so the in-memory test store needs no code):

- **`Inspect`** (queries, no side effects): `store_overview` (folder, size on disk, events, sources, segments, data
  and format version, activity *Ready / Importing / Indexing*, the last import with its speed, segments pending
  compaction), `store_segments` (a page of at most 500: rows, live rows, size, time range to the minute from the
  writer's histogram, compression codec from the Parquet footers, the first 8 source ids and their count, indexed,
  promoted fields, pending compaction), `store_fields` (type, storage *declared / promoted / partly / overflow* with
  promoted-in-n-of-N, events, fill rate, approximate distinct values: the largest per-segment estimate; at most
  2,000, most common first), `store_index` (documents vs events, size, Tantivy segments, store segments still to
  index, last commit, writer heap and threads, the last failure) and `store_memory` (DuckDB's limit, used and
  spilled bytes, threads, the console's limit, the budgets 768 MB importing / 300 MB querying, cached analyses and
  their size; the worker fills in its PID, resident memory and Low memory mode, the app the window's running jobs).
  Everything comes from the catalog (`segments`, `segment_fields`, `histogram`, `meta`, `analysis_cache`,
  `duckdb_memory()`) and the file system; no event is scanned. The last import is kept in `meta` (`last_import`, JSON),
  so it survives reopening.
- **`Maintain`**: `maintain(action)` with `Compact` (merge small segments however few there are, drop rows of removed
  sources, rebuild positions, checkpoint), `RebuildIndex` (empty the index, mark every segment unindexed, wake the
  indexing thread) and `ClearCachedAnalyses` (the analysis cache and the timeline lane tables). Each fails with
  `Busy` while an import runs. The app runs it as a *Maintenance* job (not cancellable: compaction swaps segments in
  one transaction) after the UI asked to confirm.
- **`Console`**: `sql_schema`, `sql_run`, `sql_page` (section 2).

The worker gains nine operations, **appended at the end of the op table** (`StoreOverview`, `StoreSegments`,
`StoreFields`, `StoreIndex`, `StoreMemory`, `StoreMaintain`, `SqlSchema`, `SqlRun` (cancellable), `SqlPage`). The
app's commands are `store_overview`, `store_segments`, `store_fields`, `store_index`, `store_memory`,
`store_maintenance → job id`, `sql_schema`, `sql_run → SqlOutcome` (an analysis job whose id is the channel's first
event, cancelled with `jobs_cancel`; `explain` is a flag of the run, not a command of its own) and `sql_page`. The
DTOs live in `imt_core::ports::{inspect, console}` and derive ts-rs behind imt-core's new `ts` feature;
`app/src/app/core/generated/store.ts` is generated from them and re-exported by `api.types.ts`.

### 2. The console: a sandboxed database of its own

A console query never runs in the store's database. Each store opens, on first use, an **in-memory DuckDB** that:

1. may read only the store's `segments/` folder (`allowed_directories`), then has `enable_external_access = false`,
   `autoload_known_extensions = false` and `autoinstall_known_extensions = false`;
2. has `memory_limit = 128MB` (inside the 300 MB query budget) and `threads = 2`;
3. holds the `imt` schema with the views below and `search_path = 'imt,main'`;
4. ends with `lock_configuration = true`, so no statement can change any setting (tested: `SET`, `RESET`,
   `enable_external_access`, `allowed_directories`, `lock_configuration` itself).

The tables behind `sources`, `fields` and `notes` are copied in from the catalog (and from the notes the app sends with
each run) when the store's generation or the notes changed; `events` is a view over the current segment files.
The store's catalog, its internal tables (`segments`, `segment_fields`, `events_wide`, `meta`, `analysis_cache`, the
promoted `p_*` columns) and its database file are not in the console's database at all.

**Every text is checked in Rust before it runs** (`imt-store-duck/src/console/guard.rs`):

- a lexical pass (quotes, `--` and `/* */` comments and `$tag$` strings skipped) allows exactly one statement whose
  first keyword reads (`SELECT`, `WITH`, `FROM`, `VALUES`, `TABLE`, `DESCRIBE`, `SUMMARIZE`, `PIVOT`, `UNPIVOT`,
  `SHOW`, a parenthesis); anything else (`COPY`, `ATTACH`, `INSTALL`, `LOAD`, `SET`, `PRAGMA`, `CALL`, `CREATE`,
  `INSERT`, `UPDATE`, `DELETE`, `DROP`, `EXPORT`...) is refused naming the keyword, with its byte offset and length for
  the editor to underline. A leading `EXPLAIN` turns the run into a plan; `EXPLAIN ANALYZE` is refused (it runs the
  query);
- DuckDB's own parser (`json_serialize_sql`, which serializes SELECT statements only) must answer exactly one
  statement; its syntax tree is walked: every `TABLE_FUNCTION` must be one of `range`, `generate_series`, `unnest`,
  `generate_subscripts` (no `read_*`, `glob`, `parquet_*`, `query`, `pragma_*`, `duckdb_*`), and no table name may look
  like a file or URL (`.`, `/`, `\`, `:`: DuckDB's replacement scans would read it).

The sandbox stands behind the checks (tested without them: no file outside `segments/` can be read, nothing written
outside it, no extension installed or loaded). Writes inside `segments/` would be possible for a statement that got
past both checks; there is none (COPY is not a SELECT), and the store sweeps unlisted segment files when it opens.

**Bounded.** A run keeps at most 10,000 rows (and 32 MB of cell text; cells longer than 4,096 characters are cut with
`…`) and says when it cut; the UI pages them 500 at a time from the last four results kept per store. Every column
whose type is not a plain scalar (timestamps, lists, maps, structs, decimals, huge integers) is cast to text in SQL.
A run stops after 30 s (an interrupt on the console's connection: "Stopped after 30 s. Narrow the time range, add
LIMIT or aggregate."), at its memory limit ("The query needed more than the SQL console's 128MB of memory..."), and at
once when its job is cancelled (the worker's cancel hook interrupts the statement). One run at a time per store; a
second is `Busy`. The SQL text is at most 64 KB; notes are cut to 2,048 characters, 5,000 notes.

### 3. The stable views (version 1)

Queries see only these, in the `imt` schema (also reachable unqualified). Columns are only ever added; a change of
meaning or a removal bumps `SQL_VIEWS_VERSION`. `docs/user-guide.md` lists the same columns.

| View | Columns |
|---|---|
| `events` | `id` UBIGINT, `ts` TIMESTAMP (UTC; NULL untimed), `level` VARCHAR (*Verbose ... Fatal*), `source` UINTEGER, `line` UINTEGER, `byte_offset` UBIGINT, `message` VARCHAR, `template` VARCHAR, `exception` VARCHAR, `fields` MAP(VARCHAR, VARCHAR) (every other field as text, declared fields included) |
| `sources` | `id` UINTEGER, `name`, `path`, `importer`, `parser` VARCHAR, `events`, `skipped` UBIGINT, `first_ts`, `last_ts` TIMESTAMP, `bytes` UBIGINT |
| `fields` | `key`, `type` (*text, integer, number, boolean*), `storage` (*declared, promoted, partly, overflow*) VARCHAR, `events` UBIGINT, `fill_rate` DOUBLE, `approx_distinct` UBIGINT |
| `notes` | `id`, `anchor` VARCHAR, `event_id` UBIGINT, `source_id` UINTEGER, `from_ts`, `to_ts` TIMESTAMP, `severity` VARCHAR, `tags` VARCHAR[], `text`, `author` VARCHAR, `created`, `updated` TIMESTAMP |

A result "carries event ids" when it has a column `id` of an integer type; the UI then offers *Open in Events*.

### 4. The UI

*View ▸ Store* (and *Open Store* / *Open SQL console* in the palette) opens the Store pane: tabs Overview, Segments,
Fields, Index, Memory and SQL. It shows what the engine reports and formats it; maintenance buttons are disabled with
the reason while an import runs and ask to confirm. The SQL tab has a monospace editor (`Ctrl+Enter` runs, `Esc`
stops, completion of views, columns and functions from `sql_schema`), an Explain switch, the error under the editor,
a results table that pages on, *Copy*, and *Open in Events* on rows with event ids. SQL texts are saved next to saved
queries (*Saved queries* lists QL and SQL items with badges; projects save both).

## Consequences

- Support and power users can see why a store is slow or large and answer ad-hoc questions without a new engine
  feature; the store's layout can still change freely behind the views.
- A second DuckDB instance per store costs a few MB once the console is used (its tables are small; `events` is a
  view over the segment files).
- *Open in Events* reveals the chosen row's event in the grid (it jumps to it); filtering the grid to a whole result's
  ids needs an `@id` pseudo-field in the query language, in both stores, and is left for later.
- Not done from the audit's sketch: worker uptime, last compaction time, a peak-since-start memory figure, Tantivy's
  own memory, "Recompute field statistics", "Checkpoint" and "Clear spill files" actions (compaction checkpoints),
  CSV export of a result (exports are engine work: a later `sql_export` job), the events grid component for results
  (a generic table is used).
