# 59. Pitwall as Debrief's AI engine, the built-in engine as the fallback

- Status: Accepted (2026-10-24; user: "when Pitwall is installed, Debrief uses it; the built-in engine stays as the fallback", Debrief 1.1)
- Date: 2026-10-24
- Related: ADR 0009 (built-in llama.cpp worker), ADR 0025 (images), ADR 0055 (service health), Pitwall ADRs 0001 (local only),
  0003 (API surface and compatibility), 0007 (security: keys, pairing, loopback), 0009 (app integration contract),
  0014 (presets and aliases), 0016 (slots), 0021 (what quitting Pitwall does); Paddock rules 2, 3, 7, 10–12, 19, 21, 40

## Context

Pitwall is the Paddock family's local LLM server: one model in memory serves every app, and its queue, memory release
and model library are Pitwall's job. Debrief has its own engine: a `debrief-ai-worker` process running a pinned
llama.cpp build with Debrief's own model downloads (ADR 0009). With both installed, a user has two copies of a model on
disk and possibly two models in memory.

Pitwall's ADR 0009 is the integration contract: discovery through `discovery.json` (or the well-known port 11435 on
127.0.0.1), pairing with a six-digit code the user approves in Pitwall, a `pw_` key kept in the OS credential store,
the OpenAI-compatible chat API with SSE streaming and the `/x/v1` extensions (`health`, `pair`, `select`, `models`,
`X-Pitwall-Priority`, provenance headers), typed errors, and versioning by `apiVersion` (`/x/v1` changes only
additively; a breaking change is `/x/v2`, served next to v1 for a release). Debrief follows it as written and adds no
mechanism of its own.

## Decision

### One engine port, two adapters

The assistant, chats, ai-query, narrative, human names and every other AI feature already reach the model only through
`AiCore::provider()` and the `debrief_ai::llm::LlmProvider` port. The engine choice sits behind that call:

- `debrief_ai::engine::AiEngine` is the one engine trait: `kind()` (`builtIn` | `pitwall`), `state()`, `model()`,
  `vision()`, `preload()`, `release()` and `llm()` (the `LlmProvider` features use). Adapters:
  - **`BuiltInEngine`**: the existing lazy-loaded worker (`LazyProvider`), unchanged.
  - **`PitwallEngine`**: an HTTP client of Pitwall's contract (`debrief_ai::engine::pitwall`).
- Both adapters share the same prompt preparation and output parsing (`llm::prompted::prompted_chat`): Debrief's tool
  protocol in the system prompt (Hermes `<tool_call>` tags), think-block filtering, tool-call parsing, JSON-schema mode
  and image limits. Pitwall applies the model's chat template; Debrief sends plain messages. Debrief does **not** use
  OpenAI `tools` on Pitwall: Pitwall's per-app `tools` permission is off by default (Pitwall ADR 0007), and the shared
  protocol keeps the two engines behaviourally equivalent (Pitwall ADR 0009's consequence).
- The selection is a pure function (`engine::select`) of the setting, what the Pitwall probe found and whether the
  built-in engine has a model, so every case is unit-tested.

### Selection: Settings ▸ AI ▸ AI engine

`engine` = **Automatic** (default) | **Pitwall** | **Built-in**.

| Setting | Pitwall usable* | Result |
|---|---|---|
| Built-in | any | built-in |
| Pitwall | yes | Pitwall |
| Pitwall | no | no engine; health `ai` degraded with the reason and the repair **Use the built-in engine** |
| Automatic | yes | Pitwall |
| Automatic | no, never used this session | built-in (Pitwall is offered in set-up and Settings when it is running) |
| Automatic | no, but it served this session | health `ai` degraded, requests fail with "Pitwall stopped answering", repair **Use the built-in engine** |

\* *usable* = discovery found a running Pitwall whose `apiVersion` Debrief speaks, `GET /x/v1/health` answers
`serving`, Debrief holds a key that Pitwall accepts, and `POST /x/v1/select` finds a model for Debrief's needs.

**Never silently mix.** One session uses one engine at a time; a chat never continues on the other engine without the
user's choice. When Pitwall goes away mid-session (stopped, quit with *Stop serving*, key revoked), the request in
flight fails with a plain message, health `ai` turns *degraded* (ADR 0055) and the one notice offers **Use the built-in
engine**, which switches the session (not the setting) to the built-in engine. The repair does not depend on Pitwall
(Paddock rule 19). When Pitwall returns, the probe finds it and Automatic uses it again for the next request, unless
the user switched to the built-in engine for the session.

### Discovery (Pitwall ADR 0009, as written)

- Read `discovery.json` from Pitwall's data folder (`PITWALL_HOME`, else `~/Library/Application Support/Pitwall`,
  `%APPDATA%\Pitwall`, `$XDG_DATA_HOME/pitwall`), at most 16 KiB, parsed strictly; a `baseUrl` other than
  `http://127.0.0.1:<port>` or a port below 1024 is rejected; the `pid` must be alive; `apiVersion` must be **1**.
- Without the file, try `127.0.0.1:11435` (`GET /x/v1/health`). Never anything but loopback.
- The probe runs on a background thread (every 15 s while the setting is not Built-in, and at once on demand), each
  call with a short timeout (connect 1 s, health 2 s), and never on the UI or an IPC thread (rule 10).
- "Installed" means the data folder exists; "running" means discovery plus health answered. Settings says which in
  plain words ("Pitwall 1.0.4 is running", "Pitwall is installed but not running: open it", "Pitwall was not found").

