# Manual tests

Some behaviour depends on the operating system and cannot be tested in the browser build
(Playwright and the mock backend) or with Tauri's mock runtime: the system tray, a second
launch handing over to the running instance, OS file associations and desktop notifications.
Run this checklist on each platform (Windows, macOS, Linux) before a release, against a
release build (`cd app && npx tauri build`) or `cargo build -p imt-app` plus `npm run start`.

Write down the platform, build and anything that differs from the expected result.

## Single instance

| # | Steps | Expected |
|---|-------|----------|
| 1 | Start IMT. From a terminal, run `imt-app <folder with logs>`. | The second process exits within about a second. The running IMT opens the folder in its window if that window is empty, otherwise per *Settings → Application → Window & tray → Open files from other apps in*. The IMT log has "Second launch forwarded to the running instance". |
| 2 | Set *Open files from other apps in* to **A new window**, open some files, then run `imt-app <file>` again. | A new window opens with the file and gets the focus. |
| 3 | Same with **The current window**. | The file is added to the window that last had the focus, and that window comes to the front. |
| 4 | Same with **Ask each time**. | The window comes to the front and asks "This window" or "New window". Both answers work. |
| 5 | Run `imt-app` with no arguments while IMT runs, with its window hidden in the tray. | The hidden window is shown and focused. |
| 6 | Run `imt-app --new-window`. | The running instance opens an empty new window. |
| 7 | Run `imt-app --new-instance`. | A second, independent IMT process starts. |
| 8 | Run `imt-app --safe-mode`, or set `IMT_SAFE_MODE=1`. | A separate safe-mode process starts with the safe-mode banner. The normal instance keeps running. |
| 9 | Run `imt-app relative/path` from another folder. | The path is resolved against the folder `imt-app` was run from. |
| 10 | Use *Restart* (after toggling a plugin) and *Restart as administrator* (Windows). | The new process becomes the running instance; later launches hand over to it. |

## Opening files from the OS

| # | Steps | Expected |
|---|-------|----------|
| 11 | macOS: double-click an `.imtproj` (or "Open with → IMT") while IMT runs. | Follows *Open files from other apps in*. With IMT not running, it starts and opens the project in the first window (no extra empty window). |
| 12 | Windows/Linux: double-click an `.imtproj` while IMT runs. | Same as 1 to 4 (it is a second launch). |
| 13 | macOS: hide every window in the tray, then click the Dock icon. | A window is shown again. |

## Windows

| # | Steps | Expected |
|---|-------|----------|
| 14 | Press Ctrl/Cmd+Shift+N, use *File → New window* and the tray's *New window*. | Each opens an empty window with its own workspace. |
| 15 | Open different files in two windows, run queries, turn on live tail in one. | The windows don't affect each other. Live tail updates only its own window. |
| 16 | Open a project in a window. | The OS window title and the tray's window list show the project name. |
| 17 | Close one of two windows. | Only that window closes. Its memory is freed; check with Activity Monitor or Task Manager after closing a large workspace. |
| 18 | Move and resize a window, quit, start again. | The main window comes back at the same size and position. |

## Closing and the tray

