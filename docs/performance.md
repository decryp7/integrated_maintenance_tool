# Performance record (bundle sizes and startup)

Rule 35: every release records the initial chunk size, and the budgets in `app/angular.json` are measured size plus 10%.
Measured with `cd app && npx ng build` (production) and a headless Chromium load of `dist/app/browser` with the mock
backend (browser build; the desktop app loads no mocks).

## 2026-10-23, baseline before the Angular phase-1 migration (main at 67a51214)

| Measure | Value |
|---|---|
| Initial chunks (what `ng build` budgets) | 171.76 kB raw (styles 164.42 kB, `main` 6.80 kB, polyfills 0.5 kB); 15.77 kB transfer |
| Why so small | `main.ts` loads translations, then lazily imports `bootstrap`; the real startup cost is the lazy `bootstrap` chunk graph |
| `bootstrap` chunk (static closure, 122 chunks) | 1.52 MB raw |
| Largest single chunks | notices-generated 766 kB, a shared vendor chunk 678 kB, messages-pseudo-json 563 kB (only for the pseudo locale), bootstrap 466 kB |
| Browser startup (mock backend, to `networkidle`, 5 runs) | about 1.1 s, 170 requests, 3.69 MB raw (2.5 MB of it is the closure of the mock-backends chunk, browser only) |
| Total chunks | 3 initial + 285 lazy |

Budgets set from this: `initial` warn 190 kB / error 210 kB (measured + 10%, was 1.5 MB / 3 MB); `anyScript` warn 845 kB
(largest chunk + 10%) / error 1 MB.

## 2026-10-23, after Angular phase 1 (lazy Settings screen and diagnostics collector)

| Measure | Before | After |
|---|---|---|
| Initial chunks (budgeted) | 171.76 kB | 171.76 kB (unchanged: `main` only loads translations, then `bootstrap`) |
| Startup JS in the `bootstrap` static closure, raw | 2.08 MB (123 files, incl. 563 kB pseudo-locale messages loaded only for pseudo) | 1.87 MB (115 files) |
| Same, gzip | 602 kB | 544 kB |
| Browser startup bytes (mock backend, networkidle) | 3.69 MB, 170 requests, about 1.1 s | 3.52 MB, 169 requests, about 1.1 s |

The Settings screen (with its Storage, Parsers, Keyboard and other tabs) and the diagnostics collector dialog were
imported statically by the services that open them; their `open()` now loads them on first use. Views registered through
`component: () => import(...)` were already lazy. Largest remaining startup weight (stats-json, bytes in output):
dockview-core 479 kB, the mock backends 248 kB (browser only), Angular core 158 kB, echarts about 200 kB (pulled in by
chart cards and the Home view), the share/report, tools and AI folder-setup modules (statically imported by commands).
Next candidates: lazy dialog services for share/report, the format wizard and folder setup, and an `@defer` around echarts
consumers. How to measure: `npx ng build --stats-json`, then group `dist/app/browser-stats.json` outputs by input.

## 2026-10-24, what the main window loads before first paint (branch perf/startup-bundle)

`ng build`'s `initial` budget only sees `main`, polyfills and styles, because `main.ts` imports `bootstrap` dynamically and
`bootstrap` imports `app` and `app.config` dynamically. The real first-paint set is the static-import closure of the
chunks for `src/main.ts`, `src/bootstrap.ts`, `src/app/app.ts` and `src/app/app.config.ts` (for a pop-out `popout-shell.ts`
replaces `app.ts`; for a dialog window the dialog shell, its bootstrap and its content). `app/scripts/window-weight.mjs`
measures it from `dist/app/browser-stats.json` (`npx ng build --stats-json`, then `node scripts/window-weight.mjs main`;
`npm run check:window-weight` does both) and prints the heaviest chunks and packages. CI runs it after the frontend build.

