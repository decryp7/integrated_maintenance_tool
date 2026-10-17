# MyImtPlugin

An IMT process plugin built with the C#/.NET SDK (`Imt.Plugin.Sdk`, contract `imt.plugin.v1`).

```sh
dotnet test                                                   # unit tests (the plugin over a real private socket)
dotnet publish src/MyImtPlugin -c Release -r osx-arm64        # or linux-x64, win-x64, ...
imt-plugin-check artifacts/plugin/com.example.myimtplugin/bin/osx-arm64/MyImtPlugin   # IMT's conformance kit
```

Publishing writes the folder IMT's plugin manager installs:

```
artifacts/plugin/com.example.myimtplugin/
  plugin.json                 manifest (id, version, contract, permissions, capabilities, executable)
  bin/<rid>/MyImtPlugin[.exe] one self-contained executable per OS/architecture
```

Start in `src/MyImtPlugin/LineImporter.cs` (the format) and `Plugin.cs` (what the plugin contributes:
add `ITransform`, `IMarkerKinds`, `IAnnotator`, `IExporter`, `ISettingsSection`, ... contributions there).
The SDK README (sdk/dotnet/README.md in the IMT repository) documents every interface, permissions,
packaging and debugging.
