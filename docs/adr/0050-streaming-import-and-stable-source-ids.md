# 0050. Streaming import and stable source ids

Status: accepted

## Context

Opening a large folder used to gather the complete sorted file list first, then look at every file (size, archive
entries) and only then read the first one. Source ids were handed out in the sorted order of the paths, which is why the
whole list had to exist before any file was read: nothing was imported for minutes on a tree of 100,000 files. A reload
also renumbered every source, although notes, saved queries, links and projects refer to source ids.

## Decision

**Producer-consumer pipeline.** Discovery and import run at the same time (`debrief_core::ingest::import_stream`):

- Discovery (`discover_stream`) lists folders on I/O-bound walkers (up to 16) that work ahead on the next sub-folders,
  but *emits* files in one order: a depth-first walk that visits each folder's entries by name. That is exactly how
  `PathBuf` sorts, so the stream equals the sorted list whatever the walkers did (several roots stream one after
  another unless one lies inside another, then they are merged and sorted). The walkers also stat each file and look
  for archive entries, so the dispatcher never reads a file itself.
- A bounded queue (8,192 paths) connects discovery to the dispatcher, which takes files in order, sets aside the source
  ids each needs (`reserve_sources`, in memory only: the catalog is written with the segments) and hands the file to
  a CPU-bound parse pool, keeping at most `2 x threads` files in flight. A full pool stops the dispatcher, a full
  queue stops the walk: memory stays bounded.
- Cancel stops both sides at their next entry; files found but not started are listed as not imported.
- Progress: "finding" with a growing count until the first file is read, then "importing" of the files found so far
  (`finding` set: the total still grows, no estimate) until the walk ends and the total settles.

**Determinism without a barrier.** Ids are assigned by the single dispatcher in the order of the sorted stream, so they
are identical for 1, 2, 8 or 16 workers and for any timing of listings and reads (`StreamOptions::jitter` randomises
both in tests). The alternative considered, importing into staging segments and renumbering at commit, would rewrite
every segment; ordering the stream avoids that.

**Ids are never reassigned.** New sources of an import get ids after the highest one ever used, in sorted path order.
On reload an existing source keeps its id:

1. by path (archive members by `archive!entry`);
2. else a file that is new on disk whose fingerprint (size, hash of the first and last 64 KiB, kept in
   `source_fingerprints`) equals that of exactly one source whose file is gone is that source, renamed or moved; the
   source records the new path;
3. ambiguous matches (copies of one file) are never guessed: the new files become new sources and the import report
   carries a notice.

Archive members and sources imported before fingerprints existed match by path only. Moving the whole folder keeps
the relative order, so a fresh import numbers the files identically.

## Consequences

Time to the first imported event on 100,000 small files plus three large ones fell from about 3.5 s (gather first) to
under 10 ms; total time is unchanged within noise (`crates/debrief-core/tests/stream_bench.rs`). Files are no longer
read largest-first, so a very large file found late can finish last. Reload cost grows by one fingerprint pass only
when a source's file vanished.
