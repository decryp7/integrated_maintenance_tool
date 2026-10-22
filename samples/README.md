# Samples

Generated sample data for every IMT view and every supported input format. Everything
here tells one story, so the views can be compared side by side and screenshots line up:

> **14 Oct 2026, 09:00–12:00 UTC.** An OrderService web farm (WEB01, WEB02) sits behind a
> gateway and an nginx edge, with a worker, a legacy billing service, SQL Server and a
> payments adapter in Docker/Kubernetes. From 10:30 WEB02 leaks memory, its CPU climbs,
> payment captures time out and SQL deadlocks pile up; at **10:41:24** OrderService on WEB02
> crashes (pid 4120) and restarts **46 s later** as pid 5532. Elsewhere that morning BATCH01
> loses power at 10:52 and reboots, and WEB02's clock runs 3.7 s behind.

Open the whole folder in IMT (*Open folder…*), then import
[`profiles/orderservice.imtprofile.toml`](profiles/orderservice.imtprofile.toml) in the
Profile tab for the extracted metrics, charts and dashboard.

## Regenerating

Do not edit the samples by hand: `.gitattributes` keeps `samples/**` byte-exact and the
tests compare bytes.

```sh
cargo xtask gen-samples            # everything: generate.py, then the Rust generators below
python3 samples/generate.py        # only generate.py: everything below except the crash dump and profiling/
```

The output is deterministic: each section of `generate.py` has its own `random.Random`
seed, so changing one section leaves every other file byte-identical, and running it twice
(or with any Python ≥ 3.9 on any platform) gives the same bytes. Compressed and binary
files are written by hand (stored zip/gzip entries, a pure-Python EVTX writer), so they do
not depend on the local zlib.

Two kinds of file come from Rust instead, written by `#[ignore]`d tests gated by
`IMT_REGENERATE_SAMPLES=1` (the same convention as the profiling samples); a normal test
run fails while the committed file is stale:

```sh
# windows/WEB02/OrderService.exe.4120.dmp (synthetic minidump builder in tests/support)
IMT_REGENERATE_SAMPLES=1 cargo test -p imt-plugin-windows --test samples -- --ignored
# dumps/*.dmp and the dump fuzz seeds (synthetic minidump builder, imt_plugin_dumps::synth)
IMT_REGENERATE_SAMPLES=1 cargo test -p imt-plugin-dumps --test samples -- --ignored regenerate_
# profiling/: see docs/profiling.md
```

The live-tail demo [`live/append.py`](live/append.py) is a script, not a sample: it appends
lines to a growing, rotating log until stopped (`python3 samples/live/append.py --help`).

## What is where

“Plugin” means the file is read by that plugin's importer (all are installed in the app);
without it the file is listed as unrecognised.

