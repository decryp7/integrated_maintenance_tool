# 10. Large workspaces use a persistent DuckDB store in a worker process, with a Tantivy index

- Status: amended by [ADR 0014](0014-database-first-engine.md) (2026-10-20): DuckDB is the only store, not an option for large
  workspaces; accepted 2026-10-18 after the spike in [store-spike.md](../store-spike.md)
- Date: 2026-10-18

## Context

`Workspace` keeps every record in memory as `Record`: a `String` message plus a JSON map of fields.
That costs about **1.9 KB of RAM per event** (measured: 1.92 GB at 1M events, 4.81 GB at 2.5M), so
10M events need about 19 GB. Reopening means re-importing everything, and facets and timelines are
linear scans (≈ 5 s and 3 s at 10M, extrapolated). The roadmap asks for data larger than RAM,
instant reopen, background indexing with progress and cancel, and a SQL console, behind the
`LogStore` port, while the in-memory store stays for normal sizes.

The spike compared DuckDB 1.5.6 (duckdb-rs, official prebuilt libduckdb) with DataFusion 55 over
Parquet. Each used the same Tantivy 0.27 full-text index and ran against the in-memory Workspace on
the samples scaled to 10M events. Summary at 10M (details, method and machine in the spike page):

| | DuckDB | DataFusion + Parquet |
|---|---|---|
| ingest (incl. parsing + FTS) | 138 s (72.7k ev/s) | 172 s (58.1k ev/s) |
| on disk (table + FTS), corpus 3.1 GB | 1,874 + 309 MB | **286** + 312 MB |
| peak RSS ingest / queries | 3.0 GB / 320 MB | **1.4 GB / 79 MB** |
| reopen | **23 ms** | 132–664 ms |
| unfiltered page (first/middle/last) | **2–6 ms** | 6–26 ms |
| filtered page with total | **88–127 ms** | 179–475 ms |
| facet / timeline (1000) / field stats | 112–172 / 132–344 / 252–443 ms | 145–431 / 170–517 / 182–258 ms |
| full-text (Tantivy) + fetch 500 rows | 253–305 ms | 273–369 ms |
| console cancel of a runaway query | **real interrupt, engine free at once** | returns on time, but the plan keeps its threads |
| executable growth (Windows, default release) | +5.5 MB + the 36.7 MB library | +130 MB (40.8 MB gzip for the whole exe) |

Both engines meet the interactive budget at 10M (every measured operation under 0.5 s cold).

## Decision

**Adopt DuckDB as the persistent store, running in a separate worker process** (`imt-store-worker`,
the worker pattern of ADR 0002), with a Tantivy full-text index in the same worker. A new adapter
crate (`imt-store-duck`) implements the `LogStore` port in the app by talking to the worker. The
in-memory `Workspace` stays the default for normal sizes.

Why DuckDB:

- **Faster where users feel it:** filtered pages 2–3× faster, reopen in 23 ms, a mature SQL dialect
  for the SQL console.
- **Real cancellation:** DuckDB's interrupt stops a runaway query and frees the engine at once
  (DataFusion only cancels cooperatively; a timed-out plan kept every worker busy in the spike).
- **Small app executable:** the app gains only the adapter and IPC code; DuckDB lives in the worker
  (DataFusion would add ~130 MB to the app executable).

Why out of process:

- **Crash containment.** libduckdb is C++. A segfault, abort or escaped exception in the app
  process would take every window down, and `guard::catch` can't contain native faults. The SQL
  console runs arbitrary user SQL, which is where database fuzzers keep finding crash bugs. In a
  worker, a crash costs a restart of the worker; the supervisor restarts it and the adapter reopens
  the store (stores are caches, see step 6).
- **Bounded memory.** DuckDB's memory limit and caches don't share the UI process's address space;
  when the worker exits the OS reclaims everything.
- **AGENTS.md.** IMT's own code stays Rust; the worker is a small Rust binary that links the
  prebuilt library, like `llama-server` runs as a separate process (ADR 0009).
- **One owner per file.** Only one process can open a DuckDB file read-write, so engine queries and
  the console must share one owner anyway; the worker is that owner.

The costs we accept:

- **IPC on the hot path.** Each query is one local gRPC round trip with a small Arrow result
  (pages, buckets and facets are capped by design): about 1–5 ms on queries of 100–300 ms. Ingest
  sends parsed records to the worker as Arrow batches: a few seconds on a 2-minute 10M ingest.
