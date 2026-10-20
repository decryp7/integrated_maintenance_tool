# Persistent store spike: DuckDB vs. DataFusion + Parquet (with Tantivy)

> **Historical.** This records a finished measurement; it does not describe the current design. The
> engine direction is [ADR 0014](adr/0014-database-first-engine.md) and the plan in
> [redesign-2026-10.md](redesign-2026-10.md). The spike crate stays (outside the workspace) only until
> phase R2 has moved its reusable parts (libduckdb fetch, Tantivy schema in `fts.rs`, the corpus
> generator in `corpus.rs`, the RSS guard in `rss.rs`) into the new store crates; then it is deleted.

The roadmap's Phase 6 *persistent store* (data larger than RAM, instant reopen, background
indexing with progress and cancel, SQL console) asked for a spike before a decision. This page
records how it was measured and what came out. The decision is in
[ADR 0010](adr/0010-persistent-store.md).

The code is the crate `crates/imt-store-spike` (excluded from the workspace and built on its own;
`publish = false`, not a dependency of the app or of
any shipped crate). Each engine is a cargo feature (`duck`, `fusion`, both on by default).

## What was built

**A benchmark harness, not a `LogStore` implementation.** `LogStore` has about 40 methods. Many
return types that borrow the in-memory records (`Page<'_>` holds `RecordView<'a>`), and many depend
on plugins, live tail, redaction, skew, bundles and reports. A faithful implementation is the
migration itself, not a spike. Instead the spike defines a small port, `SpikeStore`
(`src/port.rs`), with the operations that were measured, shaped like the `LogStore` requests they
stand for:

| `SpikeStore` | stands for | notes |
|---|---|---|
| `page(filter, offset, limit)` | `LogStore::page` | total + rows in time order |
| `facet(column, filter, limit)` | `LogStore::facet` | total = events with the field, exact distinct, top values |
| `timeline(filter, buckets)` | `LogStore::timeline` | per-level histogram + instance lanes |
| `field_stats(column, filter)` | `LogStore::field_stats` | count, missing, exact distinct, min/max/mean, p50/p95/p99, 20-bin histogram, top 20 |
| `search(text, limit)` | the search box / a future full-text query | hit count + first hits in time order, rows fetched |

There are three implementations:

- **`InMemory`** (`src/inmem.rs`) calls the shipped `Workspace` **through the real `LogStore`
  trait**, so the baseline is the product as it is.
- **`DuckStore`** (`src/duck.rs`) uses duckdb-rs 1.10506.1 linked against the official prebuilt
  **libduckdb 1.5.6**. `scripts/fetch-libduckdb.sh` downloads the pinned release zip into a cache
  directory, checks its SHA-256 (pinned from the GitHub release API for all six platforms), and
  prints `DUCKDB_LIB_DIR`/`DUCKDB_INCLUDE_DIR`. Nothing is compiled from C++ (`bundled` is off).
  There is one `events` table, filled with the appender and queried with parameterised SQL.
- **`FusionStore`** (`src/fusion.rs`) uses DataFusion 55.2 over Parquet (arrow/parquet 59). Each
  ingest chunk becomes one Parquet file (zstd level 3, 64k-row row groups, page statistics). Queries
  are logical plans built with the DataFrame API, not SQL text, which is how the query language
  would compile. The files are registered as a listing table with a declared sort order on `id`.

Both persistent stores use the **same Tantivy 0.27 index** (`src/fts.rs`) for message and
exception text. Indexing runs on its own thread, fed through a bounded channel. It reports progress
and can be cancelled, which rolls back to the last commit. It commits every 2M documents with
`indexed_to=<id>` as the commit payload, so a reopened store knows where to resume. The column
store is queryable before the index is complete.

### Schema (both engines)

| kind | columns |
|---|---|
| fixed | `id` u64 (time-ordered ordinal, as in `Workspace`), `ts` i64 ms, `level` u8, `source` u32, `line` u32, `markers` u64, `message`, `template`, `exception` |
| promoted fields | `machine_name` text, `process_id` i64, `source_context` text, `status_code` i64, `elapsed` f64 |
| residual | `fields`: the remaining fields as JSON text |

A value that does not fit its column's type (for example a `ProcessId` of `"n/a"`) stays in the
residual JSON, so promoting a field never loses data (`src/rows.rs`).

### Corpus: the samples scaled to 10M events

