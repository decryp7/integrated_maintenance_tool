# IMT plugin SDK for C# / .NET

`Imt.Plugin.Sdk` lets you write an IMT plugin in C#. The plugin is a separate process that speaks IMT's
stable plugin contract, `imt.plugin.v1` (gRPC + Apache Arrow, ADR 0002/0004). Use it when a plugin needs
.NET: TraceEvent, a vendor's .NET library, existing C# parsers. IMT itself never ships .NET: a plugin carries
its own self-contained runtime, and a crash or hang in it only costs that plugin (IMT restarts it with backoff
and quarantines it after repeated failures).

| Folder | What it is |
|---|---|
| `src/Imt.Plugin.Sdk` | The SDK library (net8.0 and net10.0) and NuGet package |
| `templates` | `dotnet new imt-plugin`: an importer plugin with tests |
| `examples/ToyFormatImporter` | The "hello world": an importer for `TIME\|LEVEL\|MESSAGE\|key=value…`, plus marker kinds and a setting |
| `samples/OrderAudit` | A complete plugin for a .NET order service's audit log: every capability ([walkthrough](samples/OrderAudit/README.md)) |
| `tests/Imt.Plugin.Sdk.Tests` | The SDK's tests; `tests/Imt.Plugin.Sdk.KitchenSink` is a test plugin using every capability |
| `tests/e2e-rs`, `tests/e2e/run.sh` | End to end: published plugins through `imt-plugin-check` and IMT's plugin host into a Workspace |
| `ci/dotnet-sdk.yml` | The CI workflow (copy to `.gitea/workflows/`) |

## Quick start

```sh
dotnet new install Imt.Plugin.Templates          # or: dotnet new install sdk/dotnet/artifacts/packages/Imt.Plugin.Templates.1.0.0.nupkg
dotnet new imt-plugin -n AcmeLogs --pluginId com.acme.logs
cd AcmeLogs
dotnet test                                      # runs the plugin over a real private socket, like IMT
dotnet publish src/AcmeLogs -c Release -r linux-x64
imt-plugin-check artifacts/plugin/com.acme.logs/bin/linux-x64/AcmeLogs
```

A plugin is a `Main` that builds a `PluginDefinition` and hands it to `ImtPlugin.RunAsync`:

```csharp
using Imt.Plugin;

return await ImtPlugin.RunAsync(args, PluginBuilder.Create("com.acme.logs")
    .Name("Acme logs")
    .Version("1.0.0")
    .Add(new AcmeImporter())      // IImporter / ITailImporter
    .Add(new AcmeMarkers())       // IMarkerKinds
    .Add(new AcmeSettings())      // ISettingsSection
    .Build());

public sealed class AcmeImporter : ITailImporter
{
    public string Id => "acme.log";

    public ValueTask<int> SniffAsync(ImportContext ctx, ReadOnlyMemory<byte> head, CancellationToken ct) =>
        ValueTask.FromResult(new ImportInput(head).Text.StartsWith("ACME ", StringComparison.Ordinal) ? 90 : 0);

    public async IAsyncEnumerable<Record> ImportAsync(ImportContext ctx, ImportInput input, [EnumeratorCancellation] CancellationToken ct)
    {
        uint line = 0;
        foreach (var text in input.Text.Split('\n'))
        {
            line++;
            if (Parse(text, line) is { } record) yield return record;   // batched into Arrow for you
            else ctx.Skip();                                           // counted as skipped
        }
    }

    public IAsyncEnumerable<Record>? ImportTail(ImportContext ctx, string head, string tail, CancellationToken ct) =>
        ImportAsync(ctx, new ImportInput(tail), ct);                   // or null: IMT re-imports the file
}
```

Without the template, reference the package (or `src/Imt.Plugin.Sdk` by project) from a `net10.0` console
app; see `examples/ToyFormatImporter/src/ToyFormatImporter.csproj` for the publish properties.

## API overview

Everything public lives in `Imt.Plugin` (idiomatic API), `Imt.Plugin.Arrow` (record batches),
`Imt.Plugin.Hosting` (transport) and `Imt.Plugin.Testing` (test host). The generated protobuf/gRPC types are
in `Imt.Plugin.V1` (contract) and `Imt.Ipc.V1` (control service); you rarely need them.

