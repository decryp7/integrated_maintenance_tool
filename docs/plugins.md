# Extending IMT

IMT can be extended at three levels. Pick the lowest one that does the job:

| Level | What | Needs | Use it for |
|---|---|---|---|
| 1. Profiles | TOML configuration | nothing | new text/JSON/CSV layouts, extracting values, cleaning (pipeline), markers, charts |
| 2. Engine plugins | Rust crates implementing `imt_core` traits | a rebuild of IMT | new file formats (text or binary), custom pipeline steps, exporters, collectors, reacting to workspace events |
| 3. UI plugins | Angular providers | a rebuild of the UI | new views (panels), commands, inspector sections, field formatters |

Runtime-loaded plugins (no rebuild) are planned as WASM modules implementing the same interfaces;
see [Roadmap](roadmap.md), phase 6.

## How data flows

```
files ──▶ archive expansion (.gz/.zip) ──▶ importer (binary first, then text)
      ──▶ extractors ──▶ [[pipeline]] steps (built-in and plugin transforms) ──▶ marker rules
      ──▶ workspace (merge, sort, index) ──▶ queries · timeline · charts · exporters
collectors ──▶ EventSink (e.g. rolling CLEF files) ──▶ opened like any other file
```

Every extension point sits on one of these arrows, so a plugin never needs to know about the others.

## Level 1: profiles (no code)

A profile is a TOML file describing how files are parsed and enriched. The built-in default,
`crates/imt-core/src/default_profile.toml`, documents every key, and
`samples/profiles/orderservice.imtprofile.toml` is a worked example. In the app, the Profile panel
edits profiles as a form or TOML, with live previews.

Common tasks:

```toml
[parse]                               # teach IMT a text layout (Serilog outputTemplate or regex)
output_templates = ["{Timestamp:yyyy-MM-dd HH:mm:ss.fff} [{Level:u3}] [{MachineName}] {Message:lj}{NewLine}{Exception}"]

[[extract]]                           # numbers out of text, usable in charts and filters
name = "latency"
regex = 'took (?P<ElapsedMs>[\d.]+) ?ms'
[extract.fields]
ElapsedMs = { type = "number" }

[[pipeline]]                          # clean and enrich during import
step = "drop"
when = [{ field = "SourceContext", op = "contains", value = "HealthChecks" }]
```

## Level 2: engine plugins (Rust)

### Extension points

