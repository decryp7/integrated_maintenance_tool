# 49. Lenses and filter sets

- Status: accepted (2026-10-22)
- Date: 2026-10-22
- Related: ADR 0014 (store), profile schema (`default_profile.toml`)

## Context

Analysts switch between a few recurring views ("errors only", "startup", "slow requests") and want to turn groups of
filters on and off without deleting them (the TextAnalysisTool habit). Saved queries only restore a query, and the
filter bar only holds one flat list of filters.

## Decision

**Lens** = a named view preset: query part (levels, search text, regex, query-language filter, marker kinds, field
filters), extra grid columns, row highlights and the chart to show first. The time range, instance selection and filter
sets are not part of a lens.

- Built-in lenses are profile configuration: `[[lens]]` (with `[[lens.filter]]` / `[[lens.highlight]]`) in
  `default_profile.toml`. `imt-core/src/lens.rs` parses and validates them when the profile compiles (id and title,
  unique ids, limits, filter operators and query language compiled through `CompiledQuery`, highlight operators and
  colours, chart ids) so a broken lens fails the profile with a message naming it. They reach the UI in
  `WorkspaceSummary.lenses`; a lens change is a `Presentation` profile change (no re-import).
- User lenses are saved from the current view by the UI (`LensStore`) in the project/session UI state under the key
  `lenses`. The pure model (`lens-model.ts`) parses untrusted saved state.
- **Filter set** = a named group of field filters with an enabled flag (`FilterSetStore`, key `filterSets`). The
  enabled sets' filters are added to `QueryStore.effectiveQuery`, which every view sends to the engine; the user's own
  filters stay in the query. Chips toggle a set; Alt+1 .. Alt+9 toggle the first nine.
- Persistence is versioned (`{ version: 1, items }`). A missing, malformed or newer-version value restores as empty;
  bad entries are dropped and the count is capped.

## Consequences

- The engine owns the built-in definitions and their validation; the UI only applies them. A saved lens is UI state
  (like saved queries), not engine state.
- Applying a lens replaces levels, text, filters, ql and markers; columns are replaced only when the lens names some.
- Lens highlights are added to the project's highlight rules (never duplicated) and stay after switching lenses.