### Capabilities

Add contributions with `PluginBuilder.Add(object)`. The manifest's capabilities are derived from the
interfaces each object implements; one object may implement several (e.g. an importer that is also
`IMarkerKinds`).

| Interface | Contract | What you implement |
|---|---|---|
| `IImporter` | `ImporterCap`, `Sniff`, `Import` | `Id`, `SniffAsync` (0–100), `ImportAsync` → `IAsyncEnumerable<Record>`; optional `Binary`, `Multiline` |
| `ITailImporter : IImporter` | `ImportTail` (`tail = true`) | `ImportTail(ctx, head, tail)` → records of the tail, or `null` to ask for a full re-import |
| `ITransform` / `RecordTransform` | `TransformCap`, `Transform` | `ApplyAsync(TransformBatch, TransformContext)`: change records in place, `Drop(row)`, `Mark(row, kind)`; or override `RecordTransform.Apply(record, mark, ctx)` per record |
| `IMarkerKinds` | `MarkerKindsCap`, `MarkerKinds` | `MarkerKinds`: `MarkerKind` (kind, label, colour, icon, badge, shape, role, `When` conditions) |
| `IAnnotator` | `AnnotatorCap`, `Annotate` | `Id`, `Interest` (conditions IMT filters by), `Limit`, `AnnotateAsync(AnnotationInput)` → `Annotation`s (spans/points, severity, `OnField`/`OnValue`, evidence query) |
| `IExporter` | `ExporterCap`, `Export` | `Id`, `Name`, `Extension`, `ExportAsync(IAsyncEnumerable<ExportRow>, Stream)` → rows written |
| `ISettingsSection` | `SettingsCap`, `SettingsSection`, `ApplySettings` | `Section` (typed fields: `BoolSetting`, `IntSetting`, `FloatSetting`, `TextSetting`, `EnumSetting`, `PathSetting`, `DurationSetting`, `SizeSetting`, `StringListSetting`), `ApplyAsync(SettingsValues)` |
| `IPluginService` | `ServiceCap`, `CallService` | `ServiceId`, `ServiceVersion`, `CallAsync(method, payload)` (opaque bytes typed by the service's API, ADR 0003) |
| `IEventSubscriber` | `EventsCap`, `Events` | `EventKinds` (`recordsAppended`, `profileChanged`, `workspaceCleared`), `OnEventAsync(PluginEvent)` |

`PluginBuilder` also sets identity and needs: `Name`, `Version` (semver), `Requires(id, "^1.2")`,
`Optionally(id, req)`, `Provides(id, version)` (services added with `Add` are declared automatically) and
`Permission(...)`. `Build()` checks everything IMT checks (ids, semver, duplicate ids, exporter extensions,
marker kind names, icons, badges, shapes, colours, condition operators, annotator limits, the settings schema
and its defaults) and throws `InvalidOperationException` with a message that says what to fix, so a mistake
fails when the plugin starts instead of inside IMT.

### Records

`Record` mirrors IMT's record: `Timestamp` (ms since the epoch, or `Time` as `DateTimeOffset`), `Level`,
`Message`, `Template`, `Exception`, `Fields` (a `JsonObject`: any JSON value, order kept), `Line`, and the
IMT-assigned `Id`, `Source` and `Markers`. `Levels.TryParse` reads the usual spellings (`INF`, `WARN`,
`Error`, `FTL`, ...).

### Errors

Throw `PluginException` (or `PluginException.Invalid/TooLarge/Unimplemented`) when the input or request is
wrong: it reaches IMT as a typed error and does not count against the plugin. Any other exception escaping
plugin code is treated as a bug: IMT gets `CRASHED`, counts it towards quarantine and restarts the process.
`OperationCanceledException` after a cancelled call becomes `DEADLINE_EXCEEDED`/`CANCELLED`.

| `ErrorCode` | Value | gRPC status | Meaning |
|---|---|---|---|
| `Failed` | -32000 | `ABORTED` | your code reported an error (`PluginException`) |
| `Invalid` | -32602 | `INVALID_ARGUMENT` | unknown id, bad JSON, wrong message order, invalid Arrow |
| `Unimplemented` | -32601 | `UNIMPLEMENTED` | capability not provided |
| `Crashed` | -32010 | `INTERNAL` | unhandled exception in plugin code |
| `Version` | -32030 | `FAILED_PRECONDITION` | contract major mismatch |
| `NoHandshake` | -32031 | `INVALID_ARGUMENT` | call before `Handshake` |
| `TooLarge` | -32040 | `RESOURCE_EXHAUSTED` | a limit was exceeded |

### Cancellation, deadlines, batching

Every method takes a `CancellationToken` that fires when IMT's deadline passes (IMT sets one just under each
call's watchdog budget: sniff 5 s, import 120 s, tail 10 s, annotate 10 s, ...) or the call is abandoned;
check it in long loops. IMT kills a process that ignores a deadline. Importers yield records one by one; the
SDK closes an Arrow batch at 64k rows (or the `batch_rows` IMT asks for) or about 24 MiB of text and streams it
while you keep parsing. The SDK never blocks a thread-pool thread: the stdin lifeline runs on its own thread,
exporter output is sent with async writes, and settings are applied under an async lock.

### Testing

`Imt.Plugin.Testing.PluginTestHost` starts your plugin in the test process on a fresh private socket and talks
to it exactly as IMT does (gRPC, token, Arrow both ways), with helpers for every call: `HandshakeAsync`,
`SniffAsync`, `ImportAsync`, `ImportTailAsync`, `TransformAsync`, `AnnotateAsync`, `ExportAsync`,
`ApplySettingsAsync`, plus the generated `Client`/`Control` clients for anything else. Then run IMT's
conformance kit on the published executable: `imt-plugin-check <exe> [--grant read-opened-files] [--fuzz N]`.

## How it talks to IMT

The SDK mirrors the Rust SDK (`crates/imt-plugin-sdk`, `crates/imt-ipc`) exactly:

| Concern | IMT (Rust) | This SDK |
|---|---|---|
| Launch | `imt-plugin-host` / `RemotePlugin` spawns the executable with an empty environment (only `PATH`, `TMP*`, `LANG`, ...), a temp working directory, below-normal priority, stdout/stderr discarded, and writes one line `{"address": "...", "token": "..."}` to stdin | `ImtPlugin.RunAsync` reads that line from stdin (at most 64 KiB, `LaunchInfo`); arguments are not used (`--print-manifest` is for packaging only) |
| Transport | Unix socket `w.sock` in a fresh 0700 directory (`$XDG_RUNTIME_DIR` or temp); Windows named pipe `\\.\pipe\imt-plugin-<random>` with an owner-only DACL, remote clients rejected; HTTP/2, never TCP | Kestrel `ListenUnixSocket` (then `chmod 0600` on the socket) or `ListenNamedPipe` with a `PipeSecurity` that allows only the current user and denies the NETWORK SID; HTTP/2 only; no TCP endpoint is ever opened |
| Token | 256-bit random hex, sent as `x-imt-token` on every call (including `imt.ipc.v1.Control`); `UNAUTHENTICATED` otherwise | `ImtInterceptor` checks every call of every service, compared with `CryptographicOperations.FixedTimeEquals` |
| Typed errors | JSON `{code, message, data?}` in the binary trailer `x-imt-error-bin`; gRPC status per code (table above) | `RemoteError.ToRpcException` writes the same trailer and status; `RemoteError.TryRead` reads it |
| Handshake | `Handshake` first; calls before it → `NO_HANDSHAKE`; other contract major → `VERSION`; manifest checked by `RemoteManifest::parse` | same gate and errors; the manifest comes from the `PluginDefinition` |
| Control | `imt.ipc.v1.Control`: `Status` (pid, RSS, uptime, name) for health checks, `Shutdown` | served by `ControlService` (`name` = `"<id> <version>"`) |
| Lifeline | the child exits when stdin closes (IMT gone or done) | `Lifeline.Watch` reads stdin to EOF on a background thread and stops the server |
| Shutdown | connections get a second to drain | `HostOptions.ShutdownTimeout` = 1 s; also on `Control.Shutdown` and SIGTERM |
| Limits | 64 MiB messages, 1 MiB chunks, 4 GiB per import, 1 Mi rows per batch, 100k annotator records | same constants (`Contract`); imports are also capped at the largest .NET array |
| Logging | child stderr is not read by IMT | logs go to stderr; `Console.Out` is redirected to stderr while serving, so nothing is ever written to stdout |

### Arrow record schema

Records cross as Arrow IPC *stream* bytes (`RecordBatch.arrow`), written with LZ4-frame buffer compression like
IMT; both compressed and uncompressed batches are read. Columns, in order:

| Column | Arrow type | |
|---|---|---|
| `ts` | Int64, nullable | ms since the Unix epoch (UTC) |
| `level` | UInt8 | 0 Verbose, 1 Debug, 2 Information, 3 Warning, 4 Error, 5 Fatal |
| `message` | Utf8 | |
| `template` | Utf8, nullable | |
| `exception` | Utf8, nullable | |
| `line`, `id`, `source` | UInt32 | `id`/`source` assigned by IMT |
| `markers` | UInt64 | assigned by IMT; plugins mark through transform replies |
| fields | see below | |
| extra | Utf8 | `source_name` + `group` (exports), `source_name` + `instance` (annotators) |

Fields use one of two layouts, chosen per batch exactly as the Rust encoder does: **columnar** (one nullable
column per field name, first-seen order: `f:<name>` typed Utf8/Int64/UInt64/Float64/Boolean, `j:<name>` JSON text
for objects, arrays, JSON null or mixed types; `field_order` lists column indexes for records whose order
differs) or **JSON** (one `fields` Utf8 column with a JSON object per record, used above 256 names or for sparse
names). Values are classified like `serde_json` (integers → Int64, beyond `long.MaxValue` → UInt64, other
numbers → Float64), so types, integer vs. float, and field order survive a round trip. `RecordBatchBuilder` and
`RecordBatchReader` are public for tools and tests.

## Permissions

A plugin asks for what it needs beyond the bytes IMT reads for it; the user grants it at install, and IMT
refuses to start a plugin whose permissions were not granted.

```csharp
.Permission(Permission.ReadOpenedFiles, "Converts BLG files with relog.exe, which needs the path")
.Permission(Permission.Network("symbols.example.com"), "Downloads symbols")
```

- `ReadOpenedFiles`: `ImportContext.Path` is the full path of the file (otherwise null; the plugin only gets
  the content IMT read).
- `Network(host)`: connecting to that host (empty: any). A native process can technically open sockets
  anyway: the permission is a declaration the user approves, so ask only for what you use.

Ask for nothing if you can: `samples/OrderAudit` needs no permission at all.

## Packaging per OS

Publish one self-contained single-file executable per OS/architecture (no .NET install needed on the user's
machine). The examples set:

```xml
<RuntimeIdentifiers>osx-arm64;osx-x64;linux-x64;linux-arm64;win-x64;win-arm64</RuntimeIdentifiers>
<SelfContained>true</SelfContained>            <!-- when a RuntimeIdentifier is given -->
<PublishSingleFile>true</PublishSingleFile>
<EnableCompressionInSingleFile>true</EnableCompressionInSingleFile>
<InvariantGlobalization>true</InvariantGlobalization>
<ImtPluginLayoutDir>…/artifacts/plugins/com.example.myplugin</ImtPluginLayoutDir>
```

```sh
dotnet publish src/MyPlugin -c Release -r osx-arm64   # macOS Apple silicon
dotnet publish src/MyPlugin -c Release -r linux-x64   # Linux
dotnet publish src/MyPlugin -c Release -r win-x64     # Windows (cross-publishing works from any OS)
```

With `ImtPluginLayoutDir` set, the SDK's `ImtPluginLayout` target (shipped in the package's
`buildTransitive`) copies each executable into the plugin folder IMT's plugin manager installs, and writes
`plugin.json` (by running the executable with `--print-manifest`) when the RID runs on the build machine:

