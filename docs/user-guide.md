# User guide

Reference for everyday use: which files IMT reads, how to write a profile, the query language, the
keyboard shortcuts and the command line. For what IMT can do, see [features.md](features.md); for a
first walk-through, see the two-minute tour in the [README](../README.md#quick-start).

- [Supported inputs](#supported-inputs)
- [Profiles](#profiles)
- [Query language](#query-language)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Command line](#command-line)
- [Troubleshooting](#troubleshooting)

## Supported inputs

The format is detected from the content. UTF-8 and UTF-16 are both read. Gzip files and zip archives
are detected by magic bytes and expanded (with size caps against zip bombs).

| Format | Typical files | Provided by | Notes |
|---|---|---|---|
| Serilog CLEF, Serilog `JsonFormatter`, JSON lines, whole-file JSON arrays (NLog, ECS, pino, ...) | `.clef`, `.json`, `.log` | `imt-core` JSON importer | Keys are configurable in `[parse.json]`; nested objects are flattened |
| Serilog text and console output | `.log`, `.txt` | `imt-core` text importer | Described by the same `outputTemplate` you use in `appsettings.json` |
| log4net / NLog pattern layouts and any text via regex | `.log`, `.txt` | `imt-core` text importer | Named groups in `[parse] regex`; multi-line messages and stack traces attach to their event |
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
per file), month names, 12-hour clocks, epoch seconds/milliseconds, time-only console logs, and your own
.NET (`dd/MM/yyyy HH:mm:ss.fff`) or strftime formats. Timezone for offset-less times is configurable.

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

## Keyboard shortcuts

`Mod` is `Ctrl` on Windows/Linux and `⌘` on macOS.

| Shortcut | Action |
|---|---|
| `Mod+K` | Command palette (every action is in it) |
| `F1` | Keyboard shortcuts (searchable, printable; generated from the commands) |
| `Mod+Z` / `Mod+Shift+Z` | Undo / redo the last edit (filters, marker and highlight rules, bookmarks, profile edits, dashboards, reports) |
| `Mod+O` / `Mod+Shift+O` | Open files / open folder |
| `Mod+/` | Focus the query bar |
| `Mod+Enter` | Run the query (in the query bar) |
| `Mod+Shift+M` | Analyse this moment |
| `Mod+Shift+N` | New window |
| `Mod+S` / `Mod+Shift+S` | Save project / save project as |
| `Mod+D` | Bookmark the selected event |
| `Mod+Shift+E` | Export events |
| `Mod+Shift+X` | Toggle the redacted view |
| `Mod+B` | Toggle the sidebar |
| `Mod+,` | Settings |
| `Alt+←` / `Alt+→` | Previous / next time range |
| `↑` / `↓` | Move through the events grid |

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
