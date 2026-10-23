# Debrief usability review (2026-10-23)

Independent, read-only review of `origin/main` (9e4369c2), browser build with the mock backend, driven by Playwright
(headless Chromium; one WebKit smoke), axe-core WCAG 2.2 A/AA. 1440x900 unless stated. Screenshots: `2026-10-23/`.

**Caveats.** The mock is not the reference for behaviour: data, saved-state and AI answers are scripted. Items marked
*(verify)* need the real engine. The search "graphic builder" matches nothing in the demo data, so only the empty state
could be judged. First start in the browser build opens the demo workspace directly; the welcome page was reached through
*File > Close workspace*. Timings are Playwright wall clock on a Mac mini, mock backend (UI cost only, no engine).

## Measured (perceived performance)

| Action | Result |
|---|---|
| Cold load to events grid | 1.5 s |
| Click ERR chip to filtered grid (40k events) | 97 ms (145 ms at 50M synthetic events) |
| Select a row, inspector visible | 38 ms |
| Pin inspector | 437 ms (animation) |
| Activity bar switches (Search, Analysis, Metrics) | 82-88 ms; Timeline 185 ms |
| Menu item to dialog (Share, Export) | 47-84 ms |
| Type "graphic builder" to zero-result state | 767 ms incl. typing |
| Grid scroll, 3 s continuous: 403 rows and 50M rows | 181 frames, worst frame 17 ms, 0 long tasks |
| Explain an event (AI): right-click, menu, wait | 3 s to "Explanation ready" toast (mock) |
| Tab presses from window start to the events grid | 71 |
| axe (light, dark, forced colours, reduced motion) | 0 violations in all four |

Verdict: speed is not the problem; layout and discoverability are. Nothing janky was measured.

## Top 10 changes

1. **Give the events grid and timeline the space** (U1, U2): default layout leaves 7 grid rows at 1440x900; Compare leaves 5; a pinned Inspector leaves 2.
2. **Share and other tall dialogs: sticky footer** (U3): "Create copy..." sits below the fold at 900 and 768 px.
3. **Do not open side views in a 50/50 split with the timeline** (U1): Registry/Home/Metrics halve the timeline plot to ~280 px.
4. **Tab order and F6** (U9): 71 Tab presses to reach the grid; make toolbars one tab stop each (roving), activity bar one stop.
5. **Selection should not hide what it selects** (U4): the auto-hide inspector covers the Message column of the selected row.
6. **Compare opens with a useless default** (U6): A = 3 h before the data (1 event), headline "40405 vs 1".
7. **AI first start** (U7): no prompt, no activity-bar entry until set up; setup cards are misaligned and repeat badges.
8. **Stacked toasts and two primaries** (U8): one action produced two toasts, the stale "running" one stays.
9. **Command palette ranking and duplicates** (U10): "AI" matches "an image", duplicate commands, setup ranks 9th.
10. **Colour-only level and marker cues** (U11): in forced colours the level dot and lane bars vanish; the Level column is empty for INF/DBG rows in the normal view too.

## Quick wins (each at most 1 h)

- Share dialog: hide zero-count rows in the preview table (9 of 11 are 0), make the footer sticky. (U3)
- Inspector: drop the duplicate "Error" chip (level chip and marker chip say the same). (U5)
- Inspector action row (Show +-2 min, Same CorrelationId, Copy JSON, More actions...): give them the ghost-button treatment or icons so they read as buttons. (U5)
- Toasts: dismiss the "running in Jobs" toast when the result toast arrives; only the result keeps a filled button. (U8)
- Filter bar: collapse the empty second row (No lens, View options) into the first row at >= 1280 px. (U12)
- Palette: exact/prefix matches before sub-sequence matches; hide commands that duplicate a menu entry's title. (U10)
- Level chips: `aria-pressed` and one roving tab stop. (U9)
- Compare: preset A = the selected range, B = the visible range; hide "0 ms" and "60/min" for windows with < 2 events. (U6)
- Welcome: centre the column, show Recent projects/folders and a one-line AI offer. (U7)
- Export: replace the native select with the themed listbox (rule 18). (U13)

## Findings