```
com.example.myplugin/
  plugin.json                       {"format": 1, "contract": {"kind": "grpc", "package": "imt.plugin.v1", "major": 1, "minor": 0},
                                     "executable": "my-plugin", "manifest": { …the Handshake reply in protobuf JSON… }}
  bin/osx-arm64/my-plugin
  bin/linux-x64/my-plugin
  bin/win-x64/my-plugin.exe
```

The layout is proposed in `sdk/README.md` (IMT does not install external plugins yet). Sizes: about 50 MB per
executable (ASP.NET Core + gRPC + Arrow, compressed single file). macOS binaries published on a Mac are ad-hoc
signed by the SDK; sign and notarize them for distribution. Sign Windows executables with Authenticode.

## Debugging

- **Unit tests first.** `PluginTestHost` runs the plugin in the test process: breakpoints, exceptions and
  logs all work, and the calls are the real gRPC/Arrow path.
- **Run it by hand** the way IMT does, with logs on the console:

  ```sh
  mkdir -m 700 /tmp/p && (echo '{"address":"/tmp/p/w.sock","token":"dev"}'; cat) | ./my-plugin
  ```

  (the plugin stops when you press Ctrl-D, closing stdin). On Windows use `\\.\pipe\my-plugin-dev`.
- **Attach a debugger** to the process IMT started: `Control.Status` reports its pid (IMT's Plugins panel
  shows it), or call `System.Diagnostics.Debugger.Launch()` in a debug build.
