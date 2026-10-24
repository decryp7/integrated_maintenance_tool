# Manual tests

Some behaviour depends on the operating system and cannot be tested in the browser build
(Playwright and the mock backend) or with Tauri's mock runtime: the system tray, a second
launch handing over to the running instance, OS file associations and desktop notifications.
Run this checklist on each platform (Windows, macOS, Linux) before a release, against a
release build (`cd app && npx tauri build`) or `cargo build -p debrief-app` plus `npm run start`.

Write down the platform, build and anything that differs from the expected result.

## Single instance

| # | Steps | Expected |
|---|-------|----------|
| 1 | Start Debrief. From a terminal, run `debrief-app <folder with logs>`. | The second process exits within about a second. The running Debrief opens the folder in its window if that window is empty, otherwise per *Settings → Application → Window & tray → Open files from other apps in*. The Debrief log has "Second launch forwarded to the running instance". |
| 2 | Set *Open files from other apps in* to **A new window**, open some files, then run `debrief-app <file>` again. | A new window opens with the file and gets the focus. |
| 3 | Same with **The current window**. | The file is added to the window that last had the focus, and that window comes to the front. |
| 4 | Same with **Ask each time**. | The window comes to the front and asks "This window" or "New window". Both answers work. |
| 5 | Run `debrief-app` with no arguments while Debrief runs, with its window hidden in the tray. | The hidden window is shown and focused. |
| 6 | Run `debrief-app --new-window`. | The running instance opens an empty new window. |
| 7 | Run `debrief-app --new-instance`. | A second, independent Debrief process starts. |
| 8 | Run `debrief-app --safe-mode`, or set `DEBRIEF_SAFE_MODE=1`. | A separate safe-mode process starts with the safe-mode banner. The normal instance keeps running. |
| 9 | Run `debrief-app relative/path` from another folder. | The path is resolved against the folder `debrief-app` was run from. |
| 10 | Use *Restart* (after toggling a plugin) and *Restart as administrator* (Windows). | The new process becomes the running instance; later launches hand over to it. |

## Opening files from the OS

| # | Steps | Expected |
|---|-------|----------|
| 11 | macOS: double-click an `.debriefz` project (or "Open with → Debrief") while Debrief runs. | Follows *Open files from other apps in*. With Debrief not running, it starts and opens the project in the first window (no extra empty window). |
| 12 | Windows/Linux: double-click an `.debriefz` project while Debrief runs. | Same as 1 to 4 (it is a second launch). |
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
| 19 | Fresh settings: close the last window. | The "Close Debrief?" prompt asks Quit or Keep running in tray, with *Remember my choice* checked. |
| 20 | Choose *Keep running in tray* with *Remember*. Close again. | The window hides without asking. The tray icon stays, and live tail or the process monitor keeps running. |
| 21 | Tray → *Show Debrief* (or a window's name in the list). | That window is shown and focused. |
| 22 | Tray → *Recent projects* → a project. | It opens per *Open files from other apps in*. A project that no longer exists opens nothing. |
| 23 | Start live tail and the process monitor (and a PerfMon collector on Windows), then open Tray → *Background work*. | Each item is listed with a Stop entry, and the tooltip says "N background tasks running". Stop ends it, and the status bar's *Background work* popover and the window's Live button update. |
| 24 | Tray → *Capture diagnostic screenshot*. | The last-focused window comes to the front and an error snapshot is saved (see *View ▸ Diagnostics ▸ Error snapshots*) with a picture of the window's content (no title bar) as it was on screen, at most 1600 px wide (on a Retina Mac: the window's width in points). The window stays responsive meanwhile. |
| 25 | Tray → *Quit Debrief*. | Debrief exits, including hidden windows. The next start does not report a crash. |
| 26 | Set *Start in the system tray*, then restart. | Debrief starts with only the tray icon. Tray → *Show Debrief* shows the window. |
| 27 | macOS: switch the menu bar between light and dark. | The tray icon (a template image) follows. |
| 28 | Linux without an app indicator. | No tray icon. The log has "no system tray icon", and closing the last window quits Debrief. |

## Notifications

