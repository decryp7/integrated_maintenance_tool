# Changelog

What changed for people who use Debrief, one section per release. This file is curated by hand (the in-app "What's new"
panel reads it at build time); `git cliff` lists every commit if you need the full history.

## Unreleased

### Changed

- **What's new:** The release notes are now a document in the window instead of a dialog. After an update it opens once in the background (it does not take the front or the keyboard focus), and one notification, "Updated to … · See what's new", brings it forward. *Help ▸ What's new* opens it at any time; close it like any other document. It shows the notes of the version that is running and does not open again on later starts of the same version
- **Notifications:** A notification's button that opens a view or a window (See what's new, Show in Jobs, View, Open the assistant, Open Settings…) now slides the Notifications pane in, and one that opens a view moves the keyboard focus to it; buttons that act in place (Try again, Reset, Later) keep the pane open

## 1.1.0 (2026-10-24)

### Added

- **AI:** When Pitwall runs on this computer, Debrief can use it as its AI engine: choose Use Pitwall in the AI set-up, approve the six-digit code in Pitwall, and one model serves all your apps without a second download. Settings ▸ AI ▸ AI engine chooses Automatic, Pitwall or Built-in; if Pitwall stops, Debrief says so and offers the built-in engine instead of switching by itself

### Changed

- **Notifications:** One failure is now one notification. A repeated message updates the one already on screen (with a count) instead of adding another, errors stay until you dismiss them or the problem is gone, and technical details are folded behind Details. Job results, update offers, live alerts, error snapshots and errors all appear in the same place
- **Services:** When the log store, the AI worker, a plugin, the research browser or a download stops, the status bar shows one chip with a plain explanation, the next step, the technical details and a Try again / Restart / Allow again / Install button. Every window shows the same state, including a window you open while a service is down
- **Memory limits:** When a memory limit stops the database worker, the AI worker or a plugin, the status chip says so once, with Try again and Raise the cap… (it opens the setting with a suggested value)
- **Panels:** A panel that needs the log store shows "Waiting for the log store" instead of its own red error, and loads by itself when the store is back; the assistant, the research browser, models and runtimes wait for their own service the same way

- **Layout:** Panes lay out by the room each one needs: what does not fit is tabbed or auto-hidden instead of squeezed (one notice with Undo), and returns when the window is large enough again; sizes you set by dragging are kept
- **Research browser:** When the research browser cannot start, the status chip says so with Open Settings… (Settings ▸ Web browser) or, when no browser is installed, Install

### Fixed

