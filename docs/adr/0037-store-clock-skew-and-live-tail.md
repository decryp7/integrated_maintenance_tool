# 37. Clock skew and live tail on the log store

- Status: accepted (2026-10-22)
- Date: 2026-10-22
- Amends: [ADR 0006](0006-clock-skew-applied-on-load.md) (the mechanism moves from the in-memory workspace to the store)
- Related: ADR 0014 (DuckDB store, ports, store worker), ADR 0017 (out-of-process workers), ADR 0020 (mixed
  sources), AGENTS.md (engine is the source of truth, parse don't validate, contain failures, bounded resources)

## Context

The in-memory `Workspace` shifted a source's timestamps when records entered it and re-merged them when an offset
changed; its live tail re-read changed files, computed alerts from its records and kept a per-file state. The log store
(ADR 0014) answered `unsupported` for all of it: clock skew, live alerts, the per-file tail status and "tail from end".
It also re-imported a whole file whenever it changed, which is quadratic for a log that grows all day.

## Decision

1. **Two small ports next to `Ingest`.** `Skew` (offsets by file path, set a source's offset manually or by aligning
   two events, suggest offsets from shared correlation ids) and `Tail` (how each followed file was last read, open files
   at their end). `Ingest::append_tail` returns a `TailAppend` (what changed, per kind of source) instead of a count.
   Every store implements them (`WorkspaceStore` as the reference, `DuckStore`, `RemoteStore` over six new `StoreOp`s);
   suggestions are one function over `Query::scan` (`debrief_core::skew::suggest_over`) so no store re-implements the maths.
2. **An offset is applied to the stored events.** `clock_offsets(path, offset_ms)` is the truth; events carry the
   corrected `ts`. Files read later (imports, appended lines) are shifted as they are written (`SegmentSink`); changing
   an offset rewrites the segments holding that file's events with `ts` moved by the difference, keeping each segment's
   id block (an event keeps its id when it moves in time; `OffsetChange.ids_changed_from` is always `None`), and swaps
   them for the old ones together with the offset, the sources' time ranges and the histogram in one catalog
   transaction. The time-order positions are renumbered afterwards. Query-time shifting was rejected: every SQL path,
   the histogram, the full-text index of times, the analyses and projects would have to know about it. The full-text
   index holds no times, so it is untouched. Offsets travel with the project (`project.toml` repeats them; the store
   holds them), and a restored state sets them before it reads the files.
3. **Live tail reads only what was appended.** `tail_offsets(source, byte, line, size, whole, held, note, changed_ms)`
   says where each followed file is read next. Appended lines become a new segment of the same source (new ids,
   existing ones stay), the source's counts and range move on, and the positions are extended after the events they
   sort behind (`pos::append`; a rebuild only when new events do not sort last). A file that has no row (imported in
   full) gets its position from its last event the first time it grows. What cannot be read from the middle is read
   again in full as a new source: a file that shrank, archives, binary formats, whole-file importers, files routed to
   several sources, and files whose last line was not finished when they were opened. A deleted file keeps its events.
   Per-file work runs under `guard::catch`; one unreadable file is logged and does not stop the others.
4. **The last event of a plain-text read is held back.** Text events may continue on following lines (stack traces), so
   a read that ends in the middle of the file does not write its last event; the next read starts at it. It is written
   when another event follows, or when the file has not grown for `HOLD_IDLE` (500 ms). `TailAppend.held` tells the
   caller to ask again with no paths after that time; the shell does so once per wait. One-line formats (CLEF, JSON
   lines) are never held. This costs a short delay for the newest event of a multi-line format and avoids splitting
   an event across two rows.
5. **Alerts and the tail status are computed over the ports.** `debrief_core::live::LiveSession` (one per window, in the
   shell) evaluates rules, silences and the anomaly detectors over `Query`/`Analysis` after each refresh, keeps
   cooldowns and mutes, and builds the per-file status (lines per second from the store's line counts) from
   `Tail::tail_files`. Nothing about alerts is stored in the catalog.
6. **Tail from end** is `Tail::open_at_end`: each file is planned as an import would, its source is created with no
   events, and its row in `tail_offsets` starts at the end of its last whole line.

## Consequences

- A refresh costs what was appended (plus the catalog work), not the size of the store; ids of existing events are
  stable, which bookmarks and selections rely on.
- A skew change costs the rewrite of the segments holding the file's events (as a compaction would), under the
  store's write lock, and fails with `Busy` while an import runs.
- Resolved afterwards (engine-c): the idle hold is `TailOptions.carry_flush_ms` (default 500 ms), driven by the shell,
  which asks for the held events with `append_tail(&[])` after that wait (the store has no timer); `deleted_files:
  drop` removes the sources of vanished files in `LiveSession`; `retention` was deleted (the store is on disk, so there
  is no memory to bound, and nothing evicted events). The first follow of a file imported in full re-reads its last
  event: unchanged, it is not written twice; continued by lines written since, the source is read again in full, so a
  stack trace stays one event (new source, new ids, once).
- Still open: annotators do not re-run after a tail refresh.