| # | Steps | Expected |
|---|-------|----------|
| 29 | Notifications **Alerts only** (default). Hide a window that has live tail on and a live alert rule, then make the rule fire. | One desktop notification "Debrief live alert". Another one appears at most every 30 s, and at most 5 appear in 5 minutes. |
| 30 | Same with the window visible and in front. | No notification; the window shows the alert. |
| 31 | Notifications **Alerts and finished work**. Start a large import, then switch to another window. | "Import finished" appears when it is done. |
| 32 | Notifications **Off**. | No notifications. |
| 33 | Kill Debrief (`kill -9` or Task Manager) with *Start in the system tray* on, then start it. | "Debrief did not close properly" appears. |

## Crash recovery with several windows

| # | Steps | Expected |
|---|-------|----------|
| 34 | Open three windows with different files, wait 10 s (autosave), close one, then kill Debrief. Start it again. | The main window shows the crash banner. *Restore last session* restores it and reopens the other open window with its files. The window closed before the crash is not restored. |
| 35 | Upgrade from a build that had a single `last-session.debriefproj`, after a crash. | The main window offers that session. |

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
| 44 | Close the main window while two panels are popped out. | The pop-outs close with it. Start Debrief again: they reopen where they were, or on the primary display when their monitor is gone. |

## Diagnostics: child-process logs, export without the UI, opening a report

Child processes' standard error and start/exit lines go to `<Debrief log folder>/children/<name>.log` (`ai-worker`,
`llama-server`, `plugin-host-<id>`, `plugin-<name>`; CLEF, at most 1 MB each, this session and the two before as
`<name>.1.log`, `<name>.2.log`). In debug builds `DEBRIEF_DATA_DIR=<root>` puts them under `<root>/logs/children`.

| # | Steps | Expected |
|---|-------|----------|
| 45 | Open the samples, then *Help ▸ Report a problem…*. | The file list shows `manifest.json`, `processes.json`, `session-recovery.json` and `logs/children/plugin-host-*.log` with summaries such as "Isolated plugin: 1 run, running". |
| 46 | Ask the AI assistant something, kill `llama-server` in Task Manager / `kill -9`, ask again, then report a problem. | `logs/children/llama-server.log` has a `Crashed: llama-server … ended with …` line and a second `Started llama-server (… run 2)`; `processes.json` shows 1 restart. No prompt text and no API key appear in any child log. |
| 47 | Close Debrief. Run `debrief-app --export-diagnostics` (debug: with `DEBRIEF_DATA_DIR` set) from a terminal. | No window opens; the zip's path is printed (in Documents, else Downloads) and the exit code is 0. |
| 48 | Run `debrief-app --export-diagnostics out.zip` twice, then with `--force`; then `debrief-app --export-diagnostics missing/out.zip` and `… out.txt`. | The second run fails (exit 1, "already exists; add --force"); `--force` replaces it; the missing folder and the wrong extension fail with a message saying what to fix. |
| 49 | Write `{ broken` into `config/settings.json`, then export again. | The export succeeds; stderr says `settings.json is not valid JSON …`; `system-info.json` lists it under `unreadable`. Put the file back. |
| 50 | Start in safe mode (`--safe-mode`); click *Save diagnostics…* on the banner. Kill Debrief, start it, click *Save diagnostics…* on the crash banner. | Both save a zip; `session-recovery.json` of the second says the previous run did not close properly. |
| 51 | Open `samples/diagnostics/debrief-diagnostics-sample.zip` (File ▸ Open, drag and drop, and `debrief-app <zip>`). | One lane per log (Debrief log, activity, `ai-worker`, `llama-server`, the order-audit plugin); crash (fatal) and start markers on the child lanes; the *Diagnostics report* panel opens with system info, the processes table (llama-server 1 restart, last exit code 3; the plugin quarantined), steps, breadcrumbs (click → the timeline zooms there) and the snapshot (click → large image). The JSON files are not events. |
| 52 | Edit the sample's `manifest.json` to `"schemaVersion": 99` (or break its JSON) and open it. | It opens as a plain archive with a notice saying why (newer format / not valid). |
| 53 | Restart Debrief with the sample open (session restore). | The panel shows the report again while the zip still exists. |

## Profile a .NET process (Windows, ADR 0019)

Build the fixture once: `dotnet build crates/debrief-plugin-dotnet/tests/fixtures/Busy -c Release -f net462` (and
`-p:PlatformTarget=x86 -o <dir>` for 32-bit, `-f net10.0` for .NET 10); run `Busy.exe 600` (it burns CPU, allocates,
throws, contends on a lock and sleeps for 10 minutes). the command palette's *Profile .NET process…* lists it.

