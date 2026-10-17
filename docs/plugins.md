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

All of them live in `imt_core` and are bundled by a `Plugin`:

```rust
pub trait Plugin: Send + Sync {
    fn id(&self) -> &'static str;          // reverse-DNS, unique: "com.acme.tools"
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn api_version(&self) -> u32 { imt_core::API_VERSION }
    fn register(&self, registry: &mut imt_core::PluginRegistry) -> anyhow::Result<()>;
}
```

`PluginRegistry::install` rejects plugins built for another `API_VERSION`, duplicate plugin ids
and duplicate contribution ids, and records what each plugin contributed (shown in the app's
Plugins panel).

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
   `plugins()` in `app/src-tauri/src/lib.rs`:

   ```rust
   registry.install(&imt_plugin_acme::AcmePlugin)?;
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

### Plugin rules

- Follow `AGENTS.md`: no panics on user data, typed errors at the edges, tests with high
  coverage, permissive licences only (`cargo deny check`).
- Keep ids stable: users reference transform and exporter ids from their profiles.
- Bump `version` for behaviour changes; `API_VERSION` changes only when the traits change.