| # | Steps | Expected |
|---|-------|----------|
| 19 | Fresh settings: close the last window. | The "Close IMT?" prompt asks Quit or Keep running in tray, with *Remember my choice* checked. |
| 20 | Choose *Keep running in tray* with *Remember*. Close again. | The window hides without asking. The tray icon stays, and live tail or the process monitor keeps running. |
| 21 | Tray → *Show IMT* (or a window's name in the list). | That window is shown and focused. |
| 22 | Tray → *Recent projects* → a project. | It opens per *Open files from other apps in*. A project that no longer exists opens nothing. |
| 23 | Start live tail and the process monitor (and a PerfMon collector on Windows), then open Tray → *Background work*. | Each item is listed with a Stop entry, and the tooltip says "N background tasks running". Stop ends it, and the status bar's *Background work* popover and the window's Live button update. |
| 24 | Tray → *Capture diagnostic screenshot*. | The last-focused window comes to the front and an error snapshot is saved (see *Tools → Show error snapshots*). |
| 25 | Tray → *Quit IMT*. | IMT exits, including hidden windows. The next start does not report a crash. |
| 26 | Set *Start in the system tray*, then restart. | IMT starts with only the tray icon. Tray → *Show IMT* shows the window. |
| 27 | macOS: switch the menu bar between light and dark. | The tray icon (a template image) follows. |
| 28 | Linux without an app indicator. | No tray icon. The log has "no system tray icon", and closing the last window quits IMT. |

## Notifications

| # | Steps | Expected |
|---|-------|----------|
| 29 | Notifications **Alerts only** (default). Hide a window that has live tail on and a live alert rule, then make the rule fire. | One desktop notification "IMT live alert". Another one appears at most every 30 s, and at most 5 appear in 5 minutes. |
| 30 | Same with the window visible and in front. | No notification; the window shows the alert. |
| 31 | Notifications **Alerts and finished work**. Start a large import, then switch to another window. | "Import finished" appears when it is done. |
| 32 | Notifications **Off**. | No notifications. |
| 33 | Kill IMT (`kill -9` or Task Manager) with *Start in the system tray* on, then start it. | "IMT did not close properly" appears. |

## Crash recovery with several windows

| # | Steps | Expected |
|---|-------|----------|
| 34 | Open three windows with different files, wait 10 s (autosave), close one, then kill IMT. Start it again. | The main window shows the crash banner. *Restore last session* restores it and reopens the other open window with its files. The window closed before the crash is not restored. |
| 35 | Upgrade from a build that had a single `last-session.imtproj`, after a crash. | The main window offers that session. |

## Docking, pop-out windows and file drops

Tabs and groups are dragged with pointer events (dockview `dndStrategy: 'pointer'`), not HTML5 drag-and-drop.
The windows keep Tauri's native file-drop handler (`dragDropEnabled: true`), because only it gives the dropped
files' paths, which the engine needs. HTML5 drops in WebView2 and WKWebView give `File` objects without paths.
On Windows the native handler takes over HTML5 drag-and-drop in WebView2, and a native drag inside the page
never reaches the page. Pointer events are not affected, so the tab drags are the same on all platforms. Run
these checks on Windows (WebView2) and macOS (WKWebView). Rows 36 and 39 were checked on macOS with `cliclick`
when the feature was added.

| # | Steps | Expected |
|---|-------|----------|
| 36 | Drag the Metrics tab onto the right arrow of the compass in the middle of the Events panel. | Guides show while dragging: a compass over the panel under the pointer and one arrow at the middle of each window edge. A dashed preview shows the result. Metrics docks to the right of Events. |
| 37 | Drag a tab onto the left edge arrow. | It docks along the whole left edge. |
| 38 | Drag a tab and release it away from any arrow. | Nothing moves. |
| 39 | Drag a tab out of the window (to the desktop or another monitor) and release it. | The panel opens in its own OS window under the pointer, with the title "<panel> — <project>". On Windows check that releasing over the desktop, the taskbar and another app each pops it out. |
| 40 | Drop log files from Explorer or Finder onto the main window, then onto a pop-out window. | Both open the files in the main window's workspace. |
| 41 | Drag a pop-out window by its title bar over the main window and rest it on a guide for half a second. | The main window shows its guides under the pointer. The panel docks at the guide and the pop-out closes. |
| 42 | Use the pop-out's "Drag to dock" handle to drag the window over a guide and release it. | Same as 41. On Windows the release docks it at once; on macOS it docks once the window has rested on the guide. |
| 43 | Shift+Alt+Enter with the default "Full screen does" setting, then press Escape, then Shift+Alt+Enter again. | The window goes OS full screen with the chrome and tool windows hidden. Escape does not leave full screen. The shortcut restores both. |
| 44 | Close the main window while two panels are popped out. | The pop-outs close with it. Start IMT again: they reopen where they were, or on the primary display when their monitor is gone. |

## Diagnostics: child-process logs, export without the UI, opening a report

Child processes' standard error and start/exit lines go to `<IMT log folder>/children/<name>.log` (`ai-worker`,
`llama-server`, `plugin-host-<id>`, `plugin-<name>`; CLEF, at most 1 MB each, this session and the two before as
`<name>.1.log`, `<name>.2.log`). In debug builds `IMT_DATA_DIR=<root>` puts them under `<root>/logs/children`.

