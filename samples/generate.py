#!/usr/bin/env python3
"""Generates the sample logs in this folder (deterministic).

Scenario, 14 Oct 2026 09:00-12:00 UTC: an OrderService web farm (WEB01, WEB02)
behind a gateway, a worker and a legacy log4net billing service. WEB02 leaks
memory, its CPU climbs, payment calls start timing out and the process crashes
at 10:41:24; it restarts 46 s later under a new PID.

    python3 samples/generate.py
"""

import csv
import json
import os
import random
from datetime import datetime, timedelta, timezone

ROOT = os.path.dirname(os.path.abspath(__file__))
START = datetime(2026, 10, 14, 9, 0, 0, tzinfo=timezone.utc)
END = datetime(2026, 10, 14, 12, 0, 0, tzinfo=timezone.utc)
CRASH = datetime(2026, 10, 14, 10, 41, 24, 50000, tzinfo=timezone.utc)
RESTART = datetime(2026, 10, 14, 10, 42, 10, 412000, tzinfo=timezone.utc)
INCIDENT = datetime(2026, 10, 14, 10, 30, tzinfo=timezone.utc)

rnd = random.Random(42)

TIMEOUT_STACK = (
    "System.TimeoutException: The operation has timed out.\n"
    "   at Microsoft.Data.SqlClient.SqlInternalConnection.OnError(SqlException exception, Boolean breakConnection)\n"
    "   at Microsoft.Data.SqlClient.TdsParser.ThrowExceptionAndWarning(TdsParserStateObject stateObj)\n"
    "   at Microsoft.EntityFrameworkCore.Query.Internal.SingleQueryingEnumerable`1.AsyncEnumerator.MoveNextAsync()\n"
    "   at OrderService.Payments.CaptureHandler.Handle(CaptureCommand cmd, CancellationToken ct) in /src/Payments/CaptureHandler.cs:line 88"
)
HTTP_STACK = (
    "System.Net.Http.HttpRequestException: Response status code does not indicate success: 504 (Gateway Timeout).\n"
    "   at System.Net.Http.HttpResponseMessage.EnsureSuccessStatusCode()\n"
    "   at OrderService.Payments.GatewayClient.CaptureAsync(Guid orderId) in /src/Payments/GatewayClient.cs:line 41"
)


def path(*parts):
    p = os.path.join(ROOT, *parts)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    return p


def times(start, end, mean_gap):
    t = start
    while True:
        t += timedelta(seconds=rnd.expovariate(1 / mean_gap))
        if t >= end:
            return
        yield t


def stress(t):
    """0 before the incident, rising to 1 at the crash."""
    if t < INCIDENT:
        return 0.0
    return min(1.0, (t - INCIDENT) / (CRASH - INCIDENT))


def iso(t):
    return t.isoformat(timespec="microseconds").replace("+00:00", "Z")


# --------------------------------------------------------------------------
# Serilog CLEF (Serilog.Formatting.Compact) for the web servers.