| # | Steps | Expected |
|---|-------|----------|
| 54 | Run Debrief **not** elevated. Pick the net462 `Busy.exe`, *CPU and waits*, *Record*. | The process is marked "needs administrator"; the capture fails with the administrator message and *Restart as administrator*. Pick the net10.0 `Busy.exe`: it records (EventPipe, no elevation). |
| 55 | Elevated: record the net462 x64 `Busy.exe` for 10 s (*Stop after 10 seconds*). | The capture shows elapsed time and a growing size, ends by itself, opens on Hotspots: `Busy.Program.HotLoop` is hot; *Profile findings* shows a hot path, exceptions per second with the throw site `Busy.Program.Fail`, lock contention, and "Threads were blocked …" with the sleep and lock waits; the thread timeline shows wait events. |
| 56 | Same with the **x86** (`PlatformTarget=x86`) build. | The process shows `x86`; the capture opens with managed method names (no 0x… frames for `Busy.*`). |
| 57 | *Allocations* for 10 s. | *Profile findings* shows MB allocated with `System.Byte[]` as the top type (and the large object heap share); the call tree has the type as leaf below `AllocateBuffer`. |
| 58 | *Heap snapshot*, twice, a minute apart. | Each ends by itself within seconds and opens the Memory view on *Findings* (top retainer, `[Static variables]` keeping `Busy.Cache.Head` alive); the second also shows growth since the first; *Compare snapshots* works. |
| 59 | Start a CPU capture without a duration, then *Stop*; start another and *Cancel*; start a third and close the dialog, then stop it from the Jobs pane. | Stop keeps and opens the capture (method names collected first); Cancel deletes the file; the Jobs pane lists the running capture and its Cancel stops it. |
| 60 | Start a capture, then end `Busy.exe` in Task Manager. | The capture ends by itself with "The process exited during the recording" and still opens. |
| 61 | Record with PerfView or WPR (`wpr -start CPU -start DotNET`, …, `wpr -stop trace.etl`) while `Busy.exe` runs; drop `trace.etl` on Debrief; then *File ▸ Open process from .etl…* and pick another process. | Dropping opens the busiest .NET process (notes list the others); the dialog lists the trace's processes with samples, .NET first, and opens the chosen one. Native frames are named from symbols where PDBs are found (else `module+0x…`). |
| 62 | Kill Debrief (Task Manager) during an elevated capture, start it again and capture again; run `logman query -ets`. | No `Debrief-dotnet-*` session is left running after the second capture starts. |


## Profile a native process (Windows, ADR 0022)

Build the fixture once: `cargo build -p debrief-plugin-native --example native-busy` (32-bit:
`rustup target add i686-pc-windows-msvc`, then add `--target i686-pc-windows-msvc`); run
`target\debug\examples\native-busy.exe all 600` (a busy loop, four threads contending for a lock, a heap that
grows). the command palette's *Profile native process…* lists it.

