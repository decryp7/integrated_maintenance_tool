# 20. Mixed sources: per-line formats, logical sources and per-source instance keys

- Status: accepted (2026-10-20)
- Date: 2026-10-20
- Related: [ADR 0014](0014-database-first-engine.md) (the streaming ingest path, import problems), [ADR 0004](0004-stable-plugin-abi.md)
  (the plugin contract, extended additively), [ADR 0006](0006-clock-skew-applied-on-load.md) (clock offsets)

## Context

One log file often holds more than one writer: several processes of one product, or several products when a log
server (rsyslog, a shared network folder) collects them. IMT assumed one writer per file:

1. **One format per file.** `importers/text.rs::best_format` picked the format matching most of the first 200
   lines; every later line that did not match it was appended to the previous event as a continuation line, so a
   second product's lines silently became part of the first product's events (losing their time, level and
   fields). The JSON importer looked at the first line only, so a text banner followed by JSON lines was read as
   text and the JSON lines were glued to the banner; JSON lines inside a text file were continuation lines.
2. **Identity fields of other formats were not candidates.** Syslog and journald write `Host`, `App`, `ProcId`;
   EVTX `Computer`, `ProcessId`; containers `Namespace`, `Pod`, `Container`. `[instance] candidates` knew none of
   them, so a log server's syslog became one lane.
3. **The automatic instance key was global.** A candidate counted when present on at least half of **all**
   events, so a field only one product writes (or a product writing few events) was dropped and that product's
   processes merged into one lane.
4. **One profile for the whole file.** Products sharing a file could not be told apart for grouping or labels.

Both import paths are affected: the current app path (`importers/*` + `Workspace`) and the streaming path
(`crates/imt-core/src/ingest/*`, ADR 0014) that the app switches to in phase 6c.

## Decision

### 1. Every line is read with its own format (both import paths)

`imt_core::importers::lines` (pure, shared by `TextImporter` and `ingest::text`) classifies each line on its own:

1. the **first text format in profile order** that matches starts an event (output templates first, then
   regexes, as listed);
2. else a **one-line JSON object with a timestamp key** (`[parse.json] timestamp`) is a JSON event, converted
   exactly as the JSON importer converts it;
3. else a line that does **not look like an event start** continues the event above (indented lines, stack
   frames, `Caused by`, `---`, braces of a pretty-printed object, plain wrapped text);
4. else the line looks like an event (it starts with a date, a time, a syslog priority, a month and day, or an
   epoch number) but no format reads it: it is **unmatched**. It still joins the event above, so nothing is lost
   and multi-line messages keep working, but it is reported as a new problem kind, `unmatchedLine` (warning),
   with its count and first 20 line numbers.

The file's **dominant format** (most of the first 200 lines; JSON when strictly more lines are JSON events) is
tried first as a shortcut; only the formats listed before it are then checked (`is_match`), so the profile order
holds. A line that fails the dominant format and does not look like an event start costs one cheap prefix check;
one that looks like an event goes through a `RegexSet` of all formats (compiled once per profile within the
`user_regex` size limits; formats are tried one by one if the set would be too large).

The classification is **stateless per line** (apart from the time-only midnight rollover), so a file read in
parallel chunks gives the same events as read whole (ADR 0014 §2), and `begins_event`, which moves chunk starts
to event boundaries, agrees with the parser by construction (property-tested). That is why there is no
"last matching format" cache: it would make a line's reading depend on the lines before it, and so on where a
chunk starts. The dominant format is the cache that does not.

