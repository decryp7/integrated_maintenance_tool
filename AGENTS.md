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
  cargo deny check
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
