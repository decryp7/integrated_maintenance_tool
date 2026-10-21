# 17. One IPC: framed postcard over a private local stream; plugins and workers only out of process

- Status: accepted (2026-10-20)
- Date: 2026-10-20
- Supersedes: [ADR 0004](0004-stable-plugin-abi.md) (the protobuf/gRPC plugin contract and the C#/.NET SDK) and the
  transport part of [ADR 0002](0002-microkernel-and-process-isolation.md) ("Wire protocol for out-of-process
  plugins": gRPC, Arrow IPC inside gRPC, in-process by default with opt-in isolation)
- Related: ADR 0014 (store worker, event batches), ADR 0016 (path grants, jobs, `CmdError`), ADR 0019 (.NET
  profiling: the ETW collector needs administrator rights), redesign plan R3 (`docs/redesign-2026-10.md`)

## Context

IMT runs code in other processes for three reasons: native code that can crash (llama.cpp), memory it must cap
(the DuckDB store worker) and code that parses untrusted files (every importer plugin). Before this decision:

- **Three transports.** gRPC (tonic, prost, protox, three `build.rs`, `proto/`) for the AI worker and plugins, a
  gRPC call carrying a JSON envelope for the store worker, and in-process trait objects for plugins by default.
  Each needed HTTP/2 window tuning, a token interceptor, a trailer for typed errors and a 4 GiB message limit
  for whole files.
- **Two ways to run a plugin.** In process (the default; a stack overflow, abort or OOM in a parser takes IMT
  down; a hang can only be abandoned) or isolated (opt-in per plugin, "isolated mode", automatic after a
  quarantine). Only importers, transforms, exporters, marker kinds, annotators and settings crossed the
  boundary, so tools, collectors, configuration parsers and the profiling importers could never be isolated.
- **Paths, not handles.** An isolated plugin either received the whole file in one message or, with the
  "read opened files" permission, its path; nothing stopped it from reading anything else. Tools that take
  paths (PerfMon `generate`'s `xmlPath`, collector output folders) were not checked against path grants
  (ADR 0016, "Not done yet").
- **A polyglot promise nobody uses.** A C#/.NET SDK, a gRPC conformance kit and a .NET CI job exist for
  third-party plugins in other languages; there are none, and all first-party code is Rust (ADR 0004).
- **Plugins depend on the whole engine.** Plugin crates use `imt_core` types (`Record`, `TimestampParser`,
  `ConfigParser`, settings, tools), so a plugin process links the engine, and plugin-to-plugin services
  (`Service<T>`) assume one address space.
- **No sandbox.** Children get a cleared environment and, on Windows, a job object; nothing restricts the file
  system, the network or the integrity level. ETW capture (ADR 0019) needs administrator rights, which today
  means restarting all of IMT elevated.

The user decided (2026-10-20): plugins are Rust only, lean and out of process, over one IPC that is easy to
understand; gRPC, the .NET SDK, its CI and the conformance suite are deleted; no backward compatibility before
v1; elevated plugin processes must be possible, with a narrow, authenticated channel to the unelevated app.

## Decision

### 1. One frame codec (`imt_ipc::frame`)

Every byte between two IMT processes is a **frame**: a 4-byte little-endian length, then the frame body encoded
with **postcard** (serde, compact, no schema compiler, MIT/Apache-2.0).

```text
frame   = len:u32le  body:postcard(Frame)          len ≤ max_frame (default 16 MiB, hard cap 64 MiB)
Frame   = Hello  { version, token: [u8; 32], pid }      first frame of the connecting side
        | Welcome{ version }                            the listening side accepted it
        | Call   { id, credit, body }                   a request (body: the protocol's own message)
        | Reply  { id, body }                           its answer; after Items it also ends the stream
        | Item   { id, body }                           one item of a streamed answer
        | Fail   { id, error: RemoteError }             typed error {code, message, data: optional JSON}
        | Cancel { id }                                 the caller gave up; the callee stops at its next check
        | Credit { id, n }                              the caller can take n more items
```

- **Two layers, both postcard.** The envelope above is generic; `body` is a protocol's own request or reply enum
  (plugin, AI worker, store worker). Adding a variant at the end of an enum is a minor change; anything else is
  a new `version`, refused in `Hello` with a message.
- **Bounded.** The decoder reads the length first and refuses a frame over the connection's limit before
  allocating (`IpcError::Protocol("frame of N bytes exceeds the limit of M")`, the connection is closed and the
  child counts as failed). Nested bodies are bounded by the same limit. Decoding never panics (fuzzed).
- **Symmetric.** After the handshake both sides may send `Call`s (ids are per direction). The plugin protocol
  uses this for host services: reading ranges of a host-opened file, reporting tool tasks.
- **Streams with credits.** A `Call` carries an initial credit (default 4 items); the callee sends at most that
  many `Item`s ahead of the caller's `Credit`s, so a fast producer (an importer streaming events) never fills
  the caller's memory, and a slow consumer pushes back on the producer. Items are bounded by the frame limit;
  event items carry at most 8,192 rows (`imt_batch::BATCH_ROWS`).
- **Cancel and deadlines.** A caller that stops waiting sends `Cancel`; the callee sees it through its call
  context (`is_cancelled`) and ends with `Fail{code: CANCELLED}`. A callee that does not end within the grace
  period counts as hung (the supervisor kills it). Deadlines are the caller's; nothing is sent for them.
- **JSON only where JSON is the contract.** postcard is not self-describing, so values that are JSON by nature
  (tool arguments and results, settings values, profile documents) cross as JSON text in a `Json` newtype; wire
  types never use `untagged`, `flatten`, internal tags or `skip_serializing_if`.

### 2. One connection model: the host listens, the child connects

- The host (the app, or a worker that hosts plugins) creates a **private endpoint** before starting the child:
  a Unix socket in a fresh 0700 directory, or a randomly named pipe with an owner-only DACL, remote clients
  rejected, `FIRST_PIPE_INSTANCE` (nobody can squat it), and a 256-bit random token.
- The child learns the endpoint and the token on stdin (one JSON line; stdin stays open as the lifeline) or,
  for an elevated child that has no stdin pipe, from a token file the host wrote into its private directory
  (deleted by the child after reading).
- The child connects and sends `Hello { version, token, pid }`. The host checks the token in constant time and
  **the peer's process id** (`GetNamedPipeClientProcessId` / `SO_PEERCRED` / `getpeereid`) against the process it
  started, then answers `Welcome`. Anything else closes the connection. One connection per child.
- The child exits when the connection closes (the host died or stopped it) and when stdin closes. On Windows a
  job object with `KILL_ON_JOB_CLOSE` also kills it with IMT; an elevated child puts itself into such a job.
- No TCP, no listening child, no connect-retry loop: the host accepts with a timeout (the child's start window).

### 3. `imt-plugin-api`: the contract, with no engine dependency

A new crate that plugins depend on, and nothing else of IMT (it depends on `imt-ipc` and `imt-batch`):

- **Plugin traits**: `Plugin` (manifest and contributions), `TextImporter`, `BinaryImporter`, `ConfigParser`,
  `Annotator`, `Tool`, `ProfileImporter` (profiling captures), `Settings`.
- **Value types plugins build**: `Record`, `Level`, `Fields` (moved from `imt_core::model`), the timestamp parser
  (moved from `imt_core::time`), text decoding (`decode`, `MAX_LINE_LEN`), configuration trees, settings
  sections, tool descriptions and argument schemas, marker kinds, annotations. `imt-core` re-exports them, so the
  engine's code is unchanged and there is one definition of each.
- **The manifest** (id, name, version, contract version, capabilities, permissions) with **owned ids**
  (`Arc<str>`): nothing is leaked to get a `&'static str`.
- **The plugin protocol** (`PluginRequest` / `PluginReply` enums) and the wire event (`WireEvent`, the parsed
  form of an ADR 0014 event row: time, level, message, template, exception, fields, line, byte offset, raw).
- **`serve(plugin)`**: the one child loop (read the handshake, connect, apply the sandbox, answer calls until the
  connection closes). A plugin's `main` is one line.

Event rows cross as postcard `WireEvent`s in items of at most `BATCH_ROWS` rows; the host's batcher turns them into
`imt_batch::EventBatch`es with the profile's pipeline, so the store and index see one schema (ADR 0014 §3) and
**Arrow does not cross the plugin boundary** (this resolves ADR 0014's Arrow 58/60 split: Arrow 58, inside the host
only). Importers parse before the profile pipeline, so the pipeline, marker rules and ids stay in the host.

### 4. What crosses the contract

| Contribution | Request → reply | Notes |
|---|---|---|
| Text / binary importer | `Sniff { importer, head }` → score; `Import { importer, file, ctx }` → stream of event items, then `Done { skipped }`; `ImportTail` | the file is a **host file** (below); `ctx` carries the file name, date and the timestamp settings |
| Configuration parser | `ParseConfig { parser, file, limits }` → `ConfigTree` | registry, msinfo |
| Marker kinds | in the manifest | rules are conditions; the host compiles them |
| Annotator | `Annotate { annotator, start, rows }` → annotations | rows the host selected by the declared interest |
| Settings | the section is in the manifest; `ApplySettings { json }` → ok | the host validates first; the plugin again |
| Tool | descriptions in the manifest; `Invoke { tool, op, args }` → JSON; `Stop { tool, task }` | path arguments are grant-checked by the host (below); task reports are calls back to the host |
| Profile importer | `SniffProfile`, `ImportProfile { importer, file }` → a profile (imt-profile's model) | runs in the profiling plugin, capped by bytes |
| Transform | `Transform { id, rows }` → rows, verdicts, marks | a `{ step = "plugin" }` pipeline step |

Dropped from plugins (no built-in plugin used them; the engine keeps them for its own built-ins): workspace hooks,
events, exporters, services and plugin-declared extension points, and dependencies between plugins. **Shared
code is a library**, not a plugin service: `imt-perfmon` (PerfMon CSV and `relog`) and the event-log follower
(`imt-eventlog`) are crates that plugins and the headless agent link.

**Host files instead of paths.** The host opens a file the user chose and gives the plugin a `FileRef { id, name,
len }`; the plugin reads ranges through the connection (`ReadFile { id, offset, len ≤ 1 MiB }`, a call back to the
host) or the whole file up to its importer's declared cap. A plugin never sees a path unless its manifest asks for
`Permission::OpenedFilePaths` and the user granted it (the Windows plugin, whose BLG import runs `relog` on the
file), and even then only paths of files the user opened.

**Typed path arguments.** A tool argument declared `Schema::Path { kind: File | Dir, access: Read | Write }` is
checked by the host against the calling window's path grants (ADR 0016) and canonicalised before the plugin sees
it; a path the user did not choose is refused with `notGranted`. Outputs a tool reports stay granted as before.

### 5. Out of process only, one supervisor

- **Every plugin runs in its own process**, always: in-process plugin linking, "isolated mode", "Run isolated",
  auto-isolation after a quarantine and the `--isolated` flag are deleted. The app does not link plugin crates;
  `imt-plugins` is one binary that serves any built-in (`imt-plugins --plugin <id>`), one process per plugin,
  shared by every window. Safe mode starts none.
- **The host adapter** (`imt-plugin-host`) makes a plugin process look like an `imt_core::Plugin`: its manifest
  registers proxy contributions (`RemoteImporter`, `RemoteTool`, ...) in the registry, so guards, quarantine,
  status, settings and the UI are unchanged. It is a library: the app links it, and the store worker links it to
  run importer plugins next to the store when imports move there (ADR 0014 6c), so events go from the plugin to
  the store worker directly with credits, without an app hop.
- **One supervisor** (`imt_ipc::Supervisor`): lazy start, restart with exponential backoff, quarantine after
  3 failures in 10 minutes, a **restart hook** that restores state (the AI worker reloads its model, the store
  worker reopens its stores, a plugin gets its settings again). `imt_core::guard` semantics hold across the
  boundary: a crash, hang, oversized or garbled frame is an error attributed to the plugin, counted towards its
  quarantine; a typed `Fail` is not a crash.
- **One launcher** (`imt_ipc::launch`) with a `SpawnSpec` that includes a `SandboxSpec` and an `Elevation`.

### 6. Sandbox (`SandboxSpec`)

| | Windows | Linux | macOS |
|---|---|---|---|
| Memory | job object `ProcessMemoryLimit` | `RLIMIT_DATA` (memory used; not `RLIMIT_AS`, which counts malloc arena reservations) | `RLIMIT_AS` (advisory on macOS) |
| CPU | job CPU rate cap; per-process user time | `RLIMIT_CPU` | `RLIMIT_CPU` |
| Child processes | job `ActiveProcessLimit = 1` unless allowed | `RLIMIT_NPROC` / seccomp `fork` denied | profile `process-fork` denied |
| Network | (not enforced; low integrity cannot reach the app's pipes) | seccomp: `socket(AF_INET/AF_INET6)` → `EACCES` | profile `network*` denied |
| File system | **low integrity token** (no writes outside low-IL locations) | **landlock**: no access, or read-only / read-write to the listed paths | profile: no file access except listed paths |
| Environment | cleared (`KEPT_ENV` + named variables) | same | same |

- The launcher applies what must come from outside (job object, environment, working directory, priority); the
  child **locks itself down after connecting and before handling any request** (`imt_ipc::sandbox::lockdown`:
  lowers its integrity level, sets rlimits, restricts itself with landlock and seccomp, or enters its sandbox
  profile). The connection and stdin are already open, so the lockdown needs no exceptions for them. This is the
  Chromium "lock down after init" pattern; it defends against a compromised parser, which is the threat for
  first-party plugins. Third-party binaries would need external enforcement (AppContainer / a restricted token at
  `CreateProcessAsUser`, a seccomp launcher) before IMT accepts them; none ship.
- Profiles: `SandboxSpec::locked()` (importers, parsers: no network, no file system, no children, low integrity,
  1 GiB unless the manifest declares another cap), widened only by manifest permissions the user granted
  (`OpenedFilePaths`, `RunPrograms`, `WriteFolders`, `Network`; `imt_plugin_host::sandbox_for`): the tools plugin
  asks for `RunPrograms` and `WriteFolders` (normal integrity, children allowed). Workers have their own (the
  store worker: memory cap, no file-system restriction yet; the AI worker: may start `llama-server`). The Plugins
  settings show the permissions.
- What the system cannot enforce degrades instead of failing the plugin, and is never silent: a kernel or
  container without landlock (or with landlock filtered by its seccomp profile) leaves file access unrestricted,
  one without seccomp leaves network sockets and program starts allowed; the child logs each gap as a warning
  ("sandbox degraded: …") in its log, and `imt_ipc::sandbox::applied_notes` reports them as typed `Gap`s, which the
  sandbox tests use to skip, with a message, what the machine cannot enforce. A failed resource limit, and on
  Windows a failed integrity drop, still fail the start.

### 7. Elevation

- A tool operation may declare `elevated: true` (e.g. the .NET plugin's ETW capture, ADR 0019). Calling it starts
  **a second process of the same plugin** in its elevated role (`--elevated`), which serves only the elevated
  operations of that plugin; the normal process keeps everything else.
- **Launcher decision** (pure, tested): elevation not needed → normal spawn; needed and IMT already elevated →
  normal spawn; needed on Windows → `ShellExecuteExW` with `runas` (the UAC prompt; declining is a typed
  `Denied` error with the "Restart as administrator" hint); needed elsewhere → `Denied` with the hint to run the
  collector with `sudo` (no setuid helper).
- **The channel is narrow and authenticated**: the pipe's DACL grants the user and the Administrators group (an
  over-the-shoulder administrator is another account), the token comes from a file in the host's private
  directory, and the host checks the client process id against the process `ShellExecuteExW` returned. The
  elevated process exposes only its declared operations, never writes files at paths it receives (it streams
  capture bytes back over the connection with credits; the unelevated host writes them into the granted folder),
  and exits when the connection closes. It puts itself into a kill-on-close job object with its memory cap.
- UAC is not a security boundary against code already running as the same user; the design keeps the elevated
  surface small rather than claiming isolation from such code.

### 8. Workers use the same stack

- **AI worker**: the same frames (`Hello`/`Load`/`Chat` (streamed deltas with credits)/`Unload`/`Status`), the
  same supervisor (the restart hook reloads the model). `imt-ai` is split into `imt-ai-proto` (worker messages),
  `imt-ai-runtime` (downloads and installs through one `SafeHttp`), `imt-ai` (assistant, chat, tools; it takes the
  `Query` and `Analysis` ports) and `imt-browser-agent`; the worker links only `imt-ai-proto` and `imt-ipc`.
- **Store worker**: its request enum (`OpenStore`, `Open`, `Page`, ...) replaces the JSON envelope; replies are the
  ports' owned DTOs. Batches for `scan` cross as Arrow IPC (imt-batch's Arrow 58) inside items when 6c needs them.
- **Profile worker**: `imt-profile` and its importers run in the profiling plugin process, capped by bytes (input
  size limit and the job memory cap); the app holds the resulting model and its views.
- **One redactor** (`imt_core::redact`) is used by the AI privacy filter and by child-log scrubbing.

### 9. Deleted

`tonic`, `tonic-prost`, `tonic-prost-build`, `prost` (as an IPC dependency), `protox`, `tower`, `hyper-util`; the
three `build.rs` that generated gRPC code; `proto/`, `crates/imt-ipc/proto`, `crates/imt-ai-worker/proto/llm.proto`,
`crates/imt-store-worker/proto`; `crates/imt-plugin-sdk` (SDK, Arrow conversion, `imt-plugin-check`, the toy
plugin); `sdk/dotnet` and its CI job; the isolation settings (`plugin-isolation` section, `--isolated`,
`plugin_isolation_status`); `PluginSpec` grants of the gRPC handshake. No migration of stored settings (pre-v1).

### 10. Migration of each crate

| Crate | Becomes |
|---|---|
| `imt-ipc` | frames, connection (`Peer`), endpoint, launcher with sandbox and elevation, supervisor, child loop |
| `imt-plugin-sdk` | deleted; `imt-plugin-api` (contract) |
| `imt-plugin-host` | host adapter library (no plugin crates); `imt-plugins` serves the built-ins |
| `imt-plugin-registry`, `-otel`, `-syslog`, `-web`, `-formats` | depend on `imt-plugin-api` only; importers over host files |
| `imt-plugin-windows` | importers, msinfo parser, markers, annotators on the API; event-log follower → `imt-eventlog`, PerfMon → `imt-perfmon` libraries |
| `imt-plugin-tools` | a plugin process (`SandboxSpec::tools()`), tools with typed path arguments |
| `imt-plugin-profiling` | profile importers over the API in their own process (the profile worker) |
| `imt-plugin-dotnet` (ADR 0019) | its tool and `.etl` importer over the API; ETW capture as the first elevated operation |
| `imt-ai`, `imt-ai-worker` | split as in §8; worker over frames |
| `imt-store-worker` | request enum over frames |

### 11. Testing

Fuzz targets for every frame decoder (`fuzz_ipc_frame`, `fuzz_plugin_request`, `fuzz_plugin_reply`,
`fuzz_store_request`, `fuzz_ai_request`) and the AI worker's HTTP parser (`fuzz_worker_http`). Supervision tests
with a fake child: crash, hang, oversized frame, garbage, wrong token, wrong peer process, killed child, restart
hook, quarantine. Sandbox denial tests where the platform runs in CI (Windows: low integrity cannot write the
user's temp folder, the job memory cap fails a large allocation; Linux: landlock denies a read, seccomp denies a
socket). Elevation: the launcher decision and the authenticated channel (token file, peer check) are tested; the
UAC prompt itself cannot run in CI and is a manual check.

## Consequences

- One transport to understand, test and fuzz; about 40 crates fewer in the dependency tree and no code
  generation. A frame is readable in a debugger as a Rust enum.
- A plugin bug can no longer take IMT down or read files the user did not open; plugins start lazily (the first
  import of a format, the first tool call), at the cost of one process per plugin (a few MB each) and a copy of
  event rows through a local socket (measured by `imt-plugin-host`'s transport benchmark).
- Plugins compile against a small crate, not the engine; the engine's types plugins build live in the API crate.
- Third-party plugins in other languages are no longer possible; a future need would add a second codec behind
  the same frames, not a second transport.
- The sandbox is applied by the child after connecting; it protects against exploited parsers, not against a
  malicious plugin binary (none are installable).

## Implementation notes (R3, 2026-10-21)

What landed, and where it differs from the text above:

- **Frames** (`imt_ipc::frame`, `peer`, `link`): as in §1, except that there is no `End` frame: a streamed
  answer is its `Item`s followed by the `Reply`. `RemoteError` is `{code, message, data: Option<Json>}`; retry
  advice is in the message and the error code. The `Peer` API is synchronous (handlers run on a blocking pool of
  one shared two-thread runtime), with `max_concurrent_calls` (over it: `BUSY`) and a 2 s cancel grace.
- **Descriptive types cross as JSON.** The manifest (with settings sections, tool schemas, marker kinds) is read
  once per start with `imt-plugins --manifests` and on `PluginRequest::Manifest`, as JSON: these types use serde
  attributes postcard cannot carry, and they are not on a hot path. Event rows (`WireEvent` with `WireValue`s),
  file chunks and every request/reply envelope are postcard.
- **Host files** are lent per call: the host answers the plugin's `ReadFile` (≤ 1 MiB) from the file it opened,
  on the calling thread, for the duration of the call (`imt_plugin_host::process`'s pump); the plugin sees a
  `HostFileRef`, never a path, unless it was granted `OpenedFilePaths`. Annotators read the host's view through
  `ViewSelect`/`ViewInstances`/`ViewTimeRange` callbacks with a call-scoped ticket; tools report tasks with
  `TaskStarted`/`TaskProgress`/`TaskFinished`.
- **Plugins**: every built-in runs in its own `imt-plugins --plugin <id>` process; the app links no plugin crate.
  Settings are re-applied after a restart (`Supervisor::with_restore`). Path arguments are typed
  (`Schema::Path`) and checked by `imt_core`'s `ToolHost` against the window's grants (`ToolError::PathRefused`
  → `CmdError::NotGranted`).
- **Profiling**: the profiling plugin's process is the profile worker (3 GiB cap); the model crosses as postcard
  through `imt_profile::wire`, validated on decode.
- **AI worker**: `WorkerRequest` (`Hello`, `Load`, `Chat` streamed with credits, `Unload`, `Status`, `Shutdown`)
  over frames; the gRPC code and `llm.proto` are gone.
- **Store worker**: `StoreRequest { OpenStore, CloseStore, Status, Call { store, op: StoreOp, body } }` over
  frames. The port DTOs still travel as JSON inside the frame (`Json` body), because many use serde attributes
  postcard cannot carry; moving them to postcard-safe types is part of ADR 0014 6c, together with the store
  worker running importer plugins itself.
- **Sandbox and elevation** (`imt_ipc::sandbox`, `job`, `elevation`): Windows low integrity, job memory/CPU/child
  limits and the elevated launch (`ShellExecuteExW runas`, bootstrap file `--imt-bootstrap <file>` read and
  deleted by the child, Administrators in the pipe DACL, peer PID check) are implemented and tested on Windows
  (low-integrity write denied, job blocks a child process, memory cap). The UAC prompt itself is a manual check.
  Linux (rlimits, landlock, seccomp BPF denying `AF_INET`/`AF_INET6` sockets and `execve`) and macOS
  (`sandbox_init` profile) are implemented behind `cfg` and type-checked with clippy for
  `x86_64-unknown-linux-gnu` and `x86_64-apple-darwin`; they run in CI on those runners.

- **Elevated operations** (§7, first user: `imt-plugin-dotnet`'s ETW recordings, ADR 0019): the host runs an
  operation declared `elevated` in the plugin's elevated process when the UI calls it, or when the plugin's normal
  process asks with `HostRequest::InvokeElevated` (`ToolCtx::elevated()`), which lets a plugin keep its state in the
  normal process and use the elevated one only for what needs the rights. The host accepts only operations the
  manifest declares elevated and passes its own data folder. Instead of streaming capture bytes back (§7), the
  elevated .NET process writes into the captures folder of that host-given data folder under a file name it
  checks; it never writes at a path it receives.

Deferred (tracked in `docs/redesign-2026-10.md`, R3):

- Splitting `imt-ai` into `imt-ai-proto` / `imt-ai-runtime` / `imt-ai` / `imt-browser-agent`, one `SafeHttp`, the
  assistant on the `Query`/`Analysis` ports (§8); the worker already links only `imt-ai`'s worker module and
  `imt-ipc`.
- One redactor `imt_core::redact` (§8).
- `imt-perfmon` and `imt-eventlog` as libraries (§4): the tools plugin still links `imt-core` (through
  `imt-collect` and `imt-monitor`); its process is sandboxed like any other plugin.
- Importer plugins streaming to the store worker directly (§5): needs the store worker in the workspace and its
  imports (ADR 0014 6c).
