# Command line reference

The command line runs the same engine and store worker as the app, without a window: import files and folders,
run query-language queries, run the analyses and build the incident report as HTML or PDF. It is meant for scripts,
CI and headless machines. The executable is called `imt` for now (one constant in `crates/imt-cli`, changed when the
product is renamed); the store worker (`imt-store-worker`) and libduckdb must sit next to it, or be named by
`IMT_STORE_WORKER` and `IMT_LIB_DIR`.

```text
imt import PATH... [SOURCE OPTIONS] [--json]
imt query QL [SOURCE OPTIONS] [--format table|csv|json] [--json] [--limit N]
imt analyze anomalies|patterns|exceptions|issues [SOURCE OPTIONS] [--filter QL] [--limit N] [--json]
            [--knowledge FILE]... [--fail-on-findings]
imt report --out FILE [SOURCE OPTIONS] [--format html|pdf] [--title TEXT] [--knowledge FILE]...
            [--from TIME] [--to TIME] [--top N] [--overwrite] [--json]
imt help | --version
```

## Source options

Every command that reads data takes these.

| Option | Meaning |
|---|---|
| `--open PATH` | A file or folder to import (repeatable). `import` also takes them as plain arguments. Folders are read recursively with the default profile's include and exclude globs. |
| `--store DIR` | Keep the store in `DIR`. A later run with the same `--store` reuses it: files already read are skipped, so `imt import` once and then query as often as you like. Without it a scratch store is made in the temporary folder and removed when the command ends. |

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
by count), `exceptions` (grouped by fingerprint) and `issues` (the known issues declared in the Markdown
`--knowledge` files that occur in the data, with their counts and fixes). Each result says how much of the data it
covered; limits that bounded it are printed on standard error.

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
imt import /var/log/myapp --store ./incident
imt query 'where level = "Error" | stats count() by source' --store ./incident --format csv
imt analyze exceptions --store ./incident --limit 10

# A CI gate: fail the build when a known issue shows up in the test logs
imt analyze issues --open build/logs --knowledge docs/known-issues.md --fail-on-findings --json

# A report for the ticket
imt report --open /var/log/myapp --out incident.pdf --title "Checkout outage" --from 2026-10-14 --to 2026-10-15
```