- **Disk:** 1.9 GB against Parquet's 0.29 GB for the same 10M events, untuned. Step 2 measures
  DuckDB compression settings and column types before release; stores are caches and can be
  deleted from Settings.
- **Distribution:** the worker links the official prebuilt libduckdb, fetched at **build time** in
  CI by `crates/imt-store-spike/scripts/fetch-libduckdb.sh` (pinned SHA-256 per platform; never the
  `bundled` C++ build) and shipped next to the worker; on macOS both are signed with the app.
  duckdb-rs is version-locked to the library.

### Communication

- **Transport:** `imt-ipc`, the existing local gRPC over a private Unix socket or Windows named
  pipe, with a one-time token per launch handed over on stdin; the worker exits when stdin closes.
  Nothing listens on the network.
- **Contract:** a new versioned `imt.store.v1` service (protobuf), small and purpose-built: open /
  close / ingest (client-streamed Arrow batches with progress) / page / timeline / facet / chart /
  field_stats / search / source_context / sql (console) / cancel / status. Results are Arrow record
  batches (as `imt.plugin.v1` already uses) converted into the existing DTOs.
- **Supervision:** the existing supervisor — health checks, restart on crash with back-off, call
  timeouts, a watchdog. `cancel(query_id)` calls DuckDB's interrupt; if the worker does not answer
  within a grace period it is killed and restarted.

### Migration plan

1. **Port first.** Add `imt-store-duck` implementing `LogStore` over the worker. The Tauri shell
   already depends only on the trait (ADR 0001). Start with `open`/`reload`/`clear`/`summary`/`page`/
   `timeline`/`facet`/`chart`/`field_stats`/`source_context`/`state`/`restore`/`progress`. Run every
   other operation (patterns, exceptions, anomalies, traces, compare, moment, exports, bundles,
   reports) by **streaming record batches from the worker through the existing imt-core analysis
   code**, bounded by the existing result caps. Push operations down one at a time afterwards, with
   equivalence tests.
2. **Typed columns.**
   - Always: `id` (time-ordered ordinal, as in `Workspace`), `ts`, `level`, `source`, `line`,
     `markers`, `message`, `template`, `exception`.
   - Promoted fields, from the profile: the `[instance] key` fields, typed `[[extract]]` fields, and
     fields used by `[[chart]]`, markers and dashboards. From the data: fields present in ≥ 10% of
     the first ingest chunk with one consistent type (the existing `FieldIndex` counts this).
     Promoted columns are VARCHAR, BIGINT, DOUBLE or BOOLEAN.
   - Everything else, and values that don't fit their column's type, go to a residual
     `MAP(VARCHAR, VARCHAR)` column. Measure compression (dictionary/FSST, zstd) here.
   - An optional profile key `[store] promote = [...]` (documented in `default_profile.toml`)
     overrides the choice; changing it rebuilds the table in the background.
3. **Query compilation.** `CompiledQuery` and the QL compile to parameterised SQL built by a small
   typed builder in the worker — values are always bound parameters, identifiers come only from the
   known column set, never spliced user text:
   - levels → `level IN (?)`; markers → a bitmask test; time range → `ts` bounds.
   - The search box → `ILIKE ? ESCAPE` over message/exception/template. Regex → `regexp_matches`,
     only after `user_regex::compile` has validated the pattern (DuckDB uses RE2; differences from
     the Rust `regex` crate are covered by equivalence tests and fall back to a scan in the worker).
   - `Condition` ops on promoted columns → typed comparisons; on residual fields → `map_extract`
     with imt-core's `value_text` semantics.
   - QL stages: `where` → `WHERE`, `stats ... by` → `GROUP BY`, `sort` → `ORDER BY`, `head` →
     `LIMIT`, `fields` → projection. Anything not expressible (script steps, future functions) runs
     on streamed batches in imt-core, so behaviour stays single-sourced.
   - Equivalence tests run the same `Query` against `Workspace` and the new store on `/samples` and
     generated data and compare the results, as the spike did by digest.
   - Full-text tokens go to Tantivy; a token query pre-filters with the Tantivy id set (semi-join);
     substring and regex filters stay scans.
4. **Ingest and ordering.** Ingest appends with DuckDB's Arrow appender (fixing the spike's
   row-by-row bottleneck; the worker needs only one Arrow version). Real files overlap in time, so a
   background step rewrites into time order and assigns the time-ordered ids, reporting
   `idsChangedFrom` as clock-skew changes already do (ADR 0006). Filtered pages cache the matching id
   set per query, like the in-memory `MatchCache`; keyset paging (`id > last`) serves scrolling.
