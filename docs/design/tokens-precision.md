# Direction C: Precision instrument (tokens and timeline spec)

Maximum data, minimum ink. Paper and ink surfaces, one cobalt accent, 2 px radii, mono numerals for every measured value, small-caps section labels, and tight density (20 px rows, 22 px lanes).

Values come from `TOKENS.prec` in `directions.html`. The contrast check passes for every pair; this direction has the highest margins (body text ≥ 15:1, `--faint` ≥ 4.9:1 on the selection colour).

## Type, spacing, shape, motion

| Token | Value |
|---|---|
| `--font-sans` / `--font-mono` | IBM Plex Sans (already bundled) / **IBM Plex Mono** (`@fontsource/ibm-plex-mono`; replaces JetBrains Mono so both come from one superfamily) |
| `--fs-xs/sm/md/lg/xl` | 11 / 12 / 12 / 14 / 18 px (base 12). Section labels are 10–11 px, uppercase, `letter-spacing .08em`, weight 600 |
| `--s-1..s-6` | 4 / 8 / 12 / 16 / 24 / 32 px |
| `--r-1/2/3` | 2 / 2 / 3 px |
| `--title-h` `--filter-h` `--tab-h` `--row-h` `--status-h` | 32 / 36 / 28 / 20 / 22 px; controls are 22–24 px tall |
| `--act-w` `--side-w` `--insp-w` | 40 / 224 / 292 px |
| `--dur-fast/med/slow` | 60 / 120 / 180 ms, `--ease: cubic-bezier(.3,0,0,1)` |
| Elevation | None on panels (hairlines only); `--shadow-pop` only for floating tooltips and menus |

## Colour: mapped onto styles.scss names

| IMT token | Light (paper) | Dark (ink) |
|---|---|---|
| `--bg-0` / `--bg-1` / `--bg-2` / `--bg-3` | `#f2f2ee` / `#fcfcfa` / `#f7f7f3` / `#ffffff` | `#060607` / `#0c0c0d` / `#0a0a0b` / `#131315` |
| `--bg-hover` / `--bg-active` / `--bg-sel` | `#efefea` / `#e8e8e2` / `#e0e8f8` | `#18181b` / `#1d1d21` / `#17223d` |
| `--border` / `-2` / `-3` / `--grid-line` | `#deded7` `#cdcdc5` `#a6a69e` `#ececE6` | `#242427` `#303034` `#4a4a50` `#1a1a1d` |
| `--text` / `-2` / `--muted` / `--faint` | `#0e0e0d` `#2b2b28` `#56564f` `#5f5f58` | `#f3f3f0` `#d6d6d1` `#a8a8a2` `#909089` |
| `--accent` / `--accent-ink` | `#1f4fd1` / `#fff` | `#5b8cff` / `#060607` |
| `--ok` / `--warn-text` / `--danger-text` | `#1b6b3a` / `#875600` / `#b00e28` | `#62d394` / `#e8b04f` / `#ff7d88` |
| `--lvl-info/warn/error/fatal` | `#7d7d76 #a86b00 #c8102e #b0006d` | `#8f8f89 #e0a33a #ff4d5e #ff4fa8` |
| `--mk-fatal/error/exception/warning/start/stop` | `#b0006d #c8102e #c4461a #a86b00 #1e7a45 #56564f` | `#ff4fa8 #ff4d5e #ff8a52 #e0a33a #4cc787 #b4b4ae` |
| `--density` (ink) | `#2b2b28` @ 82% | `#a8a8a2` @ 82% |
| `--hist-info` | `#8e8e86` | `#6b6b66` |
| `--lane-bar` | `transparent` (lanes are separated by a `--grid-line` baseline only) | same |
| `--src-1..6` | `#1f5fd1 #c4461a #007a55 #8a3fb8 #8a6a00 #b0306a` | `#5a8ff0 #dd7236 #1fa37f #ad78e0 #b38c2c #de6597` |

**New tokens:** the same set as Calm. Here `--brush-fill` is the *dimming* colour (paper or ink at 50–58%) laid over everything outside the selection, and `--run` is an ink grey rather than the source colour.

