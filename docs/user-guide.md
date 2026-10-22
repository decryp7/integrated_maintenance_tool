# User guide

Reference for everyday use: which files IMT reads, how to write a profile, the query language, the
internal web browser, the keyboard shortcuts and the command line. For what IMT can do, see [features.md](features.md); for a
first walk-through, see the two-minute tour in the [README](../README.md#quick-start).

- [First run and layout](#first-run-and-layout)
- [Supported inputs](#supported-inputs)
- [Profiles](#profiles)
- [Query language](#query-language)
- [Internal web browser](#internal-web-browser)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Command line](#command-line)
- [Troubleshooting](#troubleshooting)

## First run and layout

With nothing open, IMT shows the welcome page: **Open folder…**, **Open files…** or **Try a demo** (or drop
files anywhere). After opening data you get the **Focused** layout: the timeline over the events grid, with
the inspector sliding out from the right when you select an event.

- **Filter bar** (one row): search (`.*` regex, `Aa` match case), level chips, the time range (click to
  reset a zoom), UTC/local, *+ Add filter*, *Clear all* and **View options** with *Group by*, *Marked only*
  and *Always show start / stop*.
- **Search** finds words first: every word you type, in any order, the last one as the start of a word
  (`timeout exc` finds `System.TimeoutException`; `camelCase`, dots, underscores and digits split words). A
  moment later IMT adds events that contain the text only inside a word; *+N* in the search box says how many,
  and those rows are marked *partial*. `.*` (regex) and `Aa` (match case) search the text as written.
- **Timeline header**: the health check's headline, e.g. "⚠ 2 crashes · 287 errors · 1 went silent —
  Review". *N crashes* jumps to the next crash, *N errors* shows the errors and fatal events, *N went silent*
  shows the next silence, *Review* opens the Overview. The line is hidden when nothing is notable.
- **Legend**: only the marker kinds that occur in the data, on one line; *+N more* shows the rest.
- **Query bar**: `Ctrl/⌘+/` (or *Analyze ▸ Focus query bar*) opens it above the events grid.
- **Sidebar**: the Explorer; the **Fields** section starts collapsed (click its header; IMT remembers).
- **Set up analysis…** for a folder of several files is offered in Notifications (the bell).

*View ▸ Layout ▸ Full* restores the richer default (the query bar in the default layout and the Overview
opening after every import). The choice applies to new layouts (*Reset layout*); layouts you saved are kept.

## Appearance

**Settings ▸ Appearance** (`Ctrl/⌘+,`, or the palette command *Open settings: Appearance*) sets how IMT looks.
Everything applies at once, in every window, and is kept for your user (not in projects):

| Setting | Choices | Also in |
|---|---|---|
| **Theme** | *Calm* (default): neutral and quiet, colour only for the data. *Console*: soft cards, violet accent, richer colour. *Precision*: paper and ink, hairlines, mono figures, the densest view. | *View ▸ Theme*; palette *Theme: Calm* / *Theme: Console* / *Theme: Precision* |
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
system, IMT shows no transitions.

## Supported inputs

The format is detected from the content. UTF-8 and UTF-16 are both read. Gzip files and zip archives
are detected by magic bytes and expanded (with size caps against zip bombs).

| Format | Typical files | Provided by | Notes |
|---|---|---|---|
| Serilog CLEF, Serilog `JsonFormatter`, JSON lines, whole-file JSON arrays (NLog, ECS, pino, ...) | `.clef`, `.json`, `.log` | `imt-core` JSON importer | Keys are configurable in a parser's `[format.json]` (Settings ▸ Parsers); nested objects are flattened |
| Serilog text and console output | `.log`, `.txt` | `imt-core` text importer | Described by the same `outputTemplate` you use in `appsettings.json` |
| log4net / NLog pattern layouts and any text via regex | `.log`, `.txt` | `imt-core` text importer | Named groups in a parser's `[format] regexes`; multi-line messages and stack traces attach to their event |
| CSV / TSV / `;` / `|` delimited | `.csv`, `.tsv` | `imt-core` CSV importer | Delimiter and columns auto-detected |
| Windows Event Logs | `.evtx` | `imt-plugin-windows` | Binary importer; message is `Provider EventId: key=value, ...` (offline, no message DLLs) |
| System information | `.nfo` (msinfo32) | `imt-plugin-windows` | One record per row with `Category`, `Item`, `Value` |
| Performance Monitor logs | `.csv`/`.tsv` (PDH), `.blg` | `imt-plugin-windows` | `.blg` is converted with `relog` (Windows only) |
| Process Monitor exports | `.csv` (Sysinternals ProcMon) | `imt-plugin-windows` | One record per row; the level follows `Result`; optional columns are kept |
| Windows minidumps | `.dmp` | `imt-plugin-windows` | A short summary: system, process, exception (code, address, module), the crashing thread's stack (scanned, not a verified unwind) and loaded modules |
| Windows registry exports | `.reg` | `imt-plugin-registry` | One record per key/value, shown as a tree in the Registry panel |
| IIS / W3C extended, Apache / Nginx common and combined | `.log` | `imt-plugin-web` | Also the comma-separated W3C variant (Exchange tracking logs), forwarded-for client lists and extra trailing fields (Traefik) |
| HAR captures | `.har` | `imt-plugin-web` | One record per request |
| OpenTelemetry OTLP/JSON logs and traces | `.json`, `.jsonl` | `imt-plugin-otel` | Spans and span events linked by `TraceId` / `SpanId` |
| syslog (RFC 5424, RFC 3164) and journald (`journalctl -o json`) | `syslog`, `.log`, `.json` | `imt-plugin-syslog` | Severity is mapped to IMT levels |
| log4j / log4net / NLog XML events | `.xml`, `.log` | `imt-plugin-formats` | `XMLLayout` and `Log4JXmlEventLayout`; one malformed event costs only that event |
| SQL Server `ERRORLOG` | `ERRORLOG` | `imt-plugin-formats` | UTF-16 or UTF-8; error number, severity and state become fields |
| Docker `json-file` and Kubernetes CRI container logs | `*-json.log`, `*.log` | `imt-plugin-formats` | Partial lines are joined; `Stream`, pod, namespace and container are fields |
| CBS / DISM text logs | `CBS.log` | built-in text importer | Read by the default profile; a one-line profile regex adds the component as a field |

### Opening a source file outside IMT

*Open file*, *Open at line N* and *Show in folder* (row menu, inspector, Explorer row menu) work on the
files IMT opened in the window:

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
locked is still opened or shown (IMT does not read it).

Timestamps: ISO 8601/RFC 3339, Serilog defaults, `/` and `.` dates (day-first or month-first, detected
per file), a space, `T` or comma between date and time (`2025/11/08,00:04:37.759+01:00`), a dot or comma
before the fraction (`00:04:37,759`), month names, 12-hour clocks, epoch seconds/milliseconds, time-only
console logs, and your own .NET (`dd/MM/yyyy HH:mm:ss.fff`, `HH:mm:ss,fff`) or strftime formats. Timezone
for offset-less times is configurable.

**Timestamps not recognised.** When IMT reads a file's lines but not the timestamps of more than 5 % of
its events, or of none of its first 20, the file gets a warning icon in the Explorer (its tooltip counts
the lines and quotes one) and a message in the Notifications centre. Files without any date-like text
(settings files, data tables) and formats read by plugins are not flagged. IMT also looks at the
lines for a date and time it can describe (`Sat Nov 08 00:04:37 2025` → `%a %b %d %H:%M:%S %Y`) and
proposes that format only after checking that it reads them. **Fix timestamps** (the message's button,
or a click on the icon) opens the format wizard on the proposed format: the preview shows how the file
reads with it, and **Save and re-import** adds it to the profile's `timestamp_formats`. The message is
posted once per file; a reload does not repeat it.

## Profiles

A profile only needs the keys that differ from the built-in default. The full schema, with every key
documented, is [`crates/imt-core/src/default_profile.toml`](../crates/imt-core/src/default_profile.toml);
a worked example is [`samples/profiles/orderservice.imtprofile.toml`](../samples/profiles/orderservice.imtprofile.toml).

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

A file may mix formats: IMT reads every line with the first output template or regex that matches it
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
[`samples/profiles/logserver.imtprofile.toml`](../samples/profiles/logserver.imtprofile.toml) for
`samples/linux/logserver01/apps-20261014.log`. Design: [ADR 0020](adr/0020-mixed-sources.md).

## Query language

Press `Ctrl/⌘+/` to focus the query bar and `Ctrl/⌘+Enter` to run. Keywords are case-insensitive.
A query is an optional filter followed by `|` stages.

| Syntax | Meaning |
|---|---|
| `timeout` or `"connection reset"` | full text over message, exception and template |
| `level >= warning` | compare a field (`= != > >= < <=`); levels are ordered |
| `RequestPath contains "/capture"` / `!contains` | substring |
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
[`crates/imt-core/src/ql/mod.rs`](../crates/imt-core/src/ql/mod.rs).

## Internal web browser

*Search the web for this error* and the address box of the **Browser** panel (Tools ▸ Browser) open pages in
the panel itself. It docks like any other panel: drag its tab next to Timeline or Events, split it, maximise it
or pop it out into a window of its own. Searches use **Google** unless you pick another engine: Settings ▸ Web
browser ▸ Search engine sets it for every search (DuckDuckGo, Bing, or *Custom* with your own
address containing `{q}`), and the picker next to the address box or in the search dialog changes it for one
search.

**The search text.** For *Search the web for this error* and *Research this*, the AI writes the search from your
question and the event: it keeps the exception type, the decisive words of the message, error codes and the
framework or library with its version, and leaves out ids, GUIDs, host, machine and user names, paths, ports,
timestamps and other values of your installation. IMT then checks it like any search text: in *Search the web*,
what looks private or comes from your logs is taken out; in *Research this*, it is highlighted and you confirm
or edit it (*Research anyway*). Either way you see the text and can change it before anything is searched;
*Let the AI write it* drafts it again. Without a model, IMT builds the text itself, as before.

**Keyboard.** Ctrl+L (⌘+L) puts you in the address box, also while you are in the page; Esc in the page gives the
keyboard back to IMT; IMT's own shortcuts (Ctrl+Shift+…, Alt+…, F1) work while you are in the page. On macOS and
Linux, keys typed in the page stay with the page: click into IMT first.

**Menus and dialogs.** The page is drawn by the system's web view, above IMT. While a menu, dialog, toast or the
drag guides of a panel would cover it, the page steps aside and the panel says so; it comes back when they close.

When a chat's **agent mode** or a **research** drives the browser, the Browser panel says so in its header:
*AI is browsing*, what the AI does (the step, or the result page it reads) and **Stop**; its tab reads
"Browser · AI is browsing". **Stop** ends agent mode for the chat (a step waiting for your approval is denied) or
cancels the research, the same as Stop in the chat. It goes away as soon as the AI lets go of the browser: after
Stop, when the answer or the research is done, cancelled or failed, when you turn agent mode off, or when you
close the page or its IMT window. The header is part of IMT, not of the web page: a page cannot show, hide or
press it. When the AI opens a page, the Browser panel opens without taking the focus.

On Linux the page opens in a window of its own (the system's web view cannot be placed inside the panel there);
the panel keeps the address box, the notices and *AI is browsing* with Stop.

## Knowledge files: teach the AI your systems

Like `AGENTS.md` for coding assistants, **knowledge files** are Markdown notes the AI reads with every question
(chat, research, explanations, folder set-up, query drafting, images): the names of your products and services,
what each log source is, known errors and their fixes, facts about your environments. Manage them in
*Settings ▸ AI ▸ Knowledge*:

- **This project** holds files saved with the project (and the autosaved session), so a shared project carries
  them. **Every project** holds files kept in IMT's settings folder on this computer.
- **New…** writes a file in the editor (with a *Preview*), **Import…** adds an existing `.md` or `.txt` file
  (named after it), **Edit…** renames it or changes its text, **Export…** saves it as a `.md` file to share, and
  **Remove…** deletes it after asking. The checkbox turns a file off without deleting it.
- A file holds at most 64 KiB, a scope at most 32 files. The AI reads the project's files first, then the global
  ones, within about a quarter of the model's context window; when they do not all fit, the rest is cut and the AI
  is told which files were left out, so shorten or turn off what it does not need. They are background facts for
  the AI: they never replace IMT's own rules, and their text is never written to IMT's logs.

## AI runtime and model updates

IMT checks for newer **inference runtimes** (the llama.cpp builds that run the model) and newer revisions or
quantisations of your **models**. Only what IMT's built-in lists pin is ever offered: a runtime update is the
newer llama.cpp release this version of IMT pins (each file checked against its SHA-256), a model update is a
model-list entry that replaces a file you have, confirmed with the model host before it is offered.

*Settings ▸ AI* sets what happens for each kind:

- **Automatic** downloads and verifies the update in the background, as a job in the Jobs pane. The installed
  version keeps working until the new one is verified, and a running chat is never interrupted: an update
  found during a chat waits for it to finish, and a new model replaces the active one only between chats.
- **Ask** (the default) shows a message with **Update**, **Later** (ask again at the next check) and **Skip
  this version** (never offer this version again; *Update* in Settings still installs it).
- **Manual** only checks when you choose **Check now** in *Settings ▸ AI ▸ Updates*.

*Check for AI updates every (hours)* sets how often IMT checks (daily by default). Settings shows when it last
checked and when it checks next. A check or update that fails because the computer is offline or a proxy
blocks it does not interrupt you: the reason is shown in Settings and IMT tries again later (after 5 minutes,
then less and less often, at most once per interval). When a proxy intercepts HTTPS, **Download manually**
there works as described under [Troubleshooting](#behind-a-corporate-proxy-zscaler-etc).

## Keyboard shortcuts

`Mod` is `Ctrl` on Windows/Linux and `⌘` on macOS.

| Shortcut | Action |
|---|---|
| `Mod+K` | Command palette (every action is in it) |
| `F1` | Keyboard shortcuts (searchable, printable; generated from the commands) |
| `Mod+Z` / `Mod+Shift+Z` | Undo / redo the last edit (filters, marker and highlight rules, profile edits, dashboards, reports) |
| `Mod+O` / `Mod+Shift+O` | Open files / open folder |
| `Mod+/` | Focus the query bar |
| `Mod+Enter` | Run the query (in the query bar) |
| `Mod+Shift+M` | Analyse this moment |
| `Mod+Shift+N` | New window |
| `Mod+S` / `Mod+Shift+S` | Save project / save project as |
| `Mod+D` | Add a note to the selected event |
| `Mod+Shift+E` | Export events |
| `Mod+Shift+X` | Toggle the redacted view |
| `Mod+B` | Toggle the sidebar |
| `Mod+,` | Settings |
| `Alt+←` / `Alt+→` | Previous / next time range |
| `↑` / `↓` | Move through the events grid |
| `Esc` (while dragging a tab) | Cancel the drag |

Browser keys such as `F5`/`Mod+R` (reload), `Mod+F` (find, unless IMT binds it), `Mod+P` (print),
`Mod` `+`/`-`/`0` (page zoom) and `Alt+←`/`Alt+→` as browser back/forward do nothing in IMT's windows
(IMT's own bindings, such as the time-range history above, still work). Release builds also turn off
the developer tools (`F12`, `Mod+Shift+I`).

To float a panel, drag its tab off the docking guides and release it where the **Release to float**
ghost shows; release it outside the window (**Release to open in a new window**) to pop it out.

## Command line

The desktop app takes paths to open at startup and a few flags:

```sh
imt-app <folder-or-files...>        # open files and folders (in the running IMT, if there is one)
imt-app --new-window <paths...>     # open them in a new window of the running IMT
imt-app --new-instance <paths...>   # start a separate IMT process instead
imt-app --safe-mode                 # separate process, no plugins, no saved profile (also: IMT_SAFE_MODE=1)
```

IMT runs once per user: a second launch hands its paths to the running instance (at most 256),
which opens them as set in *Settings → Window & tray* and focuses its window. Paths that do not
exist are ignored; the rest are canonicalised.

Two headless tools ship in the workspace for servers and customer machines:

```sh
imt-collect [--plan plan.json|plan.toml] [--out dir] [--print-default-plan]
imt-monitor --config <monitor.json> --out <dir> [--duration <10m|30s|2h|1500ms>]
```

`imt-collect` writes an `.imtz` bundle that IMT opens directly; `imt-monitor` writes rolling CLEF files.
Run them from a checkout with `cargo run -p imt-collect -- --help` or `cargo run -p imt-monitor -- --config monitor.json --out ./logs`.

## Troubleshooting

### Behind a corporate proxy (Zscaler etc.)

Model and runtime downloads use HTTPS and check the server's certificate against the operating system's
certificate store, like your browser. Security tools that inspect HTTPS (Zscaler, Netskope, Forcepoint,
Palo Alto and similar) replace the certificate with one of their own. If their root certificate is in
Windows' certificate store (*Manage computer certificates* ▸ *Trusted Root Certification Authorities*;
IT usually installs it), IMT's downloads work without anything to set. If it is not, the download stops
with *Certificate problem: a proxy or security tool on your network intercepts HTTPS*. Then either:

- ask IT to install the proxy's root certificate in Windows' store (on macOS: the System keychain; on
  Linux: the system store, e.g. `update-ca-certificates`) and choose *Try again*; or
- choose **Download manually** in the error: IMT opens the exact file in your browser and shows its
  expected SHA-256.
  - For a **model**, choose *Add downloaded file…* (or *Add local GGUF…* in *AI: Manage models*) and
    pick the `.gguf` file. IMT checks its SHA-256 against the catalog before adding it, even if the
    browser renamed it, and rejects an incomplete or changed file.
  - For an **inference runtime**, download every file listed (CUDA builds have two), choose *Add
    downloaded files…* and pick them all without unzipping them. IMT checks each SHA-256 and unpacks
    the build into the folder shown. Alternatively set *Custom llama-server* in *Settings ▸ AI* to a
    `llama-server` IT provides.

If your network needs an explicit proxy, set *Download proxy* in *Settings ▸ AI* (`http://host:port`,
or `http://user:password@host:port` when the proxy asks for a sign-in); IMT does not read proxy
environment variables. The other errors say what to check: *the server cannot be reached* (offline or
the name does not resolve), *did not answer in time*, or *refused the download* (an HTTP status; a 403
can come from a proxy that blocks the site).
