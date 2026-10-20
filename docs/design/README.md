# IMT visual directions

Open `directions.html` in a browser. It is one self-contained file; fonts come from Google Fonts, everything else is inline. Use the cards to switch direction and the toggle for Light or Dark. Every direction shows the same incident (14 Oct, 09:00–12:00 UTC) in the focused default layout:

- timeline above events
- inspector slid in on a selected crash
- the findings line in the timeline header
- a one-row filter bar with a **View** menu
- a legend of only the kinds present, with "+2 more"
- Fields collapsed
- an activity bar with only the relevant items

Each direction has three parts:

- a 1440 × 900 main window
- a 1280 px timeline close-up with hover tooltip, brush, dense and sparse lanes, and legend
- a token table with live WCAG contrast ratios

Other files:

- `tokens-calm.md`, `tokens-observability.md`, `tokens-precision.md`: tokens mapped onto `styles.scss` names, the new tokens, the timeline spec, and what changes in which component.
- `shot-*.png`: headless Chromium screenshots of every direction and theme, used to check the rendering.

Shared rules, applied in all three:

- Marker kinds always have distinct shapes:
  - crash: burst
  - error: circle
  - warning: triangle
  - exception: diamond
  - start: play
  - stop: square
  - went silent: hatch
- Glyphs have a surface ring, and clusters merge by importance, so colour is never the only cue.
- Source palettes are fixed-order, 6 slots, and pass the dataviz validator: CVD ΔE ≥ 9 and ≥ 3:1, in both themes.
- Every text/surface pair is AA. This was checked by script; the light-theme amber and orange were darkened where they failed.
- Spacing is on the 4/8 px scale, with one sans and one mono family per direction.
- Focus is shown with a 2 px `--focus-ring`.

## Comparison

| | A · Calm IDE | B · Observability console | C · Precision instrument |
|---|---|---|---|
| Feel | VS Code / JetBrains: quiet, neutral, one blue accent | Datadog / Honeycomb: soft cards, rich severity colour, violet | Tufte / lab instrument: paper and ink, mono numerals |
| Suits | Daily, all-day use by engineers; broad audience | Support and ops triage, demos, first-run "wow" | Experts comparing many instances on big screens |
| Timeline | Neutral density areas, thin source-colour run lines, dashed crash rules | Source-colour gradient areas, labelled run pills, incident band, crash halo, floating brush pill | Barcode density, bracket runs, y-scale, PID/events/err columns, outside-dim brush |
| Findings | One calm sentence + "Review" | Coloured chips + "Review →" button | Mono key/values |
| Density | 24 px rows, 32 px lanes | 27 px rows, 33 px lanes (fewest rows visible) | 20 px rows, 22 px lanes (about 2× the rows of B) |
| Fonts | Inter + JetBrains Mono (one new package) | Geist + Geist Mono (two new) | IBM Plex Sans + IBM Plex Mono (one new; same superfamily as today) |
| Effort | **S–M**: tokens and CSS, small timeline tweaks | **M–L**: card-style Dockview, per-source gradients, chips, animation | **M**: barcode and scale renderer, lane columns, LINE column |
| Risks | Can feel "plain" if executed loosely; relies on polish | Colour inflation over time; Dockview cards fight flush groups; busiest; brightest dark theme | Austere and small for occasional users; needs a "comfortable" density; least "delight" |

## Recommendation

**Build A (Calm IDE) as the base, and borrow three things from B and one from C.**

From B:

1. **Severity chips for findings in the Overview panel.** Keep the timeline header as A's one-line sentence.
2. **Source-colour density areas, as an opt-in "Colour lanes by source" view option.** This is the most beautiful thing in B, and it carries meaning.
3. **The floating brush pill in the close-up / maximised timeline.** Keep the toolbar summary in docked layouts.

From C:

1. **The per-lane right-aligned event and error columns, plus the y-scale on the histogram,** in a "Compact" density mode.

Why A:

- It is the most "professional" over eight hours a day.
- It keeps the timeline's colour budget for the data (markers and sources), which is what should draw the eye.
- It is the closest to today's tokens and Dockview, so it is the lowest risk and can ship in one batch.
- It leaves room to grow toward B or C through view options instead of hard-coded styling.

That matches AGENTS.md: profile and view configuration rather than special cases.

If the user prefers a single pure direction, B gives the most immediate "beautiful" impression, and C gives the most expert credibility.

## Notes

- The mockup data, the timeline renderer and the tokens all live in plain JS objects (`TOKENS`, `DIRS`, `LANES`, `MARKS`) at the top of the script. Tweaking a value re-renders every view.
- The fonts would be bundled with `@fontsource/*` in the app, because the CSP forbids remote fonts. The Google Fonts link is only for the mockup.
- `color-mix()` is used for tints in the mockups. It is supported by WebView2 and WebKitGTK 2.40+; confirm the macOS minimum before relying on it.
