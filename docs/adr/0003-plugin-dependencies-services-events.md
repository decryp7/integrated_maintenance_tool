# 3. Plugin dependencies, services, extension points and events

- Status: accepted
- Date: 2026-10-17

## Context

With most features moving into plugins (ADR 0002), plugins need to build on each other: the
AI assistant can use PerfMon counters and profiler snapshots; a profiler can accept symbol
resolvers from other plugins. Without explicit contracts, plugins would reach into each other's
internals, start in the wrong order, or fail when a provider is disabled or quarantined.

## Decision

1. **Manifest.** Each plugin declares `id`, semver `version`, `api_version`, `requires`
   (`Dep::required(id, req)` / `Dep::optional(id, req)`) and `provides` (service ids with
   versions). Existing plugins get a default manifest (no dependencies), so `API_VERSION` stays 1.
2. **Resolution.** The host builds the dependency graph before activation: cycles are rejected,
   missing/incompatible/disabled/quarantined required dependencies leave the dependent
   *inactive with a stated reason*; optional ones only reduce features. Activation runs in
   topological order, deactivation in reverse. Quarantining a provider deactivates its
   dependents (cascade) and the health view shows the chain.
3. **Services.** Inter-plugin APIs are Rust traits in small, separately versioned API crates
   (`debrief-api-*`). Providers register `Arc<dyn Trait>` under a service id and version;
   consumers look them up by id and semver requirement and receive a **guarded proxy**
   (panics → errors, quarantine respected). Each API crate ships contract tests every provider
   must pass. Worker-process plugins provide the same traits through JSON-RPC adapters.
4. **Plugin-defined extension points.** A plugin may declare an extension point (id + trait);
   other plugins contribute implementations, collected by the host and guarded like core
   contributions.
5. **Event bus.** Typed events (`RecordsAppended`, `ProfileChanged`, `LiveAlertRaised`,
   `PluginStateChanged`, …) with per-subscriber bounded queues, delivered off the engine
   thread under the guard; a slow subscriber drops/coalesces, never blocks.
6. **Data over calls.** Plugins should cooperate primarily through workspace records and fields
   (queried via `LogStore`); services are for capabilities that are not records.
7. **UI mirror.** Angular features declare `provideDebriefPlugin({ id, requires })`, expose services
   via injection tokens from a shared API package, and are enabled only while their engine plugin
   is active.

## Consequences

- Plugins compose without knowing each other's internals, and failure of one degrades its
  dependents predictably instead of crashing them.
- More moving parts in the host (graph resolution, proxies, bus), all covered by tests for
  missing/incompatible/cyclic dependencies, cascade on quarantine and crashing subscribers.
- API crates become public contracts and need semver discipline.