| # | Steps | Expected |
|---|-------|----------|
| 45 | Turn plugin isolation on, open the samples, then *Help ▸ Report a problem…*. | The file list shows `manifest.json`, `processes.json`, `session-recovery.json` and `logs/children/plugin-host-*.log` with summaries such as "Isolated plugin: 1 run, running". |
| 46 | Ask the AI assistant something, kill `llama-server` in Task Manager / `kill -9`, ask again, then report a problem. | `logs/children/llama-server.log` has a `Crashed: llama-server … ended with …` line and a second `Started llama-server (… run 2)`; `processes.json` shows 1 restart. No prompt text and no API key appear in any child log. |
| 47 | Close IMT. Run `imt-app --export-diagnostics` (debug: with `IMT_DATA_DIR` set) from a terminal. | No window opens; the zip's path is printed (in Documents, else Downloads) and the exit code is 0. |
| 48 | Run `imt-app --export-diagnostics out.zip` twice, then with `--force`; then `imt-app --export-diagnostics missing/out.zip` and `… out.txt`. | The second run fails (exit 1, "already exists; add --force"); `--force` replaces it; the missing folder and the wrong extension fail with a message saying what to fix. |
| 49 | Write `{ broken` into `config/settings.json`, then export again. | The export succeeds; stderr says `settings.json is not valid JSON …`; `system-info.json` lists it under `unreadable`. Put the file back. |
| 50 | Start in safe mode (`--safe-mode`); click *Save diagnostics…* on the banner. Kill IMT, start it, click *Save diagnostics…* on the crash banner. | Both save a zip; `session-recovery.json` of the second says the previous run did not close properly. |
| 51 | Open `samples/diagnostics/imt-diagnostics-sample.zip` (File ▸ Open, drag and drop, and `imt-app <zip>`). | One lane per log (IMT log, activity, `ai-worker`, `llama-server`, the order-audit plugin); crash (fatal) and start markers on the child lanes; the *Diagnostics report* panel opens with system info, the processes table (llama-server 1 restart, last exit code 3; the plugin quarantined), steps, breadcrumbs (click → the timeline zooms there) and the snapshot (click → large image). The JSON files are not events. |
| 52 | Edit the sample's `manifest.json` to `"schemaVersion": 99` (or break its JSON) and open it. | It opens as a plain archive with a notice saying why (newer format / not valid). |
| 53 | Restart IMT with the sample open (session restore). | The panel shows the report again while the zip still exists. |

## Profile a .NET process (Windows, ADR 0019)

Build the fixture once: `dotnet build crates/imt-plugin-dotnet/tests/fixtures/Busy -c Release -f net462` (and
`-p:PlatformTarget=x86 -o <dir>` for 32-bit, `-f net10.0` for .NET 10); run `Busy.exe 600` (it burns CPU, allocates,
throws, contends on a lock and sleeps for 10 minutes). *Tools ▸ Profile .NET process…* lists it.

| # | Steps | Expected |
|---|-------|----------|
| 54 | Run IMT **not** elevated. Pick the net462 `Busy.exe`, *CPU and waits*, *Record*. | The process is marked "needs administrator"; the capture fails with the administrator message and *Restart as administrator*. Pick the net10.0 `Busy.exe`: it records (EventPipe, no elevation). |
| 55 | Elevated: record the net462 x64 `Busy.exe` for 10 s (*Stop after 10 seconds*). | The capture shows elapsed time and a growing size, ends by itself, opens on Hotspots: `Busy.Program.HotLoop` is hot; *Profile findings* shows a hot path, exceptions per second with the throw site `Busy.Program.Fail`, lock contention, and "Threads were blocked …" with the sleep and lock waits; the thread timeline shows wait events. |
| 56 | Same with the **x86** (`PlatformTarget=x86`) build. | The process shows `x86`; the capture opens with managed method names (no 0x… frames for `Busy.*`). |
| 57 | *Allocations* for 10 s. | *Profile findings* shows MB allocated with `System.Byte[]` as the top type (and the large object heap share); the call tree has the type as leaf below `AllocateBuffer`. |
| 58 | *Heap snapshot*, twice, a minute apart. | Each ends by itself within seconds and opens the Memory view on *Findings* (top retainer, `[Static variables]` keeping `Busy.Cache.Head` alive); the second also shows growth since the first; *Compare snapshots* works. |
| 59 | Start a CPU capture without a duration, then *Stop*; start another and *Cancel*; start a third and close the dialog, then stop it from the Jobs pane. | Stop keeps and opens the capture (method names collected first); Cancel deletes the file; the Jobs pane lists the running capture and its Cancel stops it. |
| 60 | Start a capture, then end `Busy.exe` in Task Manager. | The capture ends by itself with "The process exited during the recording" and still opens. |
| 61 | Record with PerfView or WPR (`wpr -start CPU -start DotNET`, …, `wpr -stop trace.etl`) while `Busy.exe` runs; drop `trace.etl` on IMT; then *File ▸ Open process from .etl…* and pick another process. | Dropping opens the busiest .NET process (notes list the others); the dialog lists the trace's processes with samples, .NET first, and opens the chosen one. Native frames are named from symbols where PDBs are found (else `module+0x…`). |
| 62 | Kill IMT (Task Manager) during an elevated capture, start it again and capture again; run `logman query -ets`. | No `IMT-dotnet-*` session is left running after the second capture starts. |


