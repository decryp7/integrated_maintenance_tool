# Commands (Windows dev machine; Git Bash and PowerShell both used)

## Rust
- `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --no-fail-fast`, `cargo fmt --all --check`, `cargo deny --workspace check`.
- Memory-light local builds (32 GB machine, parallel agents): set `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, `CARGO_BUILD_JOBS=4`, and a private `CARGO_TARGET_DIR` per worktree.
- Format only touched files: `rustfmt --edition 2021 <files>` (don't run `cargo fmt --all` while others edit).
- Store crate tests: set `DUCKDB_LIB_DIR`/`DUCKDB_INCLUDE_DIR` and PATH to the fetched libduckdb folder.
- Samples: `cargo xtask gen-samples` (uses `python`, `python3` is a Store stub on this PC).
- Notices after dependency changes: `bash scripts/notices/generate.sh`.

## UI (in app/)
- `npm ci`, `npm run i18n` (commit regenerated `src/locale/messages.xlf` / `messages.pseudo.json`), `npm run lint`, `npx ng build`, `npx ng test --watch=false`.
- Playwright: `npx playwright test --project=chromium <specs>`; parallel runs need `DEBRIEF_E2E_PORT`/`DEBRIEF_E2E_PROD_PORT`; set `CI=true` in a git worktree (else the Angular cache lands in the main checkout and crashes).
- Real-app smoke: `npm run e2e:tauri:build` then `npm run e2e:tauri` (wdio; `DEBRIEF_E2E_APP` to use another binary; `DEBRIEF_E2E_KEEP_DATA=1` keeps Debrief's logs).
- NEVER `npx prettier --write`: the code is not prettier-formatted (~140 cols); lint is the gate.

## Windows quirks
- `!`-prefixed user commands run in Bash: wrap PowerShell cmdlets as `powershell.exe -NoProfile -Command "..."`.
- `git push` from Bash sometimes fails auth; retry from PowerShell.
- Elevated actions (Defender exclusions, runner config in C:\gitea-runner, minidumps) need the user via UAC.