## Timeline visual spec

- **Label column.** 300 px, laid out as a table. The axis row carries column headers in small caps: LANE · PID · EVENTS · ERR. Each lane row has a 6 × 6 px square source swatch, *App* (500) + *INSTANCE* (`--muted`), then right-aligned mono columns: PID (`--faint`), events (`--text-2`) and errors (`--danger-text`, or "–").
- **Lanes.** 22 px (close-up 27 px). No fill. A 1 px `--grid-line` baseline under each lane. The hovered lane gets `--bg-hover`.
- **Density: barcode.** One 1 px `--density` tick per 2 px bucket, rising from the baseline. Height is √(count / global max) × (laneH − 8). This shows rhythm (periodic jobs, bursts) better than an area.
- **Sparse rule.** Below plotWidth / 6 events, draw 1 × 9 px ink ticks per event.
- **Runs.** A bracket under the barcode, `├────┤`: a 1 px `--run` line with 3 px end ticks. The duration appears in the tooltip, not on the chart.
- **Went silent.** A hatch with no outline, and a square chip in mono caps: "SILENT 47m".
- **Markers.** 9 px glyphs (crash 12 px) with a 1.25 px ring. Shapes are the same as in the other directions.
- **Count badges.** Superscript mono 9 px with a surface halo.
- **Crash rules.** A solid 1 px `--mk-fatal` line at 75% from the top of the histogram to the last lane, labelled in a bottom row in mono 600: "CRASH 10:41:24".
- **Histogram.** 40 px. 2 px bars with a 1 px gap, no rounding, ink grey (`--hist-info`) with warn and error stacked at the base. A **real y-scale** with ticks and mono labels 0 / mid / max in the label column. Title: "EVENTS / 58 s", with the date and timezone under it.
- **Axis.** Hairline ticks: 3 px every 5 min and 5 px every 15 min (major ticks in `--text-2`). Labels are mono 10.5 px, every 15 min or every 30 min when narrow; hours are bold. Vertical gridlines: solid every hour, dotted (1/3) every 15 min.
- **Brush.** **The outside is dimmed** (`--brush-fill`) over the histogram and lanes; the axis stays crisp. 1 px `--brush-edge` edges. A bracket above the histogram carries a mono readout: `10:36:00–10:52:00 · 16m00s · n=1,412`. The toolbar shows `SEL 10:36–10:52  N 1,412 [Zoom]`.
- **Hover.** A 1 px `--crosshair` line and an inverted square mono readout on the axis. The tooltip is a compact key/value **table** (KIND, TIME, LANE, MSG, AFTER, KEYS), r 2, 286 px wide.
- **Selection and focus.** A 2 px `--focus-ring` around the selected marker and the focused grid row.
- **Legend.** One 22 px row of glyph + label + mono count, then "+N more".
- **Findings line.** Mono, with a key/value rhythm: `CRASH 2  ERR 287  SILENT 1  Review →`. The values use the matching text colours (fatal, danger, warn).
- **Animation.** Functional only: 60–120 ms, no easing flourish, no loops.

## What changes where

- `styles.scss`: new values; switch the mono font to IBM Plex Mono; small-caps utility (`.section-title` already exists); 2 px radii; drop panel shadows.
- Timeline: barcode density renderer, bracket runs, table-style label column (PID, events and error columns; per-lane error counts must come from the engine. Check whether the timeline reply already carries them, otherwise add them to it), y-scale ticks, labelled crash rules, outside-dim brush, table tooltip.
- Grid: a LINE column (source line number; confirm the engine exposes it per row), 20 px rows as the default density, mono level abbreviations without glyphs, uppercase column headers.
- Tabs: uppercase 11 px with a 1 px ink underline.
- Inspector: uppercase mono keys, square level badge.

Effort: **M**. The token work is small. The barcode, scale and column work in the timeline is moderate. The main risk is that it is too austere and too small for new or occasional users, so it needs a "comfortable" density toggle (24 px rows, 28 px lanes).