## Profile a native process (Windows, ADR 0022)

Build the fixture once: `cargo build -p imt-plugin-native --example native-busy` (32-bit:
`rustup target add i686-pc-windows-msvc`, then add `--target i686-pc-windows-msvc`); run
`target\debug\examples\native-busy.exe all 600` (a busy loop, four threads contending for a lock, a heap that
grows). *Tools ▸ Profile native process…* lists it.

| # | Steps | Expected |
|---|-------|----------|
| 63 | IMT **not** elevated: pick `native-busy.exe`, *Sampling*, *Stop after 10 seconds*, *Record*. | The capture shows elapsed time and samples, ends by itself and opens on *Profile findings*: "Native hot spot: native_busy::native_busy_spin (main.rs:…)" and a wait hot spot in `native_busy_contend` "on a lock"; the call tree names frames with their source file; the notes say which modules had PDBs. `native-busy` keeps running normally. |
| 64 | Pick a service of another user (e.g. `svchost.exe`). | It is tagged *ETW only*; *Record* is disabled for *Sampling* with the hint to choose ETW. |
| 65 | Choose *ETW (administrator)*, *Heap allocations*, *Record* on `native-busy.exe`; accept the administrator prompt. | A second, elevated `imt-plugins` process starts; the capture records, ends and opens with CPU samples, waits (with the readying thread) and *Native heap growth* under `native_busy_grow` ("… MB of native heap not freed"); Hotspots has an *Unfreed bytes* column. Declining the prompt instead shows the error with *Restart as administrator*. |
| 66 | Enable *Settings ▸ Native profiling ▸ Download symbols from Microsoft*, record again. | `ntdll.dll`/`kernelbase.dll` frames get function names; PDBs appear under `<app data>\symbols\<pdb>\<key>\`. With the setting off (the default) no request is made (check with a proxy log). |
| 67 | Start a sampling capture without a duration, *Stop*; start another and *Cancel*; start one, close the dialog and cancel it from the Jobs pane. | Stop keeps and opens it (symbols resolved first); Cancel deletes the file; Jobs lists the running capture. |
| 68 | During a capture end `native-busy.exe` in Task Manager. | The capture ends with "The process exited during the capture" and still opens. |
| 69 | `wpr -start CPU -start Heap -filemode` while `native-busy` runs, `wpr -stop trace.etl`; drop `trace.etl`; then *File ▸ Open process from .etl…* and pick another process. | Dropping opens the busiest .NET process, else the busiest process, with symbols (the notes list the others); the dialog lists the processes with samples, .NET first, and opens the chosen one. |
| 70 | Kill IMT during an ETW capture, start it again and capture again; `logman query -ets`. | No `IMT-native-*` session is left running after the second capture starts. |
| 71 | The 32-bit fixture (above), sampling and ETW. | The process shows `x86`; frames are named; the sampling notes mention WOW64. |

ETW automated tests (elevated shell): `set IMT_ETW_TESTS=1`, then
`cargo test -p imt-plugin-native --test real_process -- --ignored --nocapture`.

## Smoke tests of the real app (WebdriverIO + tauri-driver)

`app/e2e-tauri/` holds a few automated smoke tests of the real app: the Rust engine, the Tauri shell and the
production UI build, no mock backend. WebdriverIO v9 (local runner, mocha) talks to `tauri-driver`, which starts
`imt-app` with `samples/serilog-clef` on its command line and forwards to the platform's WebDriver: Microsoft
Edge WebDriver (`msedgedriver`) for WebView2 on Windows, `WebKitWebDriver` for WebKitGTK on Linux. macOS has no
WKWebView driver, so they do not run there. One app session per spec file, in order; each test has a two-minute
budget and waits for conditions (no sleeps):

1. `01-start`: the main window shows with no crash/safe-mode banner and no `role=alert`; Help ▸ About IMT shows the version from `tauri.conf.json`.
2. `02-open-samples`: the samples import; the grid has rows, the status bar an event count, the timeline lanes.
3. `03-query`: `level >= Error` narrows the grid to ERR/FTL rows; Clear restores the total.
4. `04-settings-tools`: Settings switches the theme (and puts the user's theme back); View ▸ Tools ▸ Process Monitor opens the view.
5. `05-quit`: File ▸ Close window ▸ Quit exits the process and removes `running.marker`; the next start shows no crash banner.

Every spec ends with a clean quit (the config's `after` hook), so a later spec never starts after a "crash".

One-time setup:

```sh
cargo install tauri-driver --locked
cd app && npm ci
node e2e-tauri/scripts/fetch-msedgedriver.mjs   # Windows: msedgedriver matching the installed WebView2
sudo apt-get install webkit2gtk-driver xvfb      # Linux
```

The Edge driver must match the WebView2 runtime version exactly. The script reads it from the registry
(EdgeUpdate client `{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`), downloads
`https://msedgedriver.microsoft.com/<version>/edgedriver_win64.zip` into `app/e2e-tauri/.drivers/` and skips the
download when that version is already there. Run it again after WebView2 updates (a mismatch fails the session
with "session not created"); pass a version to pin one.

Build and run (from `app/`):

```sh
npm run e2e:tauri:build      # sidecars (imt-ai-worker, imt-plugins) + tauri build --debug --no-bundle, in <target>/debug
npm run e2e:tauri            # Windows
xvfb-run -a npm run e2e:tauri   # Linux (CI also wraps it in dbus-run-session)
```

The binary is looked up in `$CARGO_TARGET_DIR/debug` (else `target/debug`); `IMT_E2E_APP`, `TAURI_DRIVER` and
`IMT_E2E_NATIVE_DRIVER` override the binary, tauri-driver and the native driver. Don't run it while IMT is open:
the single-instance lock would hand the launch to the running app.

Debug-build overrides (read only when `debug_assertions` is on; release builds never look at them; a
malformed value stops the app with exit code 2 naming the variable), see `app/src-tauri/src/dev_overrides.rs`:

- `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`: merged into every window's WebView2 arguments through the API (wry's
  defaults kept, `--disable-features` lists combined). msedgedriver uses it to turn on remote debugging; WebView2
  ignores the variable itself for elevated hosts, so this is what lets the tests run from an elevated terminal
  and on Windows CI runners.
- `IMT_DATA_DIR=<absolute root>`: the app's data, config and log folders and the WebView2 user data folder
  move under `<root>/data`, `/config`, `/logs` and `/local` (`app/src-tauri/src/app_dirs.rs`).
- `IMT_OPEN_PATHS`: absolute paths (separated like `PATH`, at most 16) opened at start-up. msedgedriver turns
  launch arguments into switches on Windows, so the samples path is passed this way there.

Isolation: every run gets a fresh temporary folder as `IMT_DATA_DIR` (on Linux the XDG folders point into it
too), removed when the run ends; `IMT_E2E_KEEP_DATA=1` keeps it for inspection. A failed test saves a screenshot
and the page HTML to `app/e2e-tauri/.artifacts/`. The single-instance lock is not covered by `IMT_DATA_DIR`:
close any running IMT first.

CI: the `tauri-smoke` job in `.gitea/workflows/build.yml` runs this on `windows-latest` and `ubuntu-latest`;
both legs are required, and failure artifacts are uploaded.