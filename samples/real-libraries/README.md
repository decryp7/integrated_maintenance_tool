# Real .NET library output

Written by [`tools/dotnet-sample-gen`](../../tools/dotnet-sample-gen/README.md) through the real libraries,
not by `generate.py`; regenerate them with that tool, never by hand. Every event is invented. The files sit
outside the incident's time window (`real-libraries` is excluded from the whole-folder tests in
`crates/debrief-core/tests/it/samples.rs`).

Written on 22 Oct 2026 on Windows 11 (machine time zone Singapore, UTC+08:00) with:

| Package | Version |
|---|---|
| .NET SDK / runtime | 10.0.203 / 10.0.7 |
| Serilog | 4.4.0 |
| Serilog.Formatting.Compact | 3.0.0 |
| Serilog.Sinks.File | 7.0.0 |
| log4net | 3.5.0 |
| NLog | 6.2.2 |
| Microsoft.Extensions.Logging.Console | 10.0.13 |

| File | Library and layout | Read by | Times |
|---|---|---|---|
| `serilog-compact.clef` | Serilog `CompactJsonFormatter` | parser `serilog-clef` | UTC (`Z`) |
| `serilog-json.json` | Serilog `JsonFormatter(renderMessage: true)` | parser `serilog-json` | +09:00 |
| `serilog-text.txt` | Serilog file sink, default template, CRLF | parser `serilog-text` | +09:00 |
| `log4net-pattern.log` | log4net `PatternLayout`, CRLF | parser `log4net-pattern` | machine local, no offset (read as UTC) |
| `log4net-csv.log` | log4net CSV-like `PatternLayout`, no header | parser `log4net-csv` | machine local with offset (+08:00) |
| `log4net-log4j.xml` | log4net `XmlLayoutSchemaLog4J` | Formats plugin (`log4j-xml`) | epoch milliseconds |
| `nlog-default.log` | NLog `FileTarget` default layout | parser `nlog` | Tokyo wall clock, no offset (read as UTC) |
| `mel-console.log` | Microsoft.Extensions.Logging simple console | parser `mel-console` | time of the run, with offset (+08:00) |

Tests: `crates/debrief-core/tests/it/real_libraries.rs` (the text and JSON files) and
`crates/debrief-plugins/tests/it/real_libraries.rs` (the XML). They also hold the regression tests for the gaps these
files showed: the log4net CSV layout's unquoted messages with commas and multi-line exceptions, NLog's exception
after the `|` of `${message:withexception=true}`, and the console formatter's message moving into the exception.
