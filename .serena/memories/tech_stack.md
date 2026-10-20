# Tech stack

- Rust stable (CI uses latest stable; workspace `rust-version` 1.95), edition per `[workspace.package]`; deps centralised in root `[workspace.dependencies]` (members use `x.workspace = true`; deps used by one crate stay local).
- Profiles: `[profile.dev.package."*"] opt-level = 2`; `[profile.dist]` (thin LTO, codegen-units 1, strip) for tag/installer builds (`--profile dist`, output `target/dist/`).
- Tauri 2 (WebView2 on Windows, WebKitGTK on Linux, WKWebView on macOS); installers: NSIS on Windows, deb/rpm on Linux.
- Angular 22 (zoneless, OnPush, signals/SignalStores, standalone), Angular CDK + selective Material, Vitest + TestBed, Playwright (chromium, webkit, prod-csp projects), WebdriverIO + tauri-driver smoke tests of the real app.
- Node 24 (`app/.nvmrc`), npm 11 (`packageManager` in app/package.json), `npm ci` only.
- Charts: Apache ECharts; timeline is custom.
- Store: DuckDB 1.5.x via duckdb-rs (prebuilt libduckdb fetched by a pinned-SHA script; `DUCKDB_LIB_DIR` needed), Arrow 58 in imt-batch (plugin SDK still on Arrow 60 until R3), Tantivy for full-text.
- Plugins/workers: out-of-process over imt-ipc (gRPC being replaced by a postcard frame codec in R3).
- Licences: cargo-deny (`deny.toml`), cargo-about notices (`scripts/notices/generate.sh`).
- Dependency updates: Renovate, one grouped weekly PR (renovate.json).