def web_events(machine, pid, start, end, leaky, crash=False, restarted=False):
    ev = []
    base = {"Application": "OrderService", "MachineName": machine, "ProcessId": pid, "EnvironmentName": "Production"}

    def add(t, mt, level=None, x=None, **props):
        e = {"@t": iso(t), "@mt": mt}
        if level:
            e["@l"] = level
        if x:
            e["@x"] = x
        e.update(props)
        e.update(base)
        ev.append(e)

    add(start, "Application starting — {Application} {Version} ({Runtime})", Version="3.14.2", Runtime="net9.0",
        SourceContext="Program", **({"Reason": "after unexpected exit"} if restarted else {}))
    add(start + timedelta(milliseconds=640), "Now listening on: {address}", address="http://0.0.0.0:8080",
        SourceContext="Microsoft.Hosting.Lifetime")
    add(start + timedelta(milliseconds=702), "Application started. Press Ctrl+C to shut down.",
        SourceContext="Microsoft.Hosting.Lifetime")

    order = 88000 if machine == "WEB01" else 88001
    for t in times(start + timedelta(seconds=1), end, 3.2):
        s = stress(t) if leaky else stress(t) * 0.35
        order += 2
        elapsed = round(rnd.uniform(20, 160) + s * rnd.uniform(800, 5200), 4)
        cid = "%08x-%04x-4%03x-%04x-%012x" % (rnd.getrandbits(32), rnd.getrandbits(16), rnd.getrandbits(12), rnd.getrandbits(16), rnd.getrandbits(48))
        path_ = rnd.choice(["/api/orders", "/api/orders/{id}", "/api/orders/{id}/capture", "/api/cart"]).replace("{id}", str(order))
        status = 200
        r = rnd.random()
        if path_.endswith("/capture") and r < s * 0.6:
            status = 500
            add(t, "Payment gateway call exceeded {TimeoutMs} ms (attempt {Attempt}/{MaxAttempts}) for order {OrderId}", "Warning",
                TimeoutMs=5000, Attempt=3, MaxAttempts=3, OrderId=order, SourceContext="OrderService.Payments", CorrelationId=cid, RequestPath=path_)
            add(t + timedelta(milliseconds=210), "Failed to capture payment for order {OrderId}: request timed out", "Error", HTTP_STACK,
                OrderId=order, SourceContext="OrderService.Payments", CorrelationId=cid, RequestPath=path_)
        elif r < 0.002 + s * 0.05:
            status = 500
            add(t, "An exception occurred while iterating over the results of a query for context type '{contextType}'.", "Error", TIMEOUT_STACK,
                contextType="OrderService.Data.OrdersDbContext", SourceContext="Microsoft.EntityFrameworkCore.Query", CorrelationId=cid,
                RequestPath=path_, EventId={"Id": 10100, "Name": "Microsoft.EntityFrameworkCore.Query.QueryIterationFailed"})
        elif r < 0.02 + s * 0.08:
            add(t, "Slow request {RequestPath} took {ElapsedMs} ms", "Warning", ElapsedMs=elapsed, SourceContext="OrderService.Diagnostics",
                CorrelationId=cid, RequestPath=path_)
        elif r < 0.03:
            add(t, "Cache miss for {CacheKey}", "Debug", CacheKey=f"order:{order}", SourceContext="OrderService.Caching")
        add(t + timedelta(milliseconds=elapsed), "HTTP {RequestMethod} {RequestPath} responded {StatusCode} in {Elapsed:0.0000} ms",
            RequestMethod="POST" if path_.endswith("capture") else "GET", RequestPath=path_, StatusCode=status, Elapsed=elapsed,
            SourceContext="Serilog.AspNetCore.RequestLoggingMiddleware", CorrelationId=cid)

    # Process monitor every 30 s, structured properties (no regex needed).
    t = start + timedelta(seconds=5)
    while t < end:
        s = stress(t)
        minutes = (t - start).total_seconds() / 60
        cpu = round(min(99.5, 38 + rnd.uniform(-6, 9) + (s * 58 if leaky else s * 18)), 1)
        ws = round(520 + (minutes * 18 if leaky else 40 + rnd.uniform(-10, 10)) + rnd.uniform(0, 15), 1)
        add(t, "Process stats CPU {CpuPercent}% · WorkingSet {WorkingSetMb} MB · Threads {Threads}", "Information",
            CpuPercent=cpu, WorkingSetMb=ws, Threads=int(60 + cpu * 1.6), SourceContext="Diagnostics.ProcessMonitor")
        t += timedelta(seconds=30)

    if crash:
        add(end - timedelta(milliseconds=1), "Host terminated unexpectedly", "Fatal", TIMEOUT_STACK, SourceContext="Program")
    elif end >= END:
        add(end, "Application is shutting down...", SourceContext="Microsoft.Hosting.Lifetime")
    return ev


def write_clef(p, events):
    events.sort(key=lambda e: e["@t"])
    with open(p, "w", encoding="utf-8", newline="\n") as f:
        for e in events:
            f.write(json.dumps(e, ensure_ascii=False) + "\n")


