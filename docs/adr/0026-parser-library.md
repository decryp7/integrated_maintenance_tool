# 26. Parsers are individual TOML files in a library, matched per file

- Status: accepted (2026-10-21)
- Date: 2026-10-21
- Related: [ADR 0014](0014-database-first-engine.md) (sources record their parser, "Prepared for ADR 0026"),
  [ADR 0020](0020-mixed-sources.md) (per-line formats inside one file stay)

## Context

The profile held everything: which files to read, **how to parse them** (`[parse]`: format, templates, regexes,
timestamp formats, JSON keys, CSV columns) and how to analyse the events (extractors, markers, pipeline, charts,
spans, routes). One workspace had one profile, so one set of formats for every file. Users asked for parsers as
files they can share ("are all the parsers stored as toml for easy sharing?"), a library they manage in Settings
(add, disable, remove, update), automatic detection of which parser reads which file ("at any time we may analyze
different set of files") and export/import between users. Profile templates (`templates/*.toml`) were whole
profiles chosen once for a workspace, not per file.

## Decision

### 1. A parser is one TOML file describing one log format

```toml
schema = 1                       # parser schema version; other versions are refused with a clear message
id = "log4net-csv"               # [a-z0-9][a-z0-9._-]{0,63}, unique in the library
name = "log4net CSV (no header)"
version = "1.0.0"                # semver: decides "Update" on import
description = "..."
category = ".NET"                # grouping in Settings ▸ Parsers

[match]                          # how IMT detects files in this format (all optional)
files = ["*.csv", "*.log"]       # globs on the file name or the path relative to the opened folder
sniff = '^\d{4}/\d{2}/\d{2},'    # regex that some of the first `lines` non-empty lines must match
lines = 20                       # 1..=200
json_keys = ["@t", "@mt"]        # keys the first JSON object must have
csv_columns = 7                  # columns of the first lines (with [format.csv] delimiter)
priority = 0                     # tie-breaker, -1000..=1000

[format]                         # the former profile [parse] section (same keys; `format` is now `kind`)
kind = "csv"                     # auto | text | json | csv | an importer id (e.g. a plugin's)
identity = ["Process"]           # fields naming who wrote an event: join the instance candidates
# output_templates, regexes, timestamp_formats, assume_timezone, day_first, [format.json], [format.csv]

[[extract]]                      # optional, format-specific (same schema as the profile's)
[[pipeline]]                     # optional, format-specific (e.g. syslog's <pri> to level)
```

The types are the profile's former ones (`ParseCfg` became `FormatCfg`, `JsonCfg`, `CsvCfg`, `DayFirst`), moved to
`imt_core::parser`. Parsing a parser file is the edge: unknown keys are refused, errors carry line and column,
files over 256 KiB, more than 64 text formats and regexes over the `user_regex` limits are refused. Nothing in a
parser is executed except the regexes and pipeline steps the profile could already hold.

### 2. The library

- **Built-in parsers** ship in the engine (`crates/imt-core/parsers/*.toml`): `generic` (the formats IMT
  understood out of the box, kind `auto`, the fallback; it also reads CSV with a header and plain timestamped
  text, so those two templates have no parser of their own), the former profile templates as parsers (Serilog
  CLEF/JSON/text, log4net pattern, NLog, MEL console, Apache error, logcat, Hadoop/Spark) and `log4net-csv`
  (log4net CSV layouts without a header). Formats a plugin reads (syslog, IIS W3C, access logs, log4net/log4j XML,
  Windows EVTX) are parsers whose `kind` names the plugin's importer: they carry the identity fields and show
  in the library which files they read; without the plugin they match nothing and the generic parser applies.
  Built-ins are read-only; each can be disabled. The templates' charts and status-to-level pipeline steps were
  dropped (the plugins set levels themselves).
- **User parsers** are files `<config>/parsers/<id>.toml` (beside the profiles). A user parser with a built-in's
  id **overrides** it ("Edit" on a built-in saves such a copy; removing the copy restores the built-in).
- **The index** `<config>/parsers/library.json` (`{ "version": 1, "disabled": [ids] }`) holds the enabled flags.
  A missing or corrupt index means "all enabled" (logged); an index of a newer version is ignored the same way.
  A user file that does not parse is listed as broken with its error and never used.

### 3. Matching per file

For every file, IMT scores each **enabled** parser on the file's path and first lines:

- `files` globs, when given, must match (+20);
- each content rule given must hold: `sniff` (+40 scaled by the share of the first `lines` it matches),
  `json_keys` (+30; the first line is a JSON object with the keys and no line is a text event, so JSON mixed
  with text stays with the text importer as ADR 0020 says), `csv_columns` (+30);
- the format itself must read the head: a `text` parser scores 100 when its templates/regexes read every line of
  the first 50 that starts an event (or looks like it), else only `40 × share` (so a parser that misses event
  lines yields to the generic one, which reads them); other kinds score their importer's sniff (0..=100), and a
  parser naming an installed importer with a rule that held reads the file even when that importer's sniff does
  not recognise it.

The highest score wins; ties go to the higher `priority`, then user parsers before built-ins, then the id. The
`generic` parser has priority -100, so any specific parser that reads a file as well wins. A file no enabled parser
reads is unrecognised with the reason "no enabled parser reads this file". Within a file, ADR 0020's per-line
formats still apply (a parser's templates and regexes are tried per line).

Each source records the parser that read it (id, version, content hash: ADR 0014 preparation). The user can
override the choice per source ("Parse with…", any parser of the library including disabled ones, or
"Automatic"); the store keeps the overrides by source path and re-reads only that source.

### 4. Profile vs parser

The **parser** says how a file's bytes become events (format, timestamps, levels, identity, format-specific
extractors and pipeline steps). The **profile** says how the workspace analyses events whatever their format
(which files to collect, instance key, extractors, markers, pipeline, charts, dashboards, spans, routes,
redaction, live). The profile's `[parse]` section is **removed** (no compatibility shim; a profile that still
has one is refused with a message naming Settings ▸ Parsers). A parser's `[[extract]]` and `[[pipeline]]` run
before the profile's; its `identity` fields join `[instance] candidates`. Typed columns of the store are declared
by the profile's extractors only; fields of a parser's extractors are stored like other dynamic fields.

