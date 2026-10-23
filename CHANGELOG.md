# Changelog

What changed for people who use Debrief, one section per release. This file is curated by hand (the in-app "What's new"
panel reads it at build time); `git cliff` lists every commit if you need the full history.

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
