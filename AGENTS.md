# Debrief — engineering rules for coding agents and contributors

Debrief is an IDE for analysing logs and diagnostic files: a Rust engine (`crates/debrief-core`),
a Tauri 2 shell (`app/src-tauri`) and an Angular 22 UI (`app/src`). See README.md for
the architecture and user-facing behaviour.

## Paddock rules (binding)

The family's binding rules, copied from [paddock/docs/rules.md](https://dev.decryptology.net/decryp7/paddock/src/branch/main/docs/rules.md) (the source; update there first). Where an older rule in this file disagrees, these win.

### Way of working
1. **One app at a time.** Finish the current app before starting another; parallel agents only within that app.
2. **Every agent works in its own git worktree** with its own `CARGO_TARGET_DIR` and its own e2e port; the main checkout
   stays on `main` for the coordinator.
3. **Tests in a branch: only the affected ones** (touched crates, touched specs, related Playwright files), with
   Playwright in Chromium only. The full suite, Chromium and WebKit, runs once on the combined result before a local
   build or release, plus CI. Fix everything locally first; CI confirms, it is not where failures are found.
4. **When CI fails:** fix and re-run only the failed tests locally, push once; never push fix by fix to see what CI says.
   Lint and type-check the touched files before every push.
5. **Every bug fix starts with a failing regression test.** A test-only failure is fixed properly (deterministic, no
   sleeps), never silently skipped.
6. **Tests never hard-code the app version;** they read it from its single source and check its shape.
7. **Tests never touch the user's machine:** no real launchd/Login Items/services, no installed apps, never the
   installed Google Chrome (pinned Chrome for Testing or an explicit env var only), no `~/Documents`, `~/Desktop`,
   `~/Downloads`, no real user data, no windows on the desktop, no processes left behind. Docker: only containers you
   created.
