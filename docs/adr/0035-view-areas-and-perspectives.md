# 35. View areas and perspectives: where a view opens, and named window arrangements

- Status: accepted (2026-10-22), amended on the same day: **Overview became Home** (a document in the centre area, not a side
  tool; it opens after an import only when its own setting, off by default, says so) and **Plugin health was merged into the
  Plugins view** (tabs *Installed* and *Health*). Where this text says Overview or Plugin health, read Home or Plugins.
- Date: 2026-10-22
- Related: docs/ux-audit/README.md (A2, A3, A4, D2, H1, J2), ADR 0016 (typed commands and the view registry),
  ADR 0018 (themes), AGENTS.md (Angular is presentation; the engine decides what is a query), ADR 0030 (Store pane)

## Context

Every view opened as another tab in the group that held the timeline or the events grid. After a normal session
the top group held 22 tabs and the grid itself was hidden behind Anomalies. The window also had two layout concepts
that did not combine: a Focused/Full *layout mode* (a per-viewer preference that changed what a *new* layout
contained) and *window layouts* (named arrangements, Ctrl+Alt+1..4). The query bar was a dock panel that took a tab
and a group, next to a separate search box in the filter bar.

## Decision

1. **Every view has a home area**, declared in the view registry (`ImtView.area`, derived by `viewArea` for plugin
   views that declare none):
   - `document` (centre): Timeline, Events, query results, editors, Parsers, profiling documents, Dump, reports;
   - `analysis` (one group below the events grid): Anomalies, Patterns, Exceptions, Moment, Compare, Field
     statistics, Traces, Runs, Markers, Alerts, Tail status;
   - `side` (right tool group): Inspector, Overview, AI assistant, Browser, Notes, Saved queries;
   - `status` (bottom side bar, auto-hide): Jobs, Notifications, Plugins, Plugin health, Error snapshots.
   `DockPlacement.addView` places by area and never covers the timeline or the grid: a document opens beside them,
   an analysis view below the grid. The analysis group keeps at most 8 tabs; opening one more closes the least
   recently used tab that is neither active nor pinned. Opening an open view activates it. A view may declare a
   `minSize`; the dock refuses a split below it (A4). Panels narrower than 420 px carry `data-narrow` so their
   content folds (secondary columns drop out, action rows wrap, wide tables scroll inside the panel).
2. **Perspectives replace the layout mode.** A perspective is a named window arrangement. Four are built in
   (`dock/perspective-layouts.ts`, built from scratch into an empty dock): *Investigate* (the timeline over the
   events, the default), *Profiling* (the captures list as the landing view: Open capture, Profile a process),
   *Live* (tail status and alerts below the events) and *Diagnostics* (problems, jobs, error snapshots, store, plugin
   health below the events). A built-in one is offered only when the views it needs exist. Window layouts the user
   saves are perspectives too. Each perspective remembers how the user left it: switching away stores the
   arrangement, switching back restores it; *Reset perspective* rebuilds it. The state (`imt.perspectives`,
   versioned, malformed storage falls back to Investigate) is per viewer like the dock layout.
   Switching: the activity bar's *Perspectives* fly-out, *Window ▸ Perspectives* and `Mod+1…9`. Dock group focus
   moves to `Ctrl+Alt+1…9`, read from the physical key (`KeyboardEvent.code`; on macOS Option changes `key`).
   Opening a performance capture switches to *Profiling* once per window; the notice offers Undo.
   The *Layout* submenu, `LayoutModeStore` and the Full/Focused commands are removed (no migration before v1). The
   Overview opens after an import only when its own setting (off by default) says so.
3. **One search field** (D2). The filter bar's field takes plain words (message search, applied as you type) or a
   query. The *engine* decides which: `imt_core::ql::classify_input` (pure, bounded, never fails; a comparison counts
   only when its left side names a field the data has) behind the command `classify_search`; the UI keeps no
   second parser. The text is the query session's, so history, completions, did-you-mean and the results view are
   unchanged. The old Query panel is deleted.
4. **Activity bar and status bar** (A3). The accent bar marks the view that is *in front*; a small dot marks a view
   that is open behind another. Jobs and Notifications live in the status bar only (progress pill, bell with badge).
5. **Narrow windows** (H1). The filter bar folds the range, time zone and *Clear all* into a ⋯ menu when it is
   narrower than 1000 px; the Explorer closes by itself below 1100 px (and returns when the window is wide again,
   unless the user chose in between); the events grid keeps a readable Message column (the other columns have a
   `minmax`); the status bar drops low-priority items below 1000 px.
6. **Navigation and undo.** Back/forward (`Alt+←/→`, the mouse's back/forward buttons) walk through the places the
   user has been: events they selected, time ranges they zoomed to, views they brought forward (`NavHistory`, 50
   places, zoom steps merged). Zooming and panning are also on the undo stack (`RangeUndo`, next to the filter
   changes of `QueryUndo`), merged per gesture.
7. **Drag and drop between views.** Rows of the events grid are drag sources (`application/x-imt-event`, parsed and
   bounded on drop); dropping one on the Notes panel writes a note on that event. `Mod+D` stays the keyboard route.

## Consequences

- A user who sorted views into groups by hand keeps that arrangement (it is saved); only *new* openings follow areas.
- Plugins that want a specific home declare `area`; those that do not are placed by kind and `defaultPosition`.
- The classification runs through IPC for every pause in typing, so it is a cheap pure command with a 1 MiB argument
  limit and no store access.
- Perspectives are a per-viewer preference, not project state: opening a project restores its dock layout, not the
  active perspective's name.
