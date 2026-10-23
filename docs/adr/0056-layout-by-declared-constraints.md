# 56. Layout by declared constraints

- Status: Proposed
- Date: 2026-10-24
- Related: ADR 0024-style dock work (dockview host `app/src/app/dock/`), Paddock rules 18 and 40, AGENTS.md "Pure core, thin edges"
- Replaces: the placement heuristics in `app/src/app/dock/dock-placement.ts`

## Context

The dock (dockview) places views by position words (`center`, `right`, `bottom`) and a growing set of constants in
`dock-placement.ts` (465 lines). Each bug report was fixed by adding a rule that moved the problem elsewhere:

1. The events grid was squeezed by tool windows, so a `DOCUMENT_GRID_SHARE` of 0.35 was added.
2. Opening Home as a full-height document made Home cramped, so `FULL_HEIGHT_WIDTH_SHARE` 0.55 and
   `FULL_HEIGHT_MAX_WIDTH` 960 were added.
3. The AI assistant became 180 px wide, so `SIDE_TOOL_MIN_WIDTH` 320 was added.
4. Health finding titles were one letter wide inside Home, so `MAIN_MIN_WIDTH` 560, `FULL_HEIGHT_MIN_WIDTH` 420
   and a "tab Home into the events group when there is no room" branch (`spare < FULL_HEIGHT_MIN_WIDTH`) were added.

Facts today: a view declares `area` (`document | side | analysis | status`, `extensions/tokens.ts`, derived by
`dock/view-area.ts` when absent) and an optional `minSize`, but only **one** built-in view (`profile`, in
`builtin/builtin-views.ts`) sets `minSize`; all other minimums are constants inside the placement code, applied only at
the moment of an `addPanel`, and `setConstraints` is called only when a `minSize` exists or for side tools. There is no
preferred size or priority, so nothing can decide which view yields. Window resizes, restored layouts (`layout-store.ts`)
and user splitter drags are outside the rules entirely. `e2e/main-layout.e2e.ts` checks one scenario at 1440x900 and
1920x1080 and found the squeeze only after a human saw it. Rule 18 (no horizontal scroll at any width >= 320 px,
container queries) covers panes internally, not the dock.

The pattern, by rule 40: placement rules are *local reactions* to the last complaint. The missing rule is that
views *declare* what they need and one function decides, in one place, with all views in view.

## Decision

### 1. Every view declares its constraints

In the view contract (`DebriefView` in `extensions/tokens.ts`, plugin views included):

```ts
layout: {
  area: 'document' | 'side' | 'analysis' | 'status',
  min:  { width: number; height: number },     // never go below
  pref: { width: number; height: number },     // opening size when there is room
  priority: number,                            // higher stays visible, lower is tabbed or auto-hidden first
}
```

`minSize` and the position words become `layout`; `viewArea()` remains as the default for plugin views that declare
nothing (conservative defaults per area: side 320x240 pref 372, analysis min height 180 pref 280, document min
420x240). Built-in values (initial proposal): events 560 min width, priority 100; timeline min height 120 pref 300,
priority 90; Home document 420 min, priority 60; assistant side 320 min, pref 372, priority 70; Markers/Problems/
Notes side 320, priority 50; Explorer side 240 min, priority 80; Health findings view 360 min.

### 2. One pure layout function

```ts
planLayout(viewport: Size, open: readonly OpenView[], saved?: SavedHints): LayoutPlan
```

in `dock/layout-plan.ts` (no dockview, no DOM, no `window`). It returns, for every open view, one of: a slot in a
column/row arrangement with a size, *tabbed into* another view's group, or *auto-hidden* to an edge. Algorithm
(deterministic, stable ordering by id):

1. Place by area: document column(s) in the middle, side tools right (and left for Explorer), analysis below documents,
   status at the bottom edge.
2. Give each column its `pref`; if the sum exceeds the viewport, shrink columns toward `min`, taking from the lowest
   priority first, proportionally within equal priority.
3. If shrinking to `min` is still not enough, **do not squeeze**: take the lowest-priority view, tab it into the
   nearest compatible group (documents tab with documents; a side tool tabs with another side tool) or, if it is a
   tool, auto-hide it to its edge; repeat until everything left fits at or above `min`.
4. Never below a minimum, ever: the plan is invalid if any placed view is under `min`; this is asserted in the function
   and in its tests.
5. Fits-with-room: the leftover goes to the highest-priority document first (grid and timeline), capped by `pref` max.

`DockPlacement` shrinks to a thin edge: it translates `LayoutPlan` into dockview calls (`addPanel`, `setSize`,
`setConstraints`, auto-hide items) and reports user actions back as hints. `setConstraints` is applied for *every*
group from the views it holds (max of mins), so dragging a splitter cannot cross a minimum either.