| Trait | Registered with | Purpose |
|---|---|---|
| `importers::Importer` | `add_importer` | read a **text** format (`sniff` gets the decoded start of the file) |
| `importers::BinaryImporter` | `add_binary_importer` | read a **binary** format (`sniff` gets raw bytes; asked before text importers) |
| `plugin::RecordTransform` | `add_transform` | a pipeline step, used from profiles as `{ step = "plugin", id = "…" }` |
| `export::Exporter` | `add_exporter` | write events in a new format (streamed) |
| `plugin::Collector` | `add_collector` | produce events (monitors, probes) into an `EventSink` |
| `plugin::WorkspaceHook` | `add_hook` | be told when files are loaded, the workspace is cleared, or the profile changes |
| `plugin::markers::PluginMarkerKind` | `add_marker_kinds` | mark events on the timeline (crashes, boots, …) with rules or from a transform |
| `plugin::annotate::Annotator` | `add_annotator` | derive ranges and points (reboot windows, service uptime) from the records |
| any trait | `provide` / `service` | an API other plugins call (see [Services](#services)) |
| any trait | `declare_extension_point` / `contribute` | let other plugins extend yours (see [Extension points](#extension-points)) |
| typed events | `events().subscribe` | react to imports, appends, alerts, profile and plugin state (see [Events](#events)) |

All of them live in `imt_core` and are bundled by a `Plugin`:

```rust
pub trait Plugin: Send + Sync {
    fn id(&self) -> &'static str;          // reverse-DNS, unique: "com.acme.tools"
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;     // semver; "1" and "1.2" are read as 1.0.0 and 1.2.0
    fn api_version(&self) -> u32 { imt_core::API_VERSION }
    fn register(&self, registry: &mut imt_core::PluginRegistry) -> anyhow::Result<()>;
    fn configurable(&self) -> Option<Arc<dyn Configurable>> { None }   // see Plugin settings
    fn manifest(&self) -> Manifest { /* id, version, api_version, no dependencies */ }
    fn activate(&self, ctx: &ActivationCtx) -> anyhow::Result<()> { Ok(()) }
    fn deactivate(&self) {}
}
```

`PluginRegistry::install` rejects plugins built for another `API_VERSION`, invalid manifests,
duplicate plugin ids and duplicate contribution ids, and records what each plugin contributed
(shown in the app's Plugins panel). A plugin whose `register` fails is listed as `inactive` with
the reason, and so are the plugins that need it.

### Step by step: a new file format

The complete, real example is `crates/imt-plugin-registry` (Windows `.reg` exports).

1. **Create a crate** next to the others and add it to the workspace `members` in `Cargo.toml`:

   ```toml
   # crates/imt-plugin-acme/Cargo.toml
   [package]
   name = "imt-plugin-acme"
   version = "0.1.0"
   edition = "2021"
   publish = false

   [dependencies]
   imt-core = { path = "../imt-core" }
   anyhow = "1.0"
   serde_json = { version = "1.0", features = ["preserve_order"] }
   ```

2. **Implement the importer.** `sniff` returns a confidence from 0 (not mine) to 100. The built-ins
   score JSON lines 95, JSON documents 60, CSV 50–80 and matched text 70, so score above them
   only when your format is unambiguous (a magic header, a distinctive first line).

   ```rust
   use imt_core::importers::{ImportCtx, Imported, Importer};
   use imt_core::model::{Level, Record};

   pub struct AcmeImporter;

   impl Importer for AcmeImporter {
       fn id(&self) -> &'static str { "acme" }

       fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
           if head.starts_with("#ACME-LOG v1") { 100 } else { 0 }
       }

       fn import(&self, ctx: &ImportCtx, content: &str) -> anyhow::Result<Imported> {
           let mut out = Imported::default();
           for (i, line) in content.lines().enumerate().skip(1) {
               let Some((ts, rest)) = line.split_once('|') else { out.skipped += 1; continue };
               let mut r = Record::new(i as u32 + 1);
               r.ts = ctx.profile.timestamps.parse(ts);      // honours the profile's formats and timezone
               r.level = if rest.contains("FAIL") { Level::Error } else { Level::Information };
               r.message = rest.trim().to_string();
               r.fields.insert("System".into(), "acme".into());
               out.records.push(r);
           }
           Ok(out)
       }
   }
   ```

   Rules: never panic on any input (count bad lines in `skipped`), don't keep global state, and set
   `line` so the context view can show the original lines. Leave `id`, `source` and `markers` alone:
   the workspace assigns them.

3. **Bundle it in a plugin**:

   ```rust
   pub struct AcmePlugin;

   impl imt_core::Plugin for AcmePlugin {
       fn id(&self) -> &'static str { "com.acme.logs" }
       fn name(&self) -> &'static str { "ACME logs" }
       fn version(&self) -> &'static str { env!("CARGO_PKG_VERSION") }
       fn register(&self, r: &mut imt_core::PluginRegistry) -> anyhow::Result<()> {
           r.add_importer(Box::new(AcmeImporter));
           Ok(())
       }
   }
   ```

4. **Register it in the app**: add the crate to `app/src-tauri/Cargo.toml` and one line to
   `builtin_plugins()` in `app/src-tauri/src/lifecycle.rs` (order does not matter; activation
   follows the dependency graph):

   ```rust
   Arc::new(imt_plugin_acme::AcmePlugin),
   ```

5. **Test it** (see the registry plugin's tests for the pattern): unit tests per format feature,
   sniff positives and negatives (your sniffer must not claim the files under `/samples`), a
   garbage-input loop that must not panic, and a workspace-level test:

   ```rust
   let mut registry = imt_core::PluginRegistry::new();
   registry.install(&AcmePlugin).unwrap();
   let mut ws = imt_core::Workspace::with_registry(imt_core::Profile::default_profile(), registry).unwrap();
   ws.open(&[fixtures_dir]).unwrap();
   assert_eq!(ws.summary().sources[0].importer, "acme");
   ```

### Binary formats

Implement `BinaryImporter` instead; it receives raw bytes (`head` is the first 64 KiB). Binary
importers are consulted before any text decoding, so a magic-number check is all `sniff` needs.
Example: the `.evtx` importer in `crates/imt-plugin-windows`.

### Pipeline steps (record transforms)

```rust
use imt_core::plugin::{RecordTransform, TransformCtx, Verdict};

pub struct TeamTagger;

impl RecordTransform for TeamTagger {
    fn id(&self) -> &str { "acme.team-tagger" }
    fn apply(&self, r: &mut imt_core::model::Record, ctx: &TransformCtx) -> Verdict {
        if r.level == imt_core::model::Level::Verbose { return Verdict::Drop; }
        let team = if ctx.source_name.starts_with("billing/") { "billing" } else { "orders" };
        r.fields.insert("Team".into(), team.into());
        Verdict::Keep
    }
}
// register: r.add_transform(std::sync::Arc::new(TeamTagger))?;
```

Users then place it anywhere in their pipeline, with optional conditions:

```toml
[[pipeline]]
step = "plugin"
id = "acme.team-tagger"
when = [{ field = "Application", op = "eq", value = "OrderService" }]
```

Transforms run during import, in parallel across files, after extractors and before marker rules:
keep them deterministic and free of shared mutable state. Profiles that reference a transform
only validate where its plugin is installed.

### Exporters

```rust
use imt_core::export::{ExportRow, Exporter};

pub struct Markdown;

impl Exporter for Markdown {
    fn id(&self) -> &'static str { "markdown" }
    fn name(&self) -> &'static str { "Markdown table" }
    fn extension(&self) -> &'static str { "md" }
    fn export(&self, rows: &mut dyn Iterator<Item = ExportRow<'_>>, out: &mut dyn std::io::Write) -> anyhow::Result<usize> {
        writeln!(out, "| Time | Level | Instance | Message |\n|---|---|---|---|")?;
        let mut n = 0;
        for row in rows {
            writeln!(out, "| {:?} | {} | {} | {} |", row.record.ts, row.record.level.as_str(), row.group, row.record.message.replace('|', "\\|"))?;
            n += 1;
        }
        Ok(n)
    }
}
```

Exporters stream: write each row as it arrives and never collect them. The Export dialog lists
every registered exporter automatically.

### Collectors

A collector produces events instead of reading files. The host runs it on its own thread until
`stop` is set; it writes batches to an `EventSink` (for files, `ClefFileSink` gives rotating
Serilog-compatible CLEF that IMT opens like any log). See `crates/imt-monitor`.

```rust
fn run(&self, config: &serde_json::Value, sink: &dyn imt_core::export::EventSink,
       stop: &std::sync::atomic::AtomicBool) -> anyhow::Result<()>
```

### Workspace hooks

`WorkspaceHook` methods (`on_loaded`, `on_cleared`, `on_profile_changed`) all have empty
defaults; implement only what you need. They run synchronously on the thread that changed the
workspace, so keep them fast and hand anything slow to your own thread.

### Plugin settings

A plugin with user-facing knobs (timeouts, paths, limits) describes them once and IMT does the
rest: the Settings screen renders a form from the schema (one control per field kind, with
units, limits, help text and "Reset to default"), the engine validates every value, stores it in
`settings.json` in the config folder and hands the plugin only valid values, at startup and after
every change.

Settings are a separate, small trait, `imt_core::settings::Configurable`, returned from the
defaulted `Plugin::configurable` method (existing plugins need no change):

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

Worked example (the real one is `crates/imt-plugin-windows/src/settings.rs`):

```rust
use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
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

pub struct AcmePlugin { knobs: Arc<Knobs> }

impl imt_core::Plugin for AcmePlugin {
    // id, name, version as usual …
    fn register(&self, registry: &mut imt_core::PluginRegistry) -> anyhow::Result<()> {
        registry.add_importer(Box::new(AcmeImporter { knobs: self.knobs.clone() })); // reads knobs.timeout()
        Ok(())
    }

    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        Some(Arc::new(AcmeSettings(self.knobs.clone())))
    }
}
```

How it behaves:

- `configurable()` and `settings_section()` are called once, when the plugin is installed, and
  the schema is checked (`SettingsSection::check`: ids, limits, patterns, valid defaults). A panic
  or an invalid schema gives a section without fields that says why; nothing is ever applied to
  it.
- `apply_settings` runs under the same crash containment as every other contribution: a panic
  becomes an error shown to the user, counts towards quarantine, and the values are not saved.
- Disabled and quarantined plugins still show their section, read-only, with a note.
- Values of plugins that are not installed are kept in `settings.json`, so uninstalling and
  reinstalling a plugin keeps its settings. Stored values are re-validated on every start;
  invalid ones fall back to the default (and are logged).
- Settings are user preferences, not profile configuration: anything that describes how to parse
  a log belongs in the profile instead.

App modules (for example the AI assistant, section id `ai`) register the same trait with
`SettingsService::register` in `app/src-tauri/src/settings.rs`.

### Manifests and dependencies

A plugin describes itself with a `Manifest`: id, semver version, plugin API level, the plugins it
needs and the services it provides. The default (no dependencies) is derived from `id`,
`version` and `api_version`, so plugins that need nothing need no change.

```rust
use imt_core::plugin::Manifest;

fn manifest(&self) -> Manifest {
    Manifest::new(self.id(), self.version())
        .requires("net.decryptology.perfmon", "^1.0")     // cannot work without it
        .optionally("net.decryptology.windows", ">=0.1")  // used when present
        .provides("acme.symbols", "1.2.0")                // services it registers
}
```

Bad values (a version or requirement that does not parse, a self-dependency, duplicates) are
reported at install with the value that is wrong.

The registry resolves the graph after every install:

| Situation | Result |
|---|---|
| A required plugin is missing | `inactive`: `needs X ^1.0 (not installed)` |
| …has an incompatible version | `inactive`: `needs X ^1.0 (installed 0.9.0)` |
| …is disabled / quarantined | `inactive`: `needs X which is disabled` / `…quarantined` |
| …is itself inactive | `inactive`: `needs X which is inactive (needs Y which is quarantined)` (the chain) |
| Plugins require each other in a cycle | all of them `inactive`: `dependency cycle: a → b → a` |
| An optional plugin is unavailable | stays `active`; a warning (`optional: needs …`) |

`PluginInfo` (the Plugins panel) carries `requires`, `provides`, `inactiveReason`, `dependents`
(plugins that require this one), `warnings` and `hung`. An inactive plugin's contributions stay
registered but every call to them is refused with that reason (an importer sniffs 0, a transform
leaves records unchanged, an exporter returns the error).

**Cascade.** Quarantining a provider (after repeated crashes) or disabling it at run time
(`PluginRegistry::set_enabled`) makes its dependents inactive, transitively; resetting it
(`reset_plugin`) or enabling it again reactivates them. A `PluginStateChanged` event is published
for every change.

**Activation.** Once every plugin is installed, `PluginRegistry::resolve()` (the app calls it;
`Workspace::with_registry` too) runs `activate` hooks in dependency order: a plugin's
dependencies are active before its own hook runs, so this is where it looks up services.
`deactivate` runs in reverse order when a plugin stops being active and on `shutdown()`. An error
from `activate` leaves the plugin inactive (`activation failed: …`) until it is reset. Hooks run
only for plugins installed with `install_shared(Arc<dyn Plugin>)`: `install(&plugin)` cannot keep
the plugin, so its hooks are not called (everything else works the same).

### Services

Inter-plugin APIs are Rust traits in small, separately versioned API crates (`crates/imt-api-*`).
`crates/imt-api-perf` is the example: `trait CounterSource` (live performance counters), its
service id and version, and `contract_tests(&dyn CounterSource)`, which every provider must pass
in a `#[test]`.

```rust
// Provider, in register():
registry.provide::<dyn CounterSource>(imt_api_perf::SERVICE_ID, imt_api_perf::VERSION, Arc::new(Pdh::new()))?;

// Consumer: declare the dependency, look the service up in activate().
fn manifest(&self) -> Manifest {
    Manifest::new(self.id(), self.version()).requires("acme.perf", "^1")
}
fn activate(&self, ctx: &ActivationCtx) -> anyhow::Result<()> {
    let counters = ctx.service::<dyn CounterSource>(imt_api_perf::SERVICE_ID, "^1")?;
    *self.counters.lock().unwrap() = Some(counters);
    Ok(())
}

// Using it: the closure runs under the provider's guard and time budget.
let paths = counters.try_call(|c| c.counters())?;
let n = counters.call(|c| c.counters().map(|v| v.len()))??;
```

Lookups check the id, the trait (`TypeId`) and the semver requirement, and fail with a message
that says which one did not match (`service "x" ^2 is not available: plugin "p" provides version
1.4.0`). The `Service` handle never exposes the bare `Arc`: a panicking provider becomes an
error counted against the provider (and quarantines it), and a quarantined, hung or inactive
provider is refused (`service "x": provider "p" is quarantined`). `call_detached` runs the call on
a helper thread and stops waiting when the service budget runs out.

No built-in plugin provides `CounterSource` yet: the Windows plugin imports PerfMon logs as
records (data over calls), and live PDH sampling lives in `imt-monitor`'s collector. The contract
tests run against a test-only provider.

### Extension points

A plugin can define its own contribution point (an id and a trait); others contribute
implementations, in any install order:

```rust
// The profiler declares it…
registry.declare_extension_point::<dyn SymbolResolver>("acme.profiler.symbols")?;
// …a symbols plugin contributes…
registry.contribute::<dyn SymbolResolver>("acme.profiler.symbols", Arc::new(PdbResolver));
// …and the profiler collects them (each a guarded `Service`, attributed to its contributor).
for r in ctx.contributions::<dyn SymbolResolver>("acme.profiler.symbols") {
    if let Ok(name) = r.call(|r| r.resolve(addr)) { /* … */ }
}
```

A contribution to a point nobody declares (or with another trait) is not an error: the
contributing plugin stays active, shows a warning, and the contribution is not returned.

### Events

`registry.events()` (also `Workspace::events()` and `ActivationCtx::events()`) is a typed bus.
The engine publishes `RecordsAppended { count, sources }` (after imports and live-tail appends),
`LiveAlertRaised(LiveAlert)`, `ProfileChanged { name }`, `WorkspaceCleared` and
`PluginStateChanged { id, status }`; the app publishes `SnapshotTaken { id }`. Any
`Send + Sync + 'static` type is an event, so plugins define their own by declaring a struct.

```rust
registry.events().subscribe::<RecordsAppended>(self.id(), move |e| {
    stats.lock().unwrap().added += e.count;   // runs on the subscriber's own thread
})?;
```

Each subscriber has its own worker thread and a bounded queue (1024 events): events arrive in
publish order, `publish` never blocks the engine, and when a slow subscriber falls behind its
oldest events are dropped and counted. Delivery runs under the subscriber plugin's guard (a panic
counts towards quarantine; a quarantined or inactive subscriber is skipped). `metrics()` reports
`delivered`, `dropped`, `failures`, `skipped` and `queued` per subscriber.

### Marker kinds

A plugin can contribute marker kinds for the timeline, with default rules in the same condition
language as a profile's `[[marker]] when`:

```rust
use imt_core::plugin::markers::{MarkerRole, PluginMarkerKind};

registry.add_marker_kinds(vec![
    PluginMarkerKind::new("app-crash", "Application crash")
        .when("Provider", "eq", "Application Error")
        .when("EventId", "eq", "1000")
        .role(MarkerRole::Crash)            // start / stop / crash: always shown on the timeline
        .icon("x-circle")
        .color("#F2545B")
        .describe("A process crashed."),
])?;
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
kinds only (others are refused and noted in the Plugins panel):

```rust
fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
    if looks_like_a_deadlock(r) { ctx.mark("deadlock"); }
    Verdict::Keep
}
```

At most 64 kinds fit in total; extra plugin kinds are dropped with a note. Kinds of a quarantined,
disabled or inactive plugin are hidden (records keep the bits; they reappear after a reset).
The Windows plugin (`crates/imt-plugin-windows/src/markers.rs`) is the real example, with the
event ids and their sources documented.

### Annotators

Markers are points on one record; an annotator finds ranges (and points) across records:

```rust
impl Annotator for RebootAnnotator {
    fn id(&self) -> &str { "windows.reboots" }
    fn annotate(&self, view: &dyn AnnotationView, since: Option<RecordCursor>) -> anyhow::Result<Vec<Annotation>> {
        let boots = view.select(&ViewFilter { when: vec![/* EventId 6005 … */], ..Default::default() }, 10_000)?;
        // … pair shutdowns with boots per Computer …
        Ok(vec![Annotation { kind: "downtime".into(), from, to, instance: None,
            on: Some(FieldMatch { field: "Computer".into(), value: computer }),
            label: "Reboot of WEB01".into(), severity: Severity::Info, evidence_query }])
    }
}
// register(): registry.add_annotator(Arc::new(RebootAnnotator))?;
```

`AnnotationView` is read-only and bounded: `select` (conditions and time bounds, oldest or newest
first, capped), `query` (a full query including the query language), `instances` and
`time_range`. Annotators run after every import and after live-tail appends (`since` is the first
new record); each run returns the annotator's complete set, which replaces the previous one. Runs
are guarded with the `annotate` budget and capped at 10,000 annotations per annotator. The UI
reads them with `LogStore::annotations({ from, to, instance })`.

### Time budgets, the watchdog and isolation tiers

Every guarded call has a time budget per kind (`guard::Budgets`, set with
`PluginRegistry::set_budgets`): sniff 5 s, import 120 s, live-tail parse 10 s, transform 250 ms per
record, hook 5 s, service call 10 s, event delivery 5 s, annotator run 10 s, activation 10 s;
exports and collectors are unlimited (they stream or run until stopped).

In process, a running call cannot be pre-empted. A watchdog thread notices a call that exceeds its
budget, marks the plugin **hung** (a failure towards quarantine; `hung` in the Plugins panel), and
further calls of that plugin fail fast until the stuck call returns. Where the caller can afford
it the call runs on a helper thread and is **abandoned** when it overruns: plugin imports work on
owned copies of their input, so an import that hangs fails that file after 120 s (the thread is
left to finish; its result is discarded), and `Service::call_detached` does the same for services.

What this cannot do: stop a thread that never returns (it keeps its CPU and memory), or survive a
stack overflow, an abort or running out of memory, which end the process. That needs process
isolation (ADR 0002):

| Tier | For | Crash | Hang |
|---|---|---|---|
| In-process, guarded (today) | first-party safe Rust plugins | panic → error, quarantine | watchdog: hung, fail fast, abandon where possible |
| Plugin host process (next) | any Rust plugin, opt-in or after a quarantine | process dies, restarted with backoff | call cancelled by deadline, host killed |
| Worker process (`imt-ai-worker`) | native code (llama.cpp, PDH, profilers) | as above | as above |
| WebAssembly (planned) | third-party plugins | trapped | fuel/epoch interruption |

### Plugin rules

- Follow `AGENTS.md`: no panics on user data, typed errors at the edges, tests with high
  coverage, permissive licences only (`cargo deny check`).
- Keep ids stable: users reference transform and exporter ids from their profiles.
- Bump `version` for behaviour changes; `API_VERSION` changes only when the traits change.
