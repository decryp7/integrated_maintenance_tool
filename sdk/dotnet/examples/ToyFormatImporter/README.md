# ToyFormatImporter

The smallest useful IMT plugin in C#: an importer for the toy line format

```
2024-05-01T12:00:00.250Z|WRN|Cache miss|key=user:42|ms=1830
12:00:01|INF|time of day only, dated by the file
```

with two marker kinds (`toy-error` for errors, `toy-slow` for `ms > 1000`), one setting (longest message) and
live tail. Four small files: `Program.cs`, `ToyPlugin.cs`, `ToyImporter.cs`, `ToyMarkers.cs`, `ToySettings.cs`.

```sh
dotnet test ../ToyFormatImporter/tests
dotnet publish src -c Release -r osx-arm64       # -> sdk/dotnet/artifacts/plugins/com.example.toyformat/
imt-plugin-check ../../artifacts/plugins/com.example.toyformat/bin/osx-arm64/toy-format-importer
```

For every capability (transform, annotator, exporter, services, events), see `samples/OrderAudit`.
