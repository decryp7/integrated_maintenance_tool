# Direction A: Calm IDE (tokens and timeline spec)

The data is the only thing with colour. Neutral graphite surfaces, one blue accent (focus, selection, primary action), crisp 1 px structure, zebra lanes. This is the closest to today's IMT.

Source of truth for the values: the `TOKENS.calm` object in `directions.html`. That page also shows live contrast ratios. All text tokens are ≥ 4.5:1 on `bg-0..bg-3`, `bg-hover`, `bg-active`, `bg-sel` and `tooltip-bg`. All marker, level-graphic and source colours are ≥ 3:1 on `bg-1`, `bg-hover`, `bg-sel` and `lane-bar`. This was checked by `scratchpad/contrast.cjs` in both themes.

## Type, spacing, shape, motion

| Token | Value |
|---|---|
| `--font-sans` | Inter (bundle `@fontsource/inter` 400/500/600/700; replaces IBM Plex Sans) |
| `--font-mono` | JetBrains Mono (already bundled) |
| `--fs-xs/sm/md/lg/xl` | 11 / 12 / 13 / 15 / 20 px (base stays 13 px) |
| `--s-1..s-6` | 4 / 8 / 12 / 16 / 24 / 32 px |
| `--r-1/2/3` | 4 / 6 / 8 px (buttons and inputs 6, popovers 8) |
| `--title-h` `--filter-h` `--tab-h` `--row-h` `--status-h` | 36 / 44 / 32 / 24 (compact 22) / 24 px |
| `--act-w` `--side-w` `--insp-w` | 48 / 232 / 300 px |
| `--dur-fast/med/slow` | 90 / 160 / 240 ms, `--ease: cubic-bezier(.2,0,0,1)` |
| Elevation | `--shadow-1` hairline lift; `--shadow-pop` (menus, tooltips); `--shadow-drawer` (inspector slide-in). These replace the single `--shadow`. |

## Colour: mapped onto styles.scss names

| IMT token | Light | Dark | Notes |
|---|---|---|---|
| `--bg-0` | `#f3f4f6` | `#111214` | Chrome: title, activity and status bars |
| `--bg-1` | `#ffffff` | `#17181b` | Panels |
| `--bg-2` | `#f8f9fa` | `#141518` | Sidebar |
| `--bg-3` | `#ffffff` | `#1c1d21` | Inputs, raised |
| `--bg-hover` / `--bg-active` / `--bg-sel` | `#f2f3f5` / `#eaecf0` / `#e2ebfc` | `#1f2125` / `#24262b` / `#1c2940` | |
| `--border` / `-2` / `-3` | `#e3e5e9` / `#d8dbe0` / `#c3c7ce` | `#26282d` / `#2f3238` / `#3d4048` | Neutral grey, not blue-tinted as today |
| `--grid-line` | `#eef0f2` | `#202226` | |
| `--text` / `--text-2` | `#1a1c20` / `#373b43` | `#e7e8eb` / `#c9cbd1` | |
| `--muted` / `--faint` | `#5b606a` / `#60656f` | `#a2a6af` / `#8d929c` | `--fainter` is removed (it equalled `--faint`) |
| `--accent` / `--accent-ink` | `#2f6fe0` / `#fff` | `#376cd6` / `#fff` | Primary buttons, brush edge |
| `--accent-soft` | `rgba(47,111,224,.09)` | `rgba(74,134,240,.12)` | Selection summary, brush fill |
| `--tooltip-bg` / `--code-bg` | `#fff` / `#f6f7f9` | `#1f2125` / `#111214` | |
| `--ok` / `--warn-text` / `--danger-text` | `#0e6c44` / `#8a5a00` / `#b4232b` | `#6fcf97` / `#e6b35a` / `#ff8a8a` | |
| `--lvl-debug/info/warn/error/fatal` | `#9aa0aa #7c8391 #b7790a #d93a3a #c02a74` | `#6b7079 #8d939e #d9a03a #ee5d5d #e2589f` | |
| `--hist-info` | `#ccd1d9` | `#3f444d` | |
| `--mk-fatal` (crash and fatal) | `#c02a74` | `#ec5fa6` | |
| `--mk-error` / `--mk-exception` / `--mk-warning` | `#d93a3a` / `#cf5f1c` / `#b7790a` | `#ee5d5d` / `#f0894a` / `#d9a03a` | |
| `--mk-start` / `--mk-stop` | `#1f8a5a` / `#737985` | `#43b581` / `#9aa0aa` | |
| `--lane-bar` | `#f7f8f9` | `#1a1b1f` | Zebra fill of odd lanes |
| `--density` | `#c5cad2` | `#454a54` | Neutral area: the markers carry the colour |
| `--src-1..4` | `#2f6fe0 #c8571f #00916a #9152c4` | `#4a86f0 #d9692f #1f9e7a #a774db` | Validated with the dataviz validator (CVD ΔE ≥ 9, normal ≥ 20, ≥ 3:1) |

**New tokens:** `--accent-text` (links, "+N more", `#2560c9` / `#7aa7ff`), `--fatal-text` (`#a3215f` / `#f58cc0`), `--focus-ring` (`#2f6fe0` / `#6f9dff`), `--titlebar-bg`, `--activity-bg`, `--tabs-bg`, `--statusbar-bg`, `--statusbar-text`, `--statusbar-strong`, `--row-line`, `--readout-bg`, `--readout-text`, `--ring` (marker separation ring = lane surface), `--run`, `--brush-fill`, `--brush-edge`, `--crosshair`, `--silent-hatch`, `--src-5` (`#a07a0c` / `#ad8726`), `--src-6` (`#c23f78` / `#d65a8c`), and `--shadow-1`, `--shadow-pop`, `--shadow-drawer`. `--s1..s4` (chart series) keep their own validated set. `--chip-*` stay and map to accent-soft and accent-text.

