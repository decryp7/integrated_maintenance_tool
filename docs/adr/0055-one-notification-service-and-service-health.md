# 55. One notification service and service-health model

- Status: Proposed
- Date: 2026-10-24
- Related: ADR 0016 (command table), ADR 0050/0054 (dialogs), the notification centre (`app/src-tauri/src/notification_centre/`),
  the process watcher (`app/src-tauri/src/performance/watch.rs`), Paddock rules 19, 40 and "Where logic lives" in AGENTS.md
- Replaces: `StoreHealth` (`app/src/app/core/store-health.ts`)

## Context

One failure of the DuckDB store worker produced three toasts and a red box in every panel. What exists today:

- A Rust **notification centre** (`notification_centre/centre.rs`) with `NoticeSource`, `NoticeSeverity`,
  `dedupe_key`, `count` and `once()`. It dedupes by key, but only notices that choose to post through it benefit.
- **Separate UI channels** next to it: `WorkspaceStore.error()` rendered as a toast with a red border by
  `shell/workspace-toasts.ts`; `notification-toasts.ts`; `live/alert-toasts.ts`; `updates/updates-toast.ts`;
  `diagnostics/snapshot-toast.ts`; and a generic `ui/toast-stack.ts`. Each decides its own text, severity and
  lifetime. Job toasts, error toasts and banners do not know about each other.
- **`StoreHealth`**, a TypeScript root service filled by `invoke` through module-level `reportCommandFailure` /
  `reportCommandSuccess` hooks (`core/store-health.ts`). It recognises the failure by string-matching the message
  prefix "Debrief's log store" and clears when one of three hard-coded commands (`workspace_summary`, `query_page`,
  `query_timeline`) succeeds. The workspace toast suppresses its banner with `error.includes(d.message)`. The status
  bar and `error-details.ts` read it. It is per window, derived on the UI side, and handles one service.
- The engine already knows more: `engine/store_failure.rs` classifies the cause (`LibraryRefused`, `Quarantined`,
  ...) and `performance/watch.rs` names the capped processes `ProcessRef::{StoreWorker, Ai, Plugin}` with a
  `retryProcess` action. The AI worker (llama.cpp), research browser, downloads and plugins each report their
  failure in their own way (`ai/ai-status.store.ts`, `browser/browser-runtime.store.ts`).

The same bug kind will recur for every service: the AI worker dying will show a toast, an assistant error and a
failed job. The UI deciding what is "the same failure" from message text is the design gap (rule 40); the
engine, which knows the cause, should decide.

## Decision

Two cooperating parts, both owned by the Rust engine (AGENTS.md: the engine is the single source of truth), shown by
the UI without domain logic.

### 1. Notifications: one service, one display rule

- **Cause key.** Every notification has a `cause` key (`store.down`, `ai.down`, `job:123:failed`,
  `update.available`). A second post with the same cause *updates the existing notification in place* (count,
  latest time, details), never adds one. Today's `dedupe_key` becomes mandatory and is the cause key; the centre
  gains `update_cause` and `resolve_cause` (a cause that clears removes or marks its notification resolved).
- **Severities and channels.** `info | success | warning | error | critical`. One display rule, in one component
  (`NotificationHost`, replacing the six toast components): *one toast per cause, details collapsed behind "Details",
  updated in place; `error` and `critical` persist until resolved or dismissed, others time out; a cause that is
  also shown by service health (below) is never toasted.* Job completion, errors and update offers are *kinds* of
  notification with different severities and actions, not different channels; the Jobs pane stays the place for job
  progress.
- **Errors from commands.** `invoke` rejections no longer set `WorkspaceStore.error` text that a toast renders. A
  rejected command with a typed `CmdError` code carries a `cause` (the engine sets it for service failures) and the
  UI routes it to the notification service; panels show their own inline error with Retry (AGENTS.md) only for errors
  that belong to that request, never for "a service is down".
- Window events: `debrief://notifications` already goes to every window; the dedupe happens in Rust once, so a
  second window never repeats it.

### 2. Service health

A `ServiceHealth` registry in the engine (`app/src-tauri/src/health/`), one entry per service:

| Service | Source of state |
|---|---|
| `store` (DuckDB worker) | the store supervisor, classified by `StoreFailure` |
| `ai` (llama.cpp worker) | the AI supervisor and `performance::watch` |
| `plugins` (each plugin process) | `PluginRegistry` quarantine (`debrief_core::guard`) |
| `browser` (research browser runtime) | `browser-runtime` supervisor |
| `downloads` (model, symbol, update downloads) | the download manager (per-host back-off) |

