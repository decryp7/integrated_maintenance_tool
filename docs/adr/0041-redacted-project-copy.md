# 41. Redacted project copy

- Status: accepted (2026-10-22)
- Related: ADR 0007 (reports, PDF and redaction), ADR 0014 (DuckDB store, projects), ADR 0017 (workers), AGENTS.md
  (engine is the source of truth, bounded resources, contain failures)

## Context

Bundles of the log files were replaced by projects (`.imtz`, the store folder). A project holds the raw text of every
event in Parquet segments and the catalog (field statistics with top values, diagnostics samples, histograms), a
full-text index, and the UI state (`ui.json`: notes, AI chats, saved queries, reports), note images and video frames.
Redacting values in place would have to reach every one of those, and any missed place leaks.

## Decision

1. **Build the copy from redacted text, not by editing the original.** `Redaction::export_redacted_project` reads the
   `raw` text of every copyable source, runs the redactor over it, writes it as files in a work folder under the store's
   `tmp`, imports them into a new store with the same profile and parser library, and exports that store with the normal
   project export. Every derived thing (columns, statistics, diagnostics samples, histograms, the search index, which is
   a cache that is rebuilt when the copy opens) is computed from redacted text only, so "no unredacted value anywhere"
   holds by construction rather than by finding every place. The work folder is deleted whatever happens.
2. **Only text sources are copied.** A source is copyable when its importer is `text` or `json` and it is not a binary
   format. CSV (its header is not in the events), whole-file XML importers and binary formats (event logs, dumps) are
   left out, and the preview and the result list them with the reason (`redact::copy::excluded_reason`). If nothing is
   copyable the request is refused saying so.
3. **Pictures are not carried.** Note images and video frames cannot be redacted, so the copy has none (notes that
   point at an image show as unresolved). The profile's own `[redact]` rules are removed from the copy's profile
   overrides, since a custom rule's pattern can spell the secret it hides.
4. **The copy does not name the user's folders.** Source paths are rewritten to `redacted/<name>` and the events view,
   which stores absolute segment paths, is written empty (it is recreated whenever a store opens); roots are cleared.
   Source display names go through the redactor as well.
5. **UI state is redacted by the shell** (`Redactor::apply_json` over `ui` and `reports`) and handed to the store as the
   text of `ui.json`; the shell never sees the store's work files.
6. **The user picks rules, not fields.** The choice is built-in detectors, the profile's custom rules and the mode
   (mask, hash, drop); the preview is the existing `redact::view::summary` over those rules, plus the list of sources
   left out. The whole event text is redacted whatever the profile's `fields`, because the copy is rebuilt from `raw`.
7. **Verified by a test that searches the archive.** The store test writes logs with invented secrets, produces the
   copy and looks for each secret, and for the data folder's path, in the archive bytes and in every decompressed entry.

## Consequences

- Event ids and the order of events with equal times can differ from the original; clock corrections are carried
  (offsets are applied to the copied files by their new paths).
- A copy of a very large project takes as long as importing it again; it runs in the store worker with the long
  timeout and removes its work folder afterwards.
- Supporting more formats later means giving their importers a way to rebuild from text, or redacting their columns;
  until then they are listed as left out.
