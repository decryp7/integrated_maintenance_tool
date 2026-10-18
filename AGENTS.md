# IMT — engineering rules for coding agents and contributors

IMT is an IDE for analysing logs and diagnostic files: a Rust engine (`crates/imt-core`),
a Tauri 2 shell (`app/src-tauri`) and an Angular 22 UI (`app/src`). See README.md for
the architecture and user-facing behaviour.

## Design principles

Apply these deliberately. They are not decoration:

- **Single responsibility.** One reason to change per module, type and component. Split a
  file when it starts doing several jobs (e.g. loading vs. querying vs. aggregation); keep
  request/response DTOs apart from the logic that produces them.
- **Open/closed.** Extend by adding, not editing: new file formats are new `Importer`
  implementations registered in `Importers`; new behaviour for users is profile
  configuration (extractors, markers, charts), not hard-coded special cases.
- **Liskov.** Every `Importer`, `LogStore` or UI backend implementation must honour the
  documented contract of its trait/interface: same error semantics, no surprising panics.
- **Interface segregation.** Keep traits and TypeScript interfaces small and purpose-built;
  don't make consumers depend on methods they don't use.
- **Dependency inversion / code against abstractions.**
  - The Tauri shell depends on the `LogStore` trait, never on a concrete store, so the
    in-memory engine can be replaced (e.g. by DuckDB) without touching commands or the UI.
  - Importers are reached only through the `Importer` trait.
  - Angular components never call Tauri `invoke` directly: they go through the backend
    service injected by DI token (real Tauri backend or the mock for browser dev/tests),
    and state lives in SignalStores.
- **Pure core, thin edges.** Parsing, query evaluation, geometry, formatting and zoom math
  are pure functions that are trivially unit-testable; I/O and framework glue stay thin.
- **Ports and adapters.** `imt-core` is the core; `Importer` and `LogStore` are its ports;
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

- All code that ships inside IMT is Rust (engine, shell, plugins, workers, profiler agent) plus the
  Angular UI. Don't add C#, C++, Python or other runtimes to the app itself.
- Other languages are supported for plugins through the stable contracts (docs/adr/0004): any
  language that speaks gRPC (`imt.plugin.v1`); the WebAssembly route was dropped (ADR 0004
  amendment, 2026-10-18). The Rust and C#/.NET (`sdk/dotnet`) SDKs are official; SDKs for further languages are added to
  `sdk/<language>/` only when a real plugin needs one, are conformance-tested in CI, and are never bundled into the app.

## Where logic lives

- **The Rust engine is the single source of truth** for data and behaviour: parsing, timestamp
  handling, profiles and pipelines, filtering and the query language, aggregation, analysis
  (anomalies, patterns, exceptions, comparisons), exports, projects and plugins. Results are
  computed in Rust and returned through `LogStore` and Tauri commands.
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
    guarded adapters (`imt_core::guard`): panics become errors attributed to the plugin, and
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
- The profile schema is `crates/imt-core/src/default_profile.toml`, which also serves as its
  documentation. Keep new keys optional (merged over defaults) and documented there.

## Testing and quality gates

- Every change ships with tests. Every bug fix starts with a failing regression test.
- Coverage targets: `imt-core` ≥ 99% lines (`cargo llvm-cov -p imt-core --summary-only`),
  Angular ≥ 80% lines overall and near 100% for stores, services and pure utilities.
- End-to-end engine behaviour is tested against `/samples` (`crates/imt-core/tests`).
  Regenerate samples with `python3 samples/generate.py`; don't hand-edit them.
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
  autocomplete), themed through IMT's CSS tokens so the look stays consistent. Icons use Angular
  Material's `MatIconRegistry` with IMT's own themed SVG icon set registered as literals in the
  `imt` namespace (`<mat-icon svgIcon="imt:timeline">`, wrapped by `<imt-icon>`); no icon fonts
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
