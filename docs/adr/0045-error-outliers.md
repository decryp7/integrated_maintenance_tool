# 45. Error outliers

- Status: accepted (2026-10-22)
- Date: 2026-10-22
- Related: ADR 0014 (store ports), ADR 0042 (port table), ADR 0044 (command line)

## Context

"Errors rose" does not say why. Log platforms flag the attributes statistically over-represented among errors
("only host B", "only build 3.1"). Debrief has anomalies and Moment, but no analysis that compares a subset of events
with the rest of the same window.

## Decision

`imt_core::analysis::outliers::error_outliers(&dyn Query, &OutlierRequest)` is a **pure function over the `Query`
port**: it needs only `summary` (candidate fields), `facet` (value counts), `page` (counts and evidence) and
`check_query`. It is not a new port method, so the store worker protocol and `WIRE_VERSION` do not change, and every
store (the in-memory `Workspace` and DuckDB) gives the answer from its own aggregates. A differential test
(`imt-store-duck/tests/outliers.rs`) runs it on both.

- The window is the request's query (a filter without `|` stages); the subset is a filter (`@level >= error` by
  default) applied as a `where` stage; the rest is the window minus the subset.
- For each candidate field the subset's top 100 values and the window's top 1,000 values are taken; the rest's count is
  the window count minus the subset count. A value beyond the window facet's top values has an unknown rest count and is
  not judged (the coverage says so when more values existed).
- Score: lift with half an event added to each count (finite when the rest has none), and the G statistic of the 2x2
  table (subset or rest by with or without the value) with a one-degree-of-freedom p-value. An outlier needs at least
  three subset events, lift of 1.5 or more and G of 10.83 (p < 0.001). Ranked by G.
- Bounded: 60 fields (most populated first, `@source` always first), identifier-like fields (more than 5,000 values)
  skipped, 30 outliers by default and 200 at most, five evidence events each. What bounded the result is in
  `Coverage::limits`. The run checks the thread's `QueryControl` between fields, so Cancel works.
- The shell runs it as an analysis job (`query_outliers`); the UI panel only presents; `imt analyze outliers` is the
  command-line form. Evidence is a query-language filter the UI opens as the view, plus sample event ids.

## Consequences

- Cost is two facets per field; on very large stores the analysis is a job like the others and cancellable.
- A custom subset must be addressable as a filter; fields with unusual names (not `[@A-Za-z_][A-Za-z0-9_.-]*`) are not
  examined.
- If outliers need per-event numeric features later (not just categorical values), they become a store operation
  (a new row in the port table), not a change of this function.