Re-planning triggers: opening or closing a view, a window resize (debounced, only when a minimum would be violated or a
tabbed view could now fit), and a perspective load. A user's drag is respected between replans (the plan treats the
user's sizes as the new `pref`, still clamped to `min`); an explicit "Reset layout" returns to the pure result.

### 3. Saved user layouts that violate minimums

`layout-store.ts` layouts are loaded, then run through `planLayout` with the saved arrangement as a hint:

- If every placed view meets its minimum at the current viewport, the saved layout is kept exactly.
- If not (a smaller monitor, a new minimum in an update), the saved arrangement is kept as far as possible and the
  function applies steps 2-3: shrink toward minimums by priority, then tab/auto-hide lowest priority. The saved
  layout on disk is not rewritten; the same layout on a larger window returns to the saved arrangement.
- One notification (ADR 0055, cause `layout:fit`) says "Some panes were tabbed to fit this window" with an
  "Undo" that restores the saved arrangement unclamped for this session. Persisted layout files stay versioned and
  fall back to defaults when corrupt (AGENTS.md robustness).

### 4. Tests

- Vitest for `planLayout`: table-driven scenarios (viewports 800x600 to 2560x1440; combinations of Explorer, events,
  timeline, Home, assistant, Markers, Health), property tests that no placed view is under `min`, plans are
  deterministic, adding a lower-priority view never shrinks a higher-priority one below `pref` unless needed, and
  nothing is both placed and tabbed.
- Playwright (`e2e/main-layout.e2e.ts` extended into a scenario table): the reported scenario and the standard sets
  (Explorer + events + assistant; + Home; + Markers; + Health), each at **1440x900 and 1920x1080**, asserting every
  group width/height >= the view's declared `min` (read from the view contract, not duplicated), no text clipped to a
  letter (existing check), and no horizontal scroll. A resize test shrinks 1920 to 1440 and checks tabbing, not
  squeezing. A saved-layout test loads a layout that violates minimums.
- A lint that every built-in view declares `layout` and every `min <= pref`.

## Shared with the other Paddock apps (rule 39)

The family-level decision is Paddock ADR 0011 (layout by declared constraints). Only Debrief docks panes today, but
the other apps lay out fixed shells that need the same guarantee (never below a minimum, tab or hide before squeezing).

| Part | Where it lives |
|---|---|
| `planLayout` and its types (`ViewLayout`, `LayoutPlan`, saved-hint fitting), pure and DOM-free, with its table-driven and property tests, and a Playwright helper that asserts every pane against its declared minimum at the standard sizes | `@decryp7/paddock-ui` |
| The dockview adapter (`DockPlacement` translating a plan to `addPanel`/`setSize`/`setConstraints`/auto-hide), the built-in view declarations, the priorities, saved perspectives (`layout-store.ts`) | Debrief |

## Consequences

- The dock's behaviour is explained by data (the view declarations) plus one function, so each layout complaint is a
  declaration change with a scenario, not a new branch.
- Windows that are too small for everything now hide or tab lower-priority views instead of shrinking them; users may
  notice panes that tabbed themselves. The notification and "Undo" mitigate this.
- Plugin authors get three more optional fields with safe defaults; the plugin API version bumps.
- A rewrite of `dock-placement.ts` (465 lines) into a plan function plus an adapter; high-risk code, covered by the
  scenario tests first.

## Migration plan

1. *Before 1.1.* Write `layout-plan.ts` with tests that encode the current constants as the declared values (events 560,
   side 320, Home 420) and the four reported scenarios, run it **beside** the current code in a test that compares
   outcomes; extend `main-layout.e2e.ts` to the scenario table at both sizes.
2. *Before 1.1.* Add `layout` to the built-in views (all of them; the lint enforces it) and apply `setConstraints` for
   every group from the contract; resolves the "only one view has `minSize`" gap.
3. *Before 1.1 if the scenario table is green, otherwise right after.* Switch `DockPlacement.openView` and the Home
   tab-into-events branch to `planLayout`; delete `DOCUMENT_GRID_SHARE`, `FULL_HEIGHT_*`, `MAIN_MIN_WIDTH`,
   `TOOL_SIZE` heuristics.
4. *After 1.1.* Resize-driven replanning, saved-layout fitting with the notification and Undo, "Reset layout".
5. *After 1.1.* Plugin API fields and docs; move the plan function to `paddock-ui` when a second app has a dock.

## Alternatives considered

- **More heuristics.** The history above is the case against.
- **CSS grid/flex with `min-width` instead of dockview.** Loses drag-docking, floating and tabs (ADR for the dock);
  the constraint model fits dockview's own `setConstraints`.
- **Container queries inside each view only (rule 18).** Needed and kept, but cannot decide which view yields; they
  make a view usable at its min, the plan guarantees it gets at least that.
- **Let the user fix it.** Saved layouts do survive; the defaults and the replanning must be right first.
