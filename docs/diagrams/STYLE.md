# Diagram style

Debrief's diagrams follow the suite spec in the Paddock repo: [`diagrams/STYLE.md`](https://dev.decryptology.net/decryp7/paddock/src/branch/main/diagrams/STYLE.md)
(roles, type, grid, connectors, size limits, export). Start new diagrams from Paddock's templates and shape library
([`diagrams/README.md`](https://dev.decryptology.net/decryp7/paddock/src/branch/main/diagrams/README.md)); `scripts/export-diagrams.sh` is Paddock's,
copied unchanged.

## Debrief's colours

Applied with `node diagrams/tools/apply-accent.mjs debrief <template>` from a Paddock checkout; the generators in
[`src/`](src/README.md) use the same tokens (`app: 'debrief'`). Light / dark, stroke on fill.

| Role | Colours | Use here |
|---|---|---|
| Own container (accent) | #1D5FD1 on #E8EFFA / #5B9BFF on #202C3F | Debrief's executables and libraries with I/O: the shell, the store client and worker, the CLI tools, ports and adapters, states, use cases, classes |
| Pure core | #007A5E on #E6F2EF / #2EC4A0 on #193330 | `debrief-core` and its components, enumerations, pure functions |
| UI | #C24E00 on #F9EDE6 / #F07D3A on #382720 | the Angular UI and its folders |
| Plugin | #B0508A on #F7EEF3 / #D98BC0 on #342A35 | built-in and third-party plugins and their processes |
| Process / OS / data | #565E6B on #F0F2F5 / #A0A7B2 on #1B1E23 | helper and child processes, the data folder, notes, people |
| External | #6B7380 dashed on #F4F5F7 / #7F8794 on #17191E | the file system, hosts, Gitea, the OS webview, Chromium, web sites |

Rules that matter most here: processes are neutral (amber means warning); planned work is the role's colour, dashed, with a "planned"
tag (nothing is planned in the current diagrams); a diagram has at most 12 edges and fits 1600 × 1200 (sequences 1800 tall), so the
large ones are split (containers in three, engine components in four, UI components in three, domain model in four, plugin API in
three, ingest in two, use cases in four, deployment in three, query sequences in three, plugin sequences in two, live tail in two).

The footer reads "Debrief · updated YYYY-MM-DD". Update the date in `src/common.mjs` when a diagram changes. The 2x PNGs under
`docs/diagrams/png/` are for local review and are not committed.

## Overview

`debrief-overview.svg` is hand-drawn by `src/overview.mjs` (a copy of Paddock's `tools/overview.mjs` with Debrief's content). One file
serves light and dark through a `prefers-color-scheme` style block, so docs embed it with a plain `<img>` or Markdown image.

## Changing a diagram

Edit the generator in `src/`, run `src/generate.sh`, run `scripts/export-diagrams.sh`, then look at every changed PNG (and a dark
render) at README width. The `.drawio` files stay editable in draw.io, but re-running a generator overwrites its diagrams.
