# Development

How to build, test and release IMT, and where things live in the repository. Engineering rules for
contributors are in [AGENTS.md](../AGENTS.md); the design is in [architecture.md](architecture.md).

- [Building from source](#building-from-source)
- [Testing and quality gates](#testing-and-quality-gates)
- [CI/CD](#cicd)
- [Project layout](#project-layout)

## Building from source

### Prerequisites

- **Rust** (stable) with `rustfmt` and `clippy`.
- **Node.js ≥ 24.15** (see [`app/.nvmrc`](../app/.nvmrc)) and npm.
- **Tauri 2 platform dependencies** ([official guide](https://v2.tauri.app/start/prerequisites/)):
  - *Linux:* `libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev patchelf`
    (as installed by CI on Debian/Ubuntu).
  - *macOS:* Xcode Command Line Tools.
  - *Windows:* Microsoft C++ Build Tools and the WebView2 runtime. The installer is NSIS only (`tauri build`
    fetches NSIS itself).

### Develop

```sh
cd app
npm ci
npm start             # UI only, in a browser, against the built-in mock backend (http://localhost:4200)
npx tauri dev         # the desktop app with the real engine (starts `npm run start` for you)
```

```sh
cargo test -p imt-core        # engine tests, including end-to-end runs over /samples
```

Regenerate the sample logs with `python3 samples/generate.py` (do not edit them by hand);
[`samples/README.md`](../samples/README.md) lists every sample, its format and the views it shows.

### Visual Studio Code (Windows and macOS)

Open the **repository root** (not `app/`) and accept the recommended extensions
(`.vscode/extensions.json`: rust-analyzer, Tauri, C/C++ for debugging on Windows, CodeLLDB for
macOS and Linux, Angular, ESLint, Prettier, Playwright, draw.io, Buf for `proto/` (it needs the
[`buf` CLI](https://buf.build/docs/cli/installation/)), ...).

1. Install the prerequisites above. On macOS: `xcode-select --install`, Rust from
   [rustup.rs](https://rustup.rs), Node.js 24 (for example `brew install node@24`). On Windows: the
   Rust MSVC toolchain, the "Desktop development with C++" workload (Build Tools) and Node.js 24.
2. *Terminal ▸ Run Task… ▸ IMT: install UI dependencies* (runs `npm ci` in `app/`).
3. Run or debug:
   - *IMT: run app (tauri dev)* — the desktop app with hot reload.
   - **F5** with *Debug IMT app (Windows)* or *Debug IMT app (macOS / Linux)* — starts the UI dev
     server, builds the debug binary and attaches the native debugger (breakpoints in Rust).
     The *with the samples* variants open `samples/serilog-clef` on start.
   - *Debug UI in the browser (mock backend)* — the Angular UI alone, with Chrome DevTools-style
     breakpoints in TypeScript.
4. **Ctrl/Cmd+Shift+B** builds the debug binary; other tasks run the checks (`Rust: clippy`,
   `Rust: test workspace`, `UI: lint`, `UI: unit tests`, `UI: e2e`, ...) or all of them in order
   (*IMT: all quality gates*). `IMT: build installers (release)` runs `tauri build`.

rust-analyzer checks into its own target folder, so it never blocks a build in the terminal.

IMT's windows switch off browser behaviour (`app/src-tauri/src/native_feel.rs` for WebView2's
browser keys, `app/src/app/core/native-feel.ts` for the rest). Debug builds and `ng serve` keep the
developer tools (`F12`, `Ctrl+Shift+I`, or Shift+right-click for the browser menu with *Inspect*)
and hard reload (`Ctrl+Shift+R`, `Ctrl+F5`); `F5` and `Ctrl+R` are blocked everywhere.

### Package

```sh
cd app
npx tauri build                         # all bundle types for the current OS
npx tauri build --bundles deb,rpm       # Linux
npx tauri build --bundles nsis          # Windows
npx tauri build --bundles dmg           # macOS
```

Installers are written to `target/release/bundle/`:

| OS | Output |
|---|---|
| Linux | `bundle/deb/*.deb`, `bundle/rpm/*.rpm` |
| Windows | `bundle/nsis/*-setup.exe` |
| macOS | `bundle/dmg/*.dmg` (and `bundle/macos/IMT.app`) |

Build the UI with `npm run build` (or `npm start`), not plain `npx ng build`: the npm scripts run
[`app/scripts/build-info.mjs`](../app/scripts/build-info.mjs), which passes the commit and build date to
the About dialog (`--define IMT_BUILD_INFO`); a plain `ng build` shows them as "unknown". `tauri build`
and `tauri dev` use the npm scripts.

### Third-party notices

`THIRD_PARTY_NOTICES.md` (bundled with the app and shown in *About IMT*) and the texts compiled into
the About dialog are generated, not edited by hand:

```sh
scripts/notices/generate.sh    # needs cargo, node and cargo-about (installed if missing)
```

It collects the Rust crate licences with `cargo-about` and the npm production dependencies from
`package-lock.json`, writing `app/src-tauri/resources/THIRD_PARTY_NOTICES.md` and
`app/src/app/about/generated/`. CI regenerates them and fails if the committed files are out of date;
the installer builds regenerate them before bundling.

### Regenerating the screenshots

The README screenshots are captured by a Playwright script against the browser build with the mock
backend (1600×1000, dark theme plus one light shot). Views that are missing are skipped with a warning.

```sh
cd app
npm run e2e:install     # once: Playwright browsers
npm run screenshots     # writes docs/screenshots/*.png
```

The script is [`app/scripts/screenshots/capture.ts`](../app/scripts/screenshots/capture.ts); set
`IMT_SHOT_PORT` to use another dev-server port.

## Testing and quality gates

Every change ships with tests, and every bug fix starts with a failing regression test. Before
committing, all of these must pass:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny --workspace check
(cd app && npm audit --omit=dev --audit-level=high && npx ng build && npx ng test --watch=false)
```

- **Coverage targets:** `imt-core` ≥ 99% lines (`cargo llvm-cov -p imt-core --summary-only`); Angular
  ≥ 80% lines overall and close to 100% for stores, services and pure utilities.
- **Engine end-to-end tests** run against [`samples/`](../samples/) (`crates/imt-core/tests`).
- **UI tests** are layered: Vitest + TestBed unit and component tests, and Playwright end-to-end flows
  against the browser build (Chromium and WebKit, with accessibility checks and a production-CSP check):
  `cd app && npm run e2e`.
- **Lint and i18n:** `cd app && npm run lint` (angular-eslint); `npm run i18n` extracts the
  translations and regenerates the pseudo-locale, and CI fails on duplicate message ids or an
  out-of-date pseudo-locale.
- **Desktop rules the build enforces:** child processes start through `imt_core::process::quiet_command`
  (no console window on Windows; `clippy.toml` disallows `Command::new`), and every Tauri command that
  builds a window is `async` (a window built from a synchronous command stays white on Windows;
  `app/src-tauri/tests/window_commands.rs`).
- **Commit messages** are checked against Conventional Commits (`node scripts/commit-lint.mjs`).
- **Real-app smoke tests** drive the built Tauri app with WebdriverIO and `tauri-driver`
  (`cd app && npm run e2e:tauri:build && npm run e2e:tauri`; Windows and Linux). Checks that need a
  real desktop are in [docs/testing-manual.md](testing-manual.md).
- **Snapshot tests (insta)** pin the records each importer produces for the sample files
  (`crates/imt-core/tests/snapshots`).
- **Property tests (proptest)** assert that parsers never panic and that invariants hold;
  **fuzzers (cargo-fuzz)** cover every parser and run nightly in CI. See [docs/fuzzing.md](fuzzing.md).
- **Benchmarks (criterion)** for the engine live in `crates/imt-core/benches` (`cargo bench -p imt-core`);
  `crates/imt-plugin-host` compares in-process and out-of-process work.
- **Mutation testing (cargo-mutants)** measures how many injected bugs the engine tests catch.

### UI responsiveness rules

- Dock panels are attached to Angular's change detection only while visible (`dock/angular-panel.ts`): a hidden
  tab is not checked and its effects do not run until it is shown. A root store that queries the engine for a view
  gates its effects on a `ConsumerGate` (`core/panel-visibility.ts`) that the views register with `gate.use()`.
- Pointer moves publish at most once per animation frame (`FrameThrottle` in `shared/throttle.ts`); lines that
  follow the pointer are drawn by a directive writing styles (`TimeCursorLine`), not by re-rendering the view.
- Bursty work waits until the burst settles (`Debouncer`): bucket counts on resize (`shared/bucket-count.ts`,
  also rounded to coarse steps; each chart sizes its own), grid pages while scrolling, the inspector's source context
  while the selection moves. Failed requests are retried by Retry, never by the next scroll.
- Mock backends are reached only through `browserMocks()` (`core/mock-registry.ts`); `mock-backends.ts` is loaded
  dynamically before bootstrapping outside Tauri, so the desktop app never loads mock code. Never import a mock
  backend statically from app code.

## CI/CD

Gitea Actions workflows live in [`.gitea/workflows/`](../.gitea/workflows/):

- **`build.yml`** runs on pushes to `main`, pull requests, `v*` tags, nightly (00:30 UTC, on `main`) and on
  demand (`workflow_dispatch`):
  1. *check* (Linux): `cargo test`, `fmt`, `clippy -D warnings`, `cargo-deny`, the commit-message
     check (pull requests), the Angular build with `npm audit`, the i18n extraction check, `ng lint`,
     unit tests with coverage, the Playwright end-to-end tests, and a check that the generated
     third-party notices are up to date.
  2. *real-app smoke* (Windows and Linux): builds the debug app and runs the `tauri-driver` tests.
  3. *bundle* (matrix), only for `v*` tags, the nightly run and manual runs (not for pushes or pull
     requests, so an installer breakage shows up within 24 hours): `.deb`/`.rpm` on `ubuntu-latest`,
     the NSIS `-setup.exe` on `windows-latest` and `.dmg` on `macos-latest`, uploaded as run artifacts.
  4. *release*: on a `v*` tag, creates a Gitea release with all installers, release notes
     (git-cliff), CycloneDX SBOMs for the Rust and UI dependencies and the signed updater manifest
     `latest.json` ([docs/releasing.md](releasing.md)).
- **`fuzz.yml`** fuzzes every parser nightly (and on demand) in four shards and uploads reproducers on failure.
- **`bench.yml`** runs the engine benchmarks on pushes to `main` that change `crates/imt-core`, nightly and
  on demand, and uploads the criterion report.
- **`mutants.yml`** runs cargo-mutants weekly over `imt-core` and, on pull requests, over the changed lines.

Runners are expected to carry the labels `ubuntu-latest`, `windows-latest` and `macos-latest`. Creating
a release needs a repository secret **`RELEASE_TOKEN`** (a Gitea access token with `write:repository`
scope). Dependencies are updated by Renovate, which also pins CI actions to commit digests.

## Project layout

```text
.
├── AGENTS.md                      engineering rules for contributors and coding agents
├── assets/icon/                   application icon (SVG and PNG)
├── app/
│   ├── src/app/                   Angular 22 UI: shell, grid, timeline, inspector, analysis, profile,
│   │                              markers, live, tools, projects, lifecycle, windowing, about, dock,
│   │                              core (backend + mock)
│   ├── src-tauri/                 Tauri 2 shell: commands, capabilities, per-window workspaces, window
│   │                              manager, tray, background tasks, single instance, notifications,
│   │                              lifecycle (safe mode, crash detection, sessions)
│   ├── e2e/                       Playwright end-to-end tests (mock backend)
│   ├── e2e-tauri/                 WebdriverIO + tauri-driver smoke tests of the real app
│   └── scripts/                   README screenshot generator, build info for the About dialog,
│                                  i18n/ (locale and pseudo-locale generator)
├── crates/
│   ├── imt-core/                  engine: importers, profile, query/ql, workspace (live tail), analysis,
│   │                              export, redaction, reports, dashboards, bundles, clock skew, config diff,
│   │                              templates, plugin runtime, event bus
│   ├── imt-api-perf/              plugin service API: performance counter sources (+ contract tests)
│   ├── imt-plugin-registry/       .reg importer
│   ├── imt-plugin-windows/        .evtx, msinfo32 .nfo, Performance Monitor (.csv/.blg), Process Monitor CSV and
│   │                              minidump import, Event Log follower, Windows marker kinds and annotators
│   ├── imt-plugin-tools/          tools plugin: Performance Monitor templates and logman control, process
│   │                              monitor, diagnostics collector, elevation, live counter source
│   ├── imt-profile/               profiling engine (pure): profile and heap model, every profiling view
│   ├── imt-plugin-profiling/      capture importers: nettrace, perf script, folded, pprof, Chrome, speedscope,
│   │                              heapsnapshot
│   ├── imt-plugin-web/            IIS/W3C, Apache/Nginx, HAR
│   ├── imt-plugin-formats/        log4j/log4net/NLog XML, SQL Server ERRORLOG, Docker json-file, Kubernetes CRI
│   ├── imt-plugin-otel/           OpenTelemetry OTLP/JSON
│   ├── imt-plugin-syslog/         syslog and journald
│   ├── imt-clr-format/            capture format and pipe protocol for the planned .NET profiler (pure library)
│   ├── imt-monitor/               process monitor with .NET runtime counters (library and `imt-monitor` CLI)
│   ├── imt-collect/               diagnostics collector (library and `imt-collect` CLI)
│   ├── imt-agent/                 headless scheduled/triggered collection (library, CLI and service)
│   ├── imt-pdf/                   PDF output of incident reports (pdf-writer + svg2pdf, embedded fonts)
│   └── imt-store-spike/           persistent-store spike (DuckDB, DataFusion); excluded from the workspace
├── docs/                          architecture, roadmap, backlog, plugins, fuzzing, ADRs, draw.io diagrams, screenshots
├── fuzz/                          cargo-fuzz targets
├── samples/                       generated sample logs, profiler captures (profiling/) and an example profile
├── scripts/
│   ├── notices/                   third-party notices generator (cargo-about + npm)
│   ├── release/                   updater manifest (latest.json) generator
│   └── commit-lint.mjs            Conventional Commits check
└── .gitea/workflows/              CI (build with release, fuzz, bench, mutants)
```