8. **Every lesson goes into [paddock lessons](https://dev.decryptology.net/decryp7/paddock/src/branch/main/docs/lessons.md)**, every rule here; no `Co-Authored-By` or AI attribution anywhere.
9. **Releases only from a tag in CI**, never from a local machine; a local build for the user to try is fine but is never
   published. Never tag a `[skip ci]` commit.

### Long operations and performance
10. **Every operation runs on a background thread or worker pool,** never on the UI thread, the Tauri main thread or an
    IPC handler's thread; commands return at once and report through job events.
11. **Live, phase-labelled progress from the first moment,** each phase on its own scale (a growing count when the total
    is unknown, never a static 0); progress values never go backwards; Cancel works in every phase and leaves state
    consistent (build new data, then swap).
12. **The UI never times out a running job;** it shows "Still working".
13. **Parallel where it helps, bounded by the kind of work** (CPU: cores−1 or the Performance setting; SSD I/O 4–8; HDD
    1–2; network a few per host; memory-heavy by RAM), background priority, deterministic results.
14. **Multi-stage work is a pipeline:** producers → bounded queue → workers, stage-specific thread counts, back-pressure,
    cancel across stages, per-stage progress; determinism by commit-time ordering. IDs: number only new items in sorted
    canonical relative path order; existing IDs never change; moved/renamed files are matched by path, then fingerprint.
15. **Hash while writing;** never re-read a just-downloaded file. Large downloads use segmented (Range) downloads where
    the host supports them, with per-host caps and 429 back-off.
16. **Performance has budgets** (startup, import throughput, time to first event, query latency, scroll smoothness,
    memory) checked in the nightly benchmark workflow.

### User interface
17. **Every dialog is a real OS window** (owned by and centred on the main window, remembered size/position, Escape/Enter);
    modality is explicit per dialog (modal for confirmations, credentials, unsaved changes, pickers that feed the next step;
    modeless for tools kept open beside the work). Menus, dropdowns, autocomplete and tooltips stay in-app.
18. **Panes lay out by container queries;** paths and file names truncate in the middle with the full value in a tooltip;
    no horizontal scroll at any width ≥ 320 px.
19. **A control that repairs X never depends on X;** every "X is not running" message has a working Repair button; never
    a message that points at the page it is shown on; an installable thing is never listed as "planned".
20. **Search finds identifiers however they are written** (parts, joins, separators, case, typos) and the AI resolves the
    user's words to concrete values before querying.
21. **Tunables are Settings, knowledge lists are TOML packs** (built-in + user `packs/`, parsed once with limits, never
    executing code, reloaded without a restart).
22. **Every drawing, diagram and screenshot gets an independent review;** every Paddock image or status change also
    refreshes the profile repo.

### Platform and release
23. **macOS bundles are ad-hoc sealed** (`signingIdentity: "-"`) and CI verifies them with `codesign --verify --deep
    --strict` on the .app and the DMG; release verification runs it on the published DMG. While ad-hoc, the hardened
    runtime is off (ad-hoc code under it cannot load vendor-signed libraries: "different Team IDs"); every bundled
    Mach-O carries the same TeamIdentifier, and CI starts every sidecar from the built app (`--self-test`). The hardened
    runtime comes back with a real signing identity.
24. **Services survive updates:** re-register once per version change (unregister → `launchctl bootout` → wait →
    register) and repair a stale registration automatically.
25. **Before tagging:** `cargo deny check advisories`, one CI build with bundles, the icon check, strip whitespace from
    signing secrets, and the release notes' First launch section (until the apps are signed).
26. **Process memory on Linux:** skip sysinfo thread entries and measure PSS (`smaps_rollup`).

### Angular and SignalStore (approved 2026-10-23; reasons: [research](https://dev.decryptology.net/decryp7/paddock/src/branch/main/docs/research/2026-10-angular-and-signalstore.md))
27. Zoneless only. No `zone.js` in any app, no `provideZoneChangeDetection`. Every component declares `changeDetection: ChangeDetectionStrategy.OnPush` explicitly (v22 defaults to it, but explicit survives a lint or a future default change); `Eager` requires a comment.
28. `effect()` is only for side effects on things outside the signal graph (DOM, IPC, storage, logging, charts). State derived from other state is `computed`, `linkedSignal` or `resource`. An effect that calls `.set()`/`patchState()` needs a one-line justification comment, and `untracked()` is allowed only around the side effect, not to hide a dependency.
29. Reads of signals in an async function or effect happen before the first `await`.
30. Request state is a status union (`idle | loading | loaded | error`) from one shared `withRequestStatus` feature, never separate `loading`/`error` flags.
31. A store is changed only through its own methods; `protectedState` stays on; no `patchState` outside the store file; store files are named `*.store.ts`.
32. Provide a store at the narrowest scope (route/component/dialog window); `providedIn: 'root'` only for app-wide singletons. A store injects at most two other stores; more means a missing facade or feature.
33. New code uses `inject()`, signal `input/model/output/viewChild`, `@if/@for/@switch` with `track`, standalone components, and `[class.x]` over `ngClass`.
34. Each OS window is its own Angular bootstrap with its own small provider set. The engine (Rust) is the single source of truth; windows share state by commands plus events and never by importing another window's store state. Dialog windows load only the providers they need.
35. Heavy, rarely used panels are lazy (`@defer` or `loadComponent`); every release records the initial chunk size and the budget is tightened to measured size plus 10%.
36. Prefer `resource`/`rxResource` for new read-only loading with cancellation; keep `withMethods` async for commands. Do not adopt `@Service`, `injectAsync` or the NgRx events plugin until documented stable.
37. New forms use Signal Forms; existing reactive forms stay until touched.
38. Tests: Vitest through the Angular unit-test builder, `await fixture.whenStable()`, store tests via `TestBed`; use CDK harnesses for Material/CDK widgets; each lifecycle effect gets a test that triggers the failure.
39. A shared store feature or component used by two apps lives in `paddock-ui`, not copied.

**Design (user, 2026-10-24)**
40. **Recurring bugs mean a missing design rule.** When the same kind of bug shows up again (in one area, across dialogs,
    panes or apps), stop fixing instances: name the pattern, propose a redesign as an ADR that makes the class impossible
    or checked (a declared contract, one shared mechanism, a lint or test over every instance), and fix the instances
    through it. Until v1, check every batch of bug reports for such patterns.

**Throughput (user, 2026-10-24)**
41. **Scale parallel work to the machine.** The coordinator watches load and free disk continuously and keeps the
    machine busy but not overloaded: start more agents when load stays low, hold or stop starting them when it stays
    high (Mac mini, 14 cores: aim for a 1-minute load of about 6–16, at most ~5 heavy jobs, `CARGO_BUILD_JOBS=2` each),
    use the shared build cache (`sccache`) and copy-on-write target folders, and never let tests crowd out a release.

**Sharing (user, 2026-10-24)**
42. **Share first.** Before building anything in an app, check Paddock's shared crates and packages (crates/README.md,
    ui/README.md, docs/plans/) for it. If it exists, use it; if it is missing, generic, or another app has or will need the
    same thing (rule 39), build it in Paddock and consume the published version instead of writing an app copy. Every
    work brief includes this check; every app move and every few releases a survey looks for copies that drifted apart
    and turns them into shared work packages. App-specific domain logic stays in the app.

