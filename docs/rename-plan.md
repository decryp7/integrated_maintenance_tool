# IMT to Debrief rename plan

The product is renamed from IMT (Integrated Maintenance Tool) to **Debrief** in one mechanical sweep, done by
`scripts/rename-to-debrief.mjs`. The user approved the full rename (file formats, plugin SDK, crates); nothing is
kept for backward compatibility (pre-v1). This file is the mapping, the deny list and the manual steps. It is excluded
from the rewrite (it names the old product on purpose), as is the script and `CHANGELOG.md`.

## Run it

```sh
git switch main && git pull && git switch -c chore/rename-to-debrief      # clean tree required
node scripts/rename-to-debrief.mjs --dry-run                                # per-file summary, changes nothing
node scripts/rename-to-debrief.mjs                                          # apply + post steps (rustfmt, Cargo.lock, samples)
```

Flags: `--no-post` skips rustfmt / `cargo update` / `cargo xtask gen-samples`; `--icons DIR` selects the icon set
(default `../suite-icons/debrief`); `--force-dirty` allows a dirty tree. The script never commits. It is idempotent: a
second run finds nothing to change. It ends by listing residual `imt` matches (expected: none).

## How the mapping works

The survey (`git grep -n -i 'imt'`, about 16,200 occurrences in 2,000 files, 37 crate directories) shows that every
occurrence is one of a small number of shapes, so the script maps shapes rather than carrying 2,800 distinct tokens.
Each rule applies where "imt" starts a word or a camelCase hump (a letter directly before a lowercase "imt" means it is
inside another word and is left alone and reported).

| Shape (before) | After | Where |
|---|---|---|
| `imt-foo` | `debrief-foo` | crate names, binaries (`imt-agent`, `imt-collect`, `imt-monitor`, `imt-store-worker`, `imt-plugins`, `imt-app`), CLI, artifacts (`imt-agent.toml`, `imt-diagnostics-*.zip`, `imt-updater.key`, installers), Angular selectors/CSS classes (`imt-root`, `imt-panel`, ...), `cargo imt-plugin` |
| `imt_foo` | `debrief_foo` | Rust paths/modules/features (`imt_core::`), `imt_app_lib`, test names, env-var-like lowercase |
| `imt:name` | `debrief:name` | icon namespace (`<mat-icon svgIcon="imt:timeline">`) |
| `imt.foo.vN` | `debrief.foo.vN` | storage keys (`imt.prefs.v1`, `imt.layout.v1`, `imt.store.v1`), plugin schema ids (`imt.plugin.v1`), CLEF source names (`imt.monitor`), IPC command ids |
| `imtFoo` | `debriefFoo` | TS identifiers (`imtGhost`, `imtClickOutside`, `imtVersion`) |
| `ImtFoo` | `DebriefFoo` | TS/Rust types (`ImtBackend`, `ImtCommand`, `ImtView`, `provideImtPlugin`, `StoppedByImt`) |
| `IMT` alone | `Debrief` | prose, window titles, menu labels ("About IMT"), `product`/`application` strings, app data folder names, ADR text |
| `IMT_FOO` / `IMTFOO` (ALL-CAPS run) | `DEBRIEF_FOO` | env vars (`IMT_E2E_PORT`, `IMT_DATA_DIR`, `IMT_REGENERATE_SAMPLES`, `IMT_BENCH_*`, `IMT_LLAMA_*`, `IMT_TEST_*`, ...), Angular tokens (`IMT_COMMANDS`, `IMT_VIEWS`, `IMT_ACTIVITY_ITEMS`) |
| `.imtproj` / `.imtz` / `.imtplugin` / `.imtprofile(.toml)` | `.debriefproj` / `.debriefz` / `.debriefplugin` / `.debriefprofile(.toml)` | file extensions (also `.IMTPROJ`... in tests) |
| `application/x-imt-project`, `application/x-imt-event` | `application/x-debrief-...` | MIME types |
| `net.decryptology.imt` | `net.decryptology.debrief` | Tauri `identifier`, bundle id, app data dir (`dev_overrides.rs`, `report/cli.rs`, e2e paths) |
| `IMT — Integrated Maintenance Tool`, `IMT (Integrated Maintenance Tool)` | `Debrief` | window title, `index.html`, About dialog, README heading, diagrams |
| `IMT — integrated maintenance tool for analysing ...` | `Debrief — an IDE for analysing logs and diagnostic files` | crate/package descriptions |
| `integrated_maintenance_tool` | `debrief` | repository URL (releases page, updater endpoint, docs), `.serena`/`.idea` names |
| "an IMT ..." | "a Debrief ..." | grammar fix after the rewrite |

Tauri: `productName` `IMT` becomes `Debrief`, `mainBinaryName` `imt-app` becomes `debrief-app`, window title becomes
`Debrief`, file association ext/mime follow the table. Plugin SDK: `imt-plugin-api`, `imt-plugin-test`,
`imt-plugin-package`, `cargo-imt-plugin` (binary `cargo debrief-plugin`), templates and the `.imtplugin` package format
(its manifest/format strings such as `imt.plugin.v1`) follow the table. Paths are renamed with the same rules (37
directories and 22 files, `git mv`, directories deepest first).