| # | Steps | Expected |
|---|-------|----------|
| 63 | Debrief **not** elevated: pick `native-busy.exe`, *Sampling*, *Stop after 10 seconds*, *Record*. | The capture shows elapsed time and samples, ends by itself and opens on *Profile findings*: "Native hot spot: native_busy::native_busy_spin (main.rs:…)" and a wait hot spot in `native_busy_contend` "on a lock"; the call tree names frames with their source file; the notes say which modules had PDBs. `native-busy` keeps running normally. |
| 64 | Pick a service of another user (e.g. `svchost.exe`). | It is tagged *ETW only*; *Record* is disabled for *Sampling* with the hint to choose ETW. |
| 65 | Choose *ETW (administrator)*, *Heap allocations*, *Record* on `native-busy.exe`; accept the administrator prompt. | A second, elevated `debrief-plugins` process starts; the capture records, ends and opens with CPU samples, waits (with the readying thread) and *Native heap growth* under `native_busy_grow` ("… MB of native heap not freed"); Hotspots has an *Unfreed bytes* column. Declining the prompt instead shows the error with *Restart as administrator*. |
| 66 | Enable *Settings ▸ Native profiling ▸ Download symbols from Microsoft*, record again. | `ntdll.dll`/`kernelbase.dll` frames get function names; PDBs appear under `<app data>\symbols\<pdb>\<key>\`. With the setting off (the default) no request is made (check with a proxy log). |
| 67 | Start a sampling capture without a duration, *Stop*; start another and *Cancel*; start one, close the dialog and cancel it from the Jobs pane. | Stop keeps and opens it (symbols resolved first); Cancel deletes the file; Jobs lists the running capture. |
| 68 | During a capture end `native-busy.exe` in Task Manager. | The capture ends with "The process exited during the capture" and still opens. |
| 69 | `wpr -start CPU -start Heap -filemode` while `native-busy` runs, `wpr -stop trace.etl`; drop `trace.etl`; then *File ▸ Open process from .etl…* and pick another process. | Dropping opens the busiest .NET process, else the busiest process, with symbols (the notes list the others); the dialog lists the processes with samples, .NET first, and opens the chosen one. |
| 70 | Kill Debrief during an ETW capture, start it again and capture again; `logman query -ets`. | No `Debrief-native-*` session is left running after the second capture starts. |
| 71 | The 32-bit fixture (above), sampling and ETW. | The process shows `x86`; frames are named; the sampling notes mention WOW64. |

ETW automated tests (elevated shell): `set DEBRIEF_ETW_TESTS=1`, then
`cargo test -p debrief-plugin-native --test it real_process:: -- --ignored --nocapture`.

## Dumps, projects and third-party plugins

| # | Steps | Expected |
|---|-------|----------|
| 72 | Open `samples/dumps/OrderService.exe.4120.full.dmp` (File ▸ Open dump…, then again by drag and drop). | The Dump view opens with the crash cause and the likely culprit frame; threads, modules and findings are listed. Opening a file that is not a dump says so and names the file. |
| 73 | Open the three `samples/dumps/umdh/OrderService-4120-snap-*.txt` files with File ▸ Open UMDH snapshots…. | The UMDH heap growth view lists the growing stacks, the module and culprit-function groupings and the findings. |
| 74 | Windows: Tools ▸ Create dump of process… on a running process of your own (a small dump), into a folder you choose. Then try a protected process. | A `.dmp` appears in the folder and opens in the Dump view. The protected process fails with a message that offers *Restart as administrator*; nothing is injected into the target. |
| 75 | Save a project (`Ctrl/⌘+S`) of the samples as `.debriefz`, close Debrief, move the `samples` folder away, open the project. | It opens with its events, profile, notes and layout; search, the timeline and analyses work; the search box finds words in a moment, with a *+N* partial count. |
| 76 | Build a plugin with `cargo debrief-plugin new` and `cargo debrief-plugin package`; in the Plugins view choose *Install from file*. | An unsigned package is refused with a message about developer mode. With developer mode on (Settings ▸ Plugin trust) it installs after a warning that shows the publisher and the requested permissions. A revoked permission restarts the plugin with fewer rights. |
| 77 | With a plugin installed, kill its process from the Task Manager / `kill -9`, then use the plugin again. | The call fails with an error naming the plugin; the next use starts a new process; Debrief keeps running. |
| 78 | Settings ▸ Video: *Install* the FFmpeg runtime (Windows, Linux), or point it at your own `ffmpeg` (macOS, for example Homebrew's). Then open a short screen recording with File ▸ Open video…. | The runtime passes its self-test and shows its version; the Video view opens, the frame strip fills, the alignment names its evidence, and the video appears as its own lane. With the network off the install fails with a message, not a hang. |
| 79 | In the Video view, drop a frame, add the frame at the player position, then *Include the video in the project* and save. Move the original video and reopen the project. | The frames and notes survive; the included copy plays; a referenced (not included) video asks to be located and is accepted only with the same SHA-256. |
| 80 | Windows: Settings ▸ Symbols: turn symbol servers on with a Microsoft symbol server address, then open a dump whose modules have no local PDBs. | The Modules table shows a status per module (loaded, not on server, download failed...); downloaded PDBs land in the cache folder; with servers off nothing is requested. |
| 81 | Move a window to a second monitor, minimise another, then press `Ctrl/⌘+Shift+F12` in each. | The snapshot shows the window as it was on screen; a minimised or covered window on macOS gets the placeholder and "No screenshot: …" instead of a failure. |
| 82 | File ▸ Share redacted copy…, pick *Hash* and a few detectors, write the copy, then open it and search for an e-mail address from the original. | The preview counted what changes; the copy opens, finds no unredacted value, and says which sources (binary, CSV, XML) were left out. |

## The AI's browser (real Chromium, ADR 0043)

Needs Edge, Chrome or Chromium (or the download in Settings ▸ Web browser) and a local model.

| # | Steps | Expected |
|---|---|---|
| 83 | Settings ▸ Web browser on a machine with no Chromium-based browser. | The panel says none was found and offers *Download Chromium* (about 200 MB). Start a research: it fails with a message pointing to Settings. |
| 84 | Press *Download Chromium*, wait, then *Remove downloaded Chromium*. | Status goes downloading, installed (pinned version) and back to none; a download whose checksum does not match is refused and nothing is installed. |
| 85 | In a chat set *Browser* to *Agent* and ask it to look something up. | A Chromium window opens next to Debrief with Chrome's banner "controlled by automated test software" and a window frame in Debrief's accent colour (a browser version that ignores it is fine). Debrief shows the status chip with the activity, the site and *Stop*. Nothing is added to the pages themselves. |
| 86 | Press *Stop* on the chip during a step. | The Chromium window closes at once, the chip clears, the chat says the browser was stopped, and no Chromium process is left (`ps`/Task Manager). |
| 87 | Research a topic with *Research this* on an event. | The window stays out of sight; the chip shows "Reading n of m results"; the research log lists every page opened or skipped. |
| 88 | Research when the engine shows a consent page or a CAPTCHA. | The window comes to the front, the chip says it is waiting for you; after you finish the check the research continues and the window goes away. |
| 89 | *Search the web for this error* and a link card's *Open*. | The pages open in your default browser, not in Debrief. |
| 90 | Quit Debrief while a research is running. | No Chromium process remains. |

## Smoke tests of the real app (WebdriverIO + tauri-driver)

`app/e2e-tauri/` holds a few automated smoke tests of the real app: the Rust engine, the Tauri shell and the
production UI build, no mock backend. WebdriverIO v9 (local runner, mocha) talks to `tauri-driver`, which starts
`debrief-app` with `samples/serilog-clef` on its command line and forwards to the platform's WebDriver: Microsoft
Edge WebDriver (`msedgedriver`) for WebView2 on Windows, `WebKitWebDriver` for WebKitGTK on Linux. macOS has no
WKWebView driver, so they do not run there. One app session per spec file, in order; each test has a two-minute
budget and waits for conditions (no sleeps):

1. `01-start`: the main window shows with no crash/safe-mode banner and no `role=alert`; Help ▸ About Debrief shows the version from `tauri.conf.json`.
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
npm run e2e:tauri:build      # sidecars (debrief-ai-worker, debrief-plugins, debrief-store-worker, with libduckdb) + tauri build --debug --no-bundle, in <target>/debug
npm run e2e:tauri            # Windows
xvfb-run -a npm run e2e:tauri   # Linux (CI also wraps it in dbus-run-session)
```

