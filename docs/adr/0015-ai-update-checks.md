# 15. Update checks for AI runtimes and models

- Status: accepted
- Date: 2026-10-20

## Context

The inference runtimes (llama.cpp `llama-server` builds, [ADR 0009](0009-prebuilt-llama-server-runtimes.md)) and the
models are downloaded by the user, not bundled. Until now a newer pinned runtime (after an IMT update) or a
newer model revision reached nobody: the worker only looked in the pinned release's folder, so after an IMT
update that pinned a newer llama.cpp the AI stopped working until the user downloaded the new build by hand.

Users want periodic checks with a choice per kind (Automatic, Ask, Manual), through the existing Jobs
([ADR 0012](0012-jobs.md)) and notification centre, without ever installing unverified binaries and without
interrupting a chat.

## Decision

- **Only pinned catalogs.** An update is what IMT's own catalogs pin: a runtime update is the pinned release
  (`runtimes/llama-runtimes.json`, SHA-256 per asset) of a build installed from an older release; a model
  update is a catalog entry whose new `supersedes` list names an installed file. A model update is confirmed
  with the model host (size and SHA-256, no download) before it is offered. Nothing discovered upstream
  (e.g. GitHub's "latest release") is installed.
- **Rust decides** (`imt_ai::updates`): when a check is due, the backoff after failures (5 min doubling up to
  the interval), and the decision per update (`Install`, `Defer` while a chat runs, `Announce` once, `List`).
  The app (`ai/updates.rs`) runs it with injected clock, "busy" probe and effects; the UI only shows the
  status and sends Update / Later / Skip.
- **Keep the previous version until the new one verifies.** The worker uses the newest older release's
  builds while the pinned release has none (`runtimes::usable_tag`); a replaced build is deleted only once the
  engine no longer runs a model. A model update downloads and verifies the new file next to the old one, then
  switches the active model only when no chat runs, then deletes the old file.
- **Quiet failures.** Check and install failures are logged and shown in Settings (with *Download manually*
  when a proxy or certificate interception is the cause), never as messages; Automatic retries after the
  backoff.
- **State** (`<config>/ai/updates.json`): last check, failures, next retry, skipped and announced versions.
  Versioned; a damaged or newer file starts afresh.
- Notification messages get an optional third action (`tertiary`) for "Update / Later / Skip this version".

## Consequences

- Runtime updates arrive with IMT updates (which move the pin); a periodic check then finds and installs them
  by policy. A signed remote catalog feed would let updates arrive without an IMT update; it needs its own
  signing key and is not done here.
- The embedded model catalog has no `supersedes` entries yet: model updates appear when a catalog lists them
  (a newer IMT, or a user-provided `local-models.json`). An in-place change of a file under the same name
  cannot be detected without hashing the installed file and is not offered.
