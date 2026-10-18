# IMT design

All of IMT's design diagrams on one page, from the big picture down to individual flows. Each image
is exported from its editable draw.io source (linked under the image) with
[`scripts/export-diagrams.sh`](../scripts/export-diagrams.sh); edit the `.drawio`, then re-run the
script. The style rules are in [diagrams/STYLE.md](diagrams/STYLE.md), the reasoning behind the
design in the [architecture decision records](adr/), and the prose overview in
[architecture.md](architecture.md).

Contents: [Overview](#overview) · [C4 model](#c4-model) · [Deployment](#deployment) ·
[Use cases](#use-cases) · [Domain and plugin API](#domain-and-plugin-api) · [Flows](#flows) ·
[States](#states)

## Overview

Many sources and collectors, one Rust engine, plugins in any language, and a workbench that shows
and shares what the engine computes.

![IMT at a glance](diagrams/imt-overview.svg)

## C4 model

### Level 1 — system context

Who uses IMT and which outside systems it reads from or talks to. IMT runs offline; the network is
used only for downloads the user starts (models, AI runtimes, updates).

![C4 system context](diagrams/svg/c4-1-context.svg)

Source: [c4-1-context.drawio](diagrams/c4-1-context.drawio)

### Level 2 — containers

The runnable units and the processes they run in: the Tauri app with its linked Rust libraries,
the Angular UI in the OS webview, and the supervised child processes (out-of-process plugins, the
AI worker driving `llama-server`, the planned DuckDB store worker), plus the headless tools.
See ADR [0002](adr/0002-microkernel-and-process-isolation.md),
[0009](adr/0009-prebuilt-llama-server-runtimes.md) and [0010](adr/0010-persistent-store.md).

![C4 containers](diagrams/svg/c4-2-containers.svg)

Source: [c4-2-containers.drawio](diagrams/c4-2-containers.drawio)

### Level 3 — engine components

Inside `imt-core`: the `LogStore` port and the in-memory workspace, ingest (importers, archives,
profiles, the pipeline with calculated fields and Rhai scripts), query and aggregation, analysis,
sharing and reports, projects and configuration, and the plugin runtime with its guards.

![C4 engine components](diagrams/svg/c4-3-components-engine.svg)

Source: [c4-3-components-engine.drawio](diagrams/c4-3-components-engine.drawio)

### Level 3 — UI components

The Angular workbench: the app frame (shell, docking, pop-outs, windows, palette), the feature
areas, and the core services. Components reach the engine only through the backend service (the
Tauri backend, or the mock for browser development and tests).

![C4 UI components](diagrams/svg/c4-3-components-ui.svg)

Source: [c4-3-components-ui.drawio](diagrams/c4-3-components-ui.drawio)

## Deployment

Where each part runs: the desktop app and its child processes on a workstation, the `imt-agent`
service on monitored servers, the CI runners and release assets on Gitea, and the hosts IMT
downloads from.

![Deployment](diagrams/svg/uml-deployment.svg)

Source: [uml-deployment.drawio](diagrams/uml-deployment.drawio)

## Use cases

What support engineers, developers, administrators and plugin authors do with IMT.

![Use cases](diagrams/svg/uml-use-cases.svg)

Source: [uml-use-cases.drawio](diagrams/uml-use-cases.drawio)

## Domain and plugin API

### Domain model

Records, sources, the workspace summary and state, the profile and its sections, queries, the
pipeline and the `LogStore` port.

![Domain classes](diagrams/svg/uml-class-domain.svg)

Source: [uml-class-domain.drawio](diagrams/uml-class-domain.drawio)

### Plugin API

The plugin contract, the registry with its guarded adapters and quarantine, the extension traits
(importers, transforms, markers, annotators, tools, config parsers), and out-of-process plugins over
gRPC `imt.plugin.v1`. See ADR [0003](adr/0003-plugin-dependencies-services-events.md) and
[0004](adr/0004-stable-plugin-abi.md).

![Plugin API classes](diagrams/svg/uml-class-plugin-api.svg)

Source: [uml-class-plugin-api.drawio](diagrams/uml-class-plugin-api.drawio)

## Flows

### Ingesting a file

From a path to records: locked-file retries, archives and their limits, sniffing and decoding,
importer selection under panic containment, the pipeline steps, markers and clock skew.

![Ingest activity](diagrams/svg/uml-activity-ingest.svg)

Source: [uml-activity-ingest.drawio](diagrams/uml-activity-ingest.drawio)

### Opening a folder

`open_paths` to `LogStore::open`: parallel per-file import with progress and cancel, merging,
rebuilding, annotators and load hooks.

![Open folder sequence](diagrams/svg/uml-seq-open-folder.svg)

Source: [uml-seq-open-folder.drawio](diagrams/uml-seq-open-folder.drawio)

### Filtering, the timeline and queries

A filter change fanning out to the timeline, grid and charts (stale replies dropped), a marker click,
the inspector's source context, and a cancellable query with streamed progress.

![Query and timeline sequence](diagrams/svg/uml-seq-query-timeline.svg)

Source: [uml-seq-query-timeline.drawio](diagrams/uml-seq-query-timeline.drawio)

### The Moment view

What happened around a point in time: instances, patterns, metric windows and cached anomalies.

![Moment sequence](diagrams/svg/uml-seq-moment.svg)

Source: [uml-seq-moment.drawio](diagrams/uml-seq-moment.drawio)

### Plugin registration at start-up

Built-in and out-of-process plugins installed under guards, resolved, watched for quarantine, and
compiled into the profile; safe mode.

![Plugin registration sequence](diagrams/svg/uml-seq-plugin-registration.svg)

Source: [uml-seq-plugin-registration.drawio](diagrams/uml-seq-plugin-registration.drawio)

## States

### Live tail and alerts

Following growing files (rotation, truncation, new files), coalesced refreshes, alert rules with
cool-down and mutes, and the UI's follow mode.

![Live tail states](diagrams/svg/uml-state-live-tail.svg)

Source: [uml-state-live-tail.drawio](diagrams/uml-state-live-tail.drawio)

### Projects and sessions

Untitled and saved projects with their dirty state, autosave, and crash recovery with safe mode.

![Project states](diagrams/svg/uml-state-project.svg)

Source: [uml-state-project.drawio](diagrams/uml-state-project.drawio)

### Performance Monitor collector sets

Installing, starting and stopping Windows data collector sets from IMT, and importing their output.

![Performance Monitor states](diagrams/svg/uml-state-perfmon.svg)

Source: [uml-state-perfmon.drawio](diagrams/uml-state-perfmon.drawio)
