# Development

How to build, test and release Debrief, and where things live in the repository. Engineering rules for
contributors are in [AGENTS.md](../AGENTS.md); the design is in [architecture.md](architecture.md).

Machine setup shared by every Paddock app (tools, build cache, worktrees, test runs): [paddock/docs/dev-setup.md](https://dev.decryptology.net/decryp7/paddock/src/branch/main/docs/dev-setup.md). This page adds only what is specific to this app.

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
- **libduckdb 1.5.6** (the store, ADR 0014), the official prebuilt library, never compiled (the `bundled` feature
  needs several GB of RAM). The pins (version, assets, SHA-256) live in
  `crates/debrief-store-duck/scripts/fetch-libduckdb.sh`. Every cargo build of the workspace needs
  `DUCKDB_LIB_DIR`/`DUCKDB_INCLUDE_DIR`. Once per checkout (and again after `cargo clean` or a version bump), on any
  OS:

  ```sh
  cargo xtask libduckdb
  ```

  It fetches the pinned archive for the host, checks its SHA-256, unpacks it into
  `<target>/libduckdb/<version>/<asset>/` (`<target>` is `CARGO_TARGET_DIR` or `target/`) and writes both variables
  into `.cargo/libduckdb.toml`, which the committed `.cargo/config.toml` includes. That file is machine-local and
  git-ignored (it names the host's asset, and an absolute path when the target folder is outside the checkout); the
  committed config holds only the optional `include`. Plain `cargo build` / `cargo test` / `cargo run` then work
  in any shell, and a `DUCKDB_LIB_DIR` set in the environment still wins (CI). Run time needs no `PATH` /
  `LD_LIBRARY_PATH` either: debrief-store-duck's build script stages the runtime library (`duckdb.dll`,
  `libduckdb.so`, `libduckdb.dylib`) into its `OUT_DIR` and names it as a search path, which Cargo puts on the
  library path of every test and binary it runs (it does so only for folders inside `target/<profile>/`, so the cache
  folder itself would not do). Delete `.cargo/libduckdb.toml` to stop using it.

  Per shell instead (CI does this):

  ```sh
  eval "$(bash crates/debrief-store-duck/scripts/fetch-libduckdb.sh "$HOME/.cache/debrief-libduckdb")"   # sh, CI
  node app/scripts/libduckdb.mjs                                                                  # any OS: prints the lines to run
  ```

  `node app/scripts/libduckdb.mjs [cache-dir]` works on Windows, which has no bash: it reads the same pins from
  the script, checks the SHA-256 before unpacking, and prints PowerShell lines (`$env:DUCKDB_LIB_DIR = ...`, and the
  library folder added to `PATH`) or `export` lines for sh. `npm run tauri build` and
  `node app/scripts/build-sidecars.mjs` fetch the library themselves when `DUCKDB_LIB_DIR` is not set. The sidecar
  build copies the library into the installer's `lib/` resource folder and also next to the built store worker, so an
  app run straight from the target folder (development, the end-to-end builds) finds it. On macOS and Linux the
  worker finds it by run path (`crates/debrief-store-worker/build/rpath.rs`), because a signed, notarized app ignores
  `DYLD_LIBRARY_PATH`: next to itself (`@executable_path`, `$ORIGIN`) and in the installed resource folder
  (`@executable_path/../Resources/lib` in `Debrief.app`, `$ORIGIN/../lib/Debrief/lib` from `/usr/bin` in the deb and rpm).
  The official `libduckdb.dylib` is installed as `@rpath/libduckdb.dylib` and `libduckdb.so` has the SONAME
  `libduckdb.so`, so no `install_name_tool` / `patchelf` step is needed; the bundle job checks the packaged worker's
  run paths. Debrief also passes the library folder in `PATH` / `LD_LIBRARY_PATH` / `DYLD_LIBRARY_PATH` as a fallback
  (the only way on Windows).

### Develop

```sh
cd app
npm ci
npm start             # UI only, in a browser, against the built-in mock backend (http://localhost:4200)
npx tauri dev         # the desktop app with the real engine (starts `npm run start` for you)
```

```sh
cargo test -p debrief-core        # engine tests, including end-to-end runs over /samples
```

Regenerate the samples with `cargo xtask gen-samples` (do not edit them by hand). It runs
`samples/generate.py` (with `python3`, `python` or `py -3`, whichever works) and then the Rust generators
of the profiler captures and the crash dump (`DEBRIEF_REGENERATE_SAMPLES=1` tests in `debrief-plugin-profiling`
and `debrief-plugin-windows`); `cargo xtask libduckdb` sets up the database library (above); `cargo xtask help` lists
the tasks.
[`samples/README.md`](../samples/README.md) lists every sample, its format and the views it shows.

### Visual Studio Code (Windows and macOS)

Open the **repository root** (not `app/`) and accept the recommended extensions
(`.vscode/extensions.json`: rust-analyzer, Tauri, C/C++ for debugging on Windows, CodeLLDB for
macOS and Linux, Angular, ESLint, Prettier, Playwright, draw.io, ...).

1. Install the prerequisites above. On macOS: `xcode-select --install`, Rust from
   [rustup.rs](https://rustup.rs), Node.js 24 (for example `brew install node@24`). On Windows: the
   Rust MSVC toolchain, the "Desktop development with C++" workload (Build Tools) and Node.js 24.
2. *Terminal ▸ Run Task… ▸ Debrief: install UI dependencies* (runs `npm ci` in `app/`).
3. Run or debug:
   - *Debrief: run app (tauri dev)* — the desktop app with hot reload.
   - **F5** with *Debug Debrief app (Windows)* or *Debug Debrief app (macOS / Linux)* — starts the UI dev
     server, builds the debug binary and attaches the native debugger (breakpoints in Rust).
     The *with the samples* variants open `samples/serilog-clef` on start.
   - *Debug UI in the browser (mock backend)* — the Angular UI alone, with Chrome DevTools-style
     breakpoints in TypeScript.
4. **Ctrl/Cmd+Shift+B** builds the debug binary; other tasks run the checks (`Rust: clippy`,
   `Rust: test workspace`, `UI: lint`, `UI: unit tests`, `UI: e2e`, ...) or all of them in order
   (*Debrief: all quality gates*). `Debrief: build installers (release)` runs `tauri build`.

rust-analyzer checks into its own target folder, so it never blocks a build in the terminal.

Debrief's windows switch off browser behaviour (`app/src-tauri/src/native_feel.rs` for WebView2's
browser keys, `app/src/app/core/native-feel.ts` for the rest). Debug builds and `ng serve` keep the
developer tools (`F12`, `Ctrl+Shift+I`, or Shift+right-click for the browser menu with *Inspect*)
and hard reload (`Ctrl+Shift+R`, `Ctrl+F5`); `F5` and `Ctrl+R` are blocked everywhere. The UI is not
text-selectable by default (ADR 0028): content users copy opts in with the `debrief-selectable` class (text fields,
grid cells, `pre`/`code`, `role=alert` and `role=note` select by nature), and right-clicking selected text opens
Debrief's own menu with *Copy*.

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
| macOS | `bundle/dmg/*.dmg` (and `bundle/macos/Debrief.app`) |

Build the UI with `npm run build` (or `npm start`), not plain `npx ng build`: the npm scripts run
[`app/scripts/build-info.mjs`](../app/scripts/build-info.mjs), which passes the commit and build date to
the About dialog (`--define DEBRIEF_BUILD_INFO`); a plain `ng build` shows them as "unknown". `tauri build`
and `tauri dev` use the npm scripts.

### Third-party notices

`THIRD_PARTY_NOTICES.md` (bundled with the app and shown in *About Debrief*) and the texts compiled into
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
backend at 1440×900 and device pixel ratio 2, in both colour schemes (`name.png` is dark, `name-light.png` light). Views that
are missing are skipped with a warning.

```sh
cd app
npm run e2e:install     # once: Playwright browsers
npm run screenshots     # writes docs/screenshots/*.png
```

The script is [`app/scripts/screenshots/capture.ts`](../app/scripts/screenshots/capture.ts); set
`DEBRIEF_SHOT_PORT` to use another dev-server port, `DEBRIEF_SHOT_OUT` to write elsewhere, `DEBRIEF_SHOT_THEMES=dark` (or `light`)
for one scheme and `DEBRIEF_SHOT_ONLY=home,jobs` for named shots. The marketing images (`docs/images/debrief-*.svg` and PNGs) are
generated from these screenshots by Paddock's `docs/images/src/app-poster.mjs` (`PADDOCK_APPS=<parent of this checkout, named debrief>`).

### Diagrams and What's new

The `.drawio` sources are generated by the scripts in `docs/diagrams/src` (they need the Paddock checkout next to this repo): change a
generator, run `docs/diagrams/src/generate.sh`, then `scripts/export-diagrams.sh` (draw.io desktop; on macOS it is found at
`/Applications/draw.io.app`) and commit the `.drawio` files and the SVGs in `docs/diagrams/svg/` together. The 2x PNGs it writes
under `docs/diagrams/png/` are git-ignored. `CHANGELOG.md` is curated by hand and feeds
*Help ▸ What's new*: after editing it run `npm run whats-new` in `app/` (`npm start` and `npm run build` do it too) and
commit the regenerated `app/src/app/whats-new/changelog.generated.ts`; a node test fails when it is stale.

### App icon

The icon is drawn as SVG in [`app/src-tauri/icons-src/`](../app/src-tauri/icons-src/): a blue tile
(Debrief's accent) with two log lines, a timeline pulse and an amber marker.

| File | Used for |
| --- | --- |
| `debrief-icon.svg` | Master, 1024×1024: every bundle icon from 44 px up (`.icns`, `.ico` 48–256 px, PNGs, Store logos) |
| `debrief-icon-small.svg` | The same motif snapped to a 16 px grid with heavier strokes: the 16/24/32 px `.ico` entries, `32x32.png`, `Square30x30Logo.png` and the in-app logo |
| `debrief-tray-template.svg` | Black-on-transparent glyph for the macOS menu bar (`tray-template.png`, 44×44); other platforms use the window icon |

The installed icon set under `app/src-tauri/icons/` comes from the suite's shared icon masters (`suite-icons/debrief`),
installed by the rename script; the SVGs here are the reference copies. Check any change at 16 px on a light and a dark
taskbar. The in-app logo (title bar, About) is the inline component
[`app/src/app/shared/app-logo.ts`](../app/src/app/shared/app-logo.ts), which copies the small variant's
geometry: change both together.

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

- **Coverage targets:** `debrief-core` ≥ 99% lines (`cargo llvm-cov -p debrief-core --summary-only`); Angular
  ≥ 80% lines overall and close to 100% for stores, services and pure utilities.
- **Engine end-to-end tests** run against [`samples/`](../samples/) (`crates/debrief-core/tests`).
- **UI tests** are layered: Vitest + TestBed unit and component tests, and Playwright end-to-end flows
  against the browser build (Chromium and WebKit, with accessibility checks and a production-CSP check):
  `cd app && npm run e2e`.
- **Lint and i18n:** `cd app && npm run lint` (angular-eslint); `npm run i18n` extracts the
  translations and regenerates the pseudo-locale, and CI fails on duplicate message ids or an
  out-of-date pseudo-locale.
- **Desktop rules the build enforces:** child processes start through `debrief_core::process::quiet_command`
  (no console window on Windows; `clippy.toml` disallows `Command::new`), and every Tauri command that
  builds a window is `async` (a window built from a synchronous command stays white on Windows;
  `app/src-tauri/tests/window_commands.rs`).
- **Commit messages** are checked against Conventional Commits (`node scripts/commit-lint.mjs`).
- **Real-app smoke tests** drive the built Tauri app with WebdriverIO and `tauri-driver`
  (`cd app && npm run e2e:tauri:build && npm run e2e:tauri`; Windows and Linux). Checks that need a
  real desktop are in [docs/testing-manual.md](testing-manual.md).
- **Snapshot tests (insta)** pin the records each importer produces for the sample files
  (`crates/debrief-core/tests/snapshots`).
- **Property tests (proptest)** assert that parsers never panic and that invariants hold;
  **fuzzers (cargo-fuzz)** cover every parser and run nightly in CI. See [docs/fuzzing.md](fuzzing.md).
- **Benchmarks (criterion)** for the engine live in `crates/debrief-core/benches` (`cargo bench -p debrief-core`);
  the plugin tests over the real IPC are in `crates/debrief-plugins/tests`.
- **Mutation testing (cargo-mutants)** measures how many injected bugs the engine tests catch.
- **Live search test** of AI research against the real engines: see below. Nightly and manual CI runs only.

### Live search test

AI research reads the results page of the user's search engine in a real Chromium (ADR 0043). Search engines change
their pages without notice, so a live test searches once with every built-in engine (Google, DuckDuckGo, Bing) through
the real page scripts and the real Rust rules (`research::search`, `research::read_results`, `research::open_result`),
using the same `ChromiumPage` research uses. It is an ignored test of `debrief-app` (`app/src-tauri/src/browser/live.rs`)
and needs an installed Edge, Chrome or Chromium and the network. An engine passes with at least 3 result links and its
first result opening; a recognised consent, human-check or redirect page is a warning (the page comes to the front for
a user in the real app); anything else fails, with the page's host and path (never the query string), title,
readyState, DOM quiet time, link counts, whether the results region was found and a screenshot.

```sh
DEBRIEF_LIVE_SEARCH=1 DEBRIEF_E2E_LIVE_SEARCH_OUT=/tmp/live-search cargo test -p debrief-app --lib live_search -- --ignored --nocapture
```

The report (`live-search.json`) and screenshots land in the folder `DEBRIEF_E2E_LIVE_SEARCH_OUT` names.
`DEBRIEF_E2E_LIVE_SEARCH_QUERY` replaces the default query (`System.TimeoutException`) and `DEBRIEF_E2E_LIVE_SEARCH_ENGINES`
limits the engines (`google,bing`); use neutral text only, never anything from real logs. CI runs it on the nightly
schedule and on manual runs, never on pushes or pull requests: engines rate-limit automated traffic, so keep runs rare.

### Allocator and memory budgets

The binaries that parse and analyse (the app, `debrief-store-worker`, `debrief-plugins`) use **mimalloc v3** through the
`debrief-alloc` crate: each declares `debrief_alloc::MiMalloc` as its global allocator and calls `debrief_alloc::tune()` first
thing in `main`. mimalloc v3 keeps freed memory for a second before returning it to the OS, which added 150-200 MB to
the worker's import peak, so `tune()` sets v2's purge timing (10 ms). The options are set in code because the
workers start with a cleared environment (no `MIMALLOC_*` variables). Measurements: ADR 0014's performance notes.
`debrief_alloc::release()` gives free but still resident memory back at once; the store calls it when an import, an index build or
an analysis ends, so the worker does not idle at the size of its last burst.

Budgets (ADR 0014; tests measure peak resident memory and fail above them): the app process stays flat, under
250 MB whatever the data size; the store worker stays under 300 MB idle or querying and under 768 MB while importing;
the store worker is stopped at a **2 GiB hard cap**. Plugin processes have a cap in their manifest (1 GiB by default,
at most 4 GiB). Every cap is enforced the same way on every platform: the host (`watch_memory` in
`crates/debrief-ipc/src/process.rs`) watches the child's resident memory and stops it over the limit with a message that
says so. There are no memory rlimits: they count address space that allocators reserve up front.

The query budget (300 MB) is measured on a release build, because unoptimised code keeps about 50 MB more resident: the
`analysis_memory` test skips itself in debug builds and CI runs it with `cargo test --release` on Linux.

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

### Error snapshot capture

The picture in an error snapshot is taken by the platform webview, not drawn by the page (ADR 0034,
`app/src-tauri/src/window_capture/`): WebView2 `CapturePreview` on Windows, `WKWebView takeSnapshot` on macOS,
WebKitGTK `get_snapshot` on Linux. The platform callback only copies bytes (macOS: an uncompressed BMP); decoding,
scaling and PNG/JPEG encoding run on a blocking worker (`window_capture::bound`, unit-tested on every OS) and the
result is at most 1600 px wide (on macOS at most the window's width in points, so a Retina capture is stored at
logical size) and 3 MiB. Any failure (unsupported platform, the webview refusing, the 10 s timeout, undecodable or
oversized data) saves the snapshot with a placeholder picture and "No screenshot: …". On macOS WebKit draws nothing
for a window that is not on screen (minimised, covered, on another desktop), so such a window gets no picture. The platform halves need the
real webview, so check them by hand on each OS: Tray → *Capture diagnostic screenshot*, then *View ▸ Diagnostics ▸ Error
snapshots* (manual test 24 in [testing-manual.md](testing-manual.md)).

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
- **`fuzz.yml`** fuzzes every parser nightly (and on demand) in five shards and uploads reproducers on failure.
- **`bench.yml`** runs the engine benchmarks on pushes to `main` that change `crates/debrief-core`, nightly and
  on demand, and uploads the criterion report.
- **`mutants.yml`** runs cargo-mutants weekly over `debrief-core` and, on pull requests, over the changed lines.

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
│   ├── debrief-core/                  engine: importers, profile, query/ql, ports (the Store traits), workspace (reference
│   │                              store), live, home, search_all, analysis, known issues, incident, playbooks, export,
│   │                              redaction, reports, dashboards, bundles, clock skew, config diff, templates,
│   │                              plugin runtime, event bus
│   ├── debrief-batch/                 the event batch: stable ids, the fixed Arrow schema, a validated builder
│   ├── debrief-alloc/                 the global allocator (mimalloc v3) and its start-up tuning
│   ├── debrief-store-duck/            the store (ADR 0014): Parquet segments, DuckDB catalog and SQL, Tantivy search
│   ├── debrief-store-client/          Debrief's side of the store worker: protocol, supervision, `RemoteStore` (no DuckDB),
│   │                              the port table (`port_table.rs`, ADR 0042)
│   ├── debrief-store-worker/          the store worker process hosting every window's store (`dispatch.rs`)
│   ├── debrief-ipc/                   the one IPC (ADR 0017): frames, supervisor, launcher, sandbox, memory watcher
│   ├── debrief-plugin-api/            the contract plugins build against (never `debrief-core`); the SDK crate
│   ├── debrief-plugin-host/           starts plugin processes and registers their contributions in the engine
│   ├── debrief-plugins/               the binary that serves every built-in plugin (`--plugin <id>`)
│   ├── debrief-plugin-package/        the signed `.debriefplugin` package format (ADR 0027)
│   ├── debrief-plugin-test/           test harness for plugin authors (the real launcher and sandbox)
│   ├── debrief-plugin-e2e/            end-to-end tests of a third-party plugin from package to import
│   ├── cargo-debrief-plugin/          `cargo debrief-plugin`: new, test, package, sign
│   ├── debrief-plugin-registry/       .reg importer
│   ├── debrief-plugin-windows/        .evtx, msinfo32 .nfo, Performance Monitor (.csv/.blg), Process Monitor CSV and
│   │                              minidump import, Event Log follower, Windows marker kinds and annotators
│   ├── debrief-plugin-tools/          tools plugin: Performance Monitor templates and logman control, process
│   │                              monitor, Event Log follower, diagnostics collector, elevation
│   ├── debrief-plugin-web/            IIS/W3C, Apache/Nginx, HAR
│   ├── debrief-plugin-formats/        log4j/log4net/NLog XML, SQL Server ERRORLOG, Docker json-file, Kubernetes CRI
│   ├── debrief-plugin-otel/           OpenTelemetry OTLP/JSON
│   ├── debrief-plugin-syslog/         syslog and journald
│   ├── debrief-profile/               profiling engine (pure): profile and heap model, every profiling view
│   ├── debrief-plugin-profiling/      capture importers: nettrace, perf script, folded, pprof, Chrome, speedscope,
│   │                              heapsnapshot
│   ├── debrief-plugin-dotnet/         .NET profiling by ETW and EventPipe (ADR 0019)
│   ├── debrief-plugin-native/         native profiling by ETW or suspend-and-sample (ADR 0022)
│   ├── debrief-etw/, debrief-symbols/     ETW decoders and the NetTrace writer; PDBs by identity, symbol servers, cache (ADR 0036)
│   ├── debrief-video/                 video sources: FFmpeg runtime, sandboxed runs, frame selection, alignment (ADR 0033)
│   ├── debrief-dump/                  dump and UMDH engine (ADR 0023); `debrief-plugin-dumps` holds the readers
│   ├── debrief-notes/                 notes anchored to events, times, sources and image regions (ADR 0021)
│   ├── debrief-ai/, debrief-ai-worker/    local AI layer (assistant, browser rules, knowledge, vision) and its worker
│   ├── debrief-monitor/               process monitor with .NET runtime counters (library and `debrief-monitor` CLI)
│   ├── debrief-collect/               diagnostics collector (library and `debrief-collect` CLI)
│   ├── debrief-agent/                 headless scheduled/triggered collection (library, CLI and service)
│   └── debrief-pdf/                   PDF output of incident reports (pdf-writer + svg2pdf, embedded fonts)
├── examples/plugins/              one example plugin per kind (importer, transform, tool, collector, annotator)
├── xtask/                         `cargo xtask` (gen-samples and other tasks)
├── docs/                          architecture, roadmap, plugins, fuzzing, ADRs, draw.io diagrams, screenshots
├── fuzz/                          cargo-fuzz targets
├── samples/                       generated sample logs, profiler captures (profiling/) and an example profile
├── scripts/
│   ├── notices/                   third-party notices generator (cargo-about + npm)
│   ├── release/                   updater manifest (latest.json) generator
│   └── commit-lint.mjs            Conventional Commits check
└── .gitea/workflows/              CI (build with release, fuzz, bench, mutants)
```
