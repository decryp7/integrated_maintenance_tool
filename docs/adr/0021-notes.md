# 21. Notes: observations anchored anywhere, including regions of images

- Status: accepted (2026-10-21)
- Related: ADR 0007 (reports), ADR 0014 (projects are the store folder), ADR 0016 (command table), ADR 0025 (images
  decoded in Rust), ADR 0029 (knowledge files; the same engine-owned project record pattern), AGENTS.md

## Context

Users want to write down what they see where they see it: on a log line, a moment or range of the timeline, a
source, a marker or span, a chart point, the investigation as a whole, and on screenshots that come with the logs
("mark out an area and make an observation"). IMT had bookmarks (an event or a range with a one-line note) kept and
validated by the UI. They are a subset of this and the wrong owner.

## Decision

1. **A note is Markdown text on an anchor.** Anchors: `event` (the store's event id, with its time, source and a
   short excerpt captured by the engine when the note is made), `instant`, `range`, `source`, `marker` (a marker or
   span by name and time), `chartPoint` (chart, series, time, value), `project`, and `image` (an image's content
   key, name and pixel size, with one to 16 rectangles in the image's own pixels). A note has tags (at most 8), a
   severity (`info`, `warning`, `problem`) for its colour, an author (the OS user) and created/updated times set by
   the engine. **Notes replace bookmarks** (pre-v1: no migration).
2. **The engine owns them.** The pure crate `imt-notes` holds the types, their checks and limits (16 KiB of text,
   5 000 notes per project, 16 regions inside the image), the versioned project record (v1, read leniently:
   damaged notes are skipped and counted, a damaged or newer record restores none and says why), the AI context
   renderer and the geometry checks. The shell keeps each window's notes, answers the `notes_*` commands of the
   command table with typed `CmdError` codes, and fills event anchors from the store. The UI keeps the last record
   for saving (UI state key `notes`, like the chats and knowledge), so notes are autosaved in the session and
   travel in the `.imtz`.
3. **Event anchors are store ids.** Ids are stable for a store and its project; on reopening, the list says which
   anchors no longer resolve (`resolved: false`) instead of dropping the note. Clock-skew correction is not
   available with the store yet; when it returns it must remap event notes in the engine.
4. **Images.** Image files of the workspace (png, jpg, gif, webp, bmp) open in an image view through the engine
   (path grants or the Explorer's list, size and pixel limits, Rust decoding; the webview gets a `data:` URL of a
   clean copy). A note on an image region stores the content key (SHA-256 of the file); the engine keeps the clean
   copy in the project folder (`notes/images/<key>.png`), so an annotated image opens from a shared project without
   the original. The view zooms and pans, draws rectangles, shows regions as overlays and highlights a note's region.
5. **UI.** "Add note" from the record context menu, the inspector, the timeline (point or dragged range, and the
   context menu of a marker, run or annotation), a chart's context menu (on a point, or a moment of the plot), the
   Explorer's sources and images, and the command palette; a glyph on noted rows, markers on the timeline for time
   notes, a count badge on sources; a dockable Notes panel lists, filters (kind, tag, text), jumps, edits and deletes
   (confirmed). Markdown is rendered by `MarkdownView` (text nodes only). Note edits are not on the undo stack.
6. **AI.** Notes reach every AI feature as one more project knowledge file ("Notes", ADR 0029): notes on the selected
   events, the time range and the open image first, then project notes, then the rest, at most 16 KiB with a visible
   cut. Note text never goes to IMT's log.
7. **Reports.** "Add notes to report" adds a finding per note (severity, anchor, text), the noted events as an
   events block and time notes as an annotations block, so the PDF and HTML exports list them with no new block type.

## Consequences

- One place for observations; the bookmarks UI, its UI-side parsing and its undo entries are gone.
- An image note costs a clean copy (at most 4096 px a side) in the project.
- Notes made in a store that is later re-imported (not reopened) lose their event anchors; they stay listed as
  unresolved with their captured excerpt.
