# The IMT plugin SDK

Write a plugin for IMT in Rust, test it in the sandbox it will run in, and ship it as one signed `.imtplugin` file
that users install from the Plugins view. Design and rationale: [ADR 0027](adr/0027-plugin-sdk.md); the extension
points in depth: [plugins.md](plugins.md).

| | |
|---|---|
| SDK crate | `imt-plugin-api` (`crates/imt-plugin-api`; API reference: `cargo doc -p imt-plugin-api --open`) |
| Tool | `cargo imt-plugin` (`crates/cargo-imt-plugin`: `cargo install --path crates/cargo-imt-plugin`) |
| Test harness | `imt-plugin-test` (a dev-dependency) |
| Examples | `examples/plugins/{importer,transform,tool,collector,annotator}-example` |

## Write your first plugin

```sh
cargo install --path crates/cargo-imt-plugin          # once, from an IMT checkout
cargo imt-plugin new acme-lines --kind importer --id com.acme.lines
cd acme-lines
cargo imt-plugin test
```

`new` creates a project from IMT's example of that kind — code that IMT's CI builds and tests — with your names
in it. By default the SDK comes from IMT's repository; pin it with `--sdk-tag v…` or `--sdk-rev …`, or use a local
checkout with `--sdk-path <IMT folder>`.

```text
acme-lines/
  Cargo.toml              [package.metadata.imt-plugin] id and publisher
  src/lib.rs              the plugin: a Plugin and its contributions
  src/main.rs             fn main() { imt_plugin_api::serve(Arc::new(MyPlugin)) }
  tests/plugin.rs         conformance and golden tests with imt-plugin-test
  tests/samples/app.log   (importers) a file the importer reads
  tests/golden/app.json   (importers) what it should produce
```

The plugin is a `Plugin` that registers its contributions:

```rust
use imt_plugin_api::prelude::*;

pub struct AcmePlugin;

impl Plugin for AcmePlugin {
    fn id(&self) -> &str { "com.acme.lines" }                 // the same id as Cargo.toml's metadata
    fn name(&self) -> &str { "ACME lines" }
    fn version(&self) -> &str { env!("CARGO_PKG_VERSION") }   // the package version must match
    fn register(&self, c: &mut Contributions) -> anyhow::Result<()> {
        c.add_importer(AcmeLines);
        Ok(())
    }
}
```

What each kind implements (see the example of each):

| Kind | Implement | Register with | IMT calls it when |
|---|---|---|---|
| importer | `TextImporter` (`sniff`, `import`) or `BinaryImporter` | `add_importer` / `add_binary_importer` | a file is opened and your `sniff` scores highest |
| transform | `Transform` (`apply` → `Keep`/`Drop`) | `add_transform` | a profile has `{ step = "plugin", id = "..." }` |
| tool | `Tool` (`describe`, `invoke`) | `add_tool` | the UI or the assistant calls an operation |
| collector | a `Tool` whose operation starts a background task (`task_started`/`task_progress`/`task_finished`, `stop`) | `add_tool` | the user starts it; the Jobs pane lists it with Stop |
| annotator | `Annotator` (`annotate(view)`) | `add_annotator` (+ `add_marker_kinds`) | the timeline needs annotations |

Rules that keep plugins fast and safe:

- **Sniff cheaply and honestly**: 0–100 from the first 64 KiB; IMT's generic text importer reads any timestamped
  line, so score your own format high (the example returns 95 when most lines match).
- **Look for a cancel** in long imports: `if i % 4096 == 0 && ctx.cancelled() { bail!("cancelled") }`. IMT counts a
  plugin that runs more than 2 s past a cancel as hung.
- **Never panic on input**; return errors. A panic is contained (a typed `PANIC` error) but counts towards
  quarantine (3 failures in 10 minutes).
- **No paths, no global state across calls** beyond what your tool owns; files come from IMT.

## API reference and versions

`cargo doc -p imt-plugin-api --open` documents every public item. The crate follows semver. What IMT checks is
the **plugin protocol** your SDK speaks, `imt_plugin_api::PROTOCOL`:

| Protocol | Changes | First IMT |
|---|---|---|
| 1.0 | first public SDK (handshake frame version 2) | the release after 2026-10-21 |