State is a closed enum: `ok | starting | degraded | down | quarantined | repairing`, plus a cause (`StoreFailure`-like,
plain sentence, next step, technical detail) and a **repair action** (`restart`, `reset`, `lift_quarantine`,
`raise_cap`). Rules:

- **Rust decides, events announce.** The supervisor of each service updates the registry; the registry emits
  `debrief://service-health` (full small snapshot) to *all* windows including dialogs. A new window asks
  `health_snapshot`. Nothing is derived from error strings or from "a store command succeeded".
- **Repair never depends on the broken thing** (rule 19). `health_repair(service)` is answered by the shell, which
  owns the supervisors; it does not go through the store, the AI worker or a plugin. A "Repair" that needs the
  database to repair the database is not allowed.
- **Surfaced once.** The status bar shows one chip when anything is not `ok` (a count with a menu when several), with
  the plain message, Details (technical chain) and the Repair / Try again button. This is the only place a service
  outage is stated. It posts one notification with the cause key `health:<service>` on the transition to `down` and
  resolves it when the service is `ok` again.
- **Panels wait calmly.** A panel whose data needs a service reads `health(service)` and, while it is not `ok`,
  shows a neutral "Waiting for the log store" state (no red, no per-panel Retry) with a link to the status chip, and
  loads by itself on the `ok` transition. Commands in flight when the service dies are cancelled quietly by the
  shell and are not reported as errors; they are re-queried by the panels on recovery.
- **Same model for every service**, so adding a service is one registry entry: name, states, repair.

### UI contract

`ServiceHealthStore` (`core/service-health.store.ts`, per window, SignalStore with `withRequestStatus`, rule 30) holds
the latest snapshot only. `StoreHealth`, `reportCommandFailure`/`reportCommandSuccess`, `isStoreDown`, the string
prefix and the `storeDownText` suppression in `workspace-toasts.ts` are deleted.

## Consequences

- One failure produces one status-bar chip and one notification, in every window, with the engine's wording.
- Panels lose their own "store is down" handling; each needs the shared waiting state (a component) instead.
- The engine gains a supervisor-to-registry wiring for five services; most supervisors exist already
  (`performance/watch.rs`, `engine/store_failure.rs`), so this is an adapter layer plus the event.
- Typed `cause` on `CmdError` is a wire change (`api.types.ts` and the Rust DTO together).
- Toast components are replaced; tests that clicked specific toasts move to `NotificationHost` test ids.

## Migration plan

1. *Before 1.1.* Make `dedupe_key` mandatory in `NewNotice` (compile errors list every caller); add `update_cause`;
   route `WorkspaceStore.error` that is an engine service failure through the centre with cause `health:store`.
   This stops the three-toast bug without new services.
2. *Before 1.1.* Add `health/` with the `store` service only, `health_snapshot`, `health_repair` (reusing
   `performance_retry_store_worker`) and the event; add `ServiceHealthStore`; point the status bar,
   `error-details.ts` and the panels at it; delete `StoreHealth` and the hooks in `invoke`. Regression test first:
   a store failure yields exactly one chip, one notification and zero panel errors, in two windows.
3. *Before 1.1.* Shared "Waiting for X" panel state; replace per-panel handling of the store error.
4. *After 1.1.* Add `ai`, `plugins`, `browser`, `downloads`; fold `retryProcess` and the memory-cap actions into the
   repair actions of the registry.
5. *After 1.1.* Merge the toast components into `NotificationHost`; remove `ui/toast-stack.ts` per-feature variants;
   move job/update/alert toasts to notification kinds.
6. *After 1.1.* Lift the registry's types and `NotificationHost`/`WaitingFor` into `paddock-ui` and a Paddock crate
   (rule 39) when a second app needs them.

## Alternatives considered

- **Keep `StoreHealth` and generalise it in TypeScript.** It would keep deriving health from error text and from
  which commands succeeded, per window; the engine already has the typed cause. Rejected.
- **Dedupe only in the toast component.** Fixes the symptom for toasts, not the panel red boxes or other windows.
- **A banner per service.** Repeats the original problem as more services are added.
- **Let each panel poll its service.** Many calls for one fact; the event gives all windows the same truth.
