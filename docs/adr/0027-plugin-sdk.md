# 27. A Rust-only plugin SDK; third-party plugins as signed, contained packages

- Status: accepted (2026-10-21)
- Date: 2026-10-21
- Builds on: [ADR 0017](0017-one-ipc-out-of-process-plugins.md) (one IPC, every plugin out of process, `SandboxSpec`,
  host files instead of paths), ADR 0016 (command table, path grants, `CmdError`)
- Related: [docs/plugin-sdk.md](../plugin-sdk.md) (the author's guide), [docs/plugins.md](../plugins.md)

## Context

After ADR 0017 every plugin is a Rust program speaking postcard frames to IMT, but only IMT's own plugins exist:
they are linked into the `imt-plugins` binary, trusted, and lock themselves down after connecting ("defends against
an exploited parser, not a malicious binary"). There was no way for anyone else to write, test, ship or install a
plugin:

- the contract crate (`imt-plugin-api`) had undocumented items and a bare `CONTRACT = 1` number in the manifest,
  checked only after the plugin had started; nothing in the handshake said which side was too old;
- there was no project template, no packaging, no signing, no test harness and no example per kind;
- the app could only run plugins it ships; a binary from elsewhere would run with the user's full rights for as
  long as it chose not to call `lockdown`.

The user decided (2026-10-21): a **Rust-only** plugin SDK for performance and simplicity, with a scaffold, examples,
a test harness, signed packages and an install flow with permission review; third-party plugins always in the
strictest sandbox.

## Decision

### 1. The SDK is `imt-plugin-api` (name kept)

- The crate keeps its name (every first-party plugin already imports it); it is the SDK. `#![deny(missing_docs)]`,
  a crate-level walkthrough, `prelude`, and one entry point `serve(Arc<dyn Plugin>)`.
- **Semver for the crate**, and a separate **plugin protocol version** `PROTOCOL: ProtocolVersion { major, minor }`
  (now `1.0`): minor = something added at the end of a request/reply/host-request enum or the manifest; major =
  anything else.
- The protocol version is in the **handshake**: frame version 2 adds `protocol` to `Hello` and `Welcome`
  (`imt_ipc::frame::ProtocolVersion`), placed right after the frame version so a host reads both from a child of any
  version (`frame::handshake_versions`). The host accepts the same major and a minor it knows
  (`ProtocolVersion::accepts`) and otherwise refuses at connect time with a message that names the side to update:
  *"plugin "x" is too old: it was built for protocol 0.9 and this IMT speaks 1.0; it needs an update built for
  protocol 1.x"* / *"… needs a newer IMT: … update IMT to a version that speaks protocol 1.3 or later"*. A frame
  version mismatch gets the same kind of message. Workers (AI, store) are `UNVERSIONED` on both sides. The manifest
  carries the protocol too (replacing `contract`), so install and `Manifest::check` give the same message.
- **Cooperative cancellation**: `ImportCtx::cancelled()` and cancel-aware `HostFile` reads; anything that fails after
  a cancel is answered as `CANCELLED`. The harness found that importers otherwise ran past the 2 s grace period on
  large files.
- The sandbox policy moved into the SDK (`imt_plugin_api::sandbox`): `sandbox_for` (first-party) and
  `sandbox_for_third_party`, so the harness launches plugins exactly as IMT does; `imt-plugin-host` re-exports it.

### 2. Tooling for authors

- **`cargo imt-plugin`** (`crates/cargo-imt-plugin`): `new <name> --kind importer|transform|tool|collector|annotator`
  renders the project from the example of that kind (`examples/plugins/<kind>-example`, embedded with
  `include_str!`), so a new project is code that CI builds and tests; `test` builds the program, runs the
  conformance checks in IMT's sandbox and then `cargo test`; `package [--sign key] [--target t]…` builds release
  programs and writes the package; `keygen`; `inspect`.
- **Examples**: one small documented plugin per kind, workspace members (so `cargo test --workspace` runs their
  harness tests), lib + one-line `main`.
- **Test harness** (`crates/imt-plugin-test`, a dev-dependency): `PluginUnderTest` copies the program into a private
  folder and starts it with IMT's launcher and the third-party sandbox, behind a fake host (host files, annotator
  views, task reports). Conformance checks: manifest (valid, permission reasons, only package permissions),
  permissions (everything runs with exactly the declared grants, so undeclared use fails), typed errors (unknown
  contribution, undecodable request), bounded output (items ≤ 8,192 events), cancellation within the grace period,
  answers larger than the frame limit are typed errors. Panics → typed `PANIC` is SDK behaviour, tested against the
  harness's misbehaving fixture. Golden files (`IMT_BLESS=1`).

### 3. The package: `.imtplugin`

A zip with exactly: `manifest.toml` (format 1: id, name, version (semver), publisher, description, protocol, binary,
targets, optional `key`; `[[permissions]]` with reasons), `bin/<target-triple>/<binary>[.exe]` per target, and an
optional `signature` file (key id, public key, Ed25519 signature). Rules (`imt-plugin-package`):

- **Bounded and strict**: package ≤ 256 MiB, ≤ 64 entries, manifest ≤ 64 KiB, program ≤ 128 MiB, total ≤ 512 MiB
  counted while reading (not from headers); only the listed names (no traversal, links, encrypted or duplicate
  entries, nothing unlisted). Proptest (arbitrary and damaged packages never panic) and `fuzz_plugin_package`.
- **Digest**: SHA-256 of a canonical text (`imt-plugin-package-v1`, then `path size sha256` per file, sorted). The
  signature covers it, so changing any byte of the manifest or of any program breaks it. A signature that does not
  verify, a manifest naming a key with no signature (or the reverse), or a key id mismatch is `Tampered`, refused
  always.
- Packages may ask only for `network` and `runPrograms`. `openedFilePaths` and `writeFolders` hand a plugin the user's
  paths; third-party plugins get files through the host (ADR 0017 §4) and write only their private data folder.
  Operations that need administrator rights are first-party only.
- **ed25519-compact** (MIT, no dependencies) for signatures; keys as one line of text
  (`imt-plugin-public-key ed25519 <hex>`), key id = 16 hex digits of SHA-256(key).

### 4. Trust

- Signed by a **trusted key** → installs normally. Trusted keys: IMT's built-in first-party key (not removable) and
  keys the user adds in **Settings → Plugin trust** (one line each, `Publisher = <public key>`; validated in Rust).
- **Unsigned** or **unknown key** → only with **developer mode** (same settings section), after a warning that is
  stored with the install record.
- Verification is in Rust (`TrustSettings::decide`), at inspect and again at install; the install also requires the
  digest the user reviewed (a package that changed in between is refused).

### 5. Installing and running third-party plugins

- `imt_plugin_host::installed::PluginsDir` under `<app data>/plugins/<id>/`: a version folder
  (`v-<version>-<digest8>/bin/<program>`, `manifest.toml`, `contract.json`), `install.json` (switched atomically),
  `data/` (kept across updates). Install = verify → extract this computer's program → **start it once, contained, and
  ask its manifest** (id, version and permissions must match the package; protocol checked) → switch the record.
  Updates show permission changes; install, update, enable and remove take effect at the next start (a running
  program is never replaced); `cleanup` at start removes what nothing refers to and deletes removed plugins'
  containers. Damaged records make a plugin unavailable with a reason.
- **Verified before every start**: the program's SHA-256 is checked in the supervisor's start function; a changed
  program never runs ("reinstall the plugin").
- **Permissions are live**: `PluginProcess::set_granted` stops the process; it starts again on next use in the
  sandbox of the new grants (revoking takes effect at once).
- **Launcher containment** (`imt_ipc::sandbox::Containment::Launcher`), applied before the program's first
  instruction, because a third-party binary need not call `lockdown`:
  - **Windows**: an **AppContainer** per plugin (`imt.plugin.<hash of id>`): no access to the user's files,
    registry or objects; its program folder (read/execute) and data folder (read/write) are granted to the
    container SID; the network only through the `internetClient`/`privateNetworkClientServer` capabilities when
    `network` is granted; the host's pipe DACL adds the container SID and a low-integrity label. The process is
    created **suspended**, put into its **job object** (memory cap, CPU, `ActiveProcessLimit = 1` unless
    `runPrograms`, kill on close) and only then resumed. Environment: the kept variables plus the profile folders
    Windows needs to create the container.
  - **Linux**: rlimits, `no_new_privs`, **landlock** (read/execute only the program folder and the system library
    folders, write only the data folder) and the **seccomp** network filter, applied between `fork` and `exec`.
  - **macOS**: started through `sandbox-exec` with the sandbox profile (file access limited to its folders, network
    and `process-exec` denied except its own program).
  - The child still locks itself down on top (`lockdown` skips what is already enforced).
- The shell: commands `plugin_packages_list`, `plugin_package_inspect`, `plugin_package_install`,
  `plugin_package_remove`, `plugin_package_set_enabled`, `plugin_package_set_permissions` (one line each in the
  command table; package paths only from grants — dialog or drop; ids, digests and permission lists checked),
  `CmdError` codes `untrusted` and `incompatible` added (the package reason is in `detail`), DTOs generated with
  ts-rs (`generated/plugins.ts`). The Plugins view gains "Install from file", the review dialog (publisher, version,
  signature, description, permissions to approve, update changes, warnings), enable/disable, remove, permission
  review and the state of each installed plugin; trust is in Settings.

## Consequences

- Anyone can write a plugin in Rust with one dependency, start from a template that is known to build, test it in
  the sandbox it will run in, and ship one signed file; IMT refuses tampered packages and runs third-party code
  confined from its first instruction.
- The handshake change (frame version 2) is incompatible with plugins built before it; pre-v1, no compatibility.
- An AppContainer profile per installed plugin is created in the user's registry and deleted when the plugin is
  removed. Containment adds a few milliseconds to a plugin's start (granting the folders' ACLs is repeated per
  start so a recreated folder still works).
- Third-party plugins cannot read the user's files by path or write outside their data folder; collectors deliver
  what they collect through IMT (see "Not done yet").

## Verified vs not (2026-10-21)

- Windows (this machine and the Windows CI runner): AppContainer containment is tested with a child that skips its
  own lockdown (user files unreadable and unwritable, network refused, no child processes, data folder writable,
  wrong token refused), and end to end (`imt-plugin-e2e`: package → sign → install → contained process → import
  through the engine; a changed program never starts).
- Linux and macOS launcher containment is implemented behind `cfg` and type-checked with clippy for
  `x86_64-unknown-linux-gnu` and `x86_64-apple-darwin`; the Linux test (`a_contained_child_is_confined…`) runs on
  the Linux CI runner; macOS has no runner.

## Not done yet

- A host service for collectors to hand files to IMT for writing into a user-chosen folder (third-party plugins
  cannot be granted `writeFolders`).
- Installing without a restart (adding a plugin to running window registries).
- A key revocation list and package expiry; publishing the first-party private key's custody process.
- LPAC (less-privileged AppContainer) on Windows, which would also hide the files readable by "ALL APPLICATION
  PACKAGES".