write_clef(path("serilog-clef", "web01", "orders-20261014.clef"), web_events("WEB01", 3312, START, END, leaky=False))
write_clef(path("serilog-clef", "web02", "orders-20261014.clef"),
           web_events("WEB02", 4120, START, CRASH, leaky=True, crash=True)
           + web_events("WEB02", 5532, RESTART, END, leaky=False, restarted=True))

# --------------------------------------------------------------------------
# Serilog JsonFormatter (Serilog.Formatting.Json) for the gateway.

with open(path("serilog-json", "gateway", "gateway-20261014.json"), "w", encoding="utf-8", newline="\n") as f:
    def gw(t, level, mt, rendered, **props):
        props.update({"MachineName": "GATEWAY", "ProcessId": 1880, "Application": "Gateway", "EnvironmentName": "Production"})
        f.write(json.dumps({"Timestamp": t.isoformat(timespec="microseconds"), "Level": level, "MessageTemplate": mt,
                            "RenderedMessage": rendered, "Properties": props}) + "\n")

    gw(START, "Information", "Application starting — {Application}", "Application starting — \"Gateway\"", SourceContext="Program")
    for t in times(START + timedelta(seconds=1), END, 4.4):
        s = stress(t) if t < CRASH else (0.6 if t < RESTART + timedelta(minutes=2) else 0)
        upstream = rnd.choice(["web01:8080", "web02:8080"])
        if upstream == "web02:8080" and rnd.random() < s * 0.7:
            ms = rnd.randint(29000, 30100)
            gw(t, "Warning", "Upstream {Upstream} responded {StatusCode} after {ElapsedMs} ms",
               f"Upstream \"{upstream}\" responded 504 after {ms} ms", Upstream=upstream, StatusCode=504, ElapsedMs=ms,
               SourceContext="Yarp.ReverseProxy.Forwarder.HttpForwarder")
        else:
            ms = rnd.randint(15, 220)
            gw(t, "Information", "Proxying to {Upstream} {Path} completed in {ElapsedMs} ms",
               f"Proxying to \"{upstream}\" \"/api/orders\" completed in {ms} ms", Upstream=upstream, Path="/api/orders", ElapsedMs=ms,
               SourceContext="Yarp.ReverseProxy.Forwarder.HttpForwarder")
    gw(CRASH + timedelta(seconds=3), "Error", "Health probe for {Destination} failed: {Reason}",
       "Health probe for \"web02\" failed: \"Connection refused\"", Destination="web02", Reason="Connection refused",
       SourceContext="Yarp.ReverseProxy.Health.ActiveHealthCheckMonitor")
    gw(RESTART + timedelta(seconds=112), "Information", "Health probe for {Destination} recovered after {Failures} failures",
       "Health probe for \"web02\" recovered after 2 failures", Destination="web02", Failures=2,
       SourceContext="Yarp.ReverseProxy.Health.ActiveHealthCheckMonitor")

# --------------------------------------------------------------------------
# Serilog file sink, default text template plus {Properties:j}, for the worker.
# Its process stats are plain text: use an [[extract]] rule to chart them.

