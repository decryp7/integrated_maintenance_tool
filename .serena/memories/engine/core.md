# Engine (crates/debrief-core and the store crates)

- Today the app still uses the in-memory `Workspace` behind the wide `LogStore` trait (`crates/debrief-core/src/store.rs`); ADR 0014 replaces it (phase 6c switches the app). Don't add new features to `Workspace` that the new store would have to copy; add them behind the ports (`debrief_core::ports`).
- New engine pieces:
  - `debrief-batch`: EventId(u64, stable, never reused), SourceId, TsState (parsed/inherited/untimed), EventSchema/EventBatch (Arrow 58).
  - `debrief_core::row::RowRef`: one row abstraction over Record and Arrow rows; analyses/conditions should use it.
  - `debrief_core::ingest`: ChunkSource (newline-bounded byte chunks with offsets), FilePlan sniff, RecordBatcher, BatchSink; streaming text/JSON importers; legacy adapter.
  - `debrief-store-duck`: events in zstd Parquet segments (parallel writers, contiguous id block per segment), native DuckDB tables for meta/sources/diagnostics/marker_kinds/segments manifest/ord(id,pos); `events` is a read-only VIEW; rewrites = new segments + generation swap; pages are two-phase (ids via ord/pos, then rows).
  - Tantivy index = rebuildable cache, never shared.
- Projects (`.debrief` folder / `.debriefz` zip) are self-contained: raw text is stored, original logs optional.
- Memory budgets (ADR 0014): app flat < 250 MB; worker ≤ 300 MB idle/query, ≤ 768 MB importing (hard cap 2 GB; planned Low memory mode ≤ 400 MB); DuckDB memory_limit 256 MB.
- Known traps (engine review, ADR 0014, redesign R2): record ids were positions (don't index `records[id]` in new code); importers carry per-file state (day order, midnight rollover, line numbers) — chunking needs FilePlan; four predicate languages being unified into the QL Expr; timestamp parsing must use a stored parse environment, not `Utc::now()`/local zone.