# Releasing Debrief (and signed updates)

A `v*` tag on `main` runs `.gitea/workflows/build.yml`: the `bundle` jobs build the installers on Linux,
Windows and macOS, and the `release` job publishes a Gitea release with the installers, release notes
(`RELEASE_NOTES.md`, from git-cliff), SBOMs and `latest.json`, the release information Debrief's updater reads.
Installers are built only for tags, the nightly run of `main` and manual runs (*Run workflow* on `build`,
`workflow_dispatch`); pushes and pull requests check the code without them. To try installers before tagging,
start the workflow by hand or take them from the latest nightly run's artifacts.

```sh
git tag v1.2.0 && git push origin v1.2.0
```

The version in the tag must match `app/src-tauri/tauri.conf.json` / `Cargo.toml` (it is what Debrief compares
against). Pre-releases are not offered to users: updaters read `latest.json` of the *latest* release.

## What an installer contains

Besides the app, every installer ships the helper processes Debrief starts (built by `node app/scripts/build-sidecars.mjs`,
which `tauri build` runs): the store worker, the plugins binary, the AI worker, and the official prebuilt
**libduckdb** next to the store worker (a `lib/` resource folder). The release jobs fetch libduckdb from the pinned,
SHA-256-checked download in `crates/debrief-store-duck/scripts/fetch-libduckdb.sh`; the sidecar build does the same
through `app/scripts/libduckdb.mjs` when `DUCKDB_LIB_DIR` is not set. If the library is missing at run time, Debrief says
so in a clear message instead of failing to start the store. Third-party plugin packages are signed with their
publishers' own keys, not with the updater key (see [plugin-sdk.md](plugin-sdk.md)).

## How updates work

- **Opt-in.** Debrief never checks on its own unless the user turns on Settings ▸ Updates ▸ "Check for updates
  automatically" (off by default). Help ▸ Check for updates… always works.
- **One endpoint.** A check is one HTTPS GET of
  `https://dev.decryptology.net/decryp7/debrief/releases/download/latest/latest.json`
  (`plugins.updater.endpoints` in `tauri.conf.json`; Gitea ≥ 1.21 resolves `latest`). It carries no
  identifier or usage data; the server sees the IP address and the user agent `tauri-plugin-updater/<version>`.
- **Signatures are mandatory.** Every update is verified with the minisign public key built into the app
  before it is installed, and `requireSignedVersion` binds each signature to the version it was made for
  (GHSA-j38x-g3m3-95fr). A build made without the public key shows "Updates are not available in this build"
  and never checks; nothing unsigned is ever installed.
- **Flow.** Check → version, notes and size → download (progress, cancel) → verified → "Restart to update"
  or "Install when Debrief quits". The install runs after Debrief has closed its windows and plugins.

`latest.json` (written by `scripts/release/latest-json.mjs`):

```json
{
  "version": "1.2.0",
  "notes": "…RELEASE_NOTES.md…",
  "pub_date": "2026-10-17T12:00:00Z",
  "platforms": {
    "windows-x86_64-nsis": { "url": "https://…/releases/download/v1.2.0/DEBRIEF_1.2.0_x64-setup.exe", "signature": "…", "size": 41234567 },
    "windows-x86_64":      { "url": "…/DEBRIEF_1.2.0_x64-setup.exe", "signature": "…", "size": 41234567 },
    "darwin-aarch64-app":  { "url": "…/Debrief.app.tar.gz", "signature": "…", "size": 39000000 },
    "darwin-aarch64":      { "url": "…/Debrief.app.tar.gz", "signature": "…", "size": 39000000 },
    "linux-x86_64-deb":    { "url": "…/DEBRIEF_1.2.0_amd64.deb", "signature": "…", "size": 38000000 },
    "linux-x86_64-rpm":    { "url": "…/Debrief-1.2.0-1.x86_64.rpm", "signature": "…", "size": 38000000 }
  }
}
```

These are Tauri v2 updater artifacts (`bundle.createUpdaterArtifacts: true`): the installers themselves plus
a `.sig` each, and `Debrief.app.tar.gz` on macOS. Debrief does not use v1-compatible zip bundles (the updater is built
without its `zip` feature). `size` is Debrief's addition, shown before downloading. The app re-validates the file
(semantic version, https URLs, minisign signatures, bounded notes) and refuses it otherwise.

## One-time set-up: the signing key

1. Generate a key pair (on a trusted machine, with a strong password):

   ```sh
   cd app
   npx tauri signer generate -w ~/.tauri/debrief-updater.key
   ```

   This writes the private key `debrief-updater.key` (encrypted with the password) and the public key
   `debrief-updater.key.pub`.

2. Set three **repository secrets** in Gitea (Settings ▸ Actions ▸ Secrets):

   | Secret | Value |
   | --- | --- |
   | `TAURI_SIGNING_PRIVATE_KEY` | the contents of `debrief-updater.key` |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | its password |
   | `TAURI_UPDATER_PUBKEY` | the contents of `debrief-updater.key.pub` |

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
- `node --test scripts/release` tests the manifest generator; `cargo test -p debrief-app --lib updater` tests the
  client side (parsing, typed errors, the state machine, the "not available" build).

## macOS bundles must be ad-hoc sealed

Every Tauri app in the suite needs `bundle.macOS.signingIdentity: "-"` (Debrief: `app/src-tauri/tauri.macos.conf.json`)
and a CI step that verifies the result. Without it the bundle is only linker-signed (`flags=0x20002(adhoc,linker-signed)`,
`Sealed Resources=none`), and Gatekeeper reports a quarantined download as "Debrief.app is damaged and can't be opened"
(v1.0.0 shipped like this; fixed in 1.0.1). `scripts/ci/macos-verify-bundle.sh` runs after the macOS bundle step and
fails the job unless `codesign --verify --deep --strict` passes on the `.app` and on the `.app` inside the DMG, the
signature is not linker-signed only, resources are sealed and every nested Mach-O (sidecars, `libduckdb.dylib`) verifies.

### Hardened runtime is off for ad-hoc builds

`tauri.macos.conf.json` sets `bundle.macOS.hardenedRuntime: false`. Reproduced on 1.0.1: an ad-hoc binary signed with the
hardened runtime enforces library validation, and dyld then refuses the bundled `libduckdb.dylib` ("code signature ... not
valid for use in process: mapping process and mapped file (non-platform) have different Team IDs"; the store worker aborts
three times and is quarantined). Re-signing the library ad hoc (`app/scripts/build-sidecars.mjs`, `adhocSign`) removes the
Team ID mismatch but is not enough: library validation also rejects an ad-hoc library. Without the hardened runtime the
ad-hoc worker loads it. A hardened runtime brings nothing to a build that cannot be notarized.

When `APPLE_SIGNING_IDENTITY` names a Developer ID, the CI bundle step passes `{"bundle":{"macOS":{"hardenedRuntime":true}}}`
as a config override (required for notarization); then every bundled library must carry the same Team ID, which
`macos-verify-bundle.sh` checks. The script also runs `debrief-store-worker --self-test` and `debrief-plugins --manifests`
from the built `.app` and the DMG copy, so a load failure fails the job.

## First launch note

Apple notarization and Windows code signing are deferred until after the releases settle (decision 2026-10-23). Until then the release workflow appends a "First launch" section to every release's notes (and so to `latest.json`) explaining **Open Anyway** (macOS) and **More info > Run anyway** (Windows SmartScreen).