with open(path("serilog-text", "worker01", "worker-20261014.txt"), "w", encoding="utf-8", newline="\n") as f:
    def wk(t, lvl, msg, props, exc=None):
        stamp = t.strftime("%Y-%m-%d %H:%M:%S.") + f"{t.microsecond // 1000:03d} +00:00"
        f.write(f"{stamp} [{lvl}] {msg} {json.dumps(props)}\n")
        if exc:
            f.write(exc + "\n")

    base = {"MachineName": "WORKER01", "ProcessId": 2208, "Application": "OrderWorker"}
    batch = {"MachineName": "WORKER01", "ProcessId": 2210, "Application": "OrderWorker"}
    lines = [(START, "INF", "Application started — OrderWorker 3.14.2", dict(base, SourceContext="Program"), None)]
    for t in times(START + timedelta(seconds=2), END, 4.0):
        s = stress(t)
        backlog = int(80 + s * 1300 + rnd.uniform(0, 40))
        if rnd.random() < 0.05 + s * 0.3:
            lines.append((t, "WRN", f"Outbox dispatch timed out; backlog {backlog:,} messages", dict(base, SourceContext="Outbox.Dispatcher", Backlog=backlog), None))
        elif rnd.random() < 0.004 + s * 0.02:
            lines.append((t, "ERR", "Message handler failed for OrderCaptured", dict(base, SourceContext="Outbox.Dispatcher"), HTTP_STACK))
        else:
            lines.append((t, "INF", f"Dispatched {rnd.randint(1, 40)} outbox messages", dict(base, SourceContext="Outbox.Dispatcher"), None))
    t = START + timedelta(seconds=10)
    while t < END:
        s = stress(t)
        cpu = round(26 + rnd.uniform(-4, 8) + s * 22, 1)
        ws = rnd.choice([round(600 + rnd.uniform(0, 30), 0), round(0.6 + rnd.uniform(0, 0.03), 2)])
        unit = "MB" if ws > 10 else "GB"
        lines.append((t, "INF", f"Process stats: CPU {cpu}% · WorkingSet {ws:g} {unit} · Threads {int(40 + cpu)}", dict(base, SourceContext="Diagnostics.ProcessMonitor"), None))
        t += timedelta(seconds=30)
    rs, re_ = START + timedelta(minutes=30), START + timedelta(minutes=75)
    lines.append((rs, "INF", "Service starting: nightly reindex job", dict(batch, SourceContext="Reindex.Job"), None))
    for t in times(rs + timedelta(seconds=1), re_, 2.0):
        lines.append((t, "INF", f"Reindexed {rnd.randint(500, 2000)} orders", dict(batch, SourceContext="Reindex.Job"), None))
    lines.append((re_, "INF", "Service stopped: nightly reindex job completed", dict(batch, SourceContext="Reindex.Job"), None))
    for item in sorted(lines, key=lambda x: x[0]):
        wk(*item)

# --------------------------------------------------------------------------
# log4net PatternLayout "%date [%thread] %-5level %logger - %message%newline",
# rolled file name without a .log extension.

with open(path("log4net", "legacy-billing", "billing.log.2026-10-14"), "w", encoding="utf-8", newline="\r\n") as f:
    def l4(t, thread, level, logger, msg, exc=None):
        f.write(f"{t.strftime('%Y-%m-%d %H:%M:%S')},{t.microsecond // 1000:03d} [{thread}] {level:<5} {logger} - {msg}\n")
        if exc:
            f.write(exc + "\n")

    l4(START, 1, "INFO", "Billing.Service", "Billing service starting (v2.8.1, CLR 4.0.30319)")
    for t in times(START + timedelta(seconds=3), END, 6.0):
        s = stress(t)
        thread = rnd.randint(5, 24)
        if rnd.random() < s * 0.25:
            l4(t, thread, "WARN", "Billing.Invoices.InvoiceWriter", f"Waiting for payment confirmation of order {rnd.randint(88000, 89000)} ({rnd.randint(5, 30)} s)")
        elif rnd.random() < 0.003 + s * 0.03:
            l4(t, thread, "ERROR", "Billing.Invoices.InvoiceWriter", "Could not post invoice to ERP",
               "System.ServiceModel.CommunicationException: The underlying connection was closed.\n"
               "   at Billing.Erp.ErpClient.Post(Invoice invoice)\n   at Billing.Invoices.InvoiceWriter.Flush()")
        elif rnd.random() < 0.1:
            l4(t, thread, "DEBUG", "Billing.Cache", f"Refreshed tariff cache ({rnd.randint(100, 900)} entries)")
        else:
            l4(t, thread, "INFO", "Billing.Invoices.InvoiceWriter", f"Invoice INV-{rnd.randint(100000, 999999)} written")
    l4(END, 1, "INFO", "Billing.Service", "Billing service stopping")

# --------------------------------------------------------------------------
# Performance counter export: semicolon separated, day-first dates.

