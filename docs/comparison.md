# IMT compared with other log tools

How IMT sits among the log viewers and log platforms people already use, for anyone deciding whether
it fits their job. The IMT column is taken from the [feature list](features.md). The others are our reading
of each product's public pages as of October 2026, without testing every version; products change,
so check their current documentation before relying on a cell. Corrections are welcome.

## In one line

**The analysis of a log platform on a folder of logs, offline on your desktop: no ingestion, no
server, and nothing leaves the machine.**

IMT is for investigating after the fact: a support engineer given a zip of logs from a customer
site, a developer chasing a Windows or .NET service problem, a team in an air-gapped or regulated
environment. It does not replace fleet-wide monitoring.

## Side by side

| | **IMT** | lnav | klogg · LogExpert | LogViewPlus | Seq | Splunk · Elastic · Graylog · Loki |
|---|---|---|---|---|---|---|
| **Kind** | Desktop app (Windows, macOS, Linux) | Terminal app | Desktop viewers (klogg cross-platform, LogExpert Windows) | Windows desktop app | Self-hosted server, web UI | Server or cloud platform |
| **Getting logs in** | Open files, folders, archives, or drag and drop. The format is detected from content, per file or per line | Open files. Format detection, gzip/bzip2 support | Open files (plain text) | Open files, plus remote file access (SMB, FTP, SFTP) | Apps send logs to the server (HTTP, OTLP, seqcli) | Agents and pipelines ship and index logs |
| **Many files and machines** | One swimlane timeline with clock-skew correction | One merged, time-ordered view | One file at a time | Merged, date-sorted view | One event stream | Across the fleet, once ingested |
| **Structured fields** | Profiles (TOML), extractors, calculated fields, scripts | Format definitions, SQL over logs (SQLite) | Regex search and highlights | Parsers per file type | Structured events, SQL-style queries | Rich query languages |
| **Start/stop runs** | Built in: runs per lane or per field such as process ID, with crash and orphan flags | No | No | No | Through queries | Through queries and dashboards |
| **Analysis** | Patterns, anomalies, exceptions, comparisons, traces, the Moment view, metrics, known issues from your own notes, a one-click incident summary and stepwise playbooks | Histograms, SQL | Search | Filters, highlights | Charts, dashboards, alerts | Dashboards, alerts, machine learning (product-dependent) |
| **Windows diagnostics** | Event logs (EVTX), registry exports, Performance Monitor, IIS, crash dumps and UMDH heap logs, .NET and native profiling | Text formats | Text | Text and common formats | Through log shippers | Through agents |
| **AI** | Optional local model, runs offline, you choose the runtime; reads screenshots, screen recordings (frames) and your own knowledge files; researches the web only in IMT's own read-only browser | No | No | No | Not a focus | Cloud AI add-ons (product-dependent) |
| **Extending** | Rust plugins, each in its own sandboxed process with a memory cap, shipped as signed packages | Format files, scripts | Limited | Limited | Apps | Apps, plugins, pipelines |
| **Where data goes** | Stays on your machine | Stays on your machine | Stays on your machine | Stays on your machine | The Seq server | The platform's servers or cloud |
| **When the tool itself fails** | One-click problem report that IMT can open and analyse | Usual bug reports | Usual bug reports | Usual bug reports | Server logs | Platform support |

## Where others are the better choice

- **Continuous monitoring of a fleet**: Splunk, Elastic, Graylog and Loki ingest all the time, keep
  data for months, alert teams and scale to terabytes. IMT works on what you open.
- **A quick look over SSH**: lnav runs in a terminal on the server itself.
- **One huge plain-text file**: klogg reads straight from disk and stays fast past 10 GB.
- **Centralised structured logging for .NET teams**: Seq is a mature server workflow for that.

## Why IMT

- **Mixed evidence in one place.** Application logs, event logs, registry exports, Performance
  Monitor captures, web server logs and OpenTelemetry from several machines, on one timeline.
- **No setup before the first answer.** Formats are detected, and lifecycle markers and runs work
  out of the box. Profiles add what's specific to your product, and you can share them.
- **Private by design.** Files are opened read-only, nothing is uploaded, and AI runs locally.
- **Built to stay up.** Crashing plugins are contained, bad files are skipped with a reason,
  sessions recover, and safe mode is there when you need it.

## Sources

[lnav](https://lnav.org) · [klogg](https://github.com/variar/klogg) ·
[LogExpert](https://github.com/LogExperts/LogExpert) · [LogViewPlus](https://www.logviewplus.com) ·
[Seq](https://datalust.co/seq) · [Splunk](https://www.splunk.com) · [Elastic](https://www.elastic.co) ·
[Graylog](https://graylog.org) · [Grafana Loki](https://grafana.com/oss/loki/)