The binary is looked up in `$CARGO_TARGET_DIR/debug` (else `target/debug`); `DEBRIEF_E2E_APP`, `TAURI_DRIVER` and
`DEBRIEF_E2E_NATIVE_DRIVER` override the binary, tauri-driver and the native driver. Don't run it while Debrief is open:
the single-instance lock would hand the launch to the running app.

Debug-build overrides (read only when `debug_assertions` is on; release builds never look at them; a
malformed value stops the app with exit code 2 naming the variable), see `app/src-tauri/src/dev_overrides.rs`:

- `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`: merged into every window's WebView2 arguments through the API (wry's
  defaults kept, `--disable-features` lists combined). msedgedriver uses it to turn on remote debugging; WebView2
  ignores the variable itself for elevated hosts, so this is what lets the tests run from an elevated terminal
  and on Windows CI runners.
- `DEBRIEF_DATA_DIR=<absolute root>`: the app's data, config and log folders and the WebView2 user data folder
  move under `<root>/data`, `/config`, `/logs` and `/local` (`app/src-tauri/src/app_dirs.rs`).
- `DEBRIEF_OPEN_PATHS`: absolute paths (separated like `PATH`, at most 16) opened at start-up. msedgedriver turns
  launch arguments into switches on Windows, so the samples path is passed this way there.

Isolation: every run gets a fresh temporary folder as `DEBRIEF_DATA_DIR` (on Linux the XDG folders point into it
too), removed when the run ends; `DEBRIEF_E2E_KEEP_DATA=1` keeps it for inspection. A failed test saves a screenshot
and the page HTML to `app/e2e-tauri/.artifacts/`. The single-instance lock is not covered by `DEBRIEF_DATA_DIR`:
close any running Debrief first.

CI: the `tauri-smoke` job in `.gitea/workflows/build.yml` runs this on `windows-latest` and `ubuntu-latest`;
both legs are required, and failure artifacts are uploaded.