with open(path("csv", "perfmon-export.csv"), "w", encoding="utf-8", newline="") as f:
    w = csv.writer(f, delimiter=";")
    w.writerow(["Timestamp", "Machine", "Counter", "Value", "Message"])
    t = START
    while t <= END:
        for machine in ["WEB01", "WEB02", "WORKER01"]:
            s = stress(t) if machine == "WEB02" and t < CRASH else 0.0
            w.writerow([t.strftime("%d/%m/%Y %H:%M:%S"), machine, "Processor(_Total)\\% Processor Time", round(30 + rnd.uniform(0, 12) + s * 55, 2), ""])
            w.writerow([t.strftime("%d/%m/%Y %H:%M:%S"), machine, "Memory\\Available MBytes", round(6100 - s * 4200 + rnd.uniform(-80, 80)), ""])
        t += timedelta(minutes=1)

# --------------------------------------------------------------------------
# Files no importer handles yet (future plugins): UTF-16 registry export and
# an msinfo32 export.

with open(path("unsupported", "registry-export.reg"), "w", encoding="utf-16") as f:
    f.write("Windows Registry Editor Version 5.00\r\n\r\n[HKEY_LOCAL_MACHINE\\SOFTWARE\\Contoso\\OrderService]\r\n\"Version\"=\"3.14.2\"\r\n\"PaymentTimeoutMs\"=dword:00001388\r\n")
with open(path("unsupported", "msinfo32.nfo"), "w", encoding="utf-8") as f:
    f.write('<?xml version="1.0"?>\n<MsInfo><Metadata><Version>8.0</Version></Metadata><Category name="System Summary">'
            '<Data><Item><![CDATA[OS Name]]></Item><Value><![CDATA[Microsoft Windows Server 2022 Datacenter]]></Value></Data></Category></MsInfo>\n')

# --------------------------------------------------------------------------
# imt-plugin-formats: log4j/NLog XML events, SQL Server ERRORLOG, Docker
# json-file and Kubernetes CRI logs. Own random generator, so the files above
# do not change when this section does.

frnd = random.Random(2026)


def nano(t):
    """RFC 3339 with nanoseconds, as Docker and containerd write it."""
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + f"{t.microsecond:06d}{frnd.randint(0, 999):03d}Z"


def hexid(n):
    return "".join(frnd.choice("0123456789abcdef") for _ in range(n))


# log4net XmlLayoutSchemaLog4j (no root element: the appender appends events).
def l4x_event(t, level, logger, thread, msg, exc=None, nlog_seq=None):
    ms = int(t.timestamp() * 1000)
    ns = ' xmlns:log4j="http://jakarta.apache.org/log4j/" xmlns:nlog="http://nlog-project.org"' if nlog_seq is not None else ""
    out = [f'<log4j:event logger="{logger}" timestamp="{ms}" level="{level}" thread="{thread}"{ns}>',
           f"<log4j:message><![CDATA[{msg}]]></log4j:message>"]
    if exc:
        out.append(f"<log4j:throwable><![CDATA[{exc}]]></log4j:throwable>")
    if nlog_seq is not None:
        out.append(f"<nlog:eventSequenceNumber>{nlog_seq}</nlog:eventSequenceNumber>")
        out.append('<nlog:locationInfo assembly="OrderScheduler, Version=1.4.0.0, Culture=neutral, PublicKeyToken=null" />')
        out.append('<log4j:properties><log4j:data name="log4japp" value="OrderScheduler" /><log4j:data name="log4jmachinename" value="SCHED01" /></log4j:properties>')
    else:
        out.append(f'<log4j:locationInfo class="{logger}" method="Run" file="" line="0" />')
        out.append('<log4j:properties><log4j:data name="log4jmachinename" value="BILL01" /><log4j:data name="log4japp" value="Billing.Api.exe" /></log4j:properties>')
    out.append("</log4j:event>")
    return "".join(out) if nlog_seq is not None else "\n".join(out)


ERP_STACK = ("System.ServiceModel.CommunicationException: The underlying connection was closed.\n"
             "   at Billing.Erp.ErpClient.Post(Invoice invoice)\n   at Billing.Api.InvoiceController.Post(InvoiceDto dto)")