5. **Background indexing, progress, cancel, resume.** Import runs off the UI thread with the
   existing `ImportProgress`; the table is queryable when ingest finishes and Tantivy keeps up on
   its own thread (*indexing n%*). Tantivy commits every 2M documents with `indexed_to=<id>`;
   cancel rolls back to the last commit and the next open resumes from there.
6. **Reopen.** A store lives in `<data>/stores/<hash of roots + profile>/` with a versioned manifest
   (schema version, profile hash, per-file fingerprints, DuckDB and IMT versions, `indexed_to`).
   When the fingerprints match, reopen is the manifest plus opening the database (23 ms at 10M).
   Changed or new files are re-imported incrementally. A missing, corrupt or version-mismatched
   store is rebuilt from the sources (degrade, don't fail). Stores are caches: deleting them loses
   nothing.
7. **When to switch.** The in-memory store stays the default. Before importing, IMT estimates the
   footprint from file sizes (≈ 1.9 KB/event in RAM, ≈ 330 bytes/event of CLEF text) and what is
   already loaded. Above a budget (setting, default: the smaller of 25% of physical RAM and 4 GB),
   or when the user chooses *Open as large workspace*, it opens with the persistent store. A project
   remembers its choice. Live tail keeps using the in-memory store until a later hybrid phase.
8. **SQL console.** A separate panel whose queries run in the same worker on a **read-only
   connection**: no DDL/DML, `SET`, `COPY`, `ATTACH`, `INSTALL`/`LOAD`; extensions, file and URL
   table functions disabled (`enable_external_access = false`, `autoload_known_extensions = false`,
   `lock_configuration = true`); only the `events` and `sources` views exist. Text capped at 64 KB;
   every query has a timeout (default 30 s, at most 5 min) enforced by interrupt, then by killing
   the worker after a grace period; a per-query memory limit; a row cap on what is returned (default
   1,000, at most 10,000, paged). Results reach the UI only as typed cells; nothing is bound with
   `[innerHTML]`.

### Later: DuckDB 2.0 and its server mode

DuckDB 2.0 (announced for late 2026) promotes its client/server mode ("Quack", `CONNECT`) to
stable, alongside a reworked C API, a new storage format and async I/O. When 2.0 is released with a
patch release, duckdb-rs (or another maintained Rust client) supports it, and the server mode is
stable on all three platforms, **migrate to it**:

- Replace the custom worker's engine host with DuckDB's own server process, supervised by IMT like
  `llama-server` (ADR 0009): local only, a random port or socket, an access token, external access
  and extensions disabled. `imt-store-duck` talks to it through the Rust client; IMT keeps its own
  query builder, paging, caps and cancel semantics on top. Tantivy moves into a small Rust sidecar
  or stays in the IMT-side adapter.
- Re-run the spike's benchmarks on 2.0 first (its claimed speedups and the new storage format may
  change the disk and filtered-scan numbers) and keep the `imt.store.v1` worker as the fallback until
  the server mode has proven itself.
- The `LogStore` port and the equivalence tests make this an adapter change, not an app change.

### Licences and dependencies

DuckDB and duckdb-rs are MIT; Tantivy is MIT; Arrow (in the worker and adapter) is Apache-2.0; all
on `deny.toml`'s allow-list. DataFusion is not adopted. The spike crate stays outside the workspace
(`exclude`), `publish = false`, as the benchmark harness for regressions until the adapter lands.

## Consequences

- Workspaces of tens of millions of events reopen in milliseconds and query interactively; the
  in-memory path, and its speed for normal sizes (sub-ms cached pages), is unchanged.
- A crash in DuckDB costs a worker restart, never the app. The SQL console can be cancelled for
  real.
- A second `LogStore` implementation means the Liskov rule bites: every operation must have the
  same semantics and error behaviour in both. The equivalence tests in step 3 are the gate; features
  are not done until they work on both stores.
- One more supervised process and a new IPC contract (`imt.store.v1`) to version and test, including
  crash, hang and restart tests of the worker.
- A native library per platform to pin, verify and sign (build time only, no runtime download), and
  larger stores on disk than Parquet would give.
- DataFusion remains the fallback if the native-library or disk costs ever outweigh speed: the
  spike's DataFusion adapter shows the way.
