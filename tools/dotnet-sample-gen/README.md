# dotnet-sample-gen

A developer tool, **not part of Debrief**: a small .NET console program that logs invented events through the
real .NET logging libraries, so Debrief's parsers are tested against what those libraries actually write
rather than against hand-made imitations. The app, `cargo` and CI never build or run it (Debrief ships only
Rust and the Angular UI); its output is committed under [`samples/real-libraries/`](../../samples/real-libraries/).

It writes the same seven events (start-up, a destructured object, Japanese text, a slow call, an error with
an inner exception, a retry, a fatal `AggregateException` with two inner exceptions) through:

| Library | Output |
|---|---|
| Serilog + Serilog.Formatting.Compact | `serilog-compact.clef` (CLEF) |
| Serilog `JsonFormatter` | `serilog-json.json` |
| Serilog file sink, default text template | `serilog-text.txt` |
| log4net `PatternLayout` `%date [%thread] %-5level %logger - %message%newline%exception` | `log4net-pattern.log` |
| log4net CSV-like `PatternLayout` without a header (date, time with offset, level, code, process, pid, message) | `log4net-csv.log` |
| log4net `XmlLayoutSchemaLog4J` | `log4net-log4j.xml` |
| NLog `FileTarget` with its default layout | `nlog-default.log` |
| Microsoft.Extensions.Logging simple console (`TimestampFormat = "yyyy-MM-dd HH:mm:ss.fffzzz "`, scopes) | `mel-console.log` |

All events are invented. Serilog, log4net and NLog are given fixed instants (14 Oct 2026, 19:30–19:42 in
Tokyo, +09:00), so their files only change with a library version — except log4net's `%date`, which is the
machine's local time. The Microsoft.Extensions.Logging console formatter takes the clock's time, so
`mel-console.log` has the time and offset of the run. Stack traces name `/src/OrderService/Program.cs`
(`PathMap` in the project), never the developer's checkout.

## Running

Needs the .NET 10 SDK.

```sh
dotnet run --project tools/dotnet-sample-gen -- samples/real-libraries
```

The tool replaces every file in the folder except its `README.md`. After a run, update the versions (and
the time zone of the machine) in [`samples/real-libraries/README.md`](../../samples/real-libraries/README.md)
and run the engine tests over the files:

```sh
cargo test -p debrief-core --test it real_libraries::
cargo test -p debrief-plugins --test it real_libraries::
```

To try newer library versions, change them in `DotnetSampleGen.csproj` (or `dotnet add package <name>`).