A file whose first line is JSON but whose head also holds text events is no longer claimed by the JSON importer
(its sniff drops from 95 to 65, below the text importer's 70); the text importer reads both. Pretty-printed
multi-line JSON objects inside a mixed file are not reassembled (their lines are continuation lines); pure JSON
files keep the JSON importer.

### 2. Import problems are part of every source

`Imported` gains `problems: Problems` (the ADR 0014 model: counts per kind and at most 20 sample lines), the
streaming `EventSink` gains `problems(Problems)` for importers that aggregate them (the legacy adapter forwards
them), `SourceInfo` gains `problems: Vec<Problem>`, and `ProblemKind` gains `unmatchedLine` and `routeLimit`.
The Explorer tooltip lists them; the Problems pane (6d) reads the same kinds from the diagnostics table.

### 3. Importers declare identity fields; candidates match ignoring case

`Importer::identity_fields()` and `BinaryImporter::identity_fields()` (default: none; must not fail) name the
fields that identify who wrote an event, widest first. The guarded plugin adapters forward them (a panic yields
none and is counted against the plugin). The gRPC contract adds `repeated string identity_fields = 5` to
`ImporterCap` (additive, ADR 0004); the SDK keeps at most 8 valid names of at most 64 characters. Declared:
syslog and journald `Host`, `App`, `ProcId`; EVTX `Computer`, `ProcessId`; Docker and CRI `Namespace`, `Pod`,
`Container`, `ContainerId`. The default `[instance] candidates` add `Host`, `HostName`, `App`, `AppName`,
`ProcId`, `Pid`; candidates match field names ignoring case.

### 4. The automatic instance key is chosen per source

`imt_core::instance::instance_key` (pure; the workspace feeds it per-source field counts that its field index
now keeps per source id) chooses in two steps:

1. candidates present on at least half of **all** events (unchanged for homogeneous sets);
2. for every source none of those fields covers (a covering field varies and is present on at least half of the
   source's events): the candidates present on at least half of **that source's** events, any case variant.

Only fields with two or more values are kept; the fallbacks are unchanged. A product that is a source of its
own (a file, or a logical source after routing, §5) therefore gets lanes per process even when it writes few
events or other field names.

The key stays **one field list** for all sources, so `query::group_key` changes how a key reads: missing values
at either end are left out, and when the first field is missing the source name comes first. Lanes read like
each product's own key (`Checkout · APP01 · 3120`, `Inventory@SRV2 · 7001`, `web01 · nginx · 2211`) and keys
built from different fields cannot collide (a source-prefixed key cannot equal a key of the first field's
values). Records with none of the fields are keyed by their source, as before.

Rejected for now: a **per-source key map** (each source its own field list). It is the cleaner model, but the UI
sends the default key back as an explicit `groupBy` (`QueryStore.effectiveGroupBy`), the DuckDB read path compiles
one key expression, and both are being reworked (R2c, 6c). Follow-up below.

### 5. Routes split one file into logical sources

A profile may list `[[route]]` rules: `when` conditions (the existing condition language, as for extractors and
markers) and a `source` label template (`"{App}@{Host}"`: fields or pseudo-fields in braces, `{{`/`}}` for
braces). Routes run after extractors and the pipeline, so they can use extracted fields; the first route whose
conditions hold and whose template fields are all present and non-empty names the event's logical source. Labels
are cleaned of control characters and cut at 120 characters; a profile has at most 64 routes; a file at most 256
logical sources (further events stay in the file's own source and are reported as `routeLimit`).

**Logical sources are sources.** Everything that works per source (lanes, the source filter and colours, the
per-source instance key, clock offsets, later per-source timestamp settings) works for them without special
cases. Each has the file's `path` and `routedFrom` (the file's display name); its `group` is the file's rotation
family plus the label, so the same product in rotated copies of the file shares a group. Events no route names
stay in the file's own source (created only when it has events). File-wide counts (skipped lines, pipeline
statistics, problems, the timestamp diagnosis) go to the first source of the file. Removing one logical source
removes the file. A routed file is not tailed incrementally: a change re-reads it whole (as archive members are),
so new events find their logical sources. A file without routed events when it was opened is tailed as before;
events appended later that a route would claim stay in its own source until the file is opened again (routing in
the append path comes with the streaming path, below). The Explorer shows a split file as one expandable row (*N sources*)
with its logical sources beneath it.

Product-specific extractors, markers, pipeline steps and charts keep using `when` conditions on the fields that
tell products apart (or `@source` after routing); there are no per-product profile sections (KISS). Per-product
*parsing* settings are the one exception worth a design (follow-up: timestamps per format).

## Scenarios: what is implemented and what follows

| Scenario | Status |
|---|---|
| Per-line format matching | **Implemented** (both paths), §1 |
| Mixed JSON and text lines | **Implemented** (both paths; one-line JSON objects), §1 |
| Per-source instance key, plugin-declared identity fields | **Implemented** (two-step key, §3–4); per-source key map is a follow-up |
| Routing into logical sources | **Implemented** in the current app path; streaming path is a follow-up (below) |
| Rotated file families as one source | Follow-up |
| Interleaved multi-line events | Follow-up |
| Container wrappers with app-formatted inner text | Follow-up |
| Per-source timestamp format, day order, time zone | Follow-up |
| PID reuse after a restart | Follow-up |
| The same events imported twice | Follow-up |
| Clock skew between machines | Existing (`skew`, `shift_time`); one refinement below |

### Follow-ups, with a short design

- **Routing in the streaming path (6c).** The router (`imt_core::route`) runs in `RecordBatcher` after
  `profile.ingest`, before batches are built. `BatchSink` gains `begin_logical(parent: SourceId, label) ->
  SourceId`; a per-file `label → SourceId` map is shared by the chunk workers of one file (behind the
  `SharedSink` lock; at most 256 entries), so parallel chunks agree on ids. The `sources` table gains
  `routed_from`; `SourceReport` carries the logical sources' counts; file-wide counts stay on the first source.
- **Per-source key map.** `CompiledQuery` keeps the field list per source id when the query names no key; the
  UI stops filling `groupBy` with the default (an empty `groupBy` means "the engine's per-source key", which the
  summary describes per source); SQL builds `CASE e.source WHEN … THEN <key expr> END`. Then the two-step union,
  and the trimming in `group_key`, can go.
- **Rotated families as one logical source.** When a source's events carry no key fields, key them by the
  source's family (`SourceInfo.group`: `app.log` for `app.log.1`, `app-20261020.log`, `app.log.gz`) instead of
  the file name: a pseudo-field `@log` resolved through the source table (both stores already keep `group` per
  source), used as the last fallback of the automatic key. Not done now because `group_key` sees only source
  names, so both read paths need a source lookup, and R2c is reworking them.
- **Interleaved multi-line events.** Continuation lines carry no writer, so they cannot be attributed in general.
  Where a format repeats a writer id on every line (e.g. `[pid]` prefixes, thread ids written on continuation
  lines), a profile key `[parse] writer = "Thread"` plus an optional continuation regex with a `writer` group
  keeps one open event per writer (a bounded map, at most 64, flushed after 1,000 lines without a match); a
  continuation joins the open event of its writer, else the last event as today.
- **Container wrappers.** The Docker `json-file` and CRI importers (formats plugin) already join partial lines and
  add `stream`/container fields. Next: pass each joined `log` text through `lines::LineParser` with the profile's
  formats, so the application's own timestamp, level and fields are read, keeping the wrapper's time as a
  fallback and its fields; the parser API is public for that.
- **Per-source timestamp settings** (R2c finding 15). Products in one file may differ in timestamp format, day
  order and time zone. Since the line format identifies the product before the timestamp is parsed, settings
  belong to the format: a new `[[parse.format]]` table names an output template or regex with its own
  `timestamp_formats`, `day_first` and `timezone`, and day order is detected per format. Routes run after
  parsing and cannot do this.
- **PID reuse.** A lane keyed by a process id merges two processes that reuse the id after a restart. Split a lane
  at a lifecycle start marker that follows a stop or crash marker, or at a reboot annotation of its host, and
  suffix the key with an epoch (`4120#2`); the runs and crash detection already find these boundaries.
- **The same events twice** (a log server's copy and the local file). Detect by a fingerprint (timestamp, level,
  message hash, independent of the source) over a sliding window per pair of sources; report "N events of A also
  in B" as a `duplicateEvents` problem with a quick fix that hides one source's duplicates (a workspace-scope
  `dedupe`). In DuckDB this is a `GROUP BY` on the fingerprint.
