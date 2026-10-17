# IMT plugin SDKs

IMT's stable plugin contracts are language-neutral (ADR 0004): **`imt.plugin.v1`**, gRPC + Apache Arrow
between IMT and a plugin process (`proto/imt/plugin/v1/plugin.proto`), and later the WebAssembly component
contract `imt:plugin@1`. Any language with gRPC can implement a plugin; these SDKs make it idiomatic.
SDKs are developer tooling: nothing here ships inside IMT.

| SDK | Where | Status | Contract |
|---|---|---|---|
| Rust | `crates/imt-plugin-sdk` (`serve(plugin)`, the conformance kit and `imt-plugin-check`) | official, reference | `imt.plugin.v1` |
| C# / .NET | [`sdk/dotnet`](dotnet/README.md) (`Imt.Plugin.Sdk`, `dotnet new imt-plugin`, examples) | official | `imt.plugin.v1` |

Every SDK is conformance-tested in CI: its examples are published and must pass `imt-plugin-check`, the same
contract tests IMT runs before installing a plugin, and an end-to-end import through IMT's plugin host.

## What every SDK must do

- **Launch:** read one JSON line `{"address", "token"}` from stdin; serve gRPC (`imt.plugin.v1.Plugin` and
  `imt.ipc.v1.Control`) over HTTP/2 on that address only: a Unix socket in IMT's private 0700 directory, or an
  owner-only Windows named pipe. Never TCP.
- **Auth:** refuse any call without the `x-imt-token` metadata (constant-time compare).
- **Errors:** typed errors as JSON `{code, message, data?}` in the binary trailer `x-imt-error-bin`, with the
  gRPC status of the code (see `crates/imt-plugin-sdk/src/lib.rs`, `code`).
- **Lifeline:** exit when stdin closes; stop on `Control.Shutdown`; never write to stdout.
- **Data:** records as Arrow IPC streams in the record schema of `crates/imt-plugin-sdk/src/arrow.rs` (columnar
  or JSON fields, LZ4-frame compression accepted).

## Plugin folder layout (proposal)

IMT's plugin host can already run any executable that speaks the contract (`imt_plugin_host::RemotePlugin`,
`PluginSpec { exe, args, grants }`), but IMT does not install external plugins yet. This is the layout the
SDKs produce, for the plugin manager to install as is:

```
<plugins dir>/<plugin id>/
  plugin.json
  bin/<rid>/<executable>[.exe]      one per OS/architecture: osx-arm64, osx-x64, linux-x64, linux-arm64, win-x64, win-arm64
```

`plugin.json`:

```json
{
  "format": 1,
  "contract": { "kind": "grpc", "package": "imt.plugin.v1", "major": 1, "minor": 0 },
  "executable": "order-audit",
  "manifest": { "id": "com.example.orderaudit", "name": "Order audit", "version": "1.0.0",
                "contractMajor": 1, "contractMinor": 0, "requires": [], "provides": [],
                "permissions": [], "capabilities": [ { "importer": { "id": "orderaudit.log", "tail": true, "multiline": true } } ] }
}
```

- `manifest` is the plugin's `Handshake` reply in protobuf's canonical JSON, so the manager can show the
  name, version, dependencies and requested permissions **before** running anything, and check
  `contract.major` against the IMT version.
- On install, the manager picks `bin/<rid of this machine>/<executable>` (with `.exe` on Windows); a plugin
  without a binary for the machine is refused with a clear message. It then starts it with
  `PluginSpec::new(exe, label)` plus the user's grants, exactly as `imt-plugin-check` does, and compares the
  handshake's manifest with `plugin.json` (a mismatch is refused).
- `<plugin id>` is the directory name; versions are replaced in place (the manager keeps the previous folder
  until the new one has passed its handshake).
- The C# SDK writes this layout on `dotnet publish` (`ImtPluginLayoutDir`); the Rust SDK can do the same with
  a small `cargo xtask` (not yet written).