- **Conformance:** `imt-plugin-check <exe>` prints every check (handshake, each capability, fuzzed inputs,
  deadlines, crash-freedom); `--seed` reproduces a fuzz failure.
- IMT discards a plugin's stderr; return problems as `PluginException` messages, which IMT shows.

## Versions and compatibility

| SDK (`Imt.Plugin.Sdk`) | Contract | Arrow data plane | .NET | IMT |
|---|---|---|---|---|
| 1.0.x | `imt.plugin.v1`, minor 0 (incl. `ImportStart.batch_rows`) | columnar `f:`/`j:` fields or JSON `fields`, LZ4-frame | net8.0, net10.0 (LTS) | builds with `imt-plugin-sdk` 0.1 |

Minor contract versions only add RPCs, optional fields and enum values; the SDK tolerates unknown ones and a
plugin built with an older SDK keeps working. A new major (`imt.plugin.v2`) comes with a new SDK major; IMT
supports the previous major for at least one release. The C# code is generated from `proto/` at build time
(`Grpc.Tools`), so the SDK cannot drift from the contract it was built with.

## Dependencies

Pinned exactly in `Directory.Packages.props` (central package management); `packages.lock.json` files are
committed and CI restores with `--locked-mode`. `tools/check_licenses.py` checks every resolved package.