- **Clock skew.** The `skew` module (ADR 0006) estimates offsets per source from shared correlation ids and
  applies them on load; `shift_time` corrects a known offset in the pipeline. Offsets are stored per file path, so
  all logical sources of one file shift together; a log server's file whose hosts' clocks differ needs offsets per
  logical source (keyed by source name). Small; after routing moves to the streaming path.

## Consequences

- **Contract changes** (UI ↔ engine, mirrored): `SourceInfo.routedFrom`, `SourceInfo.problems`
  (`SourceProblem { kind, count, sampleLines }`, `SourceProblemKind`). Rust: `SourceInfo { routed_from, problems,
  identity (not serialised) }`, `ProblemKind::{UnmatchedLine, RouteLimit}`, `Problem: Deserialize`. Plugin gRPC:
  `ImporterCap.identity_fields` (field 5, optional). Engine API: `Importer::identity_fields`,
  `BinaryImporter::identity_fields` (default methods), `Imported::problems` (importers that build `Imported`
  literally add `..Default::default()`), `EventSink::problems` (default method), `PlanKind::Text { format:
  LineFormat }`, `query::default_group_by` now delegates to `instance::instance_key_from_totals`.
- **Instance keys read differently** where fields are missing at the ends (`WEB01` instead of `WEB01 · –`;
  `source · …` when the first field is missing). Saved instance selections in that form no longer match (IMT is
  pre-v1; no migration). The DuckDB read path mirrors both: `group_key_expr` builds the same key in SQL (checked
  against `group_key` for every combination of present and missing fields, and by the differential tests), and the
  store chooses the default key with the same two-step rule from one grouped count of the candidate fields per
  source (cached per data generation), with the identity fields of the sources' importers.
