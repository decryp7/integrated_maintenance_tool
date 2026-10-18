# Engineering backlog

What is left after the 2026-10-18 pass. Product features are in [roadmap.md](roadmap.md).

## Done (for reference)

- Security and supply chain: user-regex and line-size limits, cargo-deny (licences, bans,
  advisories, sources), npm audit, least-privilege capabilities, strict CSP, Renovate with pinned
  action digests, CycloneDX SBOMs and git-cliff release notes on tagged releases, signed
  auto-update (off by default).
- Tests: fuzzing (cargo-fuzz, nightly, every parser), proptest "never panics" and paging
  invariants, insta snapshots of the importers on the samples, criterion benchmarks (main pushes),
  cargo-mutants (weekly, `--in-diff` on pull requests), Playwright e2e (mock backend), WebdriverIO
  + tauri-driver smoke tests of the real app (Windows/Linux), app unit tests running on Windows
  (embedded app manifest for test binaries).
- Robustness: typed errors at the command boundary, non-blocking startup, cancellable queries with
  progress, import progress/cancel, `tracing` with daily rolling logs (no file contents or query
  text in normal mode), byte-exact test inputs on every platform (`.gitattributes`).
- Process: Conventional Commits check on pull requests, lefthook pre-commit (rustfmt, clippy,
  eslint, commit-msg), `.editorconfig`, PR template, angular-eslint, TypeScript `strict` +
  `noUncheckedIndexedAccess`.

## Open

- **Signing:** Windows Authenticode and macOS Developer ID + notarisation (needs certificates
  and a self-hosted Mac runner). The auto-update key pair and secrets: see
  [releasing.md](releasing.md).
- **WCAG 2.2 AA review:** axe runs WCAG 2.2 A/AA rules in every e2e check and passes, target
  size (2.5.8) included (dense timeline markers share one target; annotation bands are reached
  through the "Evidence…" menu). Still to do by hand: focus-not-obscured (2.4.11) with docked and
  auto-hidden panels, chart table alternatives everywhere, a screen-reader pass (NVDA, VoiceOver).
- **Opt-in crash reports:** local capture exists (crash marker and restore, error snapshots,
  "Report a problem" bundle). Uploading needs a server and a privacy review; not planned.
- **i18n of engine text:** settings schema labels, error messages, analysis sentences and AI
  text come from Rust in English; they need message ids (the UI already parses one of them, the
  `pipeline step N (kind):` prefix, which first needs a structured error code).
- **Linux/macOS checks:** code paths that were only reviewed on Windows in this pass (llama-server
  child lifetime on Linux/macOS, macOS runtime signing, non-Windows stubs of the Windows plugins)
  are covered by the Linux and macOS CI runners only.
- **Batch queries:** the monitor live charts (11 charts) and multiple timelines send one request
  each; a batched `query_charts` / `query_timelines` would save round-trips if they show up as slow.
