# IMT (Integrated Maintenance Tool): core map

Desktop IDE for analysing logs/diagnostic files. Rust engine + Tauri 2 shell + Angular UI. Pre-v1: redesign freely, no backward-compat code (docs/redesign-2026-10.md is the active plan).

## Source map
- `crates/imt-core`: engine, single source of truth (parsing, timestamps, profiles/pipelines, query language, analysis, exports, projects, plugin registry/guards). Details: `mem:engine/core`.
- `crates/imt-batch`, `crates/imt-store-duck`, `crates/imt-store-worker`: database-first engine (ADR 0014; Arrow batches, zstd Parquet segments queried by DuckDB). Part of `mem:engine/core`.
- `app/src-tauri`: Tauri shell (commands, windows, jobs, notifications, AI, internal browser, settings, paths/grants). Details: `mem:shell/core`.
- `app/src`: Angular UI, presentation only. Details: `mem:ui/core`.
- `crates/imt-ai`, `imt-ai-worker`: AI assistant + llama-server worker; `imt-ipc`: pipes/sockets, supervisor, job objects; `imt-plugin-*`: plugins; `imt-collect`, `imt-monitor`, `imt-agent`, `imt-profile`, `imt-pdf`.
- `xtask/`: `cargo xtask gen-samples`. `samples/`: generated test corpus (never hand-edit). `fuzz/`: separate cargo workspace.
- `docs/adr/`: architecture decisions (0014 database-first engine; check numbering before adding — numbers are taken by parallel work). `docs/redesign-2026-10.md`: phased redesign R0–R5.
- CI and runners: `mem:ci/core`.

## Invariants
- Rust computes; Angular renders. New UI computation → new engine op + command.
- UI ↔ backend contract: `app/src/app/core/api.types.ts` mirrored by Rust DTOs in the same change.
- Profile schema documented in `crates/imt-core/src/default_profile.toml`; new keys optional.
- Untrusted input: parsers never panic, bounded (MAX_LINE_LEN, user_regex::compile, size caps); no unwrap on user data/IO.
- Paths from the webview are never trusted: they must come from Rust-side grants (`app/src-tauri/src/paths.rs`).
- Every Tauri command async; no blocking on the main thread.
- Commits: Conventional Commits, never Co-Authored-By/AI attribution trailers.

Further: `mem:tech_stack`, `mem:suggested_commands`, `mem:conventions` (code style + design rules), `mem:task_completion` (gates before committing).