**Testing shared code (user, 2026-10-24)**
43. **Shared behaviour is tested once, in Paddock.** A shared crate or package carries the full tests of its behaviour
    (unit, robustness, fuzz, real-OS where it matters). An app tests only its wiring, through the shared contract tests
    (`paddock-e2e`, `paddock-testkit`, conformance), its own domain logic, and one real-app smoke per release. When an app
    adopts a shared package it deletes its own copy and the tests that duplicated it, and records test count and time
    before and after.

## Design principles

Apply these deliberately. They are not decoration:

- **Single responsibility.** One reason to change per module, type and component. Split a
  file when it starts doing several jobs (e.g. loading vs. querying vs. aggregation); keep
  request/response DTOs apart from the logic that produces them.
- **Open/closed.** Extend by adding, not editing: new file formats are new `Importer`
  implementations registered in `Importers`; new behaviour for users is profile
  configuration (extractors, markers, charts), not hard-coded special cases.
- **Liskov.** Every `Importer`, `Store` (and its port traits) or UI backend implementation must honour the
  documented contract of its trait/interface: same error semantics, no surprising panics.
- **Interface segregation.** Keep traits and TypeScript interfaces small and purpose-built;
  don't make consumers depend on methods they don't use.
- **Dependency inversion / code against abstractions.**
  - The Tauri shell depends on the `debrief_core::ports::Store` traits, never on a concrete store, so the
    store engine (DuckDB in the store worker today) can be replaced without touching commands or the UI.
  - Importers are reached only through the `Importer` trait.
  - Angular components never call Tauri `invoke` directly: they go through the backend
    service injected by DI token (real Tauri backend or the mock for browser dev/tests),
    and state lives in SignalStores.
- **Pure core, thin edges.** Parsing, query evaluation, geometry, formatting and zoom math
  are pure functions that are trivially unit-testable; I/O and framework glue stay thin.
- **Ports and adapters.** `debrief-core` is the core; `Importer` and `Store` (`debrief_core::ports`) are its ports;
  Tauri, future stores (DuckDB) and file-format plugins are adapters. The core never
  depends on an adapter.
- **Parse, don't validate.** Convert untrusted input (file bytes, IPC JSON, profile TOML)
  into typed values once, at the edge, and work with the types afterwards.
- **Make illegal states unrepresentable.** Prefer enums and newtypes over strings, flags and
  `Option` combinations that can disagree.
- **Command–query separation.** Queries (page, timeline, facet, chart, preview) have no side
  effects; only open/reload/clear/remove/set_profile mutate.
- Also: separation of concerns, high cohesion and low coupling, KISS, YAGNI, DRY with the
  rule of three before abstracting, composition over inheritance, Law of Demeter, least
  astonishment (sensible defaults, profile overrides), fail fast with a message that tells
  the user what to fix, and leave touched code better than you found it.

## Security and robustness

The app reads untrusted, possibly huge files and renders their content:

- Never trust the webview. Validate every IPC argument in Rust; open only paths the user
  chose (dialog, drag-and-drop, command line); canonicalize them.