The status bar is no longer an accent-blue band. It is `bg-0` with muted text. In Calm, the accent is reserved for things the user is acting on.

## Timeline visual spec

- **Layout.** The label column is 204 px: a 3 px source stripe, then *App* (600) · *INSTANCE* (muted) on line 1 and `PID · N events · E err` (11 px, errors in `--danger-text`) on line 2. The plot has a 24 px right gutter so badges and the 12:00 edge never clip.
- **Lanes.** 32 px tall (close-up and "comfortable" zoom: 39 px). Odd lanes get `--lane-bar`, the hovered lane `--bg-hover`. No separators.
- **Density.** A neutral area (`--density`) from the lane baseline (bottom −8 px), up to `laneH−13`. Height is √(count / global max), so volumes compare across lanes. Buckets are 3 px.
- **Sparse rule.** If a lane has fewer events than plotWidth / 6, draw one 1.5 × 9 px `--muted` tick per event instead of an area. Scheduler shows this in the mockup.
- **Runs.** A 2 px line in the source colour at bottom −4.5 px, with round caps. A run that spans the whole view uses 55% opacity; partial runs use 90%. A crash ends a run at the crash glyph.
- **Went silent.** A 45° hatch (`--silent-hatch`, 6 px pitch) with a 3/3 dashed outline, plus a label chip on `bg-1`: "Went silent · 47 m".
- **Markers.** Centred in the lane, 11 px glyph with a 1.5 px `--ring` stroke (paint-order stroke). Shapes:
  - crash: 8-point burst, 1.35× size
  - error: circle
  - warning: triangle
  - exception: diamond
  - start: play triangle
  - stop: rounded square
- **Clustering.** Markers closer than glyph + 3 px merge. The most important kind wins (crash > error > exception > warning > start/stop) and the counts are summed. The count badge is 9.5 px mono with a 3 px surface halo, drawn after all glyphs, and flips left at the right edge.
- **Crash rules.** A 1 px dashed (3/3) `--mk-fatal` line at 45% across all lanes at each crash.
- **Histogram.** 46 px tall, 3 px bars with a 1 px gap. Stacked from the baseline as error → warn → info, with a 1 px surface gap between segments. Error and warn segments get a minimum of 1.5 / 1 px only when they are actually present (error ≥ 0.8, warn ≥ 2 per bucket). The label column shows "Events / per 68 s · max 201".
- **Axis.** 22 px row. Mono 10.5 px labels every 15 min, or every 30 min when 15 min is narrower than 56 px. Hour labels use `--text-2`, others `--muted`, with 4 px major ticks. "Wed 14 Oct · UTC" sits in the label column.
- **Brush.** `--brush-fill` over the histogram and lanes, 1 px `--brush-edge` edges, and 6 × 18 px pill handles centred on the histogram. The summary sits in the timeline toolbar, not over the data: "⌕ 10:36–10:52 · 1,412 events [Zoom] ×".
- **Hover.** A 1 px `--crosshair` line from the histogram to the last lane. The axis readout is an inverted mono chip (`--readout-bg`). The tooltip is a 300 px card (`--tooltip-bg`, `--shadow-pop`, r 8) with glyph + kind + time, instance, quoted message, consequence ("Restarted as 5532 after 46 s"), and a footer with the key hints.
- **Selection.** The selected event's marker gets a 2 px `--focus-ring` ring. A keyboard-focused grid row gets an inset 2 px `--focus-ring` outline.
- **Legend.** Only kinds present, glyph + label + mono count. Then a divider, run swatch, and "+N more" (`--accent-text`).
- **Findings line.** Warning glyph, then "**2** crashes · **287** errors · **1** went silent", then a **Review** link.
- **Animation.**
  - Brush and zoom transitions: `--dur-med`.
  - Tooltip fade-in: `--dur-fast` after a 120 ms hover intent.
  - Inspector slide-in: 240 ms translateX with `--ease`.
  - No looping animation.
  - Under `prefers-reduced-motion`, everything is instant.

## What changes where

- `app/src/styles.scss`: replace the token values, add the new tokens, switch the Inter font import, and make the status bar neutral. Dockview tab: underline at the bottom (`inset 0 -2px`) instead of the top.
- `timeline/marker-layout.ts`: glyph sizes, the cluster gap (glyph + 3 px) and badge flipping. The rank order already exists.
- `timeline/run-track.ts` and `run-layout.ts`: thin run line instead of bars; labels only in the tooltip.
- Timeline panel template/CSS: two-line lane labels with source stripe, zebra, sparse-tick rule, dashed crash rules, silent hatch, brush handles, findings line in the header, toolbar selection summary, legend "+N more".
- `grid/events-grid.ts`: 3 px source stripe, level glyph + coloured mono abbreviation, faint fatal row tint, focus outline.
- Shell: neutral status bar, activity-bar indicator (2 px accent bar), inspector drawer shadow.
- Icon set (`imt` namespace): 1.6 px stroke, 24 grid, round caps, as drawn in the mockup.

Effort: **S–M**. This is mostly tokens and CSS, plus timeline drawing tweaks. There are no new layout primitives.
