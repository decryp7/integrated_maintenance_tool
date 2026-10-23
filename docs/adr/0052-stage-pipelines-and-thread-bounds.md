# 52. Stage pipelines and per-kind thread bounds

- Status: accepted (2026-10-23)
- Date: 2026-10-23
- Related: ADR 0012 (jobs), ADR 0014 (store), ADR 0048 (watched folders); the long-operations audit of 2026-10-23

## Context

Several long operations are a chain of stages (read, transform, commit) that ran on one thread or one stage at a time,
so a slow stage idled the others and the user saw one long wait. Each place also picked its own thread count.

## Decision

1. **`debrief_core::threads`** sizes background work by kind from the machine: `Cpu` = cores - 1 (or the Performance
   setting), `SsdIo` 4 to 8, `HddIo` 1 to 2, `Network` 4 at most, `Memory` by RAM (one thread per GiB beyond a reserve, 8 at
   most). `lower_priority()` puts a background thread behind the user's work where the OS allows: macOS QoS class
   `BACKGROUND` (efficiency cores, throttled IO), Linux nice 10 on the thread, Windows `THREAD_PRIORITY_BELOW_NORMAL`.
2. **`debrief_core::stages::Stages`** is the one pipeline: the caller's thread reads (IO) in chunks into a bounded queue,
   a bounded rayon pool maps the chunk's items (CPU), one thread commits in order. Queues are bounded (back-pressure,
   memory is a few chunks), order is kept through every stage, so the commit sees what a sequential loop would: results
   are identical to the sequential path (differential tests). Cancel stops the reader; queued chunks drain.

## Survey: which operations are stage chains

| Operation | Stages | Pipeline? |
|---|---|---|
| Import | discover, detect, parse (pool), write segments | Already one (ADR 0014 section 2; part A of the long-operations work extends it). |
| Full-text indexing | read segment batches (IO), build documents (pool), add to the writer in order | Done: batches of segments, one commit each; about 1.4x on 640k events. |
| Patterns | stream events in time order, tokenize (pool), cluster in order | Done: the clustering is order dependent, the tokenizing is not; identical patterns. |
| Exception groups | stream events, parse and fingerprint (pool), count in order | Done: identical groups, 2x or more. |
| Annotators | independent plugin calls | Parallel on a small pool, stored in registration order; cancel and counts through the view. |
| Home summary | anomalies, exception groups, timeline | One analysis job (progress, Cancel); the parts use the pipelines above. |
| Watched folders | detect, settle, batch, import | The batch decision is a pure `Batcher`; one job per batch. |
| Non-exact filters | scan to ids, then analyse | Single pass: the scan that writes the ids counts them. |
| Export | query pages, serialise, write | Recommended (part A): serialise in the pool, write in order; wins only for redaction or heavy formats. |
| Redacted copy | read, redact, write archive | Recommended (part A): sources are independent, redact on the pool, write the archive in order. |
| Search everything | scan sources, merge ranked | Already parallel per source with a 2 s budget; no change. |
| Dump analysis | read, symbolize, aggregate | Recommended: symbolize frames on the pool once PDBs are cached; do not re-read the dump when symbols arrive. |
| Symbol fetch | download (network), parse the PDB identity | A bounded gate of downloads; cancel and bytes now inside a download. Parsing is a header read, no pipeline. |
| Video analysis | hash, extract frames, pick, describe | Recommended: hash on a second thread while ffmpeg runs; the describe step is the AI queue. |
| Store compaction | read group, rewrite, rebuild positions | Recommended: rewrite groups on a pool (they are independent and atomic per group), then one rebuild. |

## Consequences

New long operations use `threads` for their bounds and `Stages` when read, work and commit can overlap. A stage that
must see items in order (clustering) keeps that stage single-threaded and moves only its pure part to the pool.
