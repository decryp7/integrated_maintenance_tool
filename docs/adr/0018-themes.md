# 18. Themes: token sets plus traits, three built-in themes

- Status: accepted (2026-10-20)
- Related: the three design directions that were prototyped as mockups (Calm IDE, Observability console, Precision
  instrument; the mockups were removed once the themes shipped), the 2026-10 redesign plan (R6), AGENTS.md (CSS custom properties for theme tokens, WCAG 2.2 AA)

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
- `precision` (2026-10-21, `themes/precision.theme.ts`) ships the Precision tokens and traits (flat, bracket runs,
  barcode lanes, tabular numerals) with IBM Plex Sans + Plex Mono; like the other themes it keeps the four
  validated source and series colours (see above). The table-style label column (PID, events, errors
  per lane) and the grid's LINE column still wait for per-lane error counts from the engine.
- ADR 0028 adds an *Interface font* setting: the operating system's font by default, the theme's face on request.

## Amendment (2026-10-23): calm is the suite-aligned default, system font by default, others opt-in

The suite design guide (one shared token vocabulary, system fonts, cobalt accent) and the design review of the app
(`design-review-debrief`) conflict with decision 6 only for the **default look**. The mechanism (tokens plus traits,
the density setting, the per-theme gates) stays; what changes is what a user gets without choosing anything:

1. **`calm` is the default** and follows the suite guide: neutral graphite surfaces, the system UI font (ADR 0028's
   *Interface font* default; the bundled Inter stays the theme's own face, one click away under *Theme font*), a
   cobalt accent (`#2f6fe0` in both schemes, `accent-text` lighter on dark so text on dark keeps 4.5:1), flat panels
   and the guide's type scale, with a `fs-hero` token (28 px) for the one hero number of a page.
2. **`console` and `precision` are opt-in alternative looks** (Settings > Appearance). They keep their own bundled
   fonts and must pass the same gates as `calm`: 4.5:1 text and 3:1 marks contrast, colour-vision-deficiency checks
   of the categorical palettes, axe in both schemes. A new token (such as `fs-hero`) is added to every theme.
3. **Typography rules that apply in every theme**: the interface font with tabular numerals for numbers and
   versions; monospace only for addresses, hashes, paths, code and raw log text; sentence-case section labels, no
   uppercase letter-spaced headings.
4. **Traits do the design work, not special cases**: `laneFill: neutral` (no hatched or dotted fills; annotation
   bands are flat tints, markers are told apart by shape) and `findings: chips` are what the Home and timeline
   changes of the design pass rely on.

Consequences: screenshots and baselines change once; no data migration (the stored theme id and font preference are
unchanged, `null` still means the default).