| Sample | Format | Read by | Shows in IMT |
|---|---|---|---|
| `serilog-clef/web01/`, `serilog-clef/web02/orders-20261014.clef` | Serilog CLEF (`@t`/`@mt`/`@l`/`@x`) | `json` | Timeline lanes per process, the WEB02 crash (Fatal) and restart, anomalies (crash, error spike, latency), exceptions and patterns, `CpuPercent`/`WorkingSetMb` charts, correlation by `CorrelationId` |
| `serilog-json/gateway/gateway-20261014.json` | Serilog `JsonFormatter` (`Properties`) | `json` | Upstream 504s from WEB02, health probe failure and recovery; calculated fields |
| `serilog-text/worker01/worker-20261014.txt` | Serilog text template + `{Properties:j}` | `text` | Extractors (process stats from text, outbox backlog), a batch job with start/stop |
| `log4net/legacy-billing/billing.log.2026-10-14` | log4net `PatternLayout`, CRLF, no extension | `text` | Regex parsing, `Thread`/`Logger` fields, multi-line exceptions |
| `android-logcat/qa-pixel07/logcat-20261014.txt` | Android logcat, `threadtime` (no year) | `text`, parser `android-logcat` | The shop app on a QA phone: slow API calls from 10:30, `FATAL EXCEPTION` at 10:41:24 (pid 4211, one event per stack line as logcat writes them) and the restart as pid 4588; all six levels; logcat's `--------- beginning of crash` divider and a cut-off line join the event before them. The year comes from the file date |
| `hadoop-spark/analytics01/spark-orders-export.log`, `hadoop-spark/dn01/hadoop-hdfs-datanode-dn01.log` | Spark log4j `yy/MM/dd HH:mm:ss`, HDFS `yyMMdd HHmmss <thread>` | `text`, parser `hadoop-spark` | Executor errors with `Caused by` traces during the SQL deadlocks, the JVM's stdout line, DataNode socket timeouts; `Logger` and `Thread` fields |
| `apache-error/edge02/error.log` | Apache httpd 2.4 `ErrorLog` | `text`, parser `apache-error` | Proxy timeouts and refused connections to WEB02 while it is down; `Module`, `Pid`, `Tid`, `Client` fields; a CGI script's Python traceback written without a prefix; a line cut short |
| `nlog/notify01/notify-scheduler-20261014.log` | NLog default layout (`${message:withexception=true}`), CRLF | `text`, parser `nlog` | SMTP failures with inner exceptions after the message's `\|`, the job paused (Fatal) after the crash, a stray `Console.WriteLine` |
| `mel-console/payments01/payments-adapter-20261014.log` | Microsoft.Extensions.Logging simple console with `TimestampFormat` | `text`, parser `mel-console` | Capture timeouts (Polly) from 10:30, a `crit` circuit breaker that opens and closes, `trce`…`crit` levels, multi-line messages, a library's own stdout line |
| `real-libraries/` | Output of the real Serilog, log4net, NLog and Microsoft.Extensions.Logging (not from `generate.py`) | per file, see [its README](real-libraries/README.md) | What the libraries really write: inner exceptions, structured properties, Japanese text, time-zone offsets. Outside the scenario's time window |
| `csv/perfmon-export.csv` | `;`-separated CSV, day-first dates | `csv` | CSV import, day-first detection, counter charts |
| `log4j-xml/` | log4net XmlLayoutSchemaLog4j, NLog Log4JXml | formats plugin (`log4j-xml`) | XML events, typed fields |
| `sqlserver/SQL01/ERRORLOG`, `sqlserver/sqlrpt01/errorlog` | SQL Server ERRORLOG (UTF-16 LE, local time UTC+2; UTF-8 on Linux) | formats plugin (`sql-errorlog`) | Deadlocks, log full, I/O errors; time zones |
| `docker/containers/…/…-json.log` | Docker `json-file` with split 16 KiB lines | formats plugin (`docker-json`) | Partial-line joining, stdout/stderr |
| `kubernetes/pods/…/consumer/0.log`, `1.log` | Kubernetes CRI | formats plugin (`cri`) | A pod crash (panic) and its restart |
| `windows/WEB02/System.evtx`, `Application.evtx` | Windows event log (EVTX, written by `generate.py`) | Windows plugin (`evtx`) | Crash markers (Application Error 1000, .NET Runtime 1026, WER 1001, SCM 7031), the service uptime annotator (OrderService running → crashed → running), Time-Service warnings behind the clock skew |
| `windows/BATCH01/System.evtx` | EVTX | Windows plugin (`evtx`) | Unexpected shutdown: Kernel-Power 41, EventLog 6008/6005/6009, Kernel-General 12; the reboot (downtime) annotator |
| `windows/WEB02/procmon-OrderService.csv` | Process Monitor “Save as CSV” (BOM, CRLF, `Date & Time`, `TID`, `Duration`) | Windows plugin (`procmon-csv`) | The process exit at 10:41:24, WerFault writing the dump, ACCESS DENIED / SHARING VIOLATION markers, the new process starting |
| `windows/WEB02/perfmon-OrderService_000001.csv` | PerfMon PDH-CSV (what `relog` makes of a `.blg`) | Windows plugin (`perfmon-csv`) | Process counters that follow the PID across the restart and are blank while it is down; machine counters |
| `windows/WEB02/OrderService.exe.4120.dmp` | Windows minidump (synthetic, from the Rust builder) | Windows plugin (`minidump`) | Crash dump summary: CLR exception 0xE0434352 in KERNELBASE.dll, stack scan, modules (coreclr, CoreLib) |
| `dumps/OrderService.exe.4120.full.dmp`, `dumps/Billing.exe.900.hang.dmp` | Full dump (x64, `Memory64List`) and a hang dump of a 32-bit .NET Framework process (synthetic, from the Rust builder) | dumps plugin (`minidump`) | Dump view: access violation, culprit frame, threads waiting on a lock; a dump without an exception, the .NET runtime |
| `dumps/umdh/OrderService-4120-snap-{1030,1035,1040}.txt`, `…-diff-1030-1040.txt` | UMDH heap snapshots (`umdh -p -f`) and UMDH's own diff (written by `generate.py`) | dumps plugin (`umdh`) | UMDH view: the order cache grows in every snapshot (leak), payment retries grow, a 32 MB single allocation, PdfLib frames without symbols |
| `windows/registry-export.reg`, `windows/msinfo32.nfo` | regedit export (UTF-16), msinfo32 report | registry plugin (`registry`), Windows plugin (`msinfo`) | Configuration as records (no timestamps) |
| `monitor/BILL01/process-monitor.clef` | IMT process monitor (`imt-monitor`) output, CLEF | `json` | “Process stats” with the .NET Framework runtime counters (`Dotnet*`: GC per generation, heap sizes, % time in GC, exceptions/s, contention), an app pool recycle detected as exit + restart (`RestartedAs`, `Restarted`) |
| `web/iis/WEB01/W3SVC1/u_ex261014.log`, `web/iis/WEB02/…` | IIS W3C extended log; WEB02's has a second header block after the restart | web plugin (`w3c`) | `time-taken` growing on WEB02, 502.3 while the app is down, 4xx/5xx levels |
| `web/nginx/edge01/access.log` | nginx combined log + `$request_time` | web plugin (`access-log`) | 504s at the edge from 10:30, request times |
| `web/har/checkout-slow.har` | HAR 1.2 (browser DevTools export) | web plugin (`har`) | A checkout whose payment capture hangs for 30 s and returns 504; a blocked request (status 0) |
| `linux/docker01/syslog` | `/var/log/syslog`, RFC 3164 (BSD) lines without PRI | syslog plugin (`syslog`) | Host/app/pid fields, dockerd health checks failing and conntrack drops during the incident. BSD lines carry no year: it comes from the file date |
| `linux/k8s-node-2/rsyslog-rfc5424.log` | RFC 5424 with structured data | syslog plugin (`syslog`) | `sd.k8s@32473.*` fields, readiness probe failures, the container crash and back-off |
| `linux/sqlrpt01/journal.json` | `journalctl -o json` | syslog plugin (`journald`) | Kernel I/O errors on `sdb` just before SQL Server error 823 on the same host |
| `linux/logserver01/apps-20261014.log` | A log server's shared file: Checkout (Serilog text, APP01/APP02, pid 3120 recycled as 3388), Inventory (log4net, pids 7001/7002 in the message), Payments (CLEF JSON lines) and haproxy lines no default format reads | `text` (per-line formats); `profiles/logserver.imtprofile.toml` adds routes | Each line read with its own format, no cross-product continuation, unmatched lines reported in the Explorer, lanes per product and process; with the profile, logical sources `Checkout@APP01`, `Checkout@APP02`, `Inventory@SRV2`, `Payments@PAY01` under the file (ADR 0020) |
| `linux/logserver01/syslog-20261014.log` | RFC 5424 relayed from web01, db01 and edge01 (nginx, sshd, postgres, haproxy) | syslog plugin (`syslog`) | One lane per host, app and process from the importer's identity fields |
| `otel/collector/traces-20261014.jsonl`, `logs-20261014.jsonl` | OTLP/JSON, Collector `file` exporter (one export per line) | OpenTelemetry plugin (`otlp`) | Traces gateway → order-service → SQL / payments-adapter with errors and exception events during the incident; logs linked by `TraceId`/`SpanId` |
| `clock-skew/gateway/`, `clock-skew/web02/requests-20261014.clef` | CLEF | `json` | Clock-skew suggestion: both log each request's `CorrelationId`, WEB02's clock is 3.742 s behind, so the suggestion proposes about +3.74 s for WEB02 |
| `archives/notify01/notify01-logs-20261014.zip`, `app.log.3.gz` | zip and gzip of a rotated set (`app.log`, `app.log.1`, `.2`, `.3`) | `text` (archives are expanded) | Transparent archive reading, rotated files in order, e-mail addresses to redact |
| `runs/batch01/report-service-20261014.txt` | Serilog text | `text` | Runs: clean start/stop pairs, a run cut off by BATCH01's power loss (no stop, the next start follows the reboot) and a run still going at 12:00 |
| `runs/shared/workers-20261014.log` | Serilog text, two processes in one file | `text` | Runs of two interleaved workers (`Worker` A/B, `ProcessId` 7101/7102) in one shared file; worker B abandons one batch (start without stop) |
| `configs/WEB01/`, `configs/WEB02/` | `appsettings.json`, `web.config` (XML), `orderservice.ini`, `otel-collector.yaml`, `OrderService.reg` (UTF-16), `msinfo32.nfo` (UTF-16) | config diff (JSON/XML/INI/YAML built in; `.reg` registry plugin; `.nfo` Windows plugin) | Configuration diff of two machines (folder snapshot or file pairs): WEB02's smaller SQL pool, debug logging, the feature flag, GC mode, memory and OS build |
| `share/orderservice-incident.imtproj` | IMT project (version 2) | *Open project* | The incident's folders with the sample profile and WEB02's clock offset already applied |
| `share/web02-clock-skew.imtz` | IMT share bundle (`imt-share` schema 1, stored zip) | *Open bundle* | A bundle as `Workspace::write_bundle` writes it: the clock-skew logs, profile, project with the offset, manifest with SHA-256 per file |
| `diagnostics/imt-diagnostics-sample.zip` | IMT diagnostics report (problem-report zip, manifest schema 1) | Diagnostics report | The Diagnostics report panel (system info, processes, steps, breadcrumbs, snapshots); one lane per log (IMT, activity, ai-worker, llama-server, an out-of-process plugin) with crash, start and quarantine markers. Written by the real report writer: `IMT_REGEN_SAMPLES=1 cargo test -p imt-app --lib activity::report::sample` (the same test checks it is byte-identical) |
| `profiles/orderservice.imtprofile.toml` | IMT profile | Profile tab | Instance key, extractors, charts and the “OrderService health” dashboard |
| `profiling/` | profiler captures | profiling plugin | See [docs/profiling.md](../docs/profiling.md) |