The profile templates gallery is superseded and removed: the formats became built-in parsers, their identity
fields went into the parsers, and their few charts were dropped (they are one line of profile each).

### 5. Engine shape

- `CompiledProfile` is now "the profile compiled with one parser" (`compile_for(profile, parser)`);
  `CompiledProfile::compile(profile)` uses the built-in `generic` parser (tests, previews).
- `ParserSet` (the library as the engine sees it: parsers, enabled flags, overrides) compiles each parser with
  the profile lazily, once, and chooses per file. Both import paths use it: the store's streaming ingest and the
  in-memory `Workspace` (still the differential test oracle).
- The `Profiles` port gains `set_parsers(ParserSetup)`: the store swaps the set and returns the sources whose
  recorded parser is no longer the one the set would use by id and hash (changed, disabled or removed); the app
  offers to re-read them. `assign_parser(source, parser_id)` is implemented (re-reads that source). Over the
  worker this is one more operation at the end of the table.
- Previews (`preview_parse`) take a parser TOML. The format wizard, timestamp quick fixes and diagnostics fixes
  edit a parser: `parser_draft` (Rust) composes a new parser or a changed copy of the source's parser from the
  wizard's regex, template or timestamp format; `QuickFix.parserOverride` is a `[format]` fragment merged over
  the parser of the affected sources.

### 6. Sharing

Export writes the parser file as stored: self-contained, with its `schema` line. Import reads the file through
the path grants, validates it in Rust (typed errors with line/column, unknown schema versions refused with "this
parser was written for parser schema N; this IMT reads schema 1 — update IMT"), and compares it with the
library: **new**, **newer** (Update: the versions' TOML diff is shown and confirmed), **same** version or
**older** (replacing needs confirmation).

## Consequences

- Settings ▸ Parsers lists built-in and user parsers with their enabled toggle, version and the sources of the
  current window each one read; adds parsers (format wizard, from a file, import), edits them in a TOML editor
  with Rust validation and a live preview against a chosen file, duplicates, removes, exports and updates them.
- Every profile change no longer touches formats, and a parser change re-reads only the sources that parser read.
- `default_profile.toml` no longer documents formats; the built-in `crates/imt-core/parsers/generic.toml`
  documents the parser schema (as `default_profile.toml` documents the profile's).
- Store format: unchanged (the parser columns came with ADR 0014 6c); overrides live in the store's `meta`.