### U1 (P1) Default layout starves the events grid and the timeline
- Where: dock default layout (`dock/perspective-layouts.ts`), area rules of ADR 0035.
- What: at 1440x900 the Timeline and a side view (Registry here) split the top half 50/50, so the timeline plot is ~280 px wide
  (lane labels take ~230 px) and the events grid, the main artefact, shows 7 rows. At 800x600 the grid is 87 px (3 rows). Opening Metrics or Home lands beside Registry the same way.
- Should: documents (timeline, events) get the width; side views open as a narrow right tool group or auto-hide; the grid keeps at least ~12 rows; a "Focus on events" maximise is one key away.
- Evidence: `01-first-start.png`, `21-800.png` (see 21-1024.png), `19-analysis.png`. Effort: M-L.

### U2 (P1) Pinning the Inspector leaves two visible grid rows
- Where: dock host, inspector pin.
- What: after *Pin Inspector* the top row becomes Timeline | Registry | Inspector, the plot is ~100 px wide and the grid shows 2 rows (measured).
- Should: pin to the right edge as a full-height column (grid keeps its height), or refuse the split below a usable minimum (ADR 0035 `minSize` is meant for this).
- Evidence: `08-inspector-pinned.png`. Effort: M.

### U3 (P1) Share redacted copy: the primary button is below the fold
- Where: share dialog component.
- What: dialog is 804 px tall in a 900 px window; "Cancel / Create copy..." are at y=988 (outside the viewport) at 900 and 768 px height, inside a scrolling body. A first-time user sees no way to finish.
  The 13-rule list is a 9-row scroller in a scroller; the preview prints 11 rows of which 9 are 0.
- Should: footer pinned; preview shows only rules with hits and a "Nothing sensitive found" success state; Mask/Hash/Drop above the long list.
- Evidence: `10-share.png`, `14-share-900.png`, `14-share-768.png`. Effort: S (footer) + S.

### U4 (P2) The auto-hide Inspector covers the Message column of the selected row
- What: selecting a row slides a 370 px flyout over the right of the grid, hiding the message you just selected and half of the Registry.
- Should: flyout offset so the selected row stays readable, or show the message first in the flyout (it does) and dim instead of cover; Esc closes (verify it does).
- Evidence: `07-inspector-flyout.png`. Effort: M.

### U5 (P2) Inspector header repeats and under-signals
- What: "Error" appears twice (level and marker kind) beside "Exception"; the action row has no button affordance; Properties values wrap mid-word (CorrelationId in three lines); `Clock` and `Notes` and `Web` sections below the fold carry rarely used actions; "Search the web for this error..." and "Research this" are always shown.
- Should: one chip per fact; actions as icon buttons with tooltips; CorrelationId middle-ellipsised with Copy (rule 9); rare sections collapsed.
- Evidence: `07-inspector-flyout.png`, `08-inspector-pinned.png`. Effort: S-M.

### U6 (P2) Compare time ranges: poor defaults, crowded placement
- What: *Current range vs previous* with the full range selected gives A = 06:00-09:00 (1 event, "0 ms", "60/min") vs B = all; the headline "Window B has 40405 events vs 1 in window A" is true and useless. The panel opens as a third band; the timeline then shows one lane, the grid 5 rows. Two nearly identical menu entries and four buttons in the panel.
- Should: when the range is the whole data, offer "Before/after the selected moment" or the first/second half; show A and B on the timeline as bands; open the panel as a document tab or maximised.
- Evidence: `18-compare.png`. Effort: M.

### U7 (P2) First start and AI setup
- Where: welcome (`welcome.ts`), AI setup wizard.
- What: welcome is a left-aligned block in the top third of a 1400 px canvas; "How Debrief works" are four inert cards; no mention of the AI. With AI not set up there is no activity-bar entry; the only routes are the sparkle in the search field and the palette. The wizard's model cards are misaligned (title at x=620, 502, 466 px; radio far from its label), repeat "Fits comfortably / Fast / Tool calling" on every card, show the same size sentence three times, the list is cut by the footer with no scroll cue, and the dialog is an in-app overlay (project rule: real OS window, due in 1.0.3).
- Should: welcome centred with recents and a quiet "Set up the AI assistant (optional)" link; wizard as one table (Model, Size, Fit, Speed) with Best preselected and a single "Download 5.6 GB and set up" button with a phase-labelled progress (progress rule).
- Evidence: `02-welcome.png`, `15-ai-not-setup.png`. Effort: M.

