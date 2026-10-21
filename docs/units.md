# Quantities: typed in transit, formatted in one place

A size, duration, rate, count, percentage or timestamp is **a number with a unit**, never a string, until the
moment a person reads it.

## The rule

1. **Rust sends typed values.** DTOs carry the raw number (`sizeBytes`, `elapsedMs`, `bytesPerSec`) and, where
   one field can mean different things, its unit as an enum. `JobProgress` has `unit: items | bytes`, so the Jobs
   pane knows whether `done / total` are files or bytes. Do not build "12.4 MB" in Rust for the UI to display.
2. **The UI formats with `app/src/app/shared/format/units.ts`** (`formatBytes`, `formatRate`, `formatEta`,
   `formatDuration`, `formatCount`, `formatPercent`, `formatRelativeTime`, `TransferMeter`). Numbers use `Intl` with
   the active locale; words go through `$localize`. Bytes are binary (1024) with the labels KB / MB / GB, as Windows
   shows them. Unknown or invalid input shows `—`.
3. **Precision stays reachable.** A humanized size or count that hides digits gets the exact value in a tooltip
   (`formatExactBytes`, `formatExactDelta`).
4. **Speed and time left come from samples**, not from a producer's guess: `TransferMeter` smooths bytes over time
   (about 3 s) and shows nothing until it has 2 s of data. Never call it from a template; feed it from an effect.
5. **Text that must be composed in Rust** (AI summaries, exports, reports, log lines, errors that are only text)
   uses `imt_plugin_api::units` (`bytes`, `exact_bytes`, `duration_ms`, `count`, `percent`), which follows the same
   rules in the source locale. `crates/imt-plugin-api/units-cases.json` is a table both implementations are tested
   against (`units.rs` and `units.spec.ts`); add a case when you change a rule.

## Enforcement

`app/src/app/shared/format/units-guard.spec.ts` scans every template for an interpolation whose last property looks
like a quantity (`...Bytes`, `...Ms`, `size`, `count`, `total`, `done`, `percent`, `rate`, `events`, `files`, ...)
that is not passed through a formatter. A new raw number fails CI. A value that really is an identifier or must be
verbatim goes into the guard's `ALLOWED` map with the reason.

## Still composed as text in Rust (candidates to convert to typed values)

Messages that reach the UI as finished sentences and are not yet structured: job step messages
(`ctx.message(...)`), `Notice` bodies of AI update checks (now formatted by `units`, but still English text), and
engine findings in `crates/imt-core/src/analysis` (anomaly, trend, alert messages). They use the shared Rust rules;
moving them to `{ key, args: [{ bytes | count | durationMs | percent | ts }] }` so the UI can localize and format is
the next step.