| Measure | Before | After | Budget (measured + 10%) |
|---|---|---|---|
| Main window first-paint JS (static closure) | 1710.5 kB, 186 chunks | 1231.3 kB, 147 chunks | CI fails over 1355 kB |
| Pop-out window (reported, not enforced) | 1460.3 kB, 163 chunks | 1170.0 kB, 148 chunks | none (boots the whole `app.config`) |
| About dialog window | 441.2 kB, 30 chunks | 442.1 kB, 30 chunks | CI fails over 480 kB |
| Initial chunks (`ng build` budget) | 173.81 kB (styles 165.56, `main` 7.72, polyfills 0.5) | 173.81 kB | warn 185 kB, error 192 kB (was 190/210) |

What moved out of the first-paint set (all load when used, with the same behaviour): the dialogs, wizards and editors that
commands and services imported statically (format wizard, folder setup, report export, incident summary, redaction,
export, import range, add-tile, clock, updates, parser editor and import confirm, mark/marker editor, infer/extract/mask/drop,
note editor, knowledge editor, package review, layout dialogs, About, What's new with the generated changelog) through
`DialogService.openLazy(() => import(...), options)`; the search-everything palette (fetched on first open); and, with
`@defer`, the filter bar (and the query field), side bar, command palette, import toast, partial-import banner and tour
overlay. Dialog windows no longer create the workspace facade, time cursor, live follow or live tail: `sharedUiState`
takes a scope (`display`, `summary`, `full`) and a dialog window asks for the narrowest one its backends allow.

What is left (about 780 kB of the 1231 kB is `src/app`, about 500 kB Angular, CDK and RxJS): the `provide*()` of about 50
features in `app.config.ts`, each of which statically pulls its stores, commands and start-up work (share 54 kB, popout 37,
profiling 30, AI 28, video 18, notes 15, dumps 11, playbooks 11 ...), the four built-in themes (27 kB, needed for the first
paint), and the dialog registry (17 kB). Getting a pop-out or the main window much lower needs lazy feature registration (a
manifest of command and view stubs per feature, the feature's code loaded on first use) and per-view provider sets for
pop-outs; that is a design change, proposed as an ADR rather than done here.

## 2026-10-24, start-up timings (branch perf/startup)

Every start logs one structured `Startup timings` line (`app/src-tauri/src/startup_timings.rs`): milliseconds since
the process start for `windowCreated`, `setupDone`, `pageLoad`, `firstPaint` (the page's first contentful paint),
`windowReady` (ADR 0058), `workspaceLoaded` and `firstInteractive` (the first frame after the load), plus `setup`'s
steps slowest first (`Steps`). The `startup_timings` command returns the same. The nightly `build` run starts the
debug app five times after a first run (`scripts/ci/startup-bench.mjs`) and checks the medians against
`scripts/ci/startup-budgets.json` (macOS; Linux reports only until its numbers are recorded).

Debug app on the Mac mini, empty data folder, median of 15 everyday starts, before → after this branch (interleaved
runs, load average 9–13 from other builds, so ±30 ms):

| Mark | Before (ms) | After (ms) |
|---|---|---|
| windowCreated | 217 | 215 |
| setupDone | 282 | 264 |
| pageLoad | 530 | 524 |
| firstPaint | 636 | 610 |
| windowReady | 637 | 611 |
| workspaceLoaded | 648 | 619 |
| firstInteractive | 653 | 624 |

Steps that left the main thread: stored settings (11–250 ms under load: applying a plugin's section starts its
process) and the tray icon (9–13 ms, now after the first paint); the store worker starts after the first paint
(about 0.5 s of background work the first query no longer waits for); the AI start-up log line no longer scans every
process. What remains before the first paint is the Tauri builder (`before setup`, ~115 ms in a debug build), creating
the main window's webview (~100 ms) and WebKit starting its content process after `setup` (~260 ms between
`setupDone` and `pageLoad`, outside Debrief's code). The native menu bar is built twice per start (the standard menus,
then the UI's once the workspace loaded), was 3–4 times.

The 1.4 s the user's 1.0.2 log showed between the window and the AI line was not reproduced with an empty data
folder (30 ms here); it depends on that Mac's state (stored settings with plugin sections, an AI model loading). The
`Steps` field of the next start's line names the step.
