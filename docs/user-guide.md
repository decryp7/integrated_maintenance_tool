# User guide

Reference for everyday use: which files Debrief reads, how to write a profile, the query language, the
internal web browser, the keyboard shortcuts and the command line. For what Debrief can do, see [features.md](features.md); for a
first walk-through, see the two-minute tour in the [README](../README.md#quick-start).

- [Glossary](#glossary)
- [First run and layout](#first-run-and-layout)
- [Projects](#projects)
- [Appearance](#appearance)
- [Supported inputs](#supported-inputs)
- [Profiles](#profiles)
- [Query language](#query-language)
- [The Store pane and the SQL console](#the-store-pane-and-the-sql-console)
- [The AI's browser](#the-ais-browser)
- [Knowledge files](#knowledge-files-teach-the-ai-your-systems), [known issues](#known-issues), the [incident summary](#incident-summary-report) and [playbooks](#investigation-playbooks)
- [AI runtime and model updates](#ai-runtime-and-model-updates)
- [Disk usage and project stores](#disk-usage-and-project-stores)
- [Keyboard shortcuts](#keyboard-shortcuts) and [changing them](#change-a-shortcut)
- [Video sources](#video-sources)
- [Dump symbols](#dump-symbols)
- [Share a redacted copy](#share-a-redacted-copy)
- [Command line](#command-line)
- [Troubleshooting](#troubleshooting)

## Glossary

One word, one meaning: menus, the palette, Settings and these docs use these terms.

| Term | Means | Not to be confused with |
|---|---|---|
| **Profile** | The parsing profile: how Debrief reads a set of logs (parsers, fields, timestamps, markers, pipeline). *Profile editor*, Settings ▸ Parsers. | A performance capture. |
| **Performance capture** | A profiler recording (`.nettrace`, `.speedscope.json`, heap snapshot…) opened with *File ▸ Open performance capture…*; listed in *Profiling ▸ Captures*. | A profile. "Profile .NET process" (a verb) records one. |
| **Project** | One `.debriefz` file holding the opened events, the profile, filters, notes and reports (*File ▸ Save project*). | The folder the logs came from. |
| **Store** | The engine's event database (segments, fields, index) behind a window. | Disk usage. |
| **Disk usage** | What Debrief keeps on disk besides projects: bundle copies, web data, caches, the folders of project stores (Settings ▸ Disk usage). | The store. |
| **Compare time ranges** | Two time ranges of the same events side by side (*Analyze ▸ Compare time ranges*). | *Compare configuration files* (two config files) and comparing two performance captures. |
| **Region** | A part of the window `F6` moves between: activity bar, side bar, filter bar, a panel group, status bar. | A panel (one tab in a group). |

## First run and layout

With nothing open, Debrief shows the welcome page: **Open folder…**, **Open files…** or **Try a demo** (or drop
files anywhere). After opening data you get the **Investigate** perspective: the timeline over the events grid, with
the inspector sliding out from the right when you select an event.

- **Filter bar** (one row): search (`.*` regex, `Aa` match case), level chips, the time range (click to
  reset a zoom), UTC/local, *+ Add filter*, *Clear all* and **View options** with *Group by*, *Marked only*
  and *Always show start / stop*.
- **Search** finds words first: every word you type, in any order, the last one as the start of a word
  (`timeout exc` finds `System.TimeoutException`; `camelCase`, dots, underscores and digits split words). A
  moment later Debrief adds events that contain the text only inside a word; *+N* in the search box says how many,
  and those rows are marked *partial*. `.*` (regex) and `Aa` (match case) search the text as written.
- **Timeline header**: the health check's headline, e.g. "⚠ 2 crashes · 287 errors · 1 went silent —
  Review". *N crashes* jumps to the next crash, *N errors* shows the errors and fatal events, *N went silent*
  shows the next silence, *Review* opens Home. The line is hidden when nothing is notable.
- **Legend**: only the marker kinds that occur in the data, on one line; *+N more* shows the rest.
- **One search field** for words and queries: plain words search the messages as you type; text with query
  structure (`level >= warning`, `| stats count() by source`, `exists(TraceId)`) is recognised by the engine as a
  query and run with `Enter`. The field grows to several lines while you type (`Shift+Enter` for a new line);
  history, *Ask AI* and *Save* sit inside it. `Ctrl/⌘+/` focuses it.
- **Sidebar**: the Explorer; the **Fields** section starts collapsed (click its header; Debrief remembers).
- **Set up analysis…** for a folder of several files is offered in Notifications (the bell).

- **Home** (*View ▸ Home*) is the dashboard of everything opened: live tiles for events, sources, errors and warnings with
  their trend, crashes, anomalies, exception groups, time span, store size and memory; the history of events by level
  (click a bar or drag across it to set the time range); the health check, most severe first, each finding with a button
  to jump to the time, show the events or fix the cause; recent activity and quick actions. The counts appear at once and
  the slower analyses fill in after. A checkbox on Home opens it after every import.
- **Timeline minimap**: a thin strip above the histogram shows the whole data range with a window for what is visible. Drag
  the window to pan, drag an edge to zoom, click the strip to centre the window, double-click to fit; with the window
  focused the arrow keys pan and `Home`/`End` go to the ends.
- **Search everything** (`Ctrl/⌘+Shift+F`) searches events, sources, fields and values, marker kinds, exception groups,
  notes and knowledge files at once and opens the match you choose.

### Auto-hide

Every tool pane (Inspector, Metrics, Problems and the like; not the timeline, events grid or other documents) has an
**Auto-hide** button (a pin) in its header, next to the panel menu. Click it, or press the shortcut (`Ctrl+Alt+H` by default; the tooltip shows your current one), and the pane
collapses into a tab on the edge it sits nearest to; hover the tab or click it to slide the pane out, and press its
**Pin** button to dock it again. Panes only auto-hide to the **left, right or bottom**, never the top. To choose the
edge, right-click the Auto-hide button (or press `Shift+F10` / the Menu key on it) and pick *Auto-hide to the left /
right / bottom*. Debrief remembers the edge per pane and saves it with the layout.

### Perspectives

A perspective is a named window arrangement. **Investigate** (the default) is the timeline over the events;
**Profiling** opens with the captures list (open a capture or profile a process); **Live** adds tail status and
alerts below the events; **Diagnostics** adds problems, jobs, error snapshots and the store. Switch
with the activity bar's *Perspectives* button, *Window ▸ Perspectives* or `Ctrl/⌘+1…9`; each perspective
remembers how you left it, and *Reset perspective* rebuilds it. *Save current as perspective…* adds your own, and
*Manage perspectives…* renames, reorders and deletes them. Opening a performance capture switches to Profiling
once (the message offers *Undo*).

Views open in their home area: documents beside the timeline and the grid, analysis views (anomalies, patterns,
exceptions, Moment, compare, ...) in one group below the grid (the least recently used tab closes beyond eight),
side tools on the right, and Jobs, Notifications and plugin panels at the bottom edge. Nothing opens over the
timeline or the grid.

Back and forward (`Alt+←` / `Alt+→`, or the mouse's back and forward buttons) walk through the events you
selected, the time ranges you zoomed to and the views you brought forward. `Ctrl/⌘+Z` also takes a zoom back.
Drag an event row onto the Notes panel to write a note on it. In a narrow window the Explorer closes by itself and
the filter bar folds the time range and time zone into a ⋯ menu.

## Dialogs are windows

Every dialog in Debrief (Settings, the AI set-up and model manager, About, Export, Share a redacted copy,
pickers such as the time range or the text encoding, ...) opens as a window of its own, with the operating system's title
bar, owned by the Debrief window you opened it from and centred on it, always on the display that window is on. Resize it or
move it beside the window: Debrief remembers its size and its place relative to the window, and uses that place next time only
when the whole dialog then fits on the window's display (otherwise it is centred on the window again). Yes/no questions ("Delete this source?") use the system's message box.

Dialog windows have **no close, minimise or zoom buttons** in the title bar: each one closes from its own content, with a
Close or Cancel button (or its answer). You can still move and resize it, and Debrief remembers its size and place.

Every dialog is one of four kinds, and the kind decides how it looks and behaves:

- **Decisions** (confirmations, unsaved changes, "Close Debrief?", pickers that feed the next step such as the time range or
  the text encoding, plugin permissions) are *modal*: the Debrief window behind them is dimmed and does not react until you
  answer. The window is exactly as big as its question, with no scroll bar. `Esc` and `Cmd+W` / `Ctrl+W` are their Cancel;
  the few that have no Cancel ("Close Debrief?", a choice between two ways forward) ignore them.
- **Forms** (rules created from an event such as Drop, Mark, Mask, Extract and Infer, Paste as log, Add tile, the redaction
  settings) have a comfortable fixed size, a scrolling body and buttons that stay in view. Forms that act on a snapshot of
  the selected event, and the first-start AI set-up choice, are modal; forms you keep beside the work (Export, Share a redacted copy, Report a problem, the alert
  rules, the clock adjustment, the editors for notes, knowledge, parsers and formats, the AI query and the profiling
  launchers) are *modeless*: the Debrief window stays usable, and opening the same dialog again brings the open window to
  the front. A modal never waits for long work; anything that takes more than a moment runs as a job (see Jobs).
- **Viewers** (Explain event and the other AI results, the incident summary, About, Keyboard shortcuts, Check
  for updates) open at least 680 x 560 and only ever grow while their content streams in; they never shrink under you.
  They are modeless and have a Close button that is always in view.
- **Tools** (Settings, the AI model manager, Research) are large modeless windows that remember their size.

`Esc` and `Cmd+W` / `Ctrl+W` close a modeless dialog (Close) and cancel a modal one (Cancel). A window that offers no Close or
Cancel of its own (still loading, an error) gets one from Debrief, so no dialog can be left without a way out.

**First-start AI set-up.** The set-up window is only a choice: pick the model and press **Download and set up**. The window
closes at once and the rest runs in the background as a job: the inference runtime first when none is installed, then the
model (downloading, checking, loading and testing it), with bytes, speed, time left and connections in Jobs and the status
bar, and "Setting up - 1.2 of 5.6 GB" in the assistant pane. When it is done a message says "AI assistant ready" with **Open
the assistant**; if it fails one plain message says why, with **Retry**. Cancel it from Jobs.

![A modeless dialog beside the work](screenshots/dialog-modeless.png)

![A modal dialog over a dimmed window](screenshots/dialog-modal.png)

![The first-start AI set-up](screenshots/dialog-ai-setup.png)

![Share a redacted copy: the buttons stay in view](screenshots/dialog-redacted-copy.png)

![Export events](screenshots/dialog-export.png)

The screenshots come from the browser build, which draws the same dialogs inside the page; on the desktop each is a real window.

Settings asks first when edits are unapplied; `Enter` runs a dialog's default
button, and focus returns to the window you came from. The command palette and the Search everything list, menus,
drop-downs and tooltips are not dialogs; they stay inside the window.

## Projects

*Save project* (`Ctrl/⌘+S`) writes the window's store as one `.debriefz` file: the imported events, the profile, filters,
time range, columns, notes, saved queries and the layout. A project opens, searches and analyses **without the
original logs** (the search index is rebuilt in the background), so it can be shared as it is. Debrief also autosaves a session per window (`last-session.debriefproj`) and offers to
restore it after a crash. Opening a project that was saved by a newer Debrief is refused with a message saying so.

## Appearance

**Settings ▸ Appearance** (`Ctrl/⌘+,`, or the palette command *Open settings: Appearance*) sets how Debrief looks.
Everything applies at once, in every window, and is kept for your user (not in projects):

| Setting | Choices | Also in |
|---|---|---|
| **Theme** | *Calm* (default): neutral and quiet, colour only for the data. *Console*: soft cards, violet accent, richer colour. *Precision*: paper and ink, hairlines, mono figures, the densest view. *High contrast*: black or white with WCAG AAA text and strong outlines. *Follow system* (default) uses Calm, or High contrast when the operating system asks for more contrast. | *View ▸ Theme*; palette *Theme: Calm* / *Theme: Console* / *Theme: Precision* |
| **Interface font** | *System* (default): the operating system's font, like other desktop apps. *Theme*: the theme's own font. Log text stays monospaced | |
| **Colour scheme** | *System* (default, follows the operating system), *Light*, *Dark* | *View ▸ Theme*; palette *Colour scheme: …* |
| **Density** | *Comfortable* (default) or *Compact* rows and timeline lanes | |
| **Timeline ▸ Colour lanes by source** | on: each lane's volume in its source colour; off: neutral grey. Each theme has a default; *Use the theme's default* returns to it | |
| **Source colours** | by file (default), by instance, off | *View ▸ Source colours* |

What changes with the theme on the timeline:

- **Runs** are a thin line in the lane's colour (Calm) or a labelled pill (Console). Runs that overlap on one lane,
  such as several views open at the same time, sit on their own sub-tracks below the lane's markers, each labelled
  with what it is; the lane grows by one thin row per sub-track.
- **Crashes** get a dashed rule across the lanes (Calm), or a soft band and a halo (Console).
- **Brushing** a range shows the range and its length in a floating pill in Console.
- **Findings** in the timeline header are a sentence (Calm) or chips (Console).

Markers differ by shape in every theme, so colour is never the only cue. With *reduce motion* set in the operating
system, Debrief shows no transitions.

## Supported inputs

The format is detected from the content. UTF-8 and UTF-16 are both read. Gzip files and zip archives
are detected by magic bytes and expanded (with size caps against zip bombs).

| Format | Typical files | Provided by | Notes |
|---|---|---|---|
| Serilog CLEF, Serilog `JsonFormatter`, JSON lines, whole-file JSON arrays (NLog, ECS, pino, ...) | `.clef`, `.json`, `.log` | `debrief-core` JSON importer | Keys are configurable in a parser's `[format.json]` (Settings ▸ Parsers); nested objects are flattened |
| Serilog text and console output | `.log`, `.txt` | `debrief-core` text importer | Described by the same `outputTemplate` you use in `appsettings.json` |
| log4net / NLog pattern layouts and any text via regex | `.log`, `.txt` | `debrief-core` text importer | Named groups in a parser's `[format] regexes`; multi-line messages and stack traces attach to their event |
| CSV / TSV / `;` / `|` delimited | `.csv`, `.tsv` | `debrief-core` CSV importer | Delimiter and columns auto-detected |
| Windows Event Logs | `.evtx` | `debrief-plugin-windows` | Binary importer; message is `Provider EventId: key=value, ...` (offline, no message DLLs) |
| System information | `.nfo` (msinfo32) | `debrief-plugin-windows` | One record per row with `Category`, `Item`, `Value` |
| Performance Monitor logs | `.csv`/`.tsv` (PDH), `.blg` | `debrief-plugin-windows` | `.blg` is converted with `relog` (Windows only) |
| Process Monitor exports | `.csv` (Sysinternals ProcMon) | `debrief-plugin-windows` | One record per row; the level follows `Result`; optional columns are kept |
| Windows minidumps | `.dmp` | `debrief-plugin-windows` | A short summary: system, process, exception (code, address, module), the crashing thread's stack (scanned, not a verified unwind) and loaded modules |
| Crash and memory dumps (minidump, heap, full) and UMDH heap logs | `.dmp`, UMDH `.txt` | `debrief-plugin-dumps` | Opened with *File ▸ Open dump…* / *Open UMDH snapshots…* (or dropped) in the Dump and UMDH views, not as log events; see [features](features.md#crash-dumps-and-umdh-heap-logs). |
| Windows registry exports | `.reg` | `debrief-plugin-registry` | One record per key/value, shown as a tree in the Registry panel |
| IIS / W3C extended, Apache / Nginx common and combined | `.log` | `debrief-plugin-web` | Also the comma-separated W3C variant (Exchange tracking logs), forwarded-for client lists and extra trailing fields (Traefik) |
| HAR captures | `.har` | `debrief-plugin-web` | One record per request |
| OpenTelemetry OTLP/JSON logs and traces | `.json`, `.jsonl` | `debrief-plugin-otel` | Spans and span events linked by `TraceId` / `SpanId` |
| syslog (RFC 5424, RFC 3164) and journald (`journalctl -o json`) | `syslog`, `.log`, `.json` | `debrief-plugin-syslog` | Severity is mapped to Debrief levels |
| log4j / log4net / NLog XML events | `.xml`, `.log` | `debrief-plugin-formats` | `XMLLayout` and `Log4JXmlEventLayout`; one malformed event costs only that event |
| SQL Server `ERRORLOG` | `ERRORLOG` | `debrief-plugin-formats` | UTF-16 or UTF-8; error number, severity and state become fields |
| Docker `json-file` and Kubernetes CRI container logs | `*-json.log`, `*.log` | `debrief-plugin-formats` | Partial lines are joined; `Stream`, pod, namespace and container are fields |
| CBS / DISM text logs | `CBS.log` | built-in text importer | Read by the default profile; a one-line profile regex adds the component as a field |

### Text encodings

Debrief detects the text encoding of every text source when it opens it: a byte-order mark settles UTF-8 and
UTF-16 (little or big endian) at once; without one the engine guesses from the bytes (UTF-8, UTF-16,
Shift_JIS, GBK, gb18030, EUC-KR, Big5, EUC-JP, the windows-125x code pages, ...). A source whose encoding is
not plain UTF-8 shows a badge in the Explorer, for example `Shift_JIS 84%`; the tooltip reads "Shift_JIS
· 84% (detected)". The percentage is the engine's confidence in its guess. Under 70% the badge gets a
warning tone: the text was guessed, and if it looks wrong, choose another encoding.

*Encoding…* in the Explorer row menu (only for text sources) lists the encodings plus *Automatic (detect)*,
with the one you chose before selected. *Apply* reads that one file again with the encoding, as a job; the
badge then says `(chosen)` with 100% confidence. *Automatic (detect)* guesses again.

To set the encoding for all files of a profile, use the `[files]` section of the profile:

```toml
[files]
encoding = "auto"        # or "Shift_JIS", "GBK", "windows-1252", ...
```

`auto` (the default) detects per file; a byte-order mark and your *Encoding…* choice win over the profile.

### Paste as log

*Edit ▸ Paste as log…* (`Mod+Shift+V`) opens a dialog with a Name (default "Pasted log") and a text box that is
filled from the clipboard when Debrief may read it; otherwise paste into the box. A counter shows the size against the
limit of 8 MiB, and *Add* is refused above it (save such text as a file and open that instead). *Add* saves the text as
a log file in the project's `pasted/` folder, so a saved project carries it, and opens it like any other log, with the
importer chosen from its content. The engine makes the name safe and unique ("web01 errors.log", then "web01 errors
(2).log"). Dropping plain text (no files) on the window opens the same dialog with that text.

### Opening a source file outside Debrief

*Open file*, *Open at line N* and *Show in folder* (row menu, inspector, Explorer row menu) work on the
files Debrief opened in the window:

| Source | Open file | Open at line | Show in folder |
|---|---|---|---|
| Text file (`.log`, `.clef`, `.csv`, ...) | Default program | VS Code (`code -g file:line`), else the default program at the top | Selects the file |
| Gzip file (`.gz`) | Default program | Not available: the lines are compressed | Selects the file |
| Binary file (`.evtx`, `.blg`, ...) | Default program | Not available: no text lines | Selects the file |
| Member of a zip archive | Not available: extract it first | Not available | Selects the archive |

VS Code is found on `PATH` (`code`; `code.cmd` on Windows) or where its installers put it
(`%LOCALAPPDATA%\Programs\Microsoft VS Code`, `%ProgramFiles%\Microsoft VS Code`,
`/Applications/Visual Studio Code.app`, `/usr/share/code`, `/snap/bin`). The default program is opened
with Explorer, `open` or `xdg-open`; files whose default program would run them (`.bat`, `.ps1`,
`.exe`, `.reg`, `.sh`, `.desktop`, ...) are never opened that way: use *Open at line* or *Show in
folder*. Links are followed first, so a link to a script is treated as the script. A file that was moved,
deleted or replaced by a folder since it was opened is reported, not opened; a file another program holds
locked is still opened or shown (Debrief does not read it).

Timestamps: ISO 8601/RFC 3339, Serilog defaults, `/` and `.` dates (day-first or month-first, detected
per file), a space, `T` or comma between date and time (`2025/11/08,00:04:37.759+01:00`), a dot or comma
before the fraction (`00:04:37,759`), month names, 12-hour clocks, epoch seconds/milliseconds, time-only
console logs, and your own .NET (`dd/MM/yyyy HH:mm:ss.fff`, `HH:mm:ss,fff`) or strftime formats. Timezone
for offset-less times is configurable.

**Timestamps not recognised.** When Debrief reads a file's lines but not the timestamps of more than 5 % of
its events, or of none of its first 20, the file gets a warning icon in the Explorer (its tooltip counts
the lines and quotes one) and a message in the Notifications centre. Files without any date-like text
(settings files, data tables) and formats read by plugins are not flagged. Debrief also looks at the
lines for a date and time it can describe (`Sat Nov 08 00:04:37 2025` → `%a %b %d %H:%M:%S %Y`) and
proposes that format only after checking that it reads them. **Fix timestamps** (the message's button,
or a click on the icon) opens the format wizard on the proposed format: the preview shows how the file
reads with it, and **Save and re-import** adds it to the profile's `timestamp_formats`. The message is
posted once per file; a reload does not repeat it.

**Problems pane.** Everything that went wrong while reading the files is listed in the **Problems** pane, like an
IDE's: open it from **View ▸ Problems**, the command palette (*View: Problems*) or the problem count in the status
bar; its tab shows the count too. Problems are grouped by file and kind (events without a readable timestamp,
lines that are not events, lines over the size limit, files no parser reads...), with how many and the first line
numbers: a line number jumps to its event in the grid (or opens the file at that line when the line was not
stored). Where the engine found a fix (for unreadable timestamps, a `[format]` change of the file's parser), the
details show it: **Preview effect** checks it on the stored events ("Fixes 1,204 of 1,204 checked events") without
changing anything, and **Apply and re-read** saves the changed parser in the library (a built-in becomes an
override) and reads its files again. **Ask the AI to fix…** opens the AI's parser proposal for the lines around
the problem; nothing changes until you review and apply it.

## Profiles

A profile only needs the keys that differ from the built-in default. The full schema, with every key
documented, is [`crates/debrief-core/src/default_profile.toml`](../crates/debrief-core/src/default_profile.toml);
a worked example is [`samples/profiles/orderservice.debriefprofile.toml`](../samples/profiles/orderservice.debriefprofile.toml).

```toml
name = "OrderService"

[instance]
key = ["MachineName", "ProcessId"]            # what one lane on the timeline is

[[extract]]                                   # turn text into typed fields
name = "process-stats"
regex = 'CPU (?P<cpu>[\d.]+)% .* WorkingSet (?P<ws>[\d.]+) ?(?P<ws_unit>KB|MB|GB)'
when = [{ field = "SourceContext", op = "eq", value = "Diagnostics.ProcessMonitor" }]
[extract.fields]
cpu = { type = "number", unit = "%" }
ws  = { type = "bytes", unit_from = "ws_unit", to = "MB" }

[[pipeline]]                                  # clean up during import
step = "drop"
when = [{ field = "SourceContext", op = "contains", value = "HealthChecks" }]

[[marker]]                                    # first matching rule is the primary marker
kind  = "gateway-timeout"
label = "Gateway timeout"
badge = "3"
shape = "hexagon"
color = "#3987e5"
description = "Payment gateway timed out; check the PSP status page"
when  = [{ field = "@message", op = "matches", value = '(?i)gateway.*time(d)? ?out' }]

[[chart]]
title = "CPU %"
y = "avg(cpu)"
bucket = "1m"
split_by = ["MachineName", "ProcessId"]
```

- **Pseudo-fields:** `@level @message @exception @template @source @line`.
- **Condition operators:** `eq ne contains notContains matches exists missing gt lt in`.
- **Extractor field types:** `string number integer bool bytes duration`.
- **Pipeline steps:** `drop keep mask dedupe remove_fields rename set set_level truncate parse_json
  parse_kv lookup shift_time calc script plugin`.
- Profiles are checked by the engine (user regexes are length- and size-limited); an invalid profile
  is rejected with a message that says what to fix.

### Files several products write (log servers)

A file may mix formats: Debrief reads every line with the first output template or regex that matches it
(templates first, in profile order), and one-line JSON objects with a timestamp key (`@t`, `timestamp`...)
as JSON events. A line no format reads continues the event above; when it looks like an event (it starts
with a date, a time or a syslog header), the Explorer's tooltip for the file reports it ("12 lines look like
events but match no format, first on line 40"): add a template or regex for that product.

Lanes follow each product's own fields: the instance key takes the fields a source writes on most of its
events, even when that product writes few of the folder's events, and importers add the fields that name
their writers (syslog `Host`/`App`/`ProcId`, Windows event logs `Computer`/`ProcessId`, containers
`Namespace`/`Pod`/`Container`).

To give each product or host of a shared file its own source, add routes. The first route whose `when`
conditions hold names the event's logical source; the Explorer lists the logical sources under the file:

```toml
[[route]]
when = [{ field = "Application", op = "exists" }]
source = "{Application}@{MachineName}"        # fields or @pseudo-fields in braces

[[route]]
when = [{ field = "Logger", op = "contains", value = "Inventory" }]
source = "Inventory@{Host}"                   # Host may come from an [[extract]] rule
```

Events no route names stay in the file's own source. Removing a logical source removes the file; a split
file is re-read whole when it changes. A worked example is
[`samples/profiles/logserver.debriefprofile.toml`](../samples/profiles/logserver.debriefprofile.toml) for
`samples/linux/logserver01/apps-20261014.log`. Design: [ADR 0020](adr/0020-mixed-sources.md).

## Query language

Press `Ctrl/⌘+/` to focus the search field and `Ctrl/⌘+Enter` to run. Keywords are case-insensitive.
A query is an optional filter followed by `|` stages.

| Syntax | Meaning |
|---|---|
| `timeout` or `"connection reset"` | full text over message, exception and template: words, the last one as a prefix (the search box's smart search, with partial matches added after the word matches) |
| `level >= warning` | compare a field (`= != > >= < <=`); levels are ordered |
| `RequestPath contains "/capture"` / `!contains` | substring of that field's text (always a substring, unlike the search box and bare words) |
| `message ~ "time(d)? ?out"` / `!~` | regular expression |
| `StatusCode in (500, 502, 504)` | one of a list |
| `exists(Exception)` / `missing(CorrelationId)` | field present or absent |
| `a and b`, `a or b`, `not a`, `( ... )` | boolean logic (`and` may be omitted) |
| `` `my field` `` / `@level` | quoted and pseudo field names |
| `\| where expr` | filter again after a stage |
| `\| stats count() as n, p95(Elapsed) by SourceContext` | aggregate; functions: `count count_distinct avg min max sum p50 p95 p99` |
| `\| sort n desc` | order the result |
| `\| head 10` (or `limit 10`) | first rows |
| `\| fields a, b` | choose columns |

Examples:

```text
level >= error and RequestPath ~ "/capture" and ElapsedMs > 5000
exists(@exception) | stats count() as n by Application, MachineName | sort n desc | head 20
SourceContext = "Outbox.Dispatcher" and message contains "timed out" | stats count() by ProcessId
level >= warning | stats p50(ElapsedMs) as p50, p95(ElapsedMs) as p95, p99(ElapsedMs) as p99 by RequestPath
```

Plain filters narrow the grid, timeline and charts; `stats` queries show a results table you can
export as CSV. The full grammar is in
[`crates/debrief-core/src/ql/mod.rs`](../crates/debrief-core/src/ql/mod.rs).

## The Store pane and the SQL console

*View ▸ Store* (palette: *Open Store*) shows what the store of the window holds and how it is doing, read-only:

- **Overview**: the store folder, its size on disk, events, sources, segments, the data version, what it is doing
  (ready, importing, indexing) and the last import with its speed.
- **Segments**: the Parquet files the events are in: rows, size, time range, compression, sources, whether the
  full-text index has them, the fields they store in their own column, and whether the next compaction rewrites
  them.
- **Fields**: each field's type, how it is stored (*declared* by the profile, *promoted* to its own column,
  *partly* promoted, or in the *overflow* map, which is slower to filter on), how many events have it and roughly
  how many distinct values it has.
- **Index**: how many events the full-text index (word search) covers, its size and whether it is catching up.
- **Memory**: the store worker's memory against its budgets (768 MB importing, 300 MB querying), DuckDB's memory
  limit and use, whether Low memory mode is on, running jobs and cached analyses.
- **Maintenance**: *Compact now*, *Rebuild index* and *Clear cached analyses* run as jobs after you confirm; they
  wait while an import runs.

The **SQL** tab (palette: *Open SQL console*) runs read-only DuckDB SQL. `Ctrl/⌘+Enter` runs, `Esc` stops, the
editor completes view, column and function names, and *Explain* shows the query plan instead of rows. A query runs
as a job (stop it in the Jobs pane too), keeps at most 10,000 rows (shown 500 at a time), stops after 30 seconds and
has 128 MB of memory. Only one `SELECT` (or `WITH`, `FROM`, `VALUES`, `DESCRIBE`...) runs at a time; statements
that change anything, settings, extensions, other databases, files and URLs are refused (the error names what and
underlines it). Rows that carry an event `id` can be opened in the Events grid. Save a query with *Save SQL*: it is
listed under Saved queries with an *SQL* badge and saved in the project.

The console sees four views (version 1; columns are only ever added):

| View | Columns |
|---|---|
| `events` | `id`, `ts` (UTC timestamp, NULL when untimed), `level` (`Verbose` … `Fatal`), `source` (a `sources.id`), `line`, `byte_offset`, `message`, `template`, `exception`, `fields` (a map of every other field as text: `fields['RequestId']`) |
| `sources` | `id`, `name`, `path`, `importer`, `parser`, `events`, `skipped`, `first_ts`, `last_ts`, `bytes` |
| `fields` | `key`, `type` (`text`, `integer`, `number`, `boolean`), `storage` (`declared`, `promoted`, `partly`, `overflow`), `events`, `fill_rate` (0–1), `approx_distinct` |
| `notes` | `id`, `anchor` (`event`, `range`, `instant`, `source`, `marker`, `chartPoint`, `project`, `image`), `event_id`, `source_id`, `from_ts`, `to_ts`, `severity`, `tags`, `text`, `author`, `created`, `updated` |

```sql
SELECT s.name, level, count(*) AS n FROM events e JOIN sources s ON s.id = e.source
WHERE ts >= TIMESTAMP '2026-10-14 10:00' GROUP BY ALL ORDER BY n DESC;
SELECT fields['RequestPath'] AS path, quantile_cont(TRY_CAST(fields['ElapsedMs'] AS DOUBLE), 0.95) AS p95
FROM events WHERE fields['ElapsedMs'] IS NOT NULL GROUP BY path ORDER BY p95 DESC LIMIT 20;
SELECT n.text, e.message FROM notes n JOIN events e ON e.id = n.event_id;
```

## The AI's browser

*Search the web for this error* opens the search in your **default browser**, and so do links from events, exceptions and
research results. Searches use **Google** unless you pick another engine: Settings ▸ Web browser ▸ Search engine sets it
for every search (DuckDuckGo, Bing, or *Custom* with your own address containing `{q}`), and the picker in the search
dialog changes it for one search.

Research and the chat's **Browser** menu (*Off*, *Read along*, *Agent*) use a real Chromium-based browser that Debrief starts
for the AI: Edge, Chrome or Chromium from your computer, or a Chromium you download in Settings ▸ Web browser (*Download
Chromium*, about 200 MB, checksum verified, on request only). *Read along*, *Agent* and research show its window
like any other window; research closes it again when it finishes (done, cancelled or failed), and Debrief closes it when you quit.
If you took part (you signed in, solved a check, navigated or opened a tab) it stays open and the chat says so, with **Close it** and **Keep it**. When a page needs you (a cookie consent or a "verify you are human" check) the window comes
to the front, you deal with it, and the AI goes on. The browser has a throwaway profile (no cookies of yours, nothing
kept) and the real user agent. If no browser is found, research and the modes say so and point to Settings.

**The search text.** For *Search the web for this error* and *Research this*, the AI writes the search from your
question and the event: it keeps the exception type, the decisive words of the message, error codes and the
framework or library with its version, and leaves out ids, GUIDs, host, machine and user names, paths, ports,
timestamps and other values of your installation. Debrief then checks it like any search text: in *Search the web*,
what looks private or comes from your logs is taken out; in *Research this*, it is highlighted and you confirm
or edit it (*Research anyway*). Either way you see the text and can change it before anything is searched;
*Let the AI write it* drafts it again. Without a model, Debrief builds the text itself, as before.

**How you can tell the AI is driving a page.** Every indicator is outside the page (Debrief never puts anything into a web
page): Chrome's banner "Chrome is being controlled by automated test software", a window frame in Debrief's accent colour
(best effort; some browser versions ignore it), and in Debrief a **status chip** with what the AI does ("Reading 3 of 5
results"), the site and **Stop**. Stop ends the browser process at once, denies a step waiting for your approval and ends
agent mode or cancels the research, the same as Stop in the chat. The chip goes away as soon as the AI lets go: after Stop,
when the answer or the research is done, cancelled or failed, or when you turn agent mode off. The research log lists every
page it opened and skipped.

## Knowledge files: teach the AI your systems

Like `AGENTS.md` for coding assistants, **knowledge files** are Markdown notes the AI reads with every question
(chat, research, explanations, folder set-up, query drafting, images): the names of your products and services,
what each log source is, known errors and their fixes, facts about your environments. Manage them in
*Settings ▸ AI ▸ Knowledge*:

- **This project** holds files saved with the project (and the autosaved session), so a shared project carries
  them. **Every project** holds files kept in Debrief's settings folder on this computer.
- **New…** writes a file in the editor (with a *Preview*), **Import…** adds an existing `.md` or `.txt` file
  (named after it), **Edit…** renames it or changes its text, **Export…** saves it as a `.md` file to share, and
  **Remove…** deletes it after asking. The checkbox turns a file off without deleting it.
- A file holds at most 64 KiB, a scope at most 32 files. The AI reads the project's files first, then the global
  ones, within about a quarter of the model's context window; when they do not all fit, the rest is cut and the AI
  is told which files were left out, so shorten or turn off what it does not need. They are background facts for
  the AI: they never replace Debrief's own rules, and their text is never written to Debrief's logs.

### Known issues

A knowledge file can declare errors you have already diagnosed. Add a fenced block labelled `known-issue`:

````text
```known-issue
title: Connection pool exhausted
pattern: (?i)timeout expired.*max pool size
fix: Raise Max Pool Size in the connection string.
  Then restart the service.
```
````

Give `title`, `fix` and one of `pattern` (a regular expression over the message and exception) or `query` (a query-language
filter); a line indented by two spaces continues the previous value. The **Problems** pane lists the declared issues
that occur in the open data, with the number of events, the fix and *Show events*. **Suggest with AI** asks the
model which of your notes fit the problems; the engine keeps only suggestions whose fix is quoted from your notes and
whose query selects events (they are marked *AI suggested, verified*). Without a model the declared issues still work.

## Watched folders and opening a time range

**Open a time range…** (the clock-like button in the Explorer) asks for a folder, then for a start and an end in UTC; only
events inside the range are imported. Leave one end empty for "from here" or "until here".

**Watched folders** (View ▸ Tools ▸ Watched folders): *Watch a folder…* picks a folder; files that appear in it are
imported into the project once they stop changing. Untick a folder to pause it, press the bin to stop watching. If your
knowledge files declare known issues, a new one found in the imported data is announced once; allow desktop
notifications to hear about it while Debrief is in the background.

## Pattern trends and placeholder values

In the **Patterns** view each template has a small bar chart of when it occurred: the time range of the analysed
events is split into 24 equal buckets, the same for every pattern, so you can tell a burst from a steady stream. Hover
it for the busiest time. Patterns with `<*>` placeholders have a **Values** button that lists, for each placeholder,
the most frequent values and how many events carried them ("which user, which order, which host"). Click a value to
filter the events by the pattern and that value. With very many distinct values the engine keeps the most frequent ones
and says the counts are approximate. Patterns from structured templates (Serilog `{Property}`) have no placeholders:
their properties are fields, so use the facets.

## Error outliers

*Analyze: Show error outliers* answers "what do the failing events have in common?". It compares the events you select
(errors and fatals by default; type any filter such as `host = "B"` in the box and press **Compare**) with the rest of
the current window and lists the field values that are clearly more common in the selection: for each, how many
selected events and how many other events carry it, the **lift** (how many times more likely it is in the selection) and
the p-value of the difference. Values need at least three selected events, a lift of 1.5 or more and p < 0.001 to be
listed. Click a row to filter the events by it, or an evidence button to jump to one event. The window is the current
query and time range, which must not contain `|` stages. The same analysis runs from the command line with
`debrief analyze outliers`.

## Incident summary report

*Report: Incident summary…* builds a report from the open data in one step: the period and levels, key moments, the
top anomalies, exceptions and error messages with the events that show them, the sources and hosts with most errors,
import problems, matched known issues with their fixes and your notes. The engine computes every number; nothing
is written by a model. With a model you can tick *Add an AI narrative*: a short write-up is added as its own first
section, marked as AI-written, citing events (citations the engine could not verify are listed). Export it from the
report view as HTML or PDF; the export has the redaction option.

## Investigation playbooks

*View: Playbooks* lists your playbooks and four templates (service crashed, errors after a change, slow requests,
sign-in failures). A run steps through the playbook: queries, filters, views to open and checks that pass or fail,
each with the engine's result; add a note to any step. Runs are saved with the project. Playbooks are JSON files you
can import, export and share; see [playbooks.md](playbooks.md).

## Pitwall as the AI engine

If [Pitwall](https://dev.decryptology.net/decryp7/pitwall), the family's local model server, runs on this computer,
Debrief can use it instead of its own engine: one model in memory serves all your apps, and Debrief does not download a
second copy. Debrief's built-in engine stays as the fallback. Everything stays on this computer either way; Debrief only
talks to Pitwall on `127.0.0.1`.

- **Connect once.** In *Set up AI assistant* choose **Use Pitwall** (or *Settings ▸ AI ▸ Connect to Pitwall…*).
  Debrief shows a six-digit code; Pitwall shows a request with the same code in *Apps & keys*. Check they match and
  choose **Approve**. Debrief keeps its key in the system's credential store (macOS Keychain, Windows Credential
  Manager); *Forget Pitwall* deletes it. Debrief never connects by itself.
- **AI engine** (*Settings ▸ AI*): **Automatic** uses Pitwall when it is running and connected, otherwise the built-in
  engine; **Pitwall** uses only Pitwall; **Built-in** uses only Debrief's own engine and models.
- **Which model.** Pitwall picks a chat model that fits Debrief's context length. To decide yourself, create a preset
  named `debrief` in Pitwall; Debrief uses it when it exists. *Settings ▸ AI* shows the model Pitwall chose.
- **If Pitwall stops** (it quits, pauses, or the key is revoked), the request that was running fails with a plain
  message and the status bar shows the AI as limited. Debrief never switches engines in the middle of your work by
  itself: choose **Use the fallback** to continue with the built-in engine for this session, or open Pitwall again.
- **No second copy.** While Pitwall is the engine, Debrief does not download or update its own runtime and models and
  does not load its own model; models you already downloaded stay until you delete them in *Manage models*.
- Debrief speaks Pitwall's connection version 1 (Pitwall 1.0.4 and later). If Pitwall reports another version,
  *Settings ▸ AI* says which app to update and Debrief uses its built-in engine.

## AI runtime and model updates

Debrief checks for newer **inference runtimes** (the llama.cpp builds that run the model) and newer revisions or
quantisations of your **models**. Only what Debrief's built-in lists pin is ever offered: a runtime update is the
newer llama.cpp release this version of Debrief pins (each file checked against its SHA-256), a model update is a
model-list entry that replaces a file you have, confirmed with the model host before it is offered.

*Settings ▸ AI* sets what happens for each kind:

- **Automatic** downloads and verifies the update in the background, as a job in the Jobs pane. The installed
  version keeps working until the new one is verified, and a running chat is never interrupted: an update
  found during a chat waits for it to finish, and a new model replaces the active one only between chats.
- **Ask** (the default) shows a message with **Update**, **Later** (ask again at the next check) and **Skip
  this version** (never offer this version again; *Update* in Settings still installs it).
- **Manual** only checks when you choose **Check now** in *Settings ▸ AI ▸ Updates*.

*Check for AI updates every (hours)* sets how often Debrief checks (daily by default). Settings shows when it last
checked and when it checks next. A check or update that fails because the computer is offline or a proxy
blocks it does not interrupt you: the reason is shown in Settings and Debrief tries again later (after 5 minutes,
then less and less often, at most once per interval). When a proxy intercepts HTTPS, **Download manually**
there works as described under [Troubleshooting](#behind-a-corporate-proxy-zscaler-etc).

## Disk usage and project stores

Settings ▸ Storage lists what Debrief keeps on disk by category (sizes are computed by the engine). Below the
categories, *Project stores* lists one row per store folder: its name, size, when it was last used and its state:

- **Open**: a window has the store open.
- **Kept for session restore**: an autosaved session reopens it, so it stays until you restore or discard that session.
- **In use by another Debrief**: another Debrief process uses it.
- **Orphaned**: nothing references it any more (for example the extraction of a project you closed).

Only orphaned stores can be removed, and never a store that is open: tick the orphaned rows and choose *Clean up
selected…*, or choose *Clean up all orphaned*. A confirmation lists the count and size and says the data cannot be
recovered. The engine checks every store again before removing it and reports the ones it skipped with the reason.
*Automatically remove orphaned stores unused for N days* removes old orphaned stores on its own; 0 (the default)
turns it off, and the largest value is 365.

## Keyboard shortcuts

`Mod` is `Ctrl` on Windows/Linux and `⌘` on macOS.

| Shortcut | Action |
|---|---|
| `Mod+K` | Command palette (every action is in it) |
| `F1` | Keyboard shortcuts (searchable, printable; generated from the commands; *Edit shortcuts…* changes them, see below) |
| `Mod+Z` / `Mod+Shift+Z` | Undo / redo the last edit (filters, marker and highlight rules, profile edits, dashboards, reports) |
| `Mod+O` / `Mod+Shift+O` | Open files / open folder |
| `Mod+F` | Find: focus the search box (*Edit ▸ Find*) |
| `Mod+/` | Focus the search field |
| `Mod+Shift+F` | Search everything |
| `Mod+Enter` | Run the query (in the search field) |
| `Mod+Shift+M` | Analyse this moment |
| `Mod+Shift+N` | New window |
| `Mod+S` / `Mod+Shift+S` | Save project / save project as |
| `Mod+D` | Add a note to the selected event |
| `Mod+Shift+E` | Export events |
| `Mod+Shift+V` | Paste as log (Edit menu) |
| `Mod+Shift+X` | Toggle the redacted view |
| `Mod+Shift+F12` | Capture a diagnostic screenshot (error snapshot) |
| `Ctrl+Alt+O` / `Ctrl+Alt+H` | Pop the panel out to a window / auto-hide or pin a tool window |
| `Shift+Alt+Enter` | Full screen |
| `Mod+B` | Toggle the sidebar |
| `F6` / `Shift+F6` | Next / previous region: activity bar, side bar, filter bar, each panel group, status bar |
| `Mod+0` | Focus the side bar |
| `Mod+1` … `Mod+9` | Switch to perspective 1 … 9 |
| `Ctrl+Alt+1` … `Ctrl+Alt+9` | Focus panel group 1 … 9 (left to right, top to bottom) |
| `Mod+Shift+G` | Focus the events grid |
| `←` / `→` (in the level chips) | Move between the chips (the chips are one Tab stop) |
| `Mod+,` | Settings |
| `Alt+←` / `Alt+→` | Go back / forward (events, time ranges, views) |
| `↑` / `↓` | Move through the events grid |
| `Esc` (while dragging a tab) | Cancel the drag |

Browser keys such as `F5`/`Mod+R` (reload), `Mod+P` (print),
`Mod` `+`/`-`/`0` (page zoom) and `Alt+←`/`Alt+→` as browser back/forward do nothing in Debrief's windows
(Debrief's own bindings, such as the time-range history above, still work). Release builds also turn off
the developer tools (`F12`, `Mod+Shift+I`).

To float a panel inside the window, choose **Float** in its tab menu or **Window ▸ Float panel**. A
tab dragged off the docking guides and released inside the window goes back where it was
(**Release to cancel**); drag it outside the window and a ghost of the new window follows the pointer
(also over other apps and monitors): release it to open the panel there.

## Change a shortcut

*Settings ▸ Keyboard* (or *Help ▸ Keyboard shortcuts ▸ Edit shortcuts…*) lists every command with its key. *Change* records a new
key and warns when another command or a built-in key already uses it (Replace or Cancel); the cheat sheet, the menus and the
palette show your keys. Overrides are kept for your user. *High contrast* is chosen automatically when the operating system
asks for more contrast (or in *View ▸ Theme*).

## What's new

*Help ▸ What's new* opens the release notes of the version you are running as a document beside the timeline (earlier
releases are folded below). After an update Debrief opens it once by itself, in the background: it joins the documents as
a tab without taking the front or the keyboard focus, and Notifications shows "Updated to *version*" with *See what's new*,
which brings it forward. Close it like any other document; it does not open again on later starts of the same version.
A fresh installation does not open it.

## Video sources

*File ▸ Open video…* (or drop a recording) analyses a screen recording and puts its moments next to the logs. The first time,
*Settings ▸ Video* downloads a pinned FFmpeg build (about 55 to 80 MB, checked before use) or lets you pick an `ffmpeg` of your
own; on macOS you provide it (for example Homebrew's). The **Video** view shows the player, the frame strip (drop frames, add
the one at the player position, switch between *changes* and *interval* mode and analyse again), the alignment with its
evidence ("the clock in frame 4 read 10:42:07", the container's creation time, the file time or a start you set), the
model's description and your note for each frame, *Explain with AI* and *Find related logs*. The video is a source with its
own lane; it stays referenced by SHA-256 unless you choose *Include the video in the project*. Up to 120 frames per video,
600 per project and 6 hours of video; a model without vision is never sent the frames.

## Dump symbols

Frames of a dump are named from PDBs whose identity (name, GUID, age) matches the module's: a PDB of the same name but another
build is never used. *Settings ▸ Dumps* sets the local symbol folders; *Settings ▸ Symbols* turns on HTTPS symbol servers
(off by default; only the PDB's name, GUID and age are sent), their size and time limits and the cache folder, which is
trimmed to its size cap. The Modules table says per module why it has no names (not needed, not found, not on the server, a
different build, unreadable, download failed) and what to do next. UMDH logs keep the names UMDH wrote.

## Share a redacted copy

**File ▸ Share redacted copy…** writes a copy of the project (`.debriefz`) that is safe to hand to someone
else. Pick the detectors (bearer tokens, JWTs, API keys, connection strings, passwords, user names in paths,
email addresses, card numbers, IP addresses, GUIDs, phone numbers, host names) and the profile's custom
`[[redact.rule]]` entries; they start as the profile selects them. Choose how a value is replaced: *Mask*
(`<email>`), *Hash* (a short code; equal values stay equal within the copy, so you can still follow one
address through the log) or *Drop* (removed). A live preview shows how many events would change, the count
per rule, up to five before/after samples and what is left out. **Create copy…** asks where to save it, like
**Save project as…**, then shows what was written.

What the copy contains: it is built from the redacted text of every text source (JSON and text logs), so its
events, field statistics, search index, diagnostics samples, notes, AI chats, saved queries and reports
contain no unredacted value of the selected rules. The whole event text is redacted whatever field it is in.
Clock corrections are kept.

What it leaves out, and says so in the preview and the result:

- Sources in formats that cannot be redacted (binary formats such as event logs and dumps, CSV and XML).
- Note images and attachments, and video frames: pictures cannot be redacted.
- The profile's own redaction rules: the copy does not carry them.

This differs from the *Redacted view*, which only masks what one window shows and changes nothing on disk,
and from exports and reports, which redact one file at a time when asked to. **Save project as…** keeps
everything, unredacted.

## Command line

The desktop app takes paths to open at startup and a few flags:

```sh
debrief-app <folder-or-files...>        # open files and folders (in the running Debrief, if there is one)
debrief-app --new-window <paths...>     # open them in a new window of the running Debrief
debrief-app --new-instance <paths...>   # start a separate Debrief process instead
debrief-app --safe-mode                 # separate process, no plugins, no saved profile (also: DEBRIEF_SAFE_MODE=1)
```

Debrief runs once per user: a second launch hands its paths to the running instance (at most 256),
which opens them as set in *Settings → Window & tray* and focuses its window. Paths that do not
exist are ignored; the rest are canonicalised.

The `debrief` command line imports files, runs queries and analyses and writes the incident report without the app
(`debrief import logs/ --store ./s`, `debrief query 'where level = "Error"' --store ./s --format csv`,
`debrief analyze exceptions --store ./s --json`, `debrief report --open logs/ --out incident.pdf`); exit codes and `--json` make it
usable in CI. See [cli.md](cli.md).

Two other headless tools ship in the workspace for servers and customer machines:

```sh
debrief-collect [--plan plan.json|plan.toml] [--out dir] [--print-default-plan]
debrief-monitor --config <monitor.json> --out <dir> [--duration <10m|30s|2h|1500ms>]
```

`debrief-collect` writes an `.debriefz` bundle that Debrief opens directly; `debrief-monitor` writes rolling CLEF files.
Run them from a checkout with `cargo run -p debrief-collect -- --help` or `cargo run -p debrief-monitor -- --config monitor.json --out ./logs`.

## Notifications and service status

Everything Debrief tells you appears as a notification in the corner of the window, and stays in the **Notifications**
list (the bell). One problem is one notification: if the same thing happens again, the notification you already have is
updated (with a count) instead of a new one appearing. Errors stay until you dismiss them or the cause is gone; other
messages disappear after a few seconds. Technical text is folded behind **Details**.

When one of Debrief's own services stops, you see a single chip at the right of the status bar instead of a message in
every panel:

| Service | What it is | Button |
|---|---|---|
| Log store | The database worker that holds your events | Restart, or Try again after repeated failures |
| AI worker | The local model | Try again |
| Plugins | Importers and tools that run in their own process | Allow again (after a plugin was switched off) |
| Research browser | The browser the assistant drives | Install (downloads the pinned Chromium) |
| Downloads | Model, runtime and update downloads | Try again (runs the failed download again) |

Choose the chip to read what happened, the next step and the technical details. The button repairs the service from
Debrief itself; it does not need the service to work. Every window shows the same state. While the log store is down, the
grid, the timeline and other panels say *Waiting for the log store* and load again by themselves when it is back.

## Troubleshooting

### Behind a corporate proxy (Zscaler etc.)

Model and runtime downloads use HTTPS and check the server's certificate against the operating system's
certificate store, like your browser. Security tools that inspect HTTPS (Zscaler, Netskope, Forcepoint,
Palo Alto and similar) replace the certificate with one of their own. If their root certificate is in
Windows' certificate store (*Manage computer certificates* ▸ *Trusted Root Certification Authorities*;
IT usually installs it), Debrief's downloads work without anything to set. If it is not, the download stops
with *Certificate problem: a proxy or security tool on your network intercepts HTTPS*. Then either:

- ask IT to install the proxy's root certificate in Windows' store (on macOS: the System keychain; on
  Linux: the system store, e.g. `update-ca-certificates`) and choose *Try again*; or
- choose **Download manually** in the error: Debrief opens the exact file in your browser and shows its
  expected SHA-256.
  - For a **model**, choose *Add downloaded file…* (or *Add local GGUF…* in *AI: Manage models*) and
    pick the `.gguf` file. Debrief checks its SHA-256 against the catalog before adding it, even if the
    browser renamed it, and rejects an incomplete or changed file.
  - For an **inference runtime**, download every file listed (CUDA builds have two), choose *Add
    downloaded files…* and pick them all without unzipping them. Debrief checks each SHA-256 and unpacks
    the build into the folder shown. Alternatively set *Custom llama-server* in *Settings ▸ AI* to a
    `llama-server` IT provides.

If your network needs an explicit proxy, set *Download proxy* in *Settings ▸ AI* (`http://host:port`,
or `http://user:password@host:port` when the proxy asks for a sign-in); Debrief does not read proxy
environment variables. The other errors say what to check: *the server cannot be reached* (offline or
the name does not resolve), *did not answer in time*, or *refused the download* (an HTTP status; a 403
can come from a proxy that blocks the site).
