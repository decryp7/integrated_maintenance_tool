# Command line reference

The command line runs the same engine and store worker as the app, without a window: import files and folders,
run query-language queries, run the analyses and build the incident report as HTML or PDF. It is meant for scripts,
CI and headless machines. The executable is called `debrief`; the store worker (`debrief-store-worker`) and libduckdb must sit next to it, or be named by
`DEBRIEF_STORE_WORKER` and `DEBRIEF_LIB_DIR`.

```text
debrief import PATH... [SOURCE OPTIONS] [--json]
debrief query QL [SOURCE OPTIONS] [--format table|csv|json] [--json] [--limit N]
debrief analyze anomalies|patterns|exceptions|issues|outliers [SOURCE OPTIONS] [--filter QL] [--limit N] [--json]
            [--knowledge FILE]... [--fail-on-findings] [--subset QL] [--fields A,B]
debrief watch DIR [--store DIR] [--knowledge FILE]... [--interval SECS] [--debounce SECS] [--once] [--json]
            [--fail-on-findings]
debrief report --out FILE [SOURCE OPTIONS] [--format html|pdf] [--title TEXT] [--knowledge FILE]...
            [--from TIME] [--to TIME] [--top N] [--overwrite] [--json]
debrief help | --version
```

## Source options

Every command that reads data takes these.

| Option | Meaning |
|---|---|
| `--open PATH` | A file or folder to import (repeatable). `import` also takes them as plain arguments. Folders are read recursively with the default profile's include and exclude globs. |
| `--since TIME`, `--until TIME` | Import only events from `TIME` / up to `TIME` (both inclusive; UTC unless an offset is given). Events outside the range are dropped before the pipeline and the store, so one hour of a very large folder costs one hour of memory and disk. Files last modified more than 26 hours before `--since` are not opened at all. Events without a readable time are left out when a range is set. Applies to files read in this run (a kept store keeps what it already holds). |
| `--store DIR` | Keep the store in `DIR`. A later run with the same `--store` reuses it: files already read are skipped, so `debrief import` once and then query as often as you like. Without it a scratch store is made in the temporary folder and removed when the command ends. |

Paths are canonicalised; a path that does not exist is an error naming it. A file no importer can read is skipped with a
note on standard error (and the command fails only when nothing at all could be read from a scratch store).

## Commands

**`import`** reads the paths into the store and prints how many events and sources it read. Use it with `--store` to
prepare a store for later runs.

**`query`** runs a [query-language](user-guide.md#query-language) text. A query that returns events prints
`id, time, level, source, message` (plus the columns of a `fields` stage); a query ending in `stats` prints its table.
`--format table` (default) aligns columns, `csv` writes RFC 4180 CSV and `json` (or `--json`) one JSON object.

**`analyze`** runs one analysis over the events matching `--filter` (a query-language filter; all events by default):
`anomalies` (error spikes, silence, crashes, slow outliers, rate changes, memory growth), `patterns` (message templates
by count), `exceptions` (grouped by fingerprint), `outliers` (values over-represented among errors, or among the events
selected by `--subset QL`, against the rest of the window; `--fields A,B` limits the fields examined; each outlier has
its counts, lift, p-value and evidence ids) and `issues` (the known issues declared in the Markdown
`--knowledge` files that occur in the data, with their counts and fixes). Each result says how much of the data it
covered; limits that bounded it are printed on standard error.

**`watch`** imports files as they appear in `DIR` (recursively, with the default profile's include and exclude
globs), until Ctrl+C or SIGTERM. A file is imported once its size and modification time have not changed for
`--debounce` seconds (default 2), so a file still being copied is not read half-written; names such as `*.part`, `*.tmp`,
`*.crdownload`, `~$*` and dotfiles and empty files are ignored; symbolic links are not followed. A file that is locked or
unreadable is retried with growing delays and given up after four attempts (a changed file gets a new chance); a file over
4 GiB is reported once and left alone; a file that vanishes is forgotten. A file that grows later is refreshed. The folder
is scanned every `--interval` seconds (default 5). With `--knowledge FILE`, each time something was imported the known
issues are matched and a new one (or one with more events) is announced once. Output is one line per event, or one JSON
object per line with `--json` (`imported`, `skipped`, `issue`). With `--once` the folder is settled and the command ends;
`--fail-on-findings` then exits 3 when a known issue was announced. Use `--store DIR` to keep what was imported.

**`report`** builds the incident summary (period, anomalies, exceptions, top errors, sources, import problems, matched
known issues) and writes it to `--out` as self-contained HTML or PDF (the format follows the extension unless
`--format` is given). `--from`/`--to` narrow the period, `--top` sets the entries per ranking (1 to 10). An existing
file is never replaced unless `--overwrite` is given; the file is written to a temporary name and renamed, so a failed
run leaves no half-written report.

Times (`--from`, `--to`) are `2026-10-01`, `2026-10-01 14:30` or `2026-10-01T14:30:00Z`, read as UTC unless they carry an
offset.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Done. |
| 1 | The work failed: a file, the store, a query, the output. The message says what to fix. |
| 2 | The command line was wrong (usage is printed). |
| 3 | Done, and `--fail-on-findings` found something (anomalies, exceptions or known issues). For CI gates. |

## JSON

With `--json` every command prints exactly one JSON object on standard output, and nothing else; notes and progress go
to standard error. Success objects have `"ok": true`; a failure prints `{"ok": false, "error": "..."}` and exits with
the failure code. `query` has `columns`, `rows` (objects keyed by column), `total` and `truncated`; `analyze` has
`kind`, `total`, `truncated`, `matched`, `analysed`, `limits`, `warnings` and `items` (the engine's own result objects,
in the same camelCase shape as the app's); `import` has `events`, `bytes`, `sources` and `unrecognized`; `report` has
`path` and `bytes`.

## Bounds

Output is always bounded: a query prints at most `--limit` rows (1000 by default, 100,000 at most), an analysis at most
`--limit` items (50 by default), and a cell is cut at 500 characters. When a result is cut, standard error says how many
rows there were. Knowledge files are limited to 4 MB. The store worker caps its own memory like the app's.

## Examples

```sh
# Prepare a store once, ask many questions
debrief import /var/log/myapp --store ./incident
debrief query 'where level = "Error" | stats count() by source' --store ./incident --format csv
debrief analyze exceptions --store ./incident --limit 10

# A CI gate: fail the build when a known issue shows up in the test logs
debrief analyze issues --open build/logs --knowledge docs/known-issues.md --fail-on-findings --json

# A report for the ticket
debrief report --open /var/log/myapp --out incident.pdf --title "Checkout outage" --from 2026-10-14 --to 2026-10-15
```
