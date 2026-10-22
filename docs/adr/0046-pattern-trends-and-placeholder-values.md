# 46. Pattern trends and placeholder values

- Status: accepted (2026-10-22)
- Date: 2026-10-22
- Related: ADR 0014 (store), ADR 0045 (error outliers)

## Context

The Patterns view lists templates with counts and first/last seen. It does not say whether a template is a burst, a
steady stream or new, nor what hides behind a `<*>` ("which user, which order"). Both answers are computed while the
miner already visits every event.

## Decision

Both live **inside `PatternMiner`** (`imt-core/src/patterns/`), so every store and the Moment view get them without a
store operation or a wire change, and the DTO `Pattern` grows four optional fields.

**Trend** (`patterns/trend.rs`). The time range is unknown until the events are read, so a `TrendScale` grows with
them: 96 fine buckets whose width starts at one second and doubles until the range fits, with every pattern's buckets
folded onto the new scale (no event is lost). At the end the fine buckets are folded onto 24 buckets fitted exactly to the
earliest and latest event seen, so doubling steps leave no empty space in a sparkline. `trend`, `trend_from` and
`trend_bucket_ms` are shared by every pattern of a run, so sparklines compare. Without a timestamp: empty.

**Placeholder values** (`patterns/values.rs`). Tokenising records the byte range each mask replaced, so the raw value
is read back without a second parse. A placeholder slot keeps a **Space-Saving** summary (Metwally et al.): 16 counters,
the smallest taken over when a new value arrives, so memory is constant and the heavy hitters are right within the
bound; while nothing was dropped the summary is exact and says so. When Drain generalises a literal into `<*>`, the
slot is seeded with the old literal and the events that had it. Values are cut at 120 characters; 8 are returned.
Structured (Serilog) templates have no placeholders: their properties are fields and facets answer them.

## Consequences

- A mined pattern costs about 1 KB more per placeholder slot and 400 bytes per trend (10,000 patterns: tens of MB at
  most); the pattern cap (10,000) bounds it.
- Counts of a high-cardinality placeholder are approximate by design; the DTO carries `exact`.
- The UI draws bars and a values list only; no statistic is computed in TypeScript.
