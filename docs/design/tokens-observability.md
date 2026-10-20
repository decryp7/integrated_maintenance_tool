# Direction B: Observability console (tokens and timeline spec)

A modern console where problems announce themselves. Soft cards on a tinted canvas, a violet accent, and richer severity colour. Gradients appear only where they encode something: lane density (volume) and the incident band (nearness to a crash).

Values come from `TOKENS.obs` in `directions.html`. All text pairs are AA (4.5:1) and all graphics are ≥ 3:1 on `bg-1`, `bg-hover`, `bg-sel` and `lane-bar`, in both themes. The light-theme amber/orange tokens were darkened to pass on the violet `bg-sel`. The source palette was validated with the dataviz validator against both white and `bg-sel`.

## Type, spacing, shape, motion

| Token | Value |
|---|---|
| `--font-sans` / `--font-mono` | Geist / Geist Mono (`@fontsource/geist-sans`, `@fontsource/geist-mono`, both OFL) |
| `--fs-xs/sm/md/lg/xl` | 11 / 12 / 13 / 16 / 22 px |
| `--s-1..s-6` | 4 / 8 / 12 / 16 / 24 / 32 px; cards sit 8 px apart on `bg-0` |
| `--r-1/2/3` | 6 / 8 / 12 px (cards 12, controls 8, chips and pills fully round) |
| `--title-h` `--filter-h` `--tab-h` `--row-h` `--status-h` | 40 / 46 / 36 / 27 / 26 px |
| `--act-w` `--side-w` `--insp-w` | 52 / 220 / 300 px |
| `--dur-fast/med/slow` | 120 / 200 / 320 ms, `--ease: cubic-bezier(.2,.8,.2,1)` |
| Elevation | `--shadow-1` card (dark: inner top highlight + 8 px soft drop); `--shadow-pop` 16/40 for tooltips, menus and the brush pill |

## Colour: mapped onto styles.scss names

| IMT token | Light | Dark |
|---|---|---|
| `--bg-0` (canvas) | `#eceff6` | `#0b0e17` |
| `--bg-1` / `--bg-2` (cards) | `#ffffff` | `#121624` |
| `--bg-3` | `#ffffff` | `#182034` |
| `--bg-hover` / `--bg-active` / `--bg-sel` | `#f3f5fa` / `#eef0f7` / `#ebe8ff` | `#1a2133` / `#222b44` / `#272c52` |
| `--border` / `-2` / `-3` | `#e1e5ef` / `#d5dae6` / `#bfc6d6` | `#222a3f` / `#2b344d` / `#3b4663` |
| `--text` / `-2` / `--muted` / `--faint` | `#141a2b` `#303951` `#555f77` `#5f6983` | `#eef1f8` `#cfd5e4` `#a3acc2` `#8e98b0` |
| `--accent` / `--accent-ink` | `#5b47e8` / `#fff` | `#6b58f5` / `#fff` |
| `--accent-soft` | `rgba(91,71,232,.10)` | `rgba(124,108,255,.16)` |
| `--ok` / `--warn-text` / `--danger-text` | `#0a6a42` / `#875800` / `#b81f33` | `#4ade9a` / `#f5c060` / `#ff8d98` |
| `--lvl-info/warn/error/fatal` | `#8790a6 #a46c00 #e5394c #d02a8a` | `#8b96b3 #f2b33d #ff5c6c #ff4fb0` |
| `--mk-fatal/error/exception/warning/start/stop` | `#d02a8a #e5394c #c9540f #a46c00 #169a62 #7a849a` | `#ff4fb0 #ff5c6c #ff8a4c #f2b33d #34d399 #94a0ba` |
| `--hist-info` | `#c4cbe0` | `#3d4a6b` |
| `--lane-bar` (lane track) | `#f6f7fb` | `#161b2b` |
| `--src-1..6` | `#3d74f0 #c95a0c #0b8f78 #7b54e8 #977000 #c8388c` | `#4f88ff #dd6f1a #16a58a #9271ff #b88912 #e25aa5` |

**New tokens:** the same set as Calm (`--accent-text`, `--fatal-text`, `--focus-ring`, chrome backgrounds, `--ring`, `--brush-*`, `--crosshair`, `--silent-hatch`, `--readout-*`, `--src-5/6`, shadows), plus a component-level `--fc` / `--fc-t` pair for finding chips. Chip and pill tints are derived with `color-mix(in srgb, <level> 15%, transparent)` instead of extra tokens. WebView2 and WebKitGTK 2.40+ support `color-mix`; check the macOS minimum.