`imt-store-spike gen <dir> 10000000` loads `/samples` with the sample profile through the real
engine: 21,651 timestamped events from 11 files spanning 4 h 46 min. It then writes 462 copies as
CLEF. Copy *k* is shifted by *k* × 4 h 48 min (back to back, so about 92 days in all). Its
`MachineName` gets the suffix `-NN` (*k* mod 20), giving 80 machine names and 3,352 instance lanes.
The result is 5,080 files, 3.1 GB and exactly 10,000,000 events. Ingest reads the corpus back
through the real importers and profile (extractors, markers), 23 copies (about 500k events) per
chunk. The copies don't overlap in time, so chunk order is time order and the store-wide ids
are time-ordered without a global sort. Real data overlaps; see the ADR for compaction.

### Method

- **Machine:** AMD Ryzen AI MAX+ 395 VM with 8 vCPUs, QEMU virtual SSD and 32 GB visible RAM.
  The RAM is shared with other builds, so runs were capped at 8 GB RSS (`SPIKE_MAX_RSS_GB`).
  Windows 11 Pro 10.0.28000, rustc 1.99.1, release profile, `-j 2`.
- **Fairness:** both engines get 4 threads and a 2 GB memory limit (DuckDB `memory_limit`,
  DataFusion `FairSpillPool`).
- **Processes:** ingest and query run in separate processes, so query RSS is not inflated by
  ingest. *Reopen* is a fresh process opening the store and reading `count`/`min(ts)`/`max(ts)`
  plus opening the Tantivy reader. Query processes were run twice, and both runs are reported for
  the persistent stores.
- **Timing:** each query runs 3 times. *first* is the first call, *best* the fastest of three. The
  Workspace caches matching ids per query, so its repeats are cache hits.
- **RSS:** the resident set size is sampled every 20 ms in a background thread (`memory-stats`),
  and the peak per phase is reported.
- **Workspace at 10M:** the in-memory store needs about **1.9 KB of RAM per event** (1.92 GB at
  1M, 4.81 GB at 2.5M events), so 10M events would need about 19 GB, more than this machine could
  give. It was measured at 1M and 2.5M; its queries are linear scans, so they scale linearly.
- **Filters:** "filtered" means level ∈ {Warning, Error, Fatal} AND text "timeout" (a
  case-insensitive substring of message, exception or template, as the search box does) AND
  `MachineName = WEB01-01`. That matches 4,296 of 10M events.

## Results

### Ingest, size, memory, reopen

| | Workspace 1M | Workspace 2.5M | DuckDB 10M | DataFusion + Parquet 10M |
|---|---|---|---|---|
| events | 995,946 | 2,511,516 | 10,000,000 | 10,000,000 |
| ingest wall time | 3.0 s | 10.0 s | **137.6 s** (table durable after 124.4 s) | **172.2 s** (table durable after 148.5 s) |
| throughput (incl. parsing and FTS) | 332k ev/s | 252k ev/s | 72.7k ev/s | 58.1k ev/s |
| of which: parsing (imt-core) / store writes | n/a | n/a | 41.3 s / 82.9 s | 74.9 s / 73.6 s |
| FTS progress when the table was done | n/a | n/a | 9.46M of 10M (finished 13 s later) | 9.46M of 10M (finished 24 s later) |
| on disk: table | none (RAM only) | none | **1,874 MB** | **286 MB** |
| on disk: Tantivy index | n/a | n/a | 309 MB | 312 MB |
| peak RSS during ingest | 1,923 MB | 4,813 MB | 3,022 MB | 1,381 MB |
| reopen | re-import (≈ ingest) | re-import | **22–23 ms**, 21 MB RSS | **132–664 ms**, 24 MB RSS |
| peak RSS during the query suite | 1,832 MB | 4,605 MB | **320 MB** | **77–79 MB** |

For scale, the corpus is 3.1 GB of CLEF text. Parquet + Tantivy together take 598 MB (19%) and
DuckDB + Tantivy 2.18 GB (70%). The DuckDB file was written with default settings (dictionary/FSST
string compression after `CHECKPOINT`), and its size was not tuned.

Ingest RSS includes imt-core parsing one ~500k-event chunk into a `Workspace` (about 1 GB) before
it is written. Ingest wall time is dominated by imt-core parsing (shared by both engines, parallel
inside each chunk) plus row-by-row conversion. The DuckDB appender is row-at-a-time through the C
API, and its Arrow appender was not usable because duckdb-rs is on arrow 58 and DataFusion on 59.

