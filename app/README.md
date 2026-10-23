# Debrief app

The Debrief desktop app: an Angular 22 UI (`src/`) in a Tauri 2 shell (`src-tauri/`). The Rust engine, the store, the
plugins and the workers live in [`../crates`](../crates); how the parts fit together is in
[`../docs/architecture.md`](../docs/architecture.md), and the build, test and CI instructions are in
[`../docs/development.md`](../docs/development.md). Engineering rules: [`../AGENTS.md`](../AGENTS.md).

| Folder | What it is |
|---|---|
| `src/app/` | The UI: standalone components, signals, `@ngrx/signals` stores, dockview panels, ECharts. Components reach the engine only through the `DebriefBackend` services injected by DI token (the Tauri backend, or the mock for the browser build and tests). The wire contract is `src/app/core/api.types.ts`. |
| `src-tauri/` | The Tauri shell: the command table, capabilities, windows, tray, jobs, sessions; no business logic. `tauri.sidecars.conf.json` adds the helper processes and libduckdb to the installers. |
| `e2e/` | Playwright end-to-end flows against the browser build with the mock backend (Chromium and WebKit). |
| `e2e-tauri/` | WebdriverIO + `tauri-driver` smoke tests of the real app (Windows and Linux), and the live search test. |
| `scripts/` | Build helpers: build info for the About dialog, the sidecar build (`build-sidecars.mjs`) and the libduckdb fetch (`libduckdb.mjs`), icon generation, the style lint, i18n extraction and the README screenshot generator. |

## Commands

Run them in this folder.

```sh
npm ci
npm start               # the UI in a browser against the mock backend, http://localhost:4200
npx tauri dev           # the desktop app with the real engine (needs libduckdb, see below)
npx tauri build         # installers for the current OS, in ../target/release/bundle/
npm run build           # production UI build (use this, not plain `ng build`: it passes the build info)
npm test                # unit and component tests (Vitest, `ng test`)
npm run lint            # angular-eslint and the style lint
npm run e2e             # Playwright flows (run `npm run e2e:install` once)
npm run i18n            # extract messages and regenerate the pseudo-locale
npm run screenshots     # regenerate ../docs/screenshots
npm run e2e:tauri:build && npm run e2e:tauri    # real-app smoke tests
```

The store worker links the official prebuilt libduckdb. `npx tauri build` and `node scripts/build-sidecars.mjs` fetch
and verify it themselves; for a plain `cargo` build or `tauri dev` on a machine without it, run
`node scripts/libduckdb.mjs` and run the lines it prints (PowerShell on Windows, sh elsewhere). Details:
[`../docs/development.md`](../docs/development.md#prerequisites).