with open(path("log4j-xml", "billing-api", "billing-api.xml"), "w", encoding="utf-8", newline="\r\n") as f:
    f.write(l4x_event(START, "INFO", "Billing.Api.Program", 1, "Billing API starting (v2.8.1)") + "\n")
    for t in times(START + timedelta(seconds=2), END, 40.0):
        s = stress(t)
        r = frnd.random()
        if r < 0.004 + s * 0.08:
            f.write(l4x_event(t, "ERROR", "Billing.Api.InvoiceController", frnd.randint(5, 30), "Posting invoice to ERP failed", ERP_STACK) + "\n")
        elif r < 0.05 + s * 0.3:
            f.write(l4x_event(t, "WARN", "Billing.Api.InvoiceController", frnd.randint(5, 30),
                              f"ERP call took {frnd.randint(2000, 9000)} ms for order {frnd.randint(88000, 89000)}") + "\n")
        else:
            f.write(l4x_event(t, "INFO", "Billing.Api.InvoiceController", frnd.randint(5, 30),
                              f"Invoice INV-{frnd.randint(100000, 999999)} <posted> & acknowledged") + "\n")

# NLog Log4JXmlEventLayout, one event per line.
with open(path("log4j-xml", "scheduler", "scheduler-20261014.nlog.xml"), "w", encoding="utf-8", newline="\n") as f:
    seq = 0
    t = START
    while t < END:
        seq += 1
        if stress(t) > 0.5 and frnd.random() < 0.5:
            f.write(l4x_event(t, "WARN", "OrderScheduler.Jobs.Reconcile", 4, "Reconcile job overran its slot", nlog_seq=seq) + "\n")
        else:
            f.write(l4x_event(t, "DEBUG", "OrderScheduler.Jobs.Reconcile", 4, f"Reconciled {frnd.randint(10, 400)} orders", nlog_seq=seq) + "\n")
        t += timedelta(minutes=2)

# SQL Server ERRORLOG: UTF-16 LE with BOM on Windows (local time UTC+2), UTF-8 on Linux.
LOCAL = timedelta(hours=2)


def sql_stamp(t, offset):
    lt = t + offset
    return lt.strftime("%Y-%m-%d %H:%M:%S.") + f"{lt.microsecond // 10000:02d}"


def errorlog(offset, lines):
    out = []
    for t, src, msg in sorted(lines, key=lambda x: x[0]):
        out.append(f"{sql_stamp(t, offset)} {src:<11} {msg}")
    return "\r\n".join(out) + "\r\n"


sql = [
    (START, "Server", "Microsoft SQL Server 2022 (RTM-CU14) (KB5038325) - 16.0.4135.4 (X64) \r\n"
     "\tJul 10 2024 14:09:09 \r\n\tCopyright (C) 2022 Microsoft Corporation\r\n"
     "\tStandard Edition (64-bit) on Windows Server 2022 Datacenter 10.0 <X64> (Build 20348: ) (Hypervisor)\r\n"),
    (START, "Server", "UTC adjustment: 2:00"),
    (START, "Server", "(c) Microsoft Corporation."),
    (START, "Server", "Server process ID is 2716."),
    (START, "Server", "Authentication mode is MIXED."),
    (START, "Server", "Logging SQL Server messages in file 'D:\\MSSQL16.MSSQLSERVER\\MSSQL\\Log\\ERRORLOG'."),
    (START + timedelta(milliseconds=450), "spid10s", "Starting up database 'master'."),
    (START + timedelta(milliseconds=980), "spid10s", "Recovery is complete. This is an informational message only. No user action is required."),
    (START + timedelta(seconds=2), "spid32s", "Starting up database 'Orders'."),
]
t = START + timedelta(minutes=15)
while t < END:
    sql.append((t, "Backup", f"Log was backed up. Database: Orders, creation date(time): 2026/03/02(10:11:12), first LSN: {frnd.randint(100, 999)}:{frnd.randint(1000, 9999)}:1, "
                "number of dump devices: 1, device information: (FILE=1, TYPE=DISK: {'L:\\Backup\\Orders_log.trn'}). This is an informational message only. No user action is required."))
    t += timedelta(minutes=15)
