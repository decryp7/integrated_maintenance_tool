# Done = these pass

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo deny --workspace check
cd app && npm run i18n && npm run lint && npx ng build && npx ng test --watch=false
```
Plus, as relevant:
- Playwright specs for touched features (`--project=chromium`; WebKit via `npx playwright install webkit` if needed).
- `bash scripts/notices/generate.sh` after dependency changes (the licences CI job fails on stale notices).
- Store crates (outside the workspace until CI fetches libduckdb): `cargo clippy/test --manifest-path crates/debrief-store-duck/Cargo.toml` with DUCKDB_LIB_DIR set.
- Real-app smoke for shell/lifecycle changes (`npm run e2e:tauri`).
- Commit with Conventional Commits, no attribution trailers; locale files regenerated; worktree clean.
- Coverage targets: debrief-core ≥ 99 % lines; Angular ≥ 80 % overall, near 100 % for stores/services/pure utilities.