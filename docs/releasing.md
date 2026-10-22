# Releasing IMT (and signed updates)

A `v*` tag on `main` runs `.gitea/workflows/build.yml`: the `bundle` jobs build the installers on Linux,
Windows and macOS, and the `release` job publishes a Gitea release with the installers, release notes
(`RELEASE_NOTES.md`, from git-cliff), SBOMs and `latest.json`, the release information IMT's updater reads.
Installers are built only for tags, the nightly run of `main` and manual runs (*Run workflow* on `build`,
`workflow_dispatch`); pushes and pull requests check the code without them. To try installers before tagging,
start the workflow by hand or take them from the latest nightly run's artifacts.

```sh
git tag v1.2.0 && git push origin v1.2.0
```

The version in the tag must match `app/src-tauri/tauri.conf.json` / `Cargo.toml` (it is what IMT compares
against). Pre-releases are not offered to users: updaters read `latest.json` of the *latest* release.

## What an installer contains

Besides the app, every installer ships the helper processes IMT starts (built by `node app/scripts/build-sidecars.mjs`,
which `tauri build` runs): the store worker, the plugins binary, the AI worker, and the official prebuilt
**libduckdb** next to the store worker (a `lib/` resource folder). The release jobs fetch libduckdb from the pinned,
SHA-256-checked download in `crates/imt-store-duck/scripts/fetch-libduckdb.sh`; the sidecar build does the same
through `app/scripts/libduckdb.mjs` when `DUCKDB_LIB_DIR` is not set. If the library is missing at run time, IMT says
so in a clear message instead of failing to start the store. Third-party plugin packages are signed with their
publishers' own keys, not with the updater key (see [plugin-sdk.md](plugin-sdk.md)).

## How updates work

- **Opt-in.** IMT never checks on its own unless the user turns on Settings ▸ Updates ▸ "Check for updates
  automatically" (off by default). Help ▸ Check for updates… always works.
- **One endpoint.** A check is one HTTPS GET of
  `https://dev.decryptology.net/decryp7/integrated_maintenance_tool/releases/download/latest/latest.json`
  (`plugins.updater.endpoints` in `tauri.conf.json`; Gitea ≥ 1.21 resolves `latest`). It carries no
  identifier or usage data; the server sees the IP address and the user agent `tauri-plugin-updater/<version>`.
- **Signatures are mandatory.** Every update is verified with the minisign public key built into the app
  before it is installed, and `requireSignedVersion` binds each signature to the version it was made for
  (GHSA-j38x-g3m3-95fr). A build made without the public key shows "Updates are not available in this build"
  and never checks; nothing unsigned is ever installed.
- **Flow.** Check → version, notes and size → download (progress, cancel) → verified → "Restart to update"
  or "Install when IMT quits". The install runs after IMT has closed its windows and plugins.

`latest.json` (written by `scripts/release/latest-json.mjs`):

```json
{
  "version": "1.2.0",
  "notes": "…RELEASE_NOTES.md…",
  "pub_date": "2026-10-17T12:00:00Z",
  "platforms": {
    "windows-x86_64-nsis": { "url": "https://…/releases/download/v1.2.0/IMT_1.2.0_x64-setup.exe", "signature": "…", "size": 41234567 },
    "windows-x86_64":      { "url": "…/IMT_1.2.0_x64-setup.exe", "signature": "…", "size": 41234567 },
    "darwin-aarch64-app":  { "url": "…/IMT.app.tar.gz", "signature": "…", "size": 39000000 },
    "darwin-aarch64":      { "url": "…/IMT.app.tar.gz", "signature": "…", "size": 39000000 },
    "linux-x86_64-deb":    { "url": "…/IMT_1.2.0_amd64.deb", "signature": "…", "size": 38000000 },
    "linux-x86_64-rpm":    { "url": "…/IMT-1.2.0-1.x86_64.rpm", "signature": "…", "size": 38000000 }
  }
}
```

These are Tauri v2 updater artifacts (`bundle.createUpdaterArtifacts: true`): the installers themselves plus
a `.sig` each, and `IMT.app.tar.gz` on macOS. IMT does not use v1-compatible zip bundles (the updater is built
without its `zip` feature). `size` is IMT's addition, shown before downloading. The app re-validates the file
(semantic version, https URLs, minisign signatures, bounded notes) and refuses it otherwise.

## One-time set-up: the signing key

1. Generate a key pair (on a trusted machine, with a strong password):

   ```sh
   cd app
   npx tauri signer generate -w ~/.tauri/imt-updater.key
   ```

   This writes the private key `imt-updater.key` (encrypted with the password) and the public key
   `imt-updater.key.pub`.

2. Set three **repository secrets** in Gitea (Settings ▸ Actions ▸ Secrets):

   | Secret | Value |
   | --- | --- |
   | `TAURI_SIGNING_PRIVATE_KEY` | the contents of `imt-updater.key` |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | its password |
   | `TAURI_UPDATER_PUBKEY` | the contents of `imt-updater.key.pub` |

   The bundle jobs then build with `createUpdaterArtifacts` and the public key (passed as
   `tauri build --config`, and as `TAURI_UPDATER_PUBKEY` for `option_env!` in `updater.rs`). Without the
   secrets (forks, pull requests) the installers are built unsigned for updates and the app reports
   "Updates are not available in this build"; the release job then publishes no `latest.json`.

   The committed `tauri.conf.json` keeps `pubkey` empty and does not set `createUpdaterArtifacts`, so local
   `npx tauri build` works without the private key. To build an updatable app locally, export the three
   variables above and pass the same `--config` as CI.

3. **Back up the private key and its password** (e.g. a password manager plus an offline copy). Never commit
   it, never put it in a `VITE_`/`NG_` variable or anything bundled into the UI.

**If the private key is lost**, no further update can be signed for the key built into existing
installations: users must download and install the next version by hand (it carries the new public key).
**If it leaks**, generate a new pair, publish a release signed with the new key, and tell users to reinstall
from the releases page; revoke the old secrets.

## Checking a release

- `latest.json` is attached to the release and lists every platform; `cat` output is in the release job log.
- An installed older version: Help ▸ Check for updates… shows the new version, notes and size; download,
  then Restart to update.
- `node --test scripts/release` tests the manifest generator; `cargo test -p imt-app --lib updater` tests the
  client side (parsing, typed errors, the state machine, the "not available" build).