### IMT-specific extensions and magic strings found

Extensions: `.imtproj` (project), `.imtz` (share bundle), `.imtplugin` (plugin package), `.imtprofile.toml`
(profile). MIME: `application/x-imt-project`, `application/x-imt-event`. Versioned ids: `imt.plugin.v1`/`v2`,
`imt.layout.v1`, `imt.store.v1`, `imt.prefs.v1`, `imt.tour.done.v1` and the other `imt.*.vN` localStorage keys.
Product strings in data: CLEF `application: "IMT"`, `imt.monitor` source (samples). No binary magic bytes carry the
name (checked `b"IMT..."`); the format identifiers above are strings inside the formats.

### Icons

Copied from `suite-icons/debrief`: `tauri/*` into `app/src-tauri/icons/`; `tray/macos/debrief-tray-18ptTemplate@2x.png`
as `icons/tray-template.png`; `debrief-windows.svg`, `small/debrief-32.svg` and `tray/svg/debrief-tray-template.svg`
into `app/src-tauri/icons-src/` (replacing the renamed `imt-icon*.svg`, `imt-tray-template.svg`); `debrief-macos.svg`
and `debrief-macos-1024.png` as `assets/icon/debrief-icon.svg` / `debrief-icon-1024.png` (README logo).

## Deny list (never rewritten) and why

| What | Why |
|---|---|
| `scripts/rename-to-debrief.mjs`, `docs/rename-plan.md` | they name the old product on purpose |
| `CHANGELOG.md` | generated history (git-cliff); it records what the product was called then |
| Lines with `"integrity": "sha...` and `checksum = "` | hashes; a camelCase-looking `Imt` in base64 is not a name |
| `IMTReportSans`, `IMTReportMono`, `IMT_Report_Sans`, `IMT_Report_Mono` (strings and file names in `crates/debrief-pdf`) | the PDF report fonts carry this family name inside the TTF `name` table and `tests/pdf.rs` asserts the embedded name; renaming the strings alone breaks them. Rebuild the fonts with `fonts/build-fonts.sh` under a new family name in a separate change |
| `net.decryptology.web`, `.windows`, `.perfmon`, `.collector`, ... | plugin ids are publisher-scoped (the author's domain), not product names |
| Binary files' contents (`docs/screenshots/*.png`, fuzz corpus binaries, zips) | text rules do not apply; zips and bundles in `samples/` are regenerated by `cargo xtask gen-samples` |
| Third-party names | none contain "imt" as a word start; any match inside another word is left alone and printed as "unmapped" (the trial had none) |

## Length-sensitive fixups

The script carries an explicit `FIXUPS` list for tests that pin a length that the longer name changes:
`container_name` in `debrief-plugin-api` (`imt.plugin.` + 16 hex = 27 becomes `debrief.plugin.` + 16 = 31). rustfmt is run
on the touched `.rs` files because longer names reflow lines.

## Manual steps after the script

1. `~/.zshenv` (and any runner/CI host profile): rename `IMT_E2E_PORT` to `DEBRIEF_E2E_PORT`, and any other
   `IMT_*` variable set outside the repo (`IMT_LLAMA_DIR`, `IMT_TEST_*`, `IMT_DATA_DIR`, ...). The script does not edit them.
2. Re-capture `docs/screenshots/*` and `docs/ux-audit/screenshots/*` (images still show the old name); the
   `app/scripts/screenshots` flow does this.
3. `app/scripts/app-icon.mjs` regenerates icons from `icons-src` masters and would overwrite the installed suite set
   with a re-render; run it only if the masters are meant to be the source of truth, otherwise retire it.
4. Rebuild the PDF report fonts under a new family name (see deny list) when wanted.
5. CI hosts: caches named `imt-libduckdb`, `imt-bench-corpus` become `debrief-*` (one fresh download); the Mac and
   Windows runners keep no repo-named state otherwise. Secret names (`TAURI_*`, `RELEASE_TOKEN`) do not change.
6. Updater: the endpoint URL now uses the `debrief` repo path; the updater key file is named `debrief-updater.key`
   (rename the file where it is stored; the public key in `tauri.conf.json` is unchanged).
7. Existing local installs keep their data under the old identifier; Debrief starts fresh (no migration by decision).
8. Move the Claude memory/project folder key if the checkout folder is renamed (noted in memory).
9. `cargo xtask gen-samples` stops at its `debrief-plugin-windows` step (`no test target named samples`): this is a
   stale entry on main, not caused by the rename. The earlier steps ran and regenerated the samples.

## Trial results (from a fresh `origin/main` at eba53424)

- `cargo check --workspace --all-targets`: clean.
- `cargo test --workspace --lib` (excluding `debrief-store-duck`, `debrief-store-worker`, `debrief-app`, which need the
  DuckDB link environment and failed to link here): all pass, `debrief-core` 640 lib tests plus its integration tests
  except `every_sample_matches_its_importer_snapshot` (the `video-frames` importer has no sample; unrelated to the rename).
- `cd app && npx ng build`: clean. `npx ng test`: 4459 passed; 4 uncaught drag-and-drop errors in `notes.spec.ts`
  and 24 lint / 6 style-token findings, all identical on unrenamed main.
