# 6. Clock-skew offsets are applied when records enter the workspace

- Status: accepted, amended by [ADR 0037](0037-store-clock-skew-and-live-tail.md) (2026-10-22): the offsets are applied to the stored
  events of the DuckDB store, through the `Skew` port; the in-memory workspace is only the reference implementation
- Date: 2026-10-17

## Context

Logs from several machines only line up on one timeline when their clocks agree. Users need to
shift a source by an offset (typed, aligned on one shared event, or suggested from correlation
ids shared across sources). The workspace keeps records in time order and a record's id is its
position; paging, the match cache, the timeline, charts, live tail and every analysis rely on
that invariant. The UI also keeps record ids (selection, bookmarks saved in projects).

Two places can apply an offset:

1. **At query time:** records keep their original timestamps; every query, view and analysis
   adds the source's offset. Ids never change, but the store is no longer in time order for
   the time the user sees, so every consumer must re-sort or translate, and each new view is a
   chance to forget.
2. **On load:** the stored timestamp is the corrected one. Everything downstream is unchanged.

## Decision

Offsets are applied on load (import, live-tail append and re-import, reload, project restore).

- Offsets are stored per source **path** (`ClockOffsets`), so they survive reloads, rotation
  (a rotated file keeps its offset; a new file at the old name gets it too) and projects
  (`.debriefproj` version 2, key `clock`, paths relative to the project like `sources`).
- Changing an offset moves one source's records with live tail's merge (two sorted runs,
  O(n), no full sort). Ids before the first moved record stay; later ones are renumbered
  exactly as a fresh load with that offset numbers them, so ids saved in a project match after
  reopening. `set_source_offset` returns the new ids of the records the caller tracks
  (selection, reference, bookmarks) and `idsChangedFrom`, like a live-tail report.
- The original timestamp is `ts − offset`; page rows carry `skewMs` and sources `offsetMs`.
- Offsets are bounded (±7 days) and validated in the engine; project files with bad values
  drop them instead of failing.
- Suggestions pair the earliest event per correlation id and source, take the median of the
  deltas within ±1 h, and report a confidence (share of pairs within 1 s of the median, scaled
  down below ten pairs). Causally related events (sent → received) are not simultaneous, so a
  suggestion is accurate up to the typical latency; the UI shows pairs and spread.

## Consequences

- Every view, filter and analysis reflects the shift with no per-view code.
- An offset change is a write (command–query separation): it renumbers ids after the first
  moved record, which the UI handles like a live-tail change.
- A store replacing the in-memory workspace (e.g. DuckDB) implements `set_source_offset` and
  `suggest_offsets` on `LogStore`; the defaults refuse / suggest nothing.
