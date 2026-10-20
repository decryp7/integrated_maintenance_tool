# 18. Themes: token sets plus traits, three built-in themes

- Status: accepted (2026-10-20)
- Related: the design directions in docs/design/ (Calm IDE, Observability console, Precision instrument), the
  redesign plan (docs/redesign-2026-10.md, R5), AGENTS.md (CSS custom properties for theme tokens, WCAG 2.2 AA)

## Context

The user asked for a "super beautiful" timeline and a beautiful, professional UI, and chose to offer three visual
directions as user-selectable themes instead of one. Today the UI has light and dark token sets in `styles.scss`, but
several components hard-code colours and sizes, and the timeline's look (density, runs, markers) is fixed in code.
The directions differ not only in colour and type but in shape: flat panels vs soft cards, line vs pill vs bracket
runs, neutral vs source-coloured vs barcode lane density, comfortable vs compact rows.

## Decision

1. **A theme is data**: an id, a name, and for each scheme (light, dark) a complete **token set** plus a small set of
   **traits**. Built-in themes: `calm` (default), `console`, `precision`. Schemes follow the OS or the user's choice.
2. **Tokens** are CSS custom properties on `:root` (colour roles, type families and scale, spacing scale, radii,
   elevation, focus ring, motion durations, chart and source palettes, marker colours). Components read only tokens;
   no hard-coded colours, fonts or sizes remain (enforced by a lint rule on component styles).
3. **Traits** are a closed set of typed switches exposed as `data-*` attributes on the root and as a signal for code
   that draws (timeline, charts): `surface` (flat | cards), `runs` (line | pill | bracket), `laneFill` (neutral |
   bySource | barcode), `crashBand` (off | on), `brushPill` (off | on), `findings` (text | chips), `numerals`
   (proportional | tabular). Each built-in theme sets them; components style variants with attribute selectors.
4. **Density** (comfortable | compact) is a separate user setting, orthogonal to the theme (row heights, paddings,
   lane heights), so every theme works for occasional users and experts.
5. **The timeline renderer** takes its look from tokens and traits only (lane fill style, run style, marker glyphs,
   crash band, brush pill, histogram style); marker kinds differ by shape, not colour alone; categorical palettes are
   validated for colour-vision deficiency per theme and scheme.
6. **Fonts are bundled** (`@fontsource/*`), never fetched (the CSP forbids remote fonts): Inter + JetBrains Mono
   (calm), Geist + Geist Mono (console), IBM Plex Sans + Plex Mono (precision).
7. **Quality gates**: every theme × scheme passes axe (WCAG 2.2 AA) in Playwright and contrast checks on all
   text/surface token pairs; reduced motion disables decorative animation; screenshot tests cover the main window and
   the timeline per theme and scheme.
8. **Settings ▸ Appearance**: theme, scheme (system/light/dark), density, and "Colour lanes by source"; the choice is
   a UI preference (persisted per user, not per project). Themes are an extension point: a plugin may contribute a
   theme as a validated token+trait file later (not in the first step).

## Consequences

- One engine serves the default look, user choice, a future high-contrast theme and plugin themes.
- About 1.5–2 days more than a single polished theme, mostly converting remaining hard-coded component styles,
  timeline style variants and the per-theme test matrix.
- Rollout: engine + `calm` (default) + `console` first; `precision` follows when the engine provides per-lane error
  counts and source line numbers (after the R2 engine switch).

## Implementation notes (first step, 2026-10-20)

- The themes live in `app/src/app/theme/` as TypeScript data (`themes/calm.theme.ts`, `themes/console.theme.ts`):
  common tokens, colour tokens per scheme, size tokens per density, and traits. `ThemeService` sets the tokens on
  `<html>` and the attributes `data-theme` (id), `data-scheme` (light | dark, "system" resolved), `data-density`
  and one `data-*` per trait (`data-lane-fill`, `data-crash-band`, …); charts and the grid read `tokens()` and
  `traits()` from it. `styles.scss` no longer defines tokens.
- The gates run as unit tests over the data (`theme-checks.spec.ts`: complete token sets, 4.5:1 text and 3:1
  marks on their surfaces, source and series palettes under protan/deutan/tritan), as `npm run lint`
  (`scripts/lint-styles.mjs`: no raw colours, font families or font sizes in component styles), and in Playwright
  (`e2e/themes.e2e.ts`: axe per theme × scheme, screenshots of the main window and the timeline in Chromium;
  baselines are per platform).
- The six-colour source palettes of the design directions fail the all-pairs check once tritanopia is included,
  so both themes keep the four validated source colours (then patterns), as before.
- Overlapping runs on a lane are stacked on labelled sub-tracks (`timeline/sub-tracks.ts`, greedy interval
  packing) below the lane's markers.
- `precision` still waits for per-lane error counts and source line numbers from the engine.