for t in times(START + timedelta(minutes=3), END, 600.0):
    t2 = t + timedelta(microseconds=10000)
    sql.append((t, "Logon", "Error: 18456, Severity: 14, State: 8."))
    sql.append((t2, "Logon", "Login failed for user 'reporting'. Reason: Password did not match that for the login provided. [CLIENT: 10.20.1.15]"))
for t in times(INCIDENT, CRASH, 50.0):
    spid = f"spid{frnd.randint(60, 140)}"
    sql.append((t, spid, "Error: 1205, Severity: 13, State: 51."))
    sql.append((t + timedelta(microseconds=10000), spid, f"Transaction (Process ID {spid[4:]}) was deadlocked on lock resources with another process and has been chosen as the deadlock victim. Rerun the transaction."))
for t in times(INCIDENT, CRASH, 180.0):
    sql.append((t, f"spid{frnd.randint(20, 40)}s", f"SQL Server has encountered {frnd.randint(2, 40)} occurrence(s) of I/O requests taking longer than 15 seconds to complete on file "
                "[L:\\Data\\Orders_log.ldf] in database id 7. The OS file handle is 0x0000000000000F2C."))
LOG_FULL = CRASH - timedelta(seconds=40)
sql.append((LOG_FULL, "spid88", "Error: 9002, Severity: 17, State: 2."))
sql.append((LOG_FULL + timedelta(microseconds=10000), "spid88", "The transaction log for database 'Orders' is full due to 'LOG_BACKUP'."))
with open(path("sqlserver", "SQL01", "ERRORLOG"), "wb") as f:
    f.write(b"\xff\xfe" + errorlog(LOCAL, sql).encode("utf-16-le"))

linux = [
    (START, "Server", "Microsoft SQL Server 2022 (RTM-CU14) (KB5038325) - 16.0.4135.4 (X64) \r\n\tJul 10 2024 14:09:09 \r\n\tCopyright (C) 2022 Microsoft Corporation\r\n"
     "\tDeveloper Edition (64-bit) on Linux (Ubuntu 22.04.4 LTS) <X64>"),
    (START, "Server", "UTC adjustment: 0:00"),
    (START, "Server", "Server process ID is 412."),
    (START + timedelta(seconds=1), "spid9s", "Starting up database 'Reporting'."),
    (CRASH - timedelta(minutes=5), "spid71", "Error: 823, Severity: 24, State: 2."),
    (CRASH - timedelta(minutes=5) + timedelta(microseconds=10000), "spid71", "The operating system returned error 21(The device is not ready.) to SQL Server during a read at offset "
     "0x0000000a4e2000 in file '/var/opt/mssql/data/Reporting.mdf'."),
    (CRASH - timedelta(minutes=4), "spid71", "Error: 17053, Severity: 16, State: 1."),
]
with open(path("sqlserver", "sqlrpt01", "errorlog"), "w", encoding="utf-8", newline="") as f:
    f.write(errorlog(timedelta(0), linux))

# Docker json-file: the payments adapter (Go, logfmt on stdout, panics on stderr).
CID = hexid(64)
with open(path("docker", "containers", CID, f"{CID}-json.log"), "w", encoding="utf-8", newline="\n") as f:
    def dj(t, text, stream="stdout"):
        f.write(json.dumps({"log": text, "stream": stream, "attrs": {"tag": "payments-adapter"}, "time": nano(t)}) + "\n")

    dj(START, "time=2026-10-14T09:00:00Z level=info msg=\"payments adapter listening\" addr=:9090\n")
    for t in times(START + timedelta(seconds=1), END, 6.0):
        s = stress(t)
        ms = int(frnd.uniform(40, 180) + s * frnd.uniform(800, 5000))
        if frnd.random() < s * 0.4:
            dj(t, f"level=error msg=\"gateway capture failed\" order={frnd.randint(88000, 89000)} err=\"context deadline exceeded\" ms={ms}\n")
        elif ms > 1000:
            dj(t, f"level=warn msg=\"slow capture\" order={frnd.randint(88000, 89000)} ms={ms}\n")
        else:
            dj(t, f"level=info msg=\"capture ok\" order={frnd.randint(88000, 89000)} ms={ms}\n")
    # A response dump longer than Docker's 16 KiB line buffer: two partial chunks.
    dump = "level=debug msg=\"gateway response\" body=" + "".join(frnd.choice("abcdef0123456789") for _ in range(20000))
    t = INCIDENT + timedelta(seconds=7)
    dj(t, dump[:16384])
    dj(t + timedelta(microseconds=3), dump[16384:] + "\n")
    dj(INCIDENT + timedelta(minutes=2), "WARNING: connection pool exhausted, waiting\n", "stderr")
    dj(INCIDENT + timedelta(minutes=2, seconds=1), "GC forced\n", "stderr")

