# IMT — Integrated Maintenance Tool

An IDE for analysing software logs and diagnostic files. Open a folder of logs
from many machines and processes, and IMT merges them into one timeline per
instance (start, stop, crash, exception, error and warning markers), a
filterable event grid, an event inspector and metric charts, all driven by an
editable parsing profile.

Built with Rust (engine + Tauri 2 desktop shell) and Angular 22 (UI).

## Layout

| Path | What |
|---|---|
| `crates/imt-core` | Engine: importers, parsing profiles, query, timeline and chart aggregation |
| `app/` | Angular UI (`app/src`) and the Tauri shell (`app/src-tauri`) |
| `app/src/app/core/api.types.ts` | The UI ↔ backend contract (Tauri commands and payloads) |
| `samples/` | Generated Serilog, log4net and CSV samples plus a sample profile |
| `src/` | The original `imt` command-line filter |
| `.gitea/workflows/build.yml` | CI: tests, lint, installers for Linux/Windows/macOS, releases on `v*` tags |

## Supported input

The format is detected from file content, not the extension (`billing.log.2026-10-14`,
`.txt`, `.clef`, files without an extension...). UTF-8 and UTF-16 are both read.

- **JSON:** Serilog CLEF (`@t/@mt/@l/@x`), Serilog `JsonFormatter`, rendered compact JSON,
  and most other JSON loggers (NLog, ECS, pino, ...), as JSON lines or a whole-file array.
- **Text:** Serilog file and console output, described by the same `outputTemplate` you put in
  `appsettings.json`; log4net/NLog pattern layouts; anything else via a regex with named
  groups. Stack traces and multi-line messages are attached to their event.
- **Delimited:** CSV, `;`, tab or `|` separated, with the delimiter and columns detected
  automatically.
- **Timestamps:** ISO 8601/RFC 3339, Serilog defaults, `/` and `.` dates in day-first or
  month-first order (detected per file), month names, 12-hour clocks, epoch seconds and
  milliseconds, time-only console logs, plus your own .NET (`dd/MM/yyyy HH:mm:ss.fff`) or
  strftime formats.

Files nothing can read (e.g. `.reg`, msinfo `.nfo`) are listed as unrecognised. They are the
place for future importer plugins: implement `imt_core::importers::Importer` and register it
in `Importers::new`.

## Parsing profiles

A profile is a TOML pipeline: **files → parse → instance → extract → markers → charts**. The
built-in default is `crates/imt-core/src/default_profile.toml`, which documents every key, and
`samples/profiles/orderservice.imtprofile.toml` is a worked example. A profile only needs the
keys that differ from the default. Edit it in the app's Profile tab (form or TOML, with live
extractor preview).

```toml
[instance]
key = ["MachineName", "ProcessId"]          # what one lane on the timeline is

[[extract]]                                  # turn text into numeric fields
name = "process-stats"
regex = 'CPU (?P<cpu>[\d.]+)% .* WorkingSet (?P<ws>[\d.]+) ?(?P<ws_unit>KB|MB|GB)'
when = [{ field = "SourceContext", op = "eq", value = "Diagnostics.ProcessMonitor" }]
[extract.fields]
cpu = { type = "number" }
ws = { type = "bytes", unit_from = "ws_unit", to = "MB" }

[[chart]]
title = "CPU %"
y = "avg(cpu)"
bucket = "1m"
split_by = ["MachineName", "ProcessId"]
```

Extracted values become fields on the event, so they work everywhere a Serilog property does:
grid columns, filters, group-by, marker rules and charts.

## Development

Requirements: Rust (stable), Node 24 (≥ 24.15, see `app/.nvmrc`), and on Linux the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
cargo test -p imt-core            # engine tests, including end-to-end runs over /samples
cd app && npm ci
npm start                         # UI only, in a browser, against a built-in mock backend
npx tauri dev                     # the desktop app with the real engine
npx tauri build                   # installers in target/release/bundle
```

`imt-app <folder-or-files...>` opens paths at startup. Regenerate the samples with
`python3 samples/generate.py`.

The original CLI still works: `cargo run -p imt -- <file-or-folder>` writes `*-filtered.log`
files using the regexes in its `confy` config.

## CI

`.gitea/workflows/build.yml` runs on Gitea Actions with runners labelled `ubuntu-latest`,
`windows-latest` and `macos-latest`: engine tests, clippy and the Angular build, then
`.deb`/`.rpm`, `.msi`/NSIS `.exe` and `.dmg` installers uploaded as run artifacts. Pushing a
`v*` tag also creates a Gitea release with the installers, which needs a `RELEASE_TOKEN`
repository secret.
