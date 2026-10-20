# Conventions

## Design (enforced by AGENTS.md)
- SOLID, ports and adapters: imt-core owns ports (store ports per ADR 0014: Ingest, Query, Analysis, Profiles, Diagnostics, Search; `&self`, Send + Sync, owned DTOs); adapters never leak into core.
- Parse, don't validate: convert IPC JSON/TOML/file bytes to typed values at the edge; enums/newtypes over strings and flag combos.
- Command–query separation; queries side-effect free.
- Contain failures: plugin code only through `imt_core::guard` adapters; per-file import under `guard::catch`; locks recover from poisoning (never unwrap a lock); every robustness behaviour has a failure-triggering test.
- Bounded resources everywhere (byte-bounded caches, result caps reported as coverage, event size cap).
- Child processes via `imt_core::process::quiet_command` (clippy.toml disallows `Command::new`); window-building commands async.
- Windows-only code behind `cfg(windows)`; CI clippy runs on Linux, so avoid platform-only unused `mut`/imports (use `cfg_attr(not(windows), allow(...))`).

## Rust style
- rustfmt max_width 140; doc comments on public items; comments only where intent isn't obvious; typed errors (thiserror) in libraries, messages at the app edge.

## UI style
- Standalone components, signals, SignalStores; backend reached only via DI-token services (never `invoke` in components); mock backends lazy-loaded via `core/mock-registry`, never in the Tauri build.
- i18n: `$localize`/`i18n` with explicit `@@` ids; WCAG 2.2 AA (axe in Playwright); CDK key managers for listboxes/roving focus; icons via `<imt-icon>` (MatIconRegistry `imt` namespace, inline SVG, CSP forbids fetched icons).
- Never bind log text with `[innerHTML]`.
- Mock backend mimics the contract; it is never the behaviour reference.

## Process
- Every change ships with tests; a bug fix starts with a failing regression test.
- Significant decisions → ADR in docs/adr/.
- Parallel work in git worktrees; shared integration files (`lib.rs` command list, `activity/ipc.rs`, `api.types.ts`, mock backend, workspace Cargo.toml, locale files) are merged by the integrator; locale files are regenerated, not hand-merged.