### Queries (ms, *first / best of 3*)

Persistent stores: first query process (right after ingest) → second query process (best).

| operation | Workspace 1M | Workspace 2.5M | DuckDB 10M | DataFusion 10M |
|---|---|---|---|---|
| page 1 (unfiltered, by time) | 5.1 / 0.1 | 21.3 / 0.2 | 5.5 / 2.5 → 2.8 | 25.7 / 9.7 → 6.7 |
| middle page | 0.4 / 0.1 | 0.5 / 0.2 | 3.8 / 2.7 → 2.5 | 17.1 / 11.8 → 8.1 |
| last page | 0.4 / 0.1 | 0.4 / 0.2 | 2.6 / 1.9 → 1.8 | 17.3 / 8.9 → 6.1 |
| filtered page 1 (level + text + field, with total) | 8.0 / 0.2 | 19.7 / 0.5 | 127 / 88 → 101 | 459 / 302 → 179 |
| filtered middle page | 0.3 / 0.1 | 0.9 / 0.6 | 88 / 88 → 97 | 320 / 320 → 197 |
| filtered last page | 0.2 / 0.2 | 0.5 / 0.3 | 101 / 93 → 97 | 475 / 382 → 192 |
| facet `MachineName` (all events) | 491 / 491 | 1,275 / 1,252 | 132 / 132 → 159 | 341 / 272 → 145 |
| facet `SourceContext` (warnings+) | 57 / 54 | 140 / 135 | 118 / 118 → 112 | 431 / 388 → 183 |
| timeline, 1000 buckets (all) | 238 / 231 | 1,064 / 739 | 294 / 211 → 195 | 517 / 261 → 195 |
| timeline, 1000 buckets (errors) | 223 / 203 | 691 / 648 | 344 / 200 → 132 | 255 / 255 → 170 |
| field stats `Elapsed` | 241 / 241 | 482 / 482 | 443 / 279 → 252 | 258 / 216 → 182 |
| full-text "timeout" (count + first 500 by time + fetch rows) | 35 / 0.3 ¹ | 109 / 0.3 ¹ | 305 / 253 → 254 | 369 / 369 → 273 |

¹ The Workspace has no inverted index. This is its substring scan (match cache on repeat).

Extrapolated linearly, the Workspace at 10M would take about 5 s per facet and 3 s per timeline,
and it would not fit in memory on this machine. Both persistent stores answer every measured
operation at 10M in **under 0.5 s cold and under 0.4 s warm**, in under 320 MB of RSS.

Unfiltered pages are id ranges (ids are time-ordered ordinals) and cost milliseconds in both
engines. Filtered pages pay a full scan for the total count (`count(*)` + `ORDER BY id LIMIT …
OFFSET …`), so they cost about 100 ms (DuckDB) and 180–460 ms (DataFusion).

### Do the stores agree?

All three agree exactly on page totals and ids, filtered totals (4,296), facet totals, distinct
counts and top values, timeline event counts, and field-stat count, distinct, mean and max. The
expected differences are:

- **Percentiles:** both engines use approximate quantiles (`approx_quantile`,
  `approx_percentile_cont`). p95 of `Elapsed` came out at 1,769–1,773 against an exact 1,771.
- **Full-text vs. substring:** Tantivy matches the *token* `timeout` (238,368 hits at 10M). The
  search box's substring also matches `TimeoutException`, `timeouts` and so on (≈ 348k at 10M,
  87,348 at 2.5M). The product needs both: tokens for search, substring/regex as a filter.
- **Lanes:** the engines key a lane by (`MachineName`, `ProcessId`), or by source when both are
  missing (3,352). The Workspace's lane rules (lifecycle, grouping) differ (127 at 2.5M). This is
  a spike simplification, not an engine limit.

### SQL console guards (`imt-store-spike console …`)

