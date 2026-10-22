# Debrief: core map

Desktop IDE for analysing logs/diagnostic files. Rust engine + Tauri 2 shell + Angular UI. Pre-v1: redesign freely, no backward-compat code (open redesign items: docs/roadmap.md).

## Source map
- `crates/debrief-core`: engine, single source of truth (parsing, timestamps, profiles/pipelines, query language, analysis, exports, projects, plugin registry/guards). Details: `mem:engine/core`.
- `crates/debrief-batch`, `crates/debrief-store-duck`, `crates/debrief-store-worker`: database-first engine (ADR 0014; Arrow batches, zstd Parquet segments queried by DuckDB). Part of `mem:engine/core`.
- `app/src-tauri`: Tauri shell (commands, windows, jobs, notifications, AI, internal browser, settings, paths/grants). Details: `mem:shell/core`.
- `app/src`: Angular UI, presentation only. Details: `mem:ui/core`.
- `crates/debrief-ai`, `debrief-ai-worker`: AI assistant + llama-server worker; `debrief-ipc`: pipes/sockets, supervisor, job objects; `debrief-plugin-*`: plugins; `debrief-collect`, `debrief-monitor`, `debrief-agent`, `debrief-profile`, `debrief-pdf`.
- `xtask/`: `cargo xtask gen-samples`. `samples/`: generated test corpus (never hand-edit). `fuzz/`: separate cargo workspace.
- `docs/adr/`: architecture decisions (0014 database-first engine; check numbering before adding — numbers are taken by parallel work).
- CI and runners: `mem:ci/core`.

## Invariants
- Rust computes; Angular renders. New UI computation → new engine op + command.
- UI ↔ backend contract: `app/src/app/core/api.types.ts` mirrored by Rust DTOs in the same change.
- Profile schema documented in `crates/debrief-core/src/default_profile.toml`; new keys optional.
- Untrusted input: parsers never panic, bounded (MAX_LINE_LEN, user_regex::compile, size caps); no unwrap on user data/IO.
- Paths from the webview are never trusted: they must come from Rust-side grants (`app/src-tauri/src/paths.rs`).
- Every Tauri command async; no blocking on the main thread.
- Commits: Conventional Commits, never Co-Authored-By/AI attribution trailers.

Further: `mem:tech_stack`, `mem:suggested_commands`, `mem:conventions` (code style + design rules), `mem:task_completion` (gates before committing).