Not generated as samples:

- **`.blg`** (binary PerfMon log): its format is undocumented and IMT converts it with
  Windows' `relog` on import, so no deterministic generator exists here; the PDH-CSV above
  is exactly what `relog` produces from one.

## Tests

- `crates/imt-core/tests/samples.rs`: everything the built-in importers read (formats,
  timestamps, lanes, charts, archives, runs markers, the clock-skew suggestion, the config
  diff, the project and the bundle).
- `crates/imt-core/tests/parsers.rs`: the built-in parsers (ADR 0026) on their samples: the right parser per
  file (also in one mixed folder), timestamps, levels, fields and multi-line events.
- `crates/imt-core/tests/importer_snapshots.rs`: insta snapshots per built-in importer.
- `crates/imt-plugins/tests/evtx_corpus.rs` (runs only with `IMT_EVTX_CORPUS=1`; nightly `corpus` workflow): about
  280 real-world event logs of [EVTX-ATTACK-SAMPLES](https://github.com/sbousseaden/EVTX-ATTACK-SAMPLES)
  (GPL-3.0, `LICENSE.GPL`), downloaded for the run (about 6 MB, pinned by commit and SHA-256, kept in
  `IMT_CORPUS_CACHE` when set) into a temporary folder and never committed or redistributed.
- `crates/imt-plugin-{windows,web,syslog,otel,registry,formats}/tests/samples.rs`: the
  plugin formats, with snapshots in each crate's `tests/snapshots/`.
- `crates/imt-plugin-profiling/tests/samples.rs`: the profiler captures.
