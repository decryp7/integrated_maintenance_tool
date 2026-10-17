# IMT architecture

This page ties together the design diagrams in [`docs/diagrams/`](diagrams/). The decisions behind
them are in [ADR 0001](adr/0001-architecture.md); extension points are in [plugins.md](plugins.md);
planned work is in [roadmap.md](roadmap.md). In short: a Rust engine (`crates/imt-core`) behind the
`LogStore` and `Importer` ports, a thin Tauri 2 shell (`app/src-tauri`), and an Angular UI
(`app/src/app`) that only talks to the engine through the `ImtBackend` service.

Parts that are not implemented yet (Moment, query bar, insights panels, project and live-tail UI,
Tauri commands for ql/anomalies/moment, Performance Monitor controller UI, Seq/Elastic/Loki and
Claude integrations) are drawn with a dashed border or labelled "in progress" / "planned".

## C4 model

| Diagram | What it shows |
|---|---|
| [c4-1-context](diagrams/c4-1-context.drawio) | IMT, its users (support engineer, developer, administrator, plugin author) and the external systems it reads from or talks to: log-producing applications, Windows Performance Monitor, Windows event log / msinfo32 / registry, Gitea, and the planned Seq/Elastic/Loki and opt-in Claude API. |
| [c4-2-containers](diagrams/c4-2-containers.drawio) | The runnable units: Angular UI in the webview, the Tauri shell, `imt-core`, the plugin crates, the `imt-monitor` and `imt-collect` CLIs, and the files they exchange (logs, `.imtproj`, `.imtz`, CLEF output, the app log). |
| [c4-3-components-engine](diagrams/c4-3-components-engine.drawio) | Modules of `imt-core`: `workspace` (the `LogStore` implementation), `store`, `api`, `importers`, `archive`, `profile`, `pipeline`, `query` and `ql`, `analysis`, `patterns`, `exceptions`, `chart`, `export`, `project`, and the plugin registry. |
| [c4-3-components-ui](diagrams/c4-3-components-ui.drawio) | Folders of the Angular app: shell, `WorkspaceStore`, the `ImtBackend` token with `TauriBackend` / `MockBackend`, views, extension tokens, docking, logger; plus the in-progress panels. |

## UML

| Diagram | What it shows |
|---|---|
| [uml-use-cases](diagrams/uml-use-cases.drawio) | Actors and use cases (open, analyse, query, anomalies, compare, Moment, export, live tail, monitor, Performance Monitor, collect, plugins, CI) with include / extend. |
| [uml-class-domain](diagrams/uml-class-domain.drawio) | `Record`, `Level`, `Query`, `CompiledQuery`, `Profile` and its sections, `CompiledProfile`, `Pipeline`, `Condition`, `Workspace`, `ProjectFile`, `SourceInfo`, `WorkspaceSummary`. |
| [uml-class-plugin-api](diagrams/uml-class-plugin-api.drawio) | The traits plugins implement (`Plugin`, `Importer`, `BinaryImporter`, `RecordTransform`, `WorkspaceHook`, `Exporter`, `Collector`, `EventSink`, `LogStore`), `PluginRegistry`, and the real plugin crates with their importers and collectors. |
| [uml-seq-open-folder](diagrams/uml-seq-open-folder.drawio) | `open_paths` to `LogStore::open`, parallel per-file import (archive, importer selection, ingest), rebuild, hooks, UI refresh. |
| [uml-seq-query-timeline](diagrams/uml-seq-query-timeline.drawio) | A filter change fanning out to `query_page` / `query_timeline` / `query_chart`, the matching cache and `CompiledQuery`, then a marker click with an anchored page and `source_context`. |
| [uml-seq-moment](diagrams/uml-seq-moment.drawio) | `Workspace::moment`: matched records, per-instance patterns, current vs previous window metrics, activity, cached anomalies (engine done, Tauri/UI in progress). |
| [uml-seq-plugin-registration](diagrams/uml-seq-plugin-registration.drawio) | `plugins()` installing each plugin (API version, duplicate id, guarded `register`), then `Workspace::with_registry` compiling the profile against plugin transforms. |
| [uml-activity-ingest](diagrams/uml-activity-ingest.drawio) | Ingest of one file with its decisions (archive, binary importer, text decoding, importer found, empty result) and the pipeline step loop (drop, keep, dedupe). |
| [uml-state-perfmon](diagrams/uml-state-perfmon.drawio) | Life cycle of a Performance Monitor collector set driven through `logman` by `PerfmonController`, with `AccessDenied` and `Unsupported`. |
| [uml-state-live-tail](diagrams/uml-state-live-tail.drawio) | Off, On and Refreshing for `set_live` and the `imt://workspace-changed` event. |
| [uml-state-project](diagrams/uml-state-project.drawio) | Unsaved, Saved, Modified, Opening, Open with missing paths reported, Rejected. |
| [uml-deployment](diagrams/uml-deployment.drawio) | Analyst machine, monitored server, and the Gitea server with `act_runner`s producing the installers. |

## Opening and editing the diagrams

The `.drawio` files are plain, uncompressed mxGraph XML, so they diff well in git.

- **diagrams.net**: open <https://app.diagrams.net>, choose *Open existing diagram* and pick the file
  (nothing is uploaded when you choose "Device").
- **draw.io desktop**: install it from the diagrams.net releases and open the file; File > Export
  produces PNG / SVG / PDF, or use the CLI: `draw.io -x -f png -o out.png file.drawio`.
- **VS Code**: install the "Draw.io Integration" extension (`hediet.vscode-drawio`); `.drawio` files
  then open in the editor and save back in place.

When saving from draw.io, keep *File > Properties > Compressed* turned off so the XML stays readable.

## Keeping diagrams in sync with the code

- Names in the diagrams are the real type, module, command and event names. When you rename or add one
  of these, update the diagrams in the same change: a new `Importer` or plugin (class and plugin
  diagrams), a new Tauri command or event (containers, sequences), a new `LogStore` method or
  `api.rs` DTO (components, domain), a new profile section (domain), a new CI job or runner (deployment).
- Treat a diagram that disagrees with the code as a bug in the diagram; fix the diagram, not the code.
- Mark unfinished work with a dashed border and "(in progress)", and remove the marker when the code lands.
- Contracts remain defined in code: `app/src/app/core/api.types.ts` and `crates/imt-core/src/api.rs` for
  the wire format, `crates/imt-core/src/default_profile.toml` for the profile schema.
