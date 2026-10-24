# Tests that depend on the environment

A few tests cannot pass in every environment. They are not skipped: they run and fail loudly where their
precondition is missing, and they pass in CI.

- **Plugin and harness tests** (`debrief-plugin-test::harness`, `debrief-plugins`, `debrief-plugin-host::remote`,
  `debrief-store-worker` parity tests that read plugin-backed formats) start the plugins binary, which locks itself
  into an OS sandbox. Inside another sandbox (a coding agent's, for example) `sandbox_init` is refused and the plugin
  exits with status 71, so the tests fail locally there. On the Linux and Windows CI runners they pass (the `rust tests`
  job of the `build` workflow on `main`).
- **Plugins binary**: `cargo test -p <crate>` does not build `debrief-plugins`; a workspace test run does. Tests that
  need it say so (`cargo build -p debrief-plugins`).
- **libduckdb**: workers find `libduckdb` through `DYLD_LIBRARY_PATH` (macOS) or `LD_LIBRARY_PATH` (Linux), set to the folder
  `DUCKDB_LIB_DIR` points at.
- **Memory budgets** (`debrief-store-worker/tests/it/worker.rs`): macOS reports a larger resident size than Linux for the
  same work, so the idle and query budget is higher there (see the constant's comment).
- **Time zones**: UI specs that assert wall-clock text pin the zone through the workspace setting; run `ng test` with
  `TZ=Pacific/Auckland` to check that none depends on the host zone.