- Heterogeneous folders get more accurate lanes: in `/samples`, the IMT diagnostics bundle's child processes now
  get one lane per process id, so the anomaly detector reports each plugin crash separately.
- Throughput: per-line matching costs one `is_match` per earlier format for event lines when the dominant format
  is not the first one, and nothing extra for continuation lines that do not look like events. Measured with
  `cargo bench -p imt-core --bench engine -- import` (250k lines, release, the shared Windows VM, other builds
  running): before → after, MiB/s: Serilog text 32.2 → 28.6–29.2, log4net 28.3 → 25.5–29.6 (three runs; criterion
  calls the differences noise, p > 0.05, except one log4net run), CSV and CLEF (code unchanged) moved −34 % to
  +2 % between runs, which bounds the noise. New case `logserver` (three formats line by line): 26.2 MiB/s, in
  line with single-format text.
- New samples (`cargo xtask gen-samples`): `linux/logserver01/apps-20261014.log` (Checkout in Serilog text,
  Inventory in log4net, Payments as CLEF lines, haproxy lines no default format reads, five process ids) with
  `profiles/logserver.imtprofile.toml` (an extractor and two routes), and `linux/logserver01/syslog-20261014.log`
  (RFC 5424 from three hosts, four apps, six processes). Tests: `crates/imt-core/tests/mixed_sources.rs`, the
  syslog plugin's samples test, property tests for the line parser and routes.

## Note (R3, ADR 0017)

Plugins now run out of process over `imt-ipc` frames, not gRPC. The plugin-facing additions live in
`imt-plugin-api`: `TextImporter::identity_fields` / `BinaryImporter::identity_fields`, carried in the manifest's
`ImporterCap.identity_fields` (at most 8 names of 1–64 characters, checked by `Manifest::check`), and
`Imported::problems` (`imt_plugin_api::problems`, which `imt_core::ingest` re-exports), sent with the end of an
import (`ImportDone` / `TailDone`). The host's importer proxies return both, so the syslog, journald, EVTX and
Docker/CRI identity fields work as before.