### Registration and consent: pairing

- Debrief never pairs on its own. The user chooses **Use Pitwall** in the AI set-up dialog (a modal decision: *Use
  Pitwall* or *Download a model for Debrief*) or **Connect to Pitwall…** in Settings ▸ AI. Only then Debrief calls
  `POST /x/v1/pair {"name":"Debrief"}` and shows the six-digit code inline in the same dialog/section: "Approve
  Debrief in Pitwall ▸ Apps & keys; check that it shows 123 456." It polls `GET /x/v1/pair/{id}` every second in the
  background (Cancel stops it), up to Pitwall's 5-minute expiry.
- Approved: the key is saved once in the OS credential store (service `Debrief`, account `Pitwall API key`; macOS
  Keychain, Windows Credential Manager, Secret Service), never in a file, a log or the UI. The setting becomes
  Pitwall when the user started from the set-up dialog; Automatic stays Automatic.
- Denied, expired or cancelled: nothing changes; the message says so. A 401 later (key revoked in Pitwall) deletes the
  stored key and reports "Pitwall no longer accepts Debrief's key; connect again" (the request fails, never silently
  falls back).
- **Forget Pitwall** in Settings deletes the key.

### Requests and capability matching

- `POST /v1/chat/completions`, `stream: true` (SSE), `Authorization: Bearer <key>`, `X-Pitwall-Priority: interactive`
  for chats and use cases the user started, `background` for name aliases. Cancel closes the connection (Pitwall
  ADR 0009); the client reads in short slices so Cancel acts within 100 ms.
- **Model:** a Pitwall preset named `debrief` when Pitwall lists one (ADR 0014: the user decides what serves Debrief),
  otherwise `auto:chat,<n>k` (the context Debrief's settings ask for), plus `,vision` for a request with images.
  Embeddings are not needed (Debrief has no vector search). Debrief checks with `POST /x/v1/select` before saying
  Pitwall is usable, and shows the model Pitwall answered (`X-Pitwall-Model`).
- Images (ADR 0025) are sent inline as `data:` URLs after Debrief's own sanitising; JSON-schema answers use
  `response_format: {type: "json_schema"}`.
- Timeouts: connect 2 s, first byte 120 s (a cold model load is reported by Pitwall, `X-Pitwall-Cold-Start-Ms`), 60 s
  between stream chunks; a stalled stream is an error, not a hang. The UI never times out the job (rule 12).
- Pitwall's typed errors map to Debrief's: 401 → key revoked; 403 → "Pitwall does not allow Debrief to …" (permissions
  in Pitwall); 404 / `preset_target_unavailable` → no model fits; 413 / 400 → invalid or too long; 429 → rate limit with
  `Retry-After`; 503 `insufficient_memory` → memory; 503 `overloaded`, 504 → busy/timeout; a dropped connection →
  "Pitwall stopped answering". Messages say what to change and where.

### Built-in downloads while Pitwall is used

- No second copy: while Pitwall is the engine, the set-up dialog's first choice is **Use Pitwall** and downloads only
  happen when the user picks *Download a model for Debrief*. Automatic runtime and model updates of the built-in engine
  are not checked or downloaded while Pitwall is the engine (they resume when the built-in engine is used). Models
  already downloaded stay; the model manager still lists them and can delete them.
- The built-in model is not preloaded at startup while Pitwall is the engine (no second model in memory).

### Privacy

Local only on both sides: Debrief talks to `127.0.0.1` only, never follows a redirect, sends no telemetry, and keeps
its own redaction and leak guard before anything reaches either engine. The key is a secret: never logged or shown.

### Compatibility

- Debrief speaks Pitwall's client contract `apiVersion` **1** (`/x/v1`, Pitwall ≥ 1.0.4). It reads only the fields it
  needs and ignores unknown ones (the contract grows additively).
- A Pitwall that reports another `apiVersion` (a future `/x/v2`-only server) is "incompatible": Settings says
  "Pitwall X speaks a newer connection version than this Debrief; update Debrief" and the built-in engine is used.
- The contract is tested against a fake Pitwall in-process (contract-shaped answers, refusal of pairing, version
  mismatch, the server dying mid-stream, slow answers). Tests never use the real data folder, port 11435, the real
  credential store or an installed Pitwall.

## Consequences

- One model in memory can serve Debrief and the rest of the family; Debrief keeps working alone and offline.
- Two engines must stay equivalent: they share the prompt and parsing code, and the Pitwall path is tested through the
  fake server with the same requests as the built-in fakes.
- A new dependency, `keyring` (the OS credential store), in the shell only.
- Pitwall's ADR 0009 said Debrief would use Pitwall only after an opt-in toggle and fall back silently per request; the
  user's 2026-10-24 decision replaces that with *Automatic* plus explicit pairing consent and no silent mixing. Pitwall's
  ADR 0009 "Debrief's side" paragraph is updated in the Pitwall release that follows Debrief.

## What Pitwall should add (next Pitwall release)

- `GET /x/v1/models` and `POST /x/v1/select`: the context each slot gets when loaded (`contextPerSlot`), so an app can
  size its prompts; today only the trained maximum (`contextMax`) is reported.
- A machine-readable "installed but not running" signal for apps (for example the discovery file left with
  `"serving": false`, or a documented app-bundle location), so apps need not guess from the data folder.
- `X-Pitwall-Job-Id` on streaming answers, so `DELETE /x/v1/jobs/{id}` can cancel a queued request without closing
  the connection.
- Update ADR 0009's "Debrief's side" to this decision.