## Timeline visual spec

- **Card.** The timeline is a card (r 12). Tabs are pill-shaped segments inside the card header. The findings appear as **chips**:
  - crash chip: burst glyph, magenta tint
  - errors chip: dot glyph, red tint
  - silent chip: hatch glyph, amber tint
  - then a "Review →" soft-accent button

  Each chip pairs a glyph with its text, so colour is never the only cue.
- **Lanes.** 33 px pitch (close-up 40 px). Each lane is a rounded track (r 7) in `--lane-bar`, inset 2 px top and bottom. The hovered lane gets `--bg-hover` and a `--border-3` outline that wraps the label too.
- **Labels.** 212 px: a 4 px source dot, *App* (600) + *INSTANCE* (500, `--text-2`), and `PID · N · E errors` (errors bold in `--danger-text`).
- **Density.** An area in the lane's **source colour**: vertical gradient from 55% to 5% opacity (more volume, more ink) with a 1.25 px top stroke in the source colour. In the close-up it is capped at `laneH−22` so run labels stay readable.
- **Sparse rule.** Below plotWidth / 6 events, draw 2.2 px dots in the source colour.
- **Runs.** A pill (r 6) in the source colour: 10% fill and a 38% stroke, spanning the run. The close-up labels partial runs inside the pill: "Run 17 m", "Run 101 m · ended by crash", "Run · silent after 132 m".
- **Went silent.** An amber hatch with a 3/3 dashed outline and an amber-bordered chip: "Went silent · 47 m".
- **Markers.** 12 px glyphs (same shapes as Calm) with a 2 px `--ring`. Crashes add a 2 px `--mk-fatal` halo that animates r 7→16 px and fades out over 1.8 s, looping. It is static at 30% under `prefers-reduced-motion`.
- **Count badges.** A pill (`bg-3`, `border-3`, 9.5 px mono 600).
- **Incident band.** A horizontal gradient (0 → 16% → 0 of `--mk-fatal`, 34 px wide) behind every crash, across the histogram and lanes.
- **Histogram.** 58 px. 4 px bars, 2 px gap, r 1.5 tops. Info bar first, then warn and error overlaid from the baseline. The label column shows "Volume / 66 s buckets" and a 3-swatch legend (Info, Warn, Error).
- **Axis.** Same rules as Calm. Hour labels are 600 weight.
- **Brush.** `--brush-fill` (violet 8–14%) with `--brush-edge` edges and pill handles. A **floating pill** on the histogram shows `10:36 → 10:52 · 16 m · 1,412 events [Zoom]` (`--shadow-pop`, r 16), centred on the brush.
- **Hover.** Crosshair, and a violet axis readout (`--readout-bg` = accent). The tooltip card has a 4 px magenta gradient band on top for crash (the band hue follows the marker kind) and the same content as Calm.
- **Selection.** A 2 px `--focus-ring` ring around the selected marker. The grid's selected row uses `bg-sel` (violet tint) plus a focus outline.
- **Animation.**
  - Card and drawer enter: `--dur-slow` with a 4 px rise and fade.
  - Brush pill: `--dur-med`.
  - Number changes in chips: 200 ms crossfade.
  - The crash halo is the only looping motion.

## What changes where

- `styles.scss`: new values, Geist fonts, card variables. Dockview needs a theme layer that renders groups as cards: `.dv-groupview` gets radius, border and shadow, and the gaps come from `--s-2` margins. **This is the riskiest part**, because Dockview's sash and group geometry assumes flush groups.
- Timeline: source-colour gradient definitions per lane (6 gradients, one per source slot), incident band layer, run pills with labels (`run-text.ts` already exists), crash halo, floating brush pill (CDK Overlay or an absolutely positioned element).
- Findings: a `FindingChip` component, reused in the Overview panel.
- Grid: level pills, 6–10% row tint for error and fatal, 4 px source stripe, 27 px rows (compact 22).
- Filter bar: level toggles become pills tinted by level when on.
- Inspector: a card that slides in from the right with `--shadow-drawer`.

Effort: **M–L**, mainly because of the card-style docking, the gradients per source, and governing the extra colour (every new feature must pick from the severity and source sets).
