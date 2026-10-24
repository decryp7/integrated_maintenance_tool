# 61. Controls stay put while a view loads

- Status: Accepted (2026-10-24, for 1.1.1)
- Date: 2026-10-24
- Related: Paddock rules 5, 18 and 40; ADR 0056 (layout by declared constraints)

## Context

Twice now a click made just after a view opened did nothing: `run-spans.e2e.ts` (rare, never traced) and
`ai-entry-points.e2e.ts` "Propose…" in the profile editor (about 1 run in 8 on WebKit). The run-spans case was put down
to the layout planner re-creating a view on grow-back (fixed in 1.1 by moving the panel instead of re-adding it, commit
9b1c6cd9), and the same was suspected here.

The trace of the profile editor failure shows something else. The planner did not touch the panel: no node of the editor
was removed, moved or re-created after it opened (checked with a mutation observer in 10 WebKit runs, and in the
failing traces). What moved was the editor's own toolbar. The view renders at once with a placeholder name ("…"); about
50 ms later the profile arrives, the name, path and "Valid" appear in the same flex row as the buttons, the row wraps,
and "Propose…" jumps from the first line to the second (from x 834 to x 380 at 1440x900). Playwright had found the
button stable (it was, until the data came), pressed it, and the press and the release landed on different elements, so
the browser fired no click.

The pattern (rule 40): **a view's controls are laid out from data that arrives after the view is shown.** Any control
whose position depends on loaded text can move under a pointer that is already on it. Users see it as a click that did
nothing; tests see it as a rare flake that waits and retries do not fix.

## Decision

1. **A view's controls may depend on the view's size, never on data it loads.** Text that arrives later (names, paths,
   counts, states) sits in a slot with a fixed flex basis (`flex: 1 1 0` with a fixed `min-width`) and truncates
   (with the full value in a tooltip, rule 18) instead of pushing or wrapping the controls. Controls that only exist
   once data is there are rendered in place from the first frame (disabled while there is nothing to act on) rather
   than inserted before other controls.
2. **The check is a press across the load.** A view that loads its data after it opens gets an end-to-end test in
   `e2e/view-controls-stay.e2e.ts`: the mock holds the view's data (`?mockHold=<command>`, released with
   `globalThis.__debriefMockRelease()`), the test presses a control while the data is held, releases the data, asserts
   the control's box did not change, then releases the mouse and expects the control's action. This is deterministic:
   the press always happens before the data and the release always after.
3. **Planning never re-creates a view either** (ADR 0056). Grow-back already moves panels; the remaining remove-and-add
   paths (a displaced view with no slot to return to, and a tool window that the plan auto-hides or docks again) are
   listed as follow-up below. They were not the cause of either reported failure.

## Consequences

- The profile editor's header is the first instance: its name, path, Unsaved and Valid are one truncating slot; its
  buttons keep their place at every load state (`view-controls-stay.e2e.ts`).
- New mock commands become holdable by adding their name to `HOLDABLE` (`core/mock/mock-options.ts`) and wrapping the
  answer in `MockBackend.held`.
- Follow-up (after 1.1.1): add the press-across-the-load test to each view that loads after it opens (Markers manager,
  Parsers, History, Health, the analysis views) and fix the ones it catches; replace the planner's last remove-and-add
  paths with moves or a renderer hand-off so a view's component survives auto-hide and grow-back.