| Package | Version | Licence | Used for |
|---|---|---|---|
| Grpc.AspNetCore (+ Server, ClientFactory, Net.Client, Core.Api) | 2.84.0 | Apache-2.0 | gRPC server on Kestrel, test client |
| Grpc.Tools | 2.84.0 | Apache-2.0 | code generation at build time (not shipped) |
| Google.Protobuf | 3.36.2 | BSD-3-Clause | messages |
| Apache.Arrow | 23.0.0 | Apache-2.0 | record batches |
| Apache.Arrow.Compression | 23.0.0 | Apache-2.0 | LZ4-frame IPC buffers (K4os.Compression.LZ4: MIT; ZstdSharp.Port: MIT) |
| xunit.v3 (tests) | 4.0.2 | Apache-2.0 | tests on Microsoft.Testing.Platform (MIT) |
| coverlet.MTP (tests) | 10.1.0 | MIT | coverage |

## Building and testing this SDK

```sh
cd sdk/dotnet
dotnet build Imt.Plugin.Sdk.slnx -warnaserror
dotnet test --solution Imt.Plugin.Sdk.slnx -- --coverlet --coverlet-output-format cobertura \
  --coverlet-exclude-by-file "**/obj/**" --coverlet-does-not-return-attribute DoesNotReturnAttribute
dotnet list Imt.Plugin.Sdk.slnx package --vulnerable --include-transitive
python3 tools/check_licenses.py
tests/e2e/run.sh            # publish, imt-plugin-check, Workspace import through imt-plugin-host
```

The conformance test in `tests/Imt.Plugin.Sdk.Tests` runs `imt-plugin-check` when it finds it (set
`IMT_PLUGIN_CHECK`, or build it into `artifacts/cargo-target`), and is skipped otherwise.