IMT runs a plugin built for the same major version and a minor version it knows. Otherwise it refuses the plugin,
at install and at start, with a message saying what to update ("is too old … needs an update built for protocol
2.x" or "needs a newer IMT … update IMT to a version that speaks protocol 1.3 or later").

## Permissions and the sandbox

A third-party plugin runs **contained from its first instruction**, in the strictest sandbox, whatever its code
does:

| | Windows | Linux | macOS |
|---|---|---|---|
| Files | AppContainer: only its program folder (read) and data folder (read/write) | landlock: same, plus system libraries (read) | sandbox profile: same |
| Network | only with `network` (AppContainer capability) | only with `network` (seccomp) | only with `network` |
| Other programs | only with `runPrograms` (job object, same container) | only with `runPrograms` | only with `runPrograms` |
| Memory | resident memory watched by IMT; the process is stopped over the cap (1 GiB unless the plugin asks for another, at most 4 GiB). No rlimits: they count the address space that allocators reserve up front | same | same |
| CPU | job object limits | `RLIMIT_CPU` | `RLIMIT_CPU` |

- Files the user opens reach your importer through IMT (read in chunks over the connection); you never see a path.
- Your tools' `ToolCtx::data_dir()` is your private data folder, kept across updates and removed with the plugin.
- Declare what you need in `Plugin::permissions` with a reason the user can read. Packages may ask only for
  `network` and `runPrograms`; `openedFilePaths` and `writeFolders` are for IMT's own plugins. The user approves each
  permission at install and can revoke it later in the Plugins view (your process restarts with fewer rights).
- The program is verified (SHA-256) before every start: a changed program never runs.

## Testing

`cargo imt-plugin test` builds your program, runs the conformance checks against it in IMT's sandbox (with your
`tests/samples/*`), then `cargo test`. In your own tests:

```rust
use imt_plugin_test::{Golden, PluginUnderTest};

#[test]
fn conforms() {
    let p = PluginUnderTest::launch(env!("CARGO_BIN_EXE_acme-lines")).unwrap();
    p.conformance_with(&["tests/samples/app.log".into()]).assert_passed();
}

#[test]
fn imports_the_sample() {
    let p = PluginUnderTest::launch(env!("CARGO_BIN_EXE_acme-lines")).unwrap();
    let out = p.import_file("tests/samples/app.log").unwrap();
    Golden::new("tests/golden").check("app", &out.records);   // IMT_BLESS=1 cargo test rewrites it
}
```

`PluginUnderTest` also calls transforms (`transform`), annotators over a view of records (`annotate`), tools
(`invoke`, `stop_task`, `task_reports`) and settings (`apply_settings`). The conformance checks: a valid manifest
with reasons for every permission; everything runs with exactly the declared grants; unknown contributions and
malformed requests get typed errors; items of at most 8,192 events; a cancelled import ends within the grace period;
answers larger than the connection's frame limit become typed errors.

## Packaging

```sh
cargo imt-plugin package                               # unsigned: installs only in developer mode
cargo imt-plugin package --sign me.secret-key --target x86_64-unknown-linux-gnu
cargo imt-plugin inspect acme-lines-0.1.0.imtplugin
```

`package` builds a release program for this computer (and each `--target`), starts it once to read its manifest
(id and version must match `Cargo.toml`; permissions and protocol come from the program), and writes
`<name>-<version>.imtplugin`:

```text
manifest.toml                        id, name, version, publisher, description, protocol, binary, targets, key, [[permissions]]
bin/<target-triple>/<name>[.exe]     one program per target
signature                            key id, public key, Ed25519 signature of the package digest
```

The digest is the SHA-256 of every file's path, size and SHA-256; changing any byte breaks the signature, and IMT
refuses such a package ("changed after it was signed").

## Signing and trust

```sh
cargo imt-plugin keygen --out me.secret-key     # keep the file private; never commit it
```

`keygen` prints your public key (`imt-plugin-public-key ed25519 …`) and its id. Publish the public key where your
users can check it. In IMT:

- **Settings → Plugin trust → Trusted publisher keys**: one line per publisher, `ACME Ltd = imt-plugin-public-key
  ed25519 …`. Packages signed with a trusted key (or IMT's own first-party key) install normally.
- **Developer mode** (same section): unsigned packages and unknown keys install after a warning. Use it while you
  develop.

## Installing (users)

Plugins view → **Install from file** (or drop a `.imtplugin` on it): IMT shows the publisher, version, signature
status, description and the permissions to approve (for an update, what changed), then installs into its data
folder. Installed plugins can be turned off, reviewed (revoke a permission), removed or reset after a quarantine;
installing, updating, enabling and removing take effect after a restart.