- Tauri capabilities stay least-privilege: only the plugins and permissions the UI uses,
  no `fs`/`shell` wildcards, no remote content. Keep the CSP free of `unsafe-eval` and
  remote origins. Never bind log text with `[innerHTML]`.
- Parsers never panic on any input and enforce limits (`MAX_LINE_LEN`, user regexes via
  `user_regex::compile`, which caps pattern length and compiled size).
- No `unwrap()`/`expect()` on user data or I/O outside tests. Library errors are typed; the
  app edge converts them to messages for the UI.
- Long-running work runs off the UI thread; large results are paged or bucketed, never
  sent in one reply.
- Logs must not contain file contents by default; any crash reporting is opt-in.

## Languages

- All code that ships inside Debrief is Rust (engine, shell, plugins, workers, profiler agent) plus the
  Angular UI. Don't add C#, C++, Python or other runtimes to the app itself.
- Plugins are Rust only and always run out of process (docs/adr/0017): a plugin depends on `debrief-plugin-api`
  alone (never on `debrief-core`), is served by `debrief_plugin_api::serve`, and talks to Debrief over the one IPC
  (`debrief-ipc`: length-prefixed postcard frames on a private local stream). There are no SDKs for other languages,
  no gRPC and no in-process plugins. Code that several plugins or the agent share is a library crate, not a
  plugin service.

## Where logic lives

- **The Rust engine is the single source of truth** for data and behaviour: parsing, timestamp
  handling, profiles and pipelines, filtering and the query language, aggregation, analysis
  (anomalies, patterns, exceptions, comparisons), exports, projects and plugins. Results are
  computed in Rust and returned through the `Store` ports and Tauri commands.
- **Angular is the presentation layer**: layout, interaction state (selection, zoom, open panels,
  unsaved form edits), rendering and formatting of what the engine returns. It must not
  re-implement domain rules. If the UI needs a computation (e.g. a parse preview, a validation,
  a count), add an engine operation and a command for it instead of writing it in TypeScript.
  Small purely visual transforms (e.g. turning rows into a tree for display, layout of markers)
  are fine.
- **The mock backend** exists for browser development and UI tests only. It mimics the contract
  with plausible data; it is never the reference for behaviour, and features are not "done"
  until they work against the real engine.
- Validation that matters is done in Rust; the UI may validate early for convenience, but the
  engine re-validates.

## Robustness

- **Contain failures.** A bug in one file, plugin, panel or command must never take down the
  app or poison later work:
  - Plugin contributions are always registered through `PluginRegistry`, which wraps them in
    guarded adapters (`debrief_core::guard`): panics become errors attributed to the plugin, and
    repeat offenders are quarantined. Never call plugin code outside these adapters.
  - Per-file work (`import_file`) runs under `guard::catch`: one bad file is reported as
    unrecognised with a reason; the rest imports.
  - Shared state behind locks recovers from poisoning (`store_read`/`store_write`); never
    `unwrap()` a lock.
  - Each UI panel has an error boundary; a failed request shows an inline error with Retry.
- **Degrade, don't fail.** Missing paths, locked files, unavailable tools, missing rights and
  platform limits produce a clear message (and a next step: retry, elevate, convert) instead of
  an exception. External tools always run with timeouts.
- **Recoverable sessions.** Work in progress is autosaved (session file) and an unclean
  shutdown offers restore and safe mode. New persistent state must be versioned and must
  tolerate missing/corrupt files by falling back to defaults.
- **Bounded resources.** Respect the existing limits (line size, regex size, archive
  expansion, result caps) and add limits for any new unbounded input.
- **Every robustness behaviour has a test that triggers the failure** (crashing plugins,
  corrupt files, locked files, poisoned locks, missing tools).

## Working in parallel (agents)

- Each task owns an explicit set of files; don't edit outside it. Shared integration points
  (`lib.rs`, `api.rs`, `store.rs`, the workspace `Cargo.toml`, `api.types.ts`) are changed by the
  integrating agent only, or by adding a single declared line.
- Don't run `cargo fmt --all` / `cargo clippy --fix` across the workspace while others are
  editing; format your own files (`rustfmt <files>`). Use a private `CARGO_TARGET_DIR` for
  coverage runs to avoid clobbering others.