### U8 (P2) Toasts: two for one action, two filled primaries
- What: *Explain this event (AI)* shows "Explaining event #289 - running in Jobs [Show in Jobs]" and, after, "Explanation ready [View]" at the same time; both have a filled blue button and cover the bottom-left of the grid.
- Should: one toast that updates in place; ghost secondary; result opens in the AI panel directly when the panel is visible.
- Evidence: `17-ai-explain.png`. Effort: S.

### U9 (P2) Keyboard: 71 Tab stops to the grid
- What: F6 cycles regions (good: status bar, activity bar, side bar, filter bar, groups), but Tab from the window start needs 71 presses: menu items, every activity-bar button, every level chip, each Explorer row's three action buttons (Actions / Adjust clock / Remove). (Earlier audit measured about 35.)
- Should: activity bar, level chips and explorer rows are roving-tabindex groups (arrows move, Tab leaves); row actions only on focus; skip link to the grid.
- Evidence: script output (see Measured); `01-first-start.png`. Effort: M.

### U10 (P2) Command palette
- What: typing "AI" shows "AI: Manage models" first, "Set up AI assistant" 9th, loose subsequence hits ("Open an image", "Time in UTC"), and duplicates ("AI: Open the assistant chat" and "View Open AI assistant"). Two label styles ("AI: title" vs category chip).
- Should: prefix/word matches first, one entry per command, one label style, recent commands at the top for an empty query.
- Evidence: `04-palette-ai.png`. Effort: S-M.

### U11 (P2) State shown by colour alone
- What: Level dots and lane bars disappear in forced-colours mode (`22-forced-colors.png`, chips lose their dots, lane colour bars vanish); in normal mode INF/DBG rows show only a grey dot, no text. axe cannot see this (0 violations).
- Should: level word or letter on every row (or a shape), `forced-color-adjust: none` with system-colour fallbacks for the dots/bars. Effort: S-M.

### U12 (P3) Filter bar: two rows at 1440, clipped at 1024
- What: the second row ("No lens", View options) is mostly empty and costs ~40 px. At 1024 the timeline toolbar clips (Fit is cut off) and a ... button appears beside No lens without a label.
- Evidence: `01-first-start.png`, `21-1024.png`. Effort: S.

### U13 (P3) Dialogs, controls
- Export: native `select` for Format and "?" icon-only help buttons; the dialog is in-app (known, 1.0.3). Fine otherwise: sensible defaults and "40,420 events will be exported" count.
- Save project in the mock saves "Untitled" without asking for a name and the title stays "Untitled" *(verify in the real app)*.
- Evidence: `11-export.png`, `12-save.png`.

### U14 (P3) Explorer and timeline polish
- Explorer rows wrap file names at 320 px ("OrderServi... . WORKER01") in the timeline lanes; folder group names (`incident-2026-10-14/web01`) repeat the same prefix on every group (rule 7: hoist it to the header).
- The timeline header lists a legend that changes with the filter, so its items jump when a level chip is toggled (compare `05-err-filter.png` and `01-first-start.png`). Effort: S.

### U15 (P3) The 2560 px window
- Layout scales by area, not content: Registry takes half of 2560 px for a sparse tree, while the Message column spans ~1300 px; text stays 12-13 px. Allow max widths for tree views or open the Inspector beside the grid by default on wide windows. Evidence: `21-2560.png`.

## What works well (keep)

- Instant filtering and scrolling at 40k and 50M rows; level chips with live "0/4.4k" counts; empty state with "Clear all filters".
- Command palette is fast and F6 region cycling exists; menus are well organised; Export shows the exact count.
- Zero axe violations in light, dark, forced colours and reduced motion; dark theme is coherent.
- Share dialog's live redaction preview (before/after lines) is excellent once the footer is reachable.

## Not covered

Real file-open dialogs, real engine timings, Tauri window dialogs, pin/auto-hide of all panes beyond the Inspector, reopening a saved project (mock saves in memory), live tail, WebKit beyond a smoke load (no errors).
