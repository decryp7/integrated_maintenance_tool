# Extending Debrief

Debrief can be extended at three levels. Pick the lowest one that does the job:

| Level | What | Needs | Use it for |
|---|---|---|---|
| 1. Profiles | TOML configuration | nothing | new text/JSON/CSV layouts, extracting values, cleaning (pipeline), markers, charts |
| 2. Plugins | Rust crates on `debrief-plugin-api`, each run in its own sandboxed process | a Rust crate (and a rebuild of the plugins binary) | new file formats (text or binary), configuration parsers, custom pipeline steps, marker kinds, annotators, tools, settings |
| 3. UI plugins | Angular providers | a rebuild of the UI | new views (panels), commands, activity-bar buttons, inspector sections, field formatters |

A plugin (level 2) depends on one small crate, `crates/debrief-plugin-api` (not on `debrief-core`), and is served by
the `debrief-plugins` binary in a process of its own: a crash, a hang or a memory leak in it cannot take Debrief down,
and the process runs with the fewest rights the plugin asked for (see
[How a plugin process runs](#how-a-plugin-process-runs)). Plugins are written in Rust; there is no other
plugin language, no in-process plugin linking and no plugin protocol to implement by hand
([ADR 0017](adr/0017-one-ipc-out-of-process-plugins.md)).

**Writing a plugin outside Debrief's repository** (a third-party plugin): start with `cargo debrief-plugin new`, test with
the `debrief-plugin-test` harness, ship a signed `.debriefplugin` package that users install from the Plugins view. The
walkthrough is [plugin-sdk.md](plugin-sdk.md); the design is [ADR 0027](adr/0027-plugin-sdk.md). Third-party
plugins run contained by the launcher (an AppContainer on Windows) and may ask only for `network` and
`runPrograms`.

## How data flows

```
files ──▶ archive expansion (.gz/.zip) ──▶ importer (binary first, then text)
      ──▶ extractors ──▶ [[pipeline]] steps (built-in and plugin transforms) ──▶ marker rules
      ──▶ workspace (merge, sort, index) ──▶ queries · timeline · charts · exporters
```

Every extension point sits on one of these arrows, so a plugin never needs to know about the others.

## Level 1: parsers and profiles (no code)

A **parser** is a TOML file describing one log format: how Debrief recognises its files and how their lines
become events (ADR 0026). Parsers live in the library (Settings ▸ Parsers), can be shared as files, and Debrief
picks one per file. The built-in `crates/debrief-core/parsers/generic.toml` documents every key:

```toml
schema = 1
id = "orders"
name = "Order service"
version = "1.0.0"
[match]
files = ["orders-*.log"]
[format]                              # teach Debrief a text layout (Serilog outputTemplate or regex)
kind = "text"
output_templates = ["{Timestamp:yyyy-MM-dd HH:mm:ss.fff} [{Level:u3}] [{MachineName}] {Message:lj}{NewLine}{Exception}"]
```

A **profile** is a TOML file describing how events are analysed and enriched, whatever their format. The
built-in default, `crates/debrief-core/src/default_profile.toml`, documents every key, and
`samples/profiles/orderservice.debriefprofile.toml` is a worked example. In the app, the Profile panel
edits profiles as a form or TOML, with live previews.

Common tasks:

```toml
[[extract]]                           # numbers out of text, usable in charts and filters
name = "latency"
regex = 'took (?P<ElapsedMs>[\d.]+) ?ms'
[extract.fields]
ElapsedMs = { type = "number" }

[[pipeline]]                          # clean and enrich during import
step = "drop"
when = [{ field = "SourceContext", op = "contains", value = "HealthChecks" }]
```


## Level 2: plugins (Rust, out of process)

### Extension points

| Trait | Registered with | Purpose |
|---|---|---|
| `TextImporter` | `add_importer` | read a **text** format (`sniff` gets the decoded start of the file) |
| `BinaryImporter` | `add_binary_importer` | read a **binary** format (`sniff` gets raw bytes; asked before text importers) |
| `config::ConfigParser` | `add_config_parser` | parse a configuration format for the configuration diff |
| `Transform` | `add_transform` | a pipeline step, used from profiles as `{ step = "plugin", id = "…" }` |
| `markers::PluginMarkerKind` | `add_marker_kinds` | mark events on the timeline (crashes, boots, …) with rules or from a transform |
| `annotate::Annotator` | `add_annotator` | derive ranges and points (reboot windows, service uptime) from the records |
| `tools::Tool` | `add_tool` | typed operations the shell dispatches generically (see [Tools](#tools)) |
| `settings::Configurable` | `set_settings` | the plugin's section in Settings (see [Plugin settings](#plugin-settings)) |
| `ProfileImporter` | `add_profile_importer` | read a profile or heap snapshot format (the profiling plugin; see [Profiling](profiling.md)) |

All of them live in `debrief_plugin_api` and are bundled by a `Plugin`:

```rust
pub trait Plugin: Send + Sync + 'static {
    fn id(&self) -> &str;                       // reverse-DNS, unique: "com.acme.logs"
    fn name(&self) -> &str;
    fn version(&self) -> &str;                  // semver
    fn register(&self, c: &mut Contributions) -> anyhow::Result<()>;
    fn permissions(&self) -> Vec<PermissionRequest> { Vec::new() }   // see Manifests and permissions
    fn memory_limit(&self) -> Option<u64> { None }                   // bytes; None = the host's default (1 GiB)
}
```

`Contributions::manifest` turns what a plugin registered into its `Manifest` (see
[Manifests and permissions](#manifests-and-permissions)). The host refuses a plugin built for another
plugin protocol (`debrief_plugin_api::PROTOCOL`, checked in the handshake), with an invalid manifest, or with a duplicate contribution id; a plugin whose `register`
fails is listed as inactive with the reason.

Exporters, collectors, workspace hooks, services, events, extension points declared by plugins and plugin
dependencies exist inside `debrief-core` for the engine's own built-in code; they are not part of the plugin
contract and a plugin cannot use them.

### Step by step: a new file format

The complete, real examples are `crates/debrief-plugin-syslog` (text) and `crates/debrief-plugin-registry`.

1. **Create a crate** next to the others and add it to the workspace `members` in `Cargo.toml`. It
   depends on `debrief-plugin-api` only:

   ```toml
   # crates/debrief-plugin-acme/Cargo.toml
   [package]
   name = "debrief-plugin-acme"
   version = "0.1.0"
   edition = "2021"
   publish = false

   [dependencies]
   debrief-plugin-api = { path = "../debrief-plugin-api" }
   anyhow = "1.0"
   ```

2. **Implement the importer.** `sniff` returns a confidence from 0 (not mine) to 100. The built-ins
   score JSON lines 95, JSON documents 60, CSV 50–80 and matched text 70, so score above them
   only when your format is unambiguous (a magic header, a distinctive first line).

   ```rust
   use debrief_plugin_api::importer::{ImportCtx, Imported, TextImporter};
   use debrief_plugin_api::model::{Level, Record};

   pub struct AcmeImporter;

   impl TextImporter for AcmeImporter {
       fn id(&self) -> &str { "acme" }

       fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
           if head.starts_with("#ACME-LOG v1") { 100 } else { 0 }
       }

       fn import(&self, ctx: &ImportCtx, content: &str) -> anyhow::Result<Imported> {
           let mut out = Imported::default();
           for (i, line) in content.lines().enumerate().skip(1) {
               let Some((ts, rest)) = line.split_once('|') else { out.skipped += 1; continue };
               let mut r = Record::new(i as u32 + 1);
               r.ts = ctx.timestamps.parse(ts);      // honours the profile's formats and timezone
               r.level = if rest.contains("FAIL") { Level::Error } else { Level::Information };
               r.message = rest.trim().to_string();
               r.fields.insert("System".into(), "acme".into());
               out.records.push(r);
           }
           Ok(out)
       }
   }
   ```

   `ImportCtx` carries `file_name`, `path` (the last four components of the path, or the full path with the
   `OpenedFilePaths` permission), `file_date` (for logs that only have a time of day) and the profile's
   `timestamps`. Rules: never panic on any input (count bad lines in `skipped`), don't keep global state, and
   set `line` so the context view can show the original lines. Leave `id`, `source` and `markers` alone: the
   workspace assigns them. A text importer may also implement `import_tail` (live tail) and `multiline`.

3. **Bundle it in a plugin**:

   ```rust
   use debrief_plugin_api::{Contributions, Plugin};

   pub struct AcmePlugin;

   impl Plugin for AcmePlugin {
       fn id(&self) -> &str { "com.acme.logs" }
       fn name(&self) -> &str { "ACME logs" }
       fn version(&self) -> &str { env!("CARGO_PKG_VERSION") }
       fn register(&self, c: &mut Contributions) -> anyhow::Result<()> {
           c.add_importer(AcmeImporter);
           Ok(())
       }
   }
   ```

4. **Add it to the plugins binary**: add the crate to `crates/debrief-plugins/Cargo.toml` and one line to
   `builtins()` in `crates/debrief-plugins/src/main.rs`. Debrief reads every manifest once at start
   (`debrief-plugins --manifests`) and starts `debrief-plugins --plugin com.acme.logs` when the plugin is first needed:

   ```rust
   Arc::new(debrief_plugin_acme::AcmePlugin),
   ```

   Build the binary for the app with `scripts/build-plugins.sh` (release builds do it through
   `app/scripts/build-sidecars.mjs`); Debrief finds it next to its executable, `DEBRIEF_PLUGINS` overrides the path
   for development.

5. **Test it**: unit tests per format feature (call the importer directly), sniff positives and negatives
   (your sniffer must not claim the files under `/samples`), a garbage-input loop that must not panic, and a
   test over the real IPC in `crates/debrief-plugins/tests` (see `common::workspace`), where the plugin runs in its
   own process and the file comes through the connection:

   ```rust
   let mut ws = common::workspace(&["com.acme.logs"]);
   ws.open(&[fixtures_dir]).unwrap();
   assert_eq!(ws.summary().sources[0].importer, "acme");
   ```

### Binary formats

Implement `BinaryImporter` instead (`add_binary_importer`); it receives raw bytes (`sniff` gets the first
bytes). Binary importers are consulted before any text decoding, so a magic-number check is all `sniff`
needs. Example: the `.evtx` importer in `crates/debrief-plugin-windows`.

### Pipeline steps (record transforms)

```rust
use debrief_plugin_api::model::{Level, Record};
use debrief_plugin_api::{Transform, TransformCtx, Verdict};

pub struct TeamTagger;

impl Transform for TeamTagger {
    fn id(&self) -> &str { "acme.team-tagger" }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        if r.level == Level::Verbose { return Verdict::Drop; }
        let team = if ctx.source_name.starts_with("billing/") { "billing" } else { "orders" };
        r.fields.insert("Team".into(), team.into());
        Verdict::Keep
    }
}
// register: c.add_transform(TeamTagger);
```

Users then place it anywhere in their pipeline, with optional conditions:

```toml
[[pipeline]]
step = "plugin"
id = "acme.team-tagger"
when = [{ field = "Application", op = "eq", value = "OrderService" }]
```

Transforms run during import, in parallel across files, after extractors and before marker rules; records
cross to the plugin process in batches. Keep them deterministic and free of shared mutable state. Profiles
that reference a transform only validate where its plugin is installed.

### Plugin settings

A plugin with user-facing knobs (timeouts, paths, limits) describes them once and Debrief does the
rest: the Settings screen renders a form from the schema (one control per field kind, with
units, limits, help text and "Reset to default"), the engine validates every value, stores it in
`settings.json` in the config folder and hands the plugin only valid values, at startup, after
every change and after the plugin process restarted.

Settings are a separate, small trait, `debrief_plugin_api::settings::Configurable`, registered with
`Contributions::set_settings`:

```rust
pub trait Configurable: Send + Sync {
    fn settings_section(&self) -> SettingsSection;                  // the schema
    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()>;
}
```

Field kinds: `Bool`, `Int { min, max, step, unit }`, `Float { min, max, step, unit }`,
`Text { max_len, pattern }`, `Enum { options }`, `Path { dir }`, `Duration { min_ms, max_ms }`
(values in milliseconds), `Size { min_bytes, max_bytes }` (bytes) and `StringList { max_items }`.
Mark a field `needs_restart()` when it only takes effect after a restart (the UI then offers
one) and `as_advanced()` to hide it behind "Show advanced".

Worked example (the real one is `crates/debrief-plugin-windows/src/settings.rs`):

```rust
use debrief_plugin_api::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Shared by the settings object and the contributions that use the value.
#[derive(Default)]
pub struct Knobs { timeout_ms: AtomicU64 }

impl Knobs {
    pub fn timeout(&self) -> Duration { Duration::from_millis(self.timeout_ms.load(Ordering::Relaxed)) }
}

struct AcmeSettings(Arc<Knobs>);

impl Configurable for AcmeSettings {
    fn settings_section(&self) -> SettingsSection {
        // The id and owner are replaced by the plugin's; the title defaults to the plugin name.
        SettingsSection::new("acme", "Acme tools", SectionOwner::Core)
            .describe("How Acme talks to its converter.")
            .field(
                SettingField::new("timeout", "Converter timeout", FieldKind::Duration { min_ms: 1_000, max_ms: 600_000 }, 30_000)
                    .describe("How long one conversion may take before it is stopped."),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        // `values` passed validation: every field is present, typed and in range.
        let timeout = values.duration("timeout").ok_or_else(|| anyhow::anyhow!("missing timeout"))?;
        self.0.timeout_ms.store(timeout.as_millis() as u64, Ordering::Relaxed);
        Ok(())
    }
}

// in Plugin::register: c.set_settings(Arc::new(AcmeSettings(self.knobs.clone())));
```

How it behaves:

- The schema is read once, with the manifest, and checked (`SettingsSection::check`: ids, limits, patterns,
  valid defaults). An invalid schema gives a section without fields that says why; nothing is applied to it.
- `apply_settings` runs in the plugin process under the same crash containment as every other call: a
  failure is an error shown to the user, counts towards quarantine, and the values are not saved.
- If the plugin process restarts (crash, hang), the supervisor re-applies the stored values to the new one.
- Disabled and quarantined plugins still show their section, read-only, with a note.
- Values of plugins that are not installed are kept in `settings.json`, so uninstalling and
  reinstalling a plugin keeps its settings. Stored values are re-validated on every start;
  invalid ones fall back to the default (and are logged).
- Settings are user preferences, not profile configuration: anything that describes how to parse
  a log belongs in the profile instead.

App modules (for example the AI assistant, section id `ai`) register the same trait with
`SettingsService::register` in `app/src-tauri/src/settings.rs`.

### Manifests and permissions

`Contributions::manifest(plugin)` builds the `Manifest` the host reads before it runs any plugin code:
id, name, semver version, `protocol` (`debrief_plugin_api::PROTOCOL`, `major.minor`; a plugin built for another
major or a newer minor is refused with a message saying whether the plugin or Debrief needs an update), the permissions it asks for, an optional `memory_limit` (bytes, at most 4 GiB;
default 1 GiB) and the list of its contributions (importers, configuration parsers, marker kinds,
annotators, transforms, tools, settings, profile importers). `Manifest::check` rejects invalid ids and
duplicate contributions. Plugins do not depend on each other: a plugin is self-contained.

A plugin asks for rights beyond parsing what the host hands it:

```rust
fn permissions(&self) -> Vec<PermissionRequest> {
    vec![PermissionRequest { permission: Permission::RunPrograms, reason: "runs logman and wevtutil".into() }]
}
```

A plugin gets a right only when it asked for it **and** it is granted. The built-ins are first-party and are granted
what they ask for. A third-party plugin is granted what the user approved when installing it, and the Plugins view
revokes a permission later (the process restarts with fewer rights); see [plugin-sdk.md](plugin-sdk.md).
Everything else is locked down. What each one widens in the plugin's sandbox (`sandbox_for` in
`crates/debrief-plugin-host/src/sandbox.rs`):

| Permission | The plugin may | Sandbox change |
|---|---|---|
| none (default) | parse what the host hands it | no files, no network, no other programs, low integrity, memory cap |
| `OpenedFilePaths` | see the full paths of files the user opened and read them directly (e.g. to run `relog` on a `.blg`) | file system readable (still low integrity) |
| `RunPrograms` | start other programs (`logman`, `wevtutil`, `relog`) | child processes allowed, normal integrity (programs inherit its token), file system unrestricted |
| `WriteFolders` | write into folders the user chose (collector output) | normal integrity, file system unrestricted |
| `Network` | open network connections | network allowed |

Without `OpenedFilePaths` a plugin never sees a full path: an importer gets the file name and the last four
path components (`ImportCtx::path`) and reads the file through the connection (see
[How a plugin process runs](#how-a-plugin-process-runs)).

### Marker kinds

A plugin can contribute marker kinds for the timeline, with default rules in the same condition
language as a profile's `[[marker]] when`:

```rust
use debrief_plugin_api::markers::{MarkerRole, PluginMarkerKind};

c.add_marker_kinds(vec![
    PluginMarkerKind::new("app-crash", "Application crash")
        .when("Provider", "eq", "Application Error")
        .when("EventId", "eq", "1000")
        .role(MarkerRole::Crash)            // start / stop / crash: always shown on the timeline
        .icon("x-circle")
        .color("#F2545B")
        .describe("A process crashed."),
]);
```

Kinds are shown as `<plugin-id>/<kind>`, with `owner` (plugin id, `profile` or `builtin`) and
`role` in the summary's `markerKinds`. Priority (the first match is a record's primary marker):
the profile's own rules, then plugin kinds, then the built-in defaults. Users switch kinds off or
restyle them in the profile:

```toml
[[marker_override]]
kind = "net.decryptology.windows/app-crash"
enabled = false
```

A transform can also mark a record for logic a condition cannot express, with its own plugin's
kinds only (others are refused and noted in the Plugins view):

```rust
fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
    if looks_like_a_deadlock(r) { ctx.mark("deadlock"); }
    Verdict::Keep
}
```

At most 64 kinds fit in total; extra plugin kinds are dropped with a note. Kinds of a quarantined,
disabled or inactive plugin are hidden (records keep the bits; they reappear after a reset).
The Windows plugin (`crates/debrief-plugin-windows/src/markers.rs`) is the real example, with the
event ids and their sources documented.

### Annotators

Markers are points on one record; an annotator finds ranges (and points) across records:

```rust
use debrief_plugin_api::annotate::{Annotation, AnnotationView, Annotator, FieldMatch, RecordCursor, Severity, ViewFilter};

impl Annotator for RebootAnnotator {
    fn id(&self) -> &str { "windows.reboots" }
    fn annotate(&self, view: &dyn AnnotationView, since: Option<RecordCursor>) -> anyhow::Result<Vec<Annotation>> {
        let boots = view.select(&ViewFilter { when: vec![/* EventId 6005 … */], ..Default::default() }, 10_000)?;
        // … pair shutdowns with boots per Computer …
        Ok(vec![Annotation { kind: "downtime".into(), from, to, instance: None,
            on: Some(FieldMatch { field: "Computer".into(), value: computer }),
            label: "Reboot of WEB01".into(), severity: Severity::Info, evidence }])
    }
}
// register: c.add_annotator(RebootAnnotator);
```

`AnnotationView` is read-only and bounded, and in a plugin process every call goes to the host (which holds
the events): `select` (conditions and time bounds, oldest or newest first, at most 100,000 records),
`instances` and `time_range`. Annotators run after every import and after live-tail appends (`since` is the
first new record); each run returns the annotator's complete set, which replaces the previous one. Runs are
guarded with the `annotate` budget and capped at 10,000 annotations per annotator. `evidence` is the query
that shows an annotation's records (free text, time range, field filters). The UI reads annotations with
`LogStore::annotations({ from, to, instance })`.

### Tools

A tool is a set of typed operations the shell runs on the user's request: controlling Performance
Monitor, running the process monitor, collecting a diagnostics bundle. The shell knows no tool; it
has two generic commands, `tools_list` and `tool_invoke { tool, op, args }`, so a tool can be
disabled, quarantined and left out in safe mode like any contribution (ADR 0002).

```rust
use debrief_plugin_api::tools::{OpSpec, PathAccess, PathKind, Platform, Schema, Tool, ToolCtx, ToolInfo, ToolTask};

impl Tool for Sets {
    fn id(&self) -> &str { "perfmon.sets" }
    fn describe(&self) -> ToolInfo {
        ToolInfo::new("perfmon.sets", "Performance Monitor collector sets")
            .only_on(&[Platform::Windows])          // listed elsewhere as unsupported, with the reason
            .op(OpSpec::new("control", "Start or stop a set")
                .args(Schema::object().field("name", Schema::text(256)).field("action", Schema::one_of(["start", "stop"])))
                .budget(Some(Duration::from_secs(900))))
            .op(OpSpec::new("export", "Save a set's counters")
                .args(Schema::object().field("target", Schema::path(PathKind::File, PathAccess::Write)))
                .elevated())                        // runs in an elevated process (UAC prompt on Windows)
    }
    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> anyhow::Result<Value> {
        // `args` already matched the op's schema; parse it into your own types.
        ctx.task_started(ToolTask { id: "perfmon:Debrief".into(), label: "PerfMon collection".into(), kind: "perfmon".into(), stoppable: true });
        Ok(Value::Null)
    }
    fn stop(&self, task: &str, ctx: &ToolCtx) -> anyhow::Result<()> { /* stop it */ ctx.task_finished(task); Ok(()) }
}
// register: c.add_tool(Arc::new(Sets));
```

- `tools_list` reports every tool with its `availability`: `available`, `unsupported` (another
  platform; the reason names it) or `pluginUnavailable` (the plugin is inactive, quarantined or not
  responding; the plugin's reason).
- `tool_invoke` checks the arguments against the op's `Schema` (errors name the location:
  `perfmon.sets control: args.action "x" is not one of: start, stop`), then runs the op in the plugin's
  process under the plugin's guard: an op that runs past its `budget` (default 30 s) fails and the process
  is replaced.
- **Typed path arguments.** Declare a path as `Schema::path(PathKind::File | Dir, PathAccess::Read | Write)`,
  never as text. The host canonicalises it and checks it against the window's path grants (the places the
  user chose through a dialog, drag-and-drop or the command line); a path outside them is refused with
  `notGranted` and the tool never runs. The tool receives the checked, canonical path.
- **Elevated operations.** `OpSpec::elevated()` marks an operation that needs administrator rights. The
  host starts a separate elevated process of the plugin for these operations only (on Windows through a UAC
  prompt) and serves only the elevated operations there; everything else stays unelevated. The plugin's
  normal process can run them itself with `ctx.elevated()` (`ElevatedOps::invoke`), so its state stays
  unelevated and the elevated process does only what needs the rights; the host accepts only operations the
  manifest declares elevated, and a declined prompt is a `ToolFailure::denied`. The elevated process writes
  nothing at paths it receives (the .NET plugin's ETW recorder writes only into the captures folder of the data
  folder the host passes, ADR 0019). `ToolInfo::needs_elevation` is the tool-level hint for the UI.
- Background work is reported with `ctx.task_started/task_progress/task_finished`; the shell lists
  it in the tray and status bar and stops it through `Tool::stop`.

The UI reads `tools_list` (`ToolCatalog`): panels and commands of tools that are not listed are
hidden (safe mode, plugin disabled), and a listed tool whose plugin cannot serve it shows the
reason (`debrief-tool-unavailable`).

### Time budgets, the watchdog and supervision

Every call to a plugin has a time budget per kind (`guard::Budgets`): sniff 5 s, import 120 s, live-tail
parse 10 s, transform 250 ms per record, annotator run 10 s, tool operations 30 s unless the op sets its own
`budget`. The host's deadline is 90% of the budget, so the call fails with a message before the watchdog marks
the plugin hung. Because every plugin is a process, a hang or a crash is contained:

| Failure | What happens |
|---|---|
| The plugin panics or the process dies (abort, stack overflow, out of memory, killed by the sandbox's limits) | the call fails with an error attributed to the plugin; the supervisor restarts the process on next use with exponential backoff (500 ms, doubling, at most 8 s) and re-applies the plugin's settings |
| A call overruns its deadline (a hang or an endless loop) | the call fails, the process is killed and replaced; the plugin is marked `hung` in the Plugins view |
| A frame is larger than the connection's limit (16 MiB) or malformed | the frame is refused and the connection is closed; counted as a failure |
| Repeated failures (three within ten minutes) | the plugin is **quarantined**: no more restarts and its contributions are hidden until the user resets it in the Plugins view |

Failures also go through the engine's `PluginRegistry` guards, so plugin status, quarantine, enabling and
disabling and safe mode work the same for every plugin.

### How a plugin process runs

The details are in [ADR 0017](adr/0017-one-ipc-out-of-process-plugins.md); what a plugin author needs to know:

- **One loop.** A plugin's `main` is `debrief_plugin_api::serve(Arc::new(MyPlugin))`; `debrief-plugins` does it for
  every built-in. The app links no plugin crate; `crates/debrief-plugin-host` registers a proxy for each
  contribution into `debrief-core`'s `PluginRegistry`.
- **Frames.** Host and child talk over `crates/debrief-ipc`: frames of a `u32` little-endian length and a
  postcard payload (`Hello`, `Welcome`, `Call`, `Reply`, `Item`, `Fail`, `Cancel`, `Credit`), at most 16 MiB
  each; streamed answers use credits so a slow reader is never flooded. The host listens on a private
  endpoint and the child connects with a 256-bit token, and the host checks the peer's process id.
  Descriptive types (the manifest) cross as JSON, event rows and file chunks as postcard.
- **Handshake.** `Hello` carries the frame version, the plugin protocol version and the token; the host answers
  `Welcome` with its own protocol version, then reads the manifest. A wrong token or process id closes the
  connection; a protocol it cannot serve is refused with a message saying which side to update.
- **Sandbox.** The launcher starts the child with a cleared environment inside the sandbox of `sandbox_for`:
  Windows runs it under a job object (CPU and child-process limits) at low integrity; Linux uses
  `RLIMIT_CPU`, landlock and a seccomp filter that denies network sockets and `execve`; macOS uses a sandbox
  profile. Permissions widen it as in the table above. The memory cap (1 GiB unless the manifest asks for more, at most
  4 GiB) is the same on every platform: a host watcher (`watch_memory` in `crates/debrief-ipc/src/process.rs`) reads the
  child's resident memory and stops it over the cap with a message that says so. There are no memory rlimits, because
  they count the address space that allocators such as mimalloc reserve up front. File confinement is kept on all
  platforms: landlock and seccomp on Linux, an AppContainer on Windows, a sandbox profile on macOS.
- **Host files.** For a file, the host opens it and the plugin reads ranges through the connection
  (up to 1 MiB per read, for the duration of the call); the plugin sees a file reference and a name, never a
  path, unless it was granted `OpenedFilePaths`.
- **Callbacks.** Annotators read the host's events through view calls; tools report tasks with
  `TaskStarted`/`TaskProgress`/`TaskFinished`.
- **Elevation.** An elevated operation starts the plugin's second process through the platform's elevation
  (UAC on Windows) with a bootstrap file the child reads and deletes; it serves only the plugin's elevated
  operations.
- **Other children.** The AI worker (`debrief-ai-worker`) and the store worker (`debrief-store-worker`) use the same
  frames; the profiling plugin's process is the profile worker.

### Plugin rules

- Follow `AGENTS.md`: no panics on user data, typed errors at the edges, tests with high
  coverage, permissive licences only (`cargo deny check`).
- Keep ids stable: users reference transform and marker ids from their profiles.
- Bump `version` for behaviour changes; `PROTOCOL` changes only when the plugin protocol changes (minor: something
  added at the end; major: anything else).

## Level 3: UI plugins (Angular)

### UI contributions

A UI plugin is a set of contributions registered with one provider call in `app/src/app/app.config.ts`
(`provideDebriefPlugin`, public API in `app/src/app/extensions/index.ts`). Built-in features use the same extension
points, so whatever a plugin adds behaves like the built-in parts:

| Contribution | Token | What it adds |
|---|---|---|
| `DebriefView` | `DEBRIEF_VIEWS` | a dockable panel, loaded lazily, listed in the View menu under its `category`; its optional `layout` (`area`, `min`, `pref`, `priority`; see below) says where it opens and what it needs ([ADR 0035](adr/0035-view-areas-and-perspectives.md), [ADR 0056](adr/0056-layout-by-declared-constraints.md)) |
| `DebriefCommand` | `DEBRIEF_COMMANDS` | a palette / menu action with an optional key binding |
| `DebriefActivityItem` | `DEBRIEF_ACTIVITY_ITEMS` | a button in the left activity bar |
| `DebriefInspectorSection` | `DEBRIEF_INSPECTOR_SECTIONS` | a block in the event inspector |
| `DebriefFieldFormatter` | `DEBRIEF_FIELD_FORMATTERS` | display formatting of a field |

**View layout and the UI plugin API version.** `UI_PLUGIN_API_VERSION` (exported from `extensions`) is 2: version 2 added
`DebriefView.layout`, which replaces `minSize` and the old position-only placement. A plugin may declare
`apiVersion` in `provideDebriefPlugin({ apiVersion: 2, ... })`; a plugin without it is treated as version 1 and its
views get the default layout of their area, and one that needs a newer contract than the build provides is refused
with a message naming both versions.

```ts
layout: {
  area: 'side',                      // 'document' | 'analysis' | 'side' | 'status'
  min:  { width: 320, height: 240 }, // the dock never makes the view smaller than this
  pref: { width: 372, height: 400 }, // its opening size when there is room
  priority: 50,                      // 1-100; higher stays visible, lower is tabbed or auto-hidden first
}
```

Every part is optional except `area`; a missing part takes the area's default (side 320 x 240 preferring 372, analysis
420 x 180, document 420 x 240, status 320 x 180). The declaration is parsed once at registration: numbers are bounded,
`pref` is lifted to `min`, `priority` is clamped to 1-100, and an unusable declaration is ignored (the area default
applies). When there is no room the dock tabs or auto-hides the lowest-priority view instead of squeezing anything
below its minimum, and returns it when the window grows again. Floating and popped-out windows are outside the plan.

**Activity-bar buttons.** Every button of the left bar (Explorer, Search, Timeline, Analysis, Metrics, Live tail,
AI, Tools, Profile editor, Settings) is an `DebriefActivityItem` contributed by the feature that owns it. An item shows
a view (`run: { view }`: highlighted while the view is open, hidden while the view is not offered), runs a command
(`run: { command }`: the tooltip shows the command's key binding), or opens a fly-out menu (`run: { menu }`, built
on every opening; `viewCategoryMenu('Tools')` lists the offered views of a View-menu category). `order` places it
(built-ins use multiples of 100: Explorer 100, Search 200, Timeline 300, Analysis 400, Metrics 500, Live tail 600,
AI 700, Tools 800, Profile editor 900; `section: 'bottom'` pins Settings below); higher orders collapse into the More
fly-out first when the window is short. The optional `when`, `badge`, `active`, `toggle`, `pulse` and `disabled`
(a reason, shown as the tooltip) receive the `CommandContext` and may read signals. Contributions are validated: a
duplicate id or a malformed item is logged and skipped, and a function that throws hides the item (`when`) or drops
just that extra, so a plugin bug never breaks the bar.

```ts
// my-plugin.ts: a panel, a command that opens it, and an activity-bar button between Analysis and Metrics.
export function provideMyPlugin(): EnvironmentProviders {
  return provideDebriefPlugin({
    views: [{ id: 'my-view', title: 'My view', icon: 'puzzle', component: () => import('./my-view').then((m) => m.MyView) }],
    commands: [{ id: 'my.show', title: 'Show my view', keybinding: 'Mod+Alt+M', run: (ctx) => ctx.showView('my-view') }],
    activityItems: [
      // The tooltip reads "My view (Ctrl+Alt+M)": the binding comes from the command.
      { id: 'my-plugin', label: 'My view', icon: 'puzzle', order: 450, run: { command: 'my.show' }, badge: (ctx) => ctx.summary?.sources.length ?? null },
    ],
  });
}
```

A feature whose item reads services (signals for `active` or `badge`) registers a factory instead, one line in its
`provide…()` function: `{ provide: DEBRIEF_ACTIVITY_ITEMS, multi: true, useFactory: () => myItems(inject(MyStore)) }`.