- **Start-up:** A window shows "Starting Debrief…" until it is ready; a window that does not finish starting is reloaded once (keeping your query, filters and selection), then Debrief asks whether to reload, restart in Safe Mode or keep waiting
- **Research browser:** Closing the research browser's window, quitting it or a crash no longer stops the next research; the browser opens on the display of the Debrief window that started it
- **Dialogs:** Every dialog is now one of four kinds (decision, form, viewer, tool) that decides its size, buttons and keys: result windows open at least 680 x 560 and only grow, questions are exactly as big as their text, and every window has a Close or Cancel button that stays in view (What's new no longer scrolls its Got it button out of sight)
- **Dialogs:** A dialog window may call only the commands it needs, so a bug in one window can no longer do what only the main window should; Restart to apply in Settings and the plugin windows, and Restart as administrator in the profiling and diagnostics windows, now restart through the window that opened them and keep its session
- **Dialogs:** Open recent project hands the choice back to the window you were working in, which asks about unsaved changes and opens it; What's new always shows the notes of the version that is running
- **Dialogs:** With several displays, a dialog always opens on the display of the window that opened it, at a size that fits that display; a remembered position is kept relative to that window and only used when the dialog then stays on its display, and short dialogs no longer jump in height after opening

## 1.0.2 (2026-10-23)

### Added

- **Search:** The search box suggests "Did you mean CompanyA.Graphic.Builder.exe (ProcessName)?" when your words match a value in your logs, even with different separators, case or small typos
- **Tool panes:** Every tool pane has an auto-hide (pin) button in its header; right-click it to choose the left, right or bottom edge, remembered per pane. The tooltip shows your current shortcut
- **Command line:** Import progress by phase on a terminal, `--quiet` to turn it off, and Ctrl+C cancels the import and exits with code 130
- **Assistant:** It finds events and values by name, trying wider matches when the exact words are not there and saying which step found them
- **Jobs:** Every long task shows its phase (finding files, checking, importing, compacting, indexing, saving) with its own progress, in the toast, the Jobs pane and the status bar; opening a dump, making a redacted copy, installing the browser and downloading an update are jobs with byte progress and Cancel

### Fixed

- **Dialogs:** Dialog windows have no title-bar buttons and close from their content (Close or Cancel; `Esc` and `Cmd+W` act as Cancel, and "Close Debrief?" ignores them); the note, knowledge, parser and format editors, the incident summary, Search the web and Report export are tools you can keep beside the work
- **Dialogs:** Windows that fill in as they load or stream (Explain event, proposals, the incident summary, research, What's new) open at a usable size and only grow
- **AI setup:** Pick the model, press Download and set up, and the window closes; the download runs in the background (Jobs, status bar, the assistant pane) and a message says when the assistant is ready or why it failed, with Retry
- **About:** It shows the real version, fills its window with the notices scrolling inside and a pinned footer, and loads the 15,000-line third-party notices only when asked (no longer part of the app's JavaScript)
- **Research:** The research browser opens as a normal window and closes itself when research ends, unless you used it; it also closes when you quit
- **Research:** Closing the research browser window yourself no longer stops research from working: the next research opens a new window, and a research whose window you close stops with a plain message and Retry
- **AI setup:** Model and runtime downloads are checked while they are written, so the progress no longer restarts for a second pass
- **AI models:** Whether a model fits is judged from its weights plus the cache for the real context size, against the memory that is free now
- **Search:** A phrase such as "graphic builder" finds Graphic.Builder whatever the separators or capitalisation
- **Home:** Home reflows in a narrow pane, and long paths and file names shorten in the middle with the full value in a tooltip
- **Import:** Opening a very large folder reports finding, checking and importing with live counts from the first moment and walks the folders in parallel
- **Reload:** A cancelled reload says "Reload cancelled — previous data kept"
- **Monitor:** Linux threads are no longer counted as processes

## 1.0.1 (2026-10-23)

### Fixed

- **macOS:** The app opens after download instead of being reported as damaged

## 1.0.0 (2026-10-23)

### Added

- **Debrief:** The product has a new name, Debrief: the app, the command line (`debrief`), the project and plugin file formats, the SDK and the crates all carry the new name
- **Command line:** The `debrief` tool imports, queries, analyses, watches and reports headlessly with the same engine as the app, for scripts and CI gates
- **Outliers:** Error outliers show which hosts, builds or other fields are over-represented among errors compared with the rest of the same time span
- **Patterns:** Every message pattern has a trend sparkline on one shared scale, and a Values list shows what was behind each placeholder
- **Import:** Import only a time window (for example one hour of a 20 GB folder); everything outside it is skipped
- **Watched folders:** Debrief imports new files as they settle in folders you watch and announces known issues when they appear
- **Lenses:** Save a view as a lens (levels, search, filters, extra columns, highlights and the first chart) and switch between lenses from one menu in the filter row
- **Design:** A design pass over every screen: themed selects, switches and number fields, one tabs style, plugins as one table with a side panel, a one-line timeline legend with a popover and folded marker counts, hover row actions, calm empty states, a Home that fits a 1440 x 900 window and a colour-blind-safe pair for comparing A and B
- **Shortcuts:** Edit every key binding in Settings > Keyboard, with conflict warnings; the F1 cheat sheet shows your bindings
- **Themes:** A high contrast theme (WCAG AAA text, strong outlines and focus rings) that is chosen automatically when your system asks for more contrast; "Follow system" is the new default theme choice
- **What's new:** This panel, shown once after an update and always under Help > What's new
- **Parsers:** Generate a parser with AI, check it against your file and keep it in the parser library
- **Store:** A Store pane with reports, maintenance and a read-only SQL console
- **Settings:** Performance settings for memory caps, CPU use and presets, with suggested values
- **Home:** A dashboard of everything you opened: live tiles, the history of events by level (click or drag to set the time range), a health check with a button for each finding, recent activity and quick actions; it replaces the Overview view
- **Timeline:** A minimap above the histogram shows the whole data range; drag to pan, drag an edge to zoom, double-click to fit
- **Search:** Search everything (Ctrl/⌘+Shift+F) looks in events, sources, fields and their values, marker kinds, exception groups, notes and knowledge files, and opens the match
- **Perspectives:** Named window arrangements (Investigate, Profiling, Live, Diagnostics and your own) switched with Ctrl/⌘+1 to 9; every view opens in its own area and never over the timeline or the grid; Alt+Left and Alt+Right walk back and forward through events, ranges and views
- **Video:** Open a screen recording, let Debrief pick the frames that matter, align them to the logs and have the local model describe them; each video is a source with its own lane
- **Investigate:** Declare known issues in your knowledge files, build an incident summary report in one step and follow investigation playbooks (four templates included)
- **Encodings:** Debrief detects the text encoding of every source (Shift_JIS, GBK, EUC-KR, Big5, windows-125x and more), shows its confidence and lets you choose another one
- **Paste:** Edit > Paste as log adds pasted text, or text dropped on the window, to the project as a log
- **Share:** File > Share redacted copy writes a project copy built from redacted text, with a live preview of what changes and what is left out
- **Dumps:** Symbols are matched by the PDB's identity, not its name, and can be downloaded from symbol servers you enable; each module says why it has no names
- **Store:** Compact, rebuild the index and clear cached analyses from the Store pane; the SQL console keeps event ids, opens rows in the grid and exports results to CSV
- **Plugins:** One Plugins view with Installed and Health tabs; a Parsers view for the parser library
- **Live tail:** Alerts, the tail status, clock skew and the last event of a file are handled by the store, so they work on large projects too

### Changed

- **Menus:** Theme choices live under View > Theme; live tail moved to View > Live
- **Palette:** The command palette matches word starts and lists recent commands first
- **Layout:** The Focused and Full layouts are gone; perspectives take their place
- **Search:** One search field takes words or a query, and the engine tells which
- **Error snapshots:** The picture comes from the system's web view and no longer freezes the window

### Fixed

- **Time:** Dates without a year are read near the file's own date
- **Memory:** Memory is given back to the system after an import, an index build and each analysis