# Sort the Docker file by time (the dump was appended out of order).
p = path("docker", "containers", CID, f"{CID}-json.log")
with open(p, encoding="utf-8") as f:
    rows = f.read().splitlines()
rows.sort(key=lambda l: json.loads(l)["time"])
with open(p, "w", encoding="utf-8", newline="\n") as f:
    f.write("\n".join(rows) + "\n")

# Kubernetes CRI: the order-events consumer pod, JSON (zap) on stdout, klog and a
# panic on stderr; it crashes with WEB02 and restarts (0.log, then 1.log).
POD_UID = f"{hexid(8)}-{hexid(4)}-{hexid(4)}-{hexid(4)}-{hexid(12)}"
POD_DIR = ("kubernetes", "pods", f"shop_order-events-7c9d8b5f4-x2k4p_{POD_UID}", "consumer")


def cri_file(name, start, end, crash):
    rows = []

    def cri(t, text, stream="stdout", tag="F"):
        rows.append((t, f"{nano(t)} {stream} {tag} {text}"))

    cri(start, json.dumps({"level": "info", "ts": iso(start), "msg": "consumer started", "group": "order-events"}))
    cri(start + timedelta(milliseconds=12), "I1014 " + start.strftime("%H:%M:%S.%f") + "       1 leaderelection.go:250] attempting to acquire leader lease shop/order-events", "stderr")
    for t in times(start + timedelta(seconds=1), end, 5.0):
        s = stress(t)
        if frnd.random() < s * 0.3:
            cri(t, json.dumps({"level": "error", "ts": iso(t), "msg": "handler failed", "topic": "orders.captured", "error": "upstream timeout"}))
        elif frnd.random() < 0.02 + s * 0.2:
            cri(t, "W1014 " + t.strftime("%H:%M:%S.%f") + "       1 reflector.go:424] watch of *v1.ConfigMap ended with: too old resource version", "stderr")
        else:
            cri(t, json.dumps({"level": "info", "ts": iso(t), "msg": "consumed batch", "count": frnd.randint(1, 50), "lagMs": int(20 + s * 4000)}))
    if crash:
        long = json.dumps({"level": "debug", "ts": iso(INCIDENT), "msg": "batch snapshot", "payload": "".join(frnd.choice("xyz") for _ in range(20000))})
        t = INCIDENT + timedelta(seconds=3)
        rows.append((t, f"{nano(t)} stdout P {long[:16384]}"))
        t2 = t + timedelta(microseconds=1)
        rows.append((t2, f"{nano(t2)} stdout F {long[16384:]}"))
        for i, text in enumerate(["panic: runtime error: invalid memory address or nil pointer dereference",
                                  "[signal SIGSEGV: segmentation violation code=0x1 addr=0x0 pc=0x6f2a1c]", "",
                                  "goroutine 41 [running]:"]):
            cri(end + timedelta(microseconds=i), text, "stderr")
    rows.sort(key=lambda r: r[0])
    with open(path(*POD_DIR, name), "w", encoding="utf-8", newline="\n") as f:
        for _, line in rows:
            f.write(line + "\n")


cri_file("0.log", START, CRASH, crash=True)
cri_file("1.log", RESTART + timedelta(seconds=3), END, crash=False)

print("samples written to", ROOT)
