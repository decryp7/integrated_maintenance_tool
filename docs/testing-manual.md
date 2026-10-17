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