- Commit only complete, owned work. Never commit a workspace `Cargo.toml` listing crates whose
  sources aren't committed. After committing, verify the committed tree in isolation
  (`git archive HEAD` → build and test) before pushing larger changes.
- Windows-only code must at least type-check: `cargo check --target x86_64-pc-windows-msvc`
  for crates that don't need the Windows SDK; otherwise rely on the Windows CI runner and say so.

## Contracts

- The UI ↔ backend wire contract is `app/src/app/core/api.types.ts` (camelCase JSON).
  Any change there must be mirrored in the Rust DTOs, and vice versa, in the same change.
- The profile schema is `crates/debrief-core/src/default_profile.toml`, which also serves as its
  documentation. Keep new keys optional (merged over defaults) and documented there.

## Testing and quality gates

- Every change ships with tests. Every bug fix starts with a failing regression test.
- Coverage targets: `debrief-core` ≥ 99% lines (`cargo llvm-cov -p debrief-core --summary-only`),
  Angular ≥ 80% lines overall and near 100% for stores, services and pure utilities.
- End-to-end engine behaviour is tested against `/samples` (`crates/debrief-core/tests`).
  Regenerate samples with `cargo xtask gen-samples`; don't hand-edit them.
- UI tests are layered: unit/component tests (Vitest + TestBed) for logic and components;
  Playwright end-to-end flows against the browser build with the mock backend (Chromium and
  WebKit); a few WebdriverIO + `tauri-driver` smoke tests of the real app (Windows/Linux only).
- Coverage is necessary, not sufficient: assert behaviour, include edge cases and error
  paths. Planned next: proptest "never panics" properties and cargo-fuzz targets for every
  parser, and cargo-mutants to measure test strength.
- Before committing, all of these must pass:

  ```sh
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  cargo deny --workspace check
  (cd app && npm audit --omit=dev --audit-level=high && npx ng build && npx ng test --watch=false)
  ```

## Dependencies and releases

- Prefer well-maintained, permissively licensed dependencies (`deny.toml` lists the allowed
  licences). Commit `Cargo.lock` and `package-lock.json`; install with `npm ci` in CI.
- Renovate updates dependencies and pins CI actions to commit digests. Gitea Actions needs
  the v3 artifact actions.
- Use Conventional Commits (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `ci:`, `chore:`)
  so changelogs can be generated. Releases are `v*` tags following semver.
- Record significant architecture decisions as ADRs in `docs/adr/`.

## Style

- Match the surrounding code: naming, comment density (doc comments on public items,
  comments only where intent isn't obvious), idioms. Rust uses `rustfmt.toml`
  (max_width 140).
- UI: standalone components, signals, CSS custom properties for theme tokens (dark and
  light), accessible markup (real buttons, labels, aria-labels on icon buttons).
- UI building blocks: use **Angular CDK** (`@angular/cdk`: menu, overlay, dialog, a11y, listbox,
  scrolling, drag-drop) for behaviour and accessibility instead of hand-rolling them, and
  **Angular Material** selectively for complex inputs (e.g. slide toggle, tooltip, datepicker,
  autocomplete), themed through Debrief's CSS tokens so the look stays consistent. Icons use Angular
  Material's `MatIconRegistry` with Debrief's own themed SVG icon set registered as literals in the
  `debrief` namespace (`<mat-icon svgIcon="debrief:timeline">`, wrapped by `<debrief-icon>`); no icon fonts
  and no icon URLs fetched at runtime (the CSP forbids it).
- Charts use Apache ECharts; the timeline is custom. Validate categorical palettes for
  colour-vision deficiency before adding series colours.

## Working practices

- Research before choosing libraries or patterns when unsure, and prefer
  well-maintained, permissively licensed (MIT/Apache-2.0) dependencies.
- Navigate code with Serena's symbol tools (find symbol, references, overview) when they
  are available instead of reading whole files; it saves context and cost.
- Delegate mechanical or well-specified work (refactors with a precise spec, test
  scaffolding, docs) to cheaper models (Sonnet/Haiku subagents). Keep architecture and
  review on the main model.