| check | DuckDB (read-only database, `enable_external_access=false`, `lock_configuration=true`) | DataFusion (`SQLOptions`: no DDL, no DML, no statements; only registered tables) |
|---|---|---|
| `SELECT … GROUP BY level` | ok, 10 ms | ok, 24–193 ms |
| row cap (1000) on `SELECT id FROM events` | ok, 1000 rows | ok, 1000 rows |
| `DELETE` / `CREATE TABLE` | refused (read-only) | refused at planning |
| `read_csv('C:/Windows/win.ini')` | refused (file system disabled) | refused (no such table function) |
| `COPY … TO 'file'` | refused (file system disabled) | refused at planning |
| runaway cross join, 2 s timeout | **interrupted after 2.2–3.6 s; the next query ran in 16 ms** | first attempt: `tokio::time::timeout` returned after **7,419 s**. With the deadline on an OS thread and `abort()`: returned at 2.0 s, but **the aborted plan kept every worker busy and the next query timed out too** |

DuckDB's `interrupt()` is a real cancellation. DataFusion's cancellation is cooperative: a
non-yielding operator (here a nested-loop join) keeps its threads until it finishes. The ADR
therefore runs the DataFusion console out of process.

### Build, binary size, licences, platforms

| | DuckDB (prebuilt) | DataFusion + Parquet | Tantivy |
|---|---|---|---|
| licence | MIT (DuckDB and duckdb-rs) | Apache-2.0 (DataFusion, Arrow, Parquet) | MIT |
| native code | C++ `duckdb.dll` / `libduckdb.so` / `libduckdb.dylib`, loaded in process | none (pure Rust; zstd-sys is C, already common) | none |
| shipped size (Windows, default release profile) | **+5.5 MB** exe (duckdb-rs + arrow 58) **+ 36.7 MB** `duckdb.dll` (12.7 MB gzip; release zips: Windows 13.4 MB, Linux 41 MB, macOS universal 36 MB) | **+129.8 MB** exe (no LTO/strip; the whole exe compresses to 40.8 MB gzip) | part of the 7.6 MB base |
| build | download + SHA-256 check of a pinned release per target; version locked to duckdb-rs (1.10506.x = 1.5.6); the library must be shipped next to the exe (or downloaded like the llama runtimes, ADR 0009) | Rust only, but heavy: the first `-j 2` release build of the spike (both engines, ~600 crates) took **35 min** | Rust |
| platforms | official builds for Windows x64/arm64, Linux x64/arm64 (glibc and musl), macOS universal | wherever Rust builds | wherever Rust builds |
| crash containment | a C++ crash or abort takes the process down; `guard::catch` can't contain it | Rust panics are caught like any plugin | Rust |

`cargo deny --workspace check licenses bans` passes with the spike in the workspace: every
dependency of DataFusion, Arrow, Parquet, Tantivy and duckdb-rs is on `deny.toml`'s allow-list.

**Binary size** (Windows x64, the workspace's default release profile, the same one the app
uses: no LTO, no strip): the spike executable with neither engine (imt-core + Tantivy + CLI) is
7.6 MB, with DuckDB 13.1 MB (+ the 36.7 MB DLL next to it), with DataFusion 137.4 MB, with both
138.5 MB. DataFusion's size is mostly monomorphised kernels. Trimming features further (the spike
already turns off compression codecs, crypto, encoding, nested and regex functions), `lto = "thin"`,
`codegen-units = 1` and `strip` would shrink it, but these were not measured, because changing the
profile means a full rebuild.

## Reproducing

```sh
eval "$(crates/imt-store-spike/scripts/fetch-libduckdb.sh target/agent-store/libduckdb)"
export CARGO_TARGET_DIR=target/agent-store
cargo build -j 2 --release --manifest-path crates/imt-store-spike/Cargo.toml --target-dir target/agent-store
B=target/agent-store/release/imt-store-spike
$B gen  "$TMP/corpus" 10000000              # 5,080 CLEF files, 3.1 GB, ~16 min
$B mem  "$TMP/corpus" 116                   # Workspace, 2.5M events (needs ~5 GB)
$B ingest duck   "$TMP/corpus" "$TMP/duck"   && $B query duck   "$TMP/duck"
$B ingest fusion "$TMP/corpus" "$TMP/fusion" && $B query fusion "$TMP/fusion"
$B console fusion "$TMP/fusion"             # SQL console guards only
```

Each line of output is one JSON measurement. `cargo test --manifest-path crates/imt-store-spike/Cargo.toml` covers the pure parts:
filter → SQL and filter → DataFusion compilation (including escaping and injection), column
promotion, bucketing, Tantivy background indexing, resume payload, lenient query parsing, and cancel
→ rollback.
