#!/usr/bin/env python3
"""Generates the sample logs in this folder (deterministic).

Scenario, 14 Oct 2026 09:00-12:00 UTC: an OrderService web farm (WEB01, WEB02)
behind a gateway, a worker and a legacy log4net billing service. WEB02 leaks
memory, its CPU climbs, payment calls start timing out and the process crashes
at 10:41:24; it restarts 46 s later under a new PID.

    python3 samples/generate.py
"""

import csv
import hashlib
import io
import json
import os
import random
import struct
import zipfile
import zlib
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
# Windows configuration exports read by the registry and Windows plugins (the
# built-in importers do not handle them): a UTF-16 registry export and an
# msinfo32 report. `newline=""` keeps the bytes the same on every platform.

with open(path("windows", "registry-export.reg"), "w", encoding="utf-16", newline="") as f:
    f.write("Windows Registry Editor Version 5.00\r\n\r\n[HKEY_LOCAL_MACHINE\\SOFTWARE\\Contoso\\OrderService]\r\n\"Version\"=\"3.14.2\"\r\n\"PaymentTimeoutMs\"=dword:00001388\r\n")
with open(path("windows", "msinfo32.nfo"), "w", encoding="utf-8", newline="\n") as f:
    f.write('<?xml version="1.0"?>\n<MsInfo><Metadata><Version>8.0</Version></Metadata><Category name="System Summary">'
            '<Data><Item><![CDATA[OS Name]]></Item><Value><![CDATA[Microsoft Windows Server 2022 Datacenter]]></Value></Data></Category></MsInfo>\n')

# --------------------------------------------------------------------------
# debrief-plugin-formats: log4j/NLog XML events, SQL Server ERRORLOG, Docker
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


# ==========================================================================
# Samples for every view and input format (see samples/README.md). Each
# section has its own random generator, so changing one leaves every other
# file (and everything above) byte-identical. Binary and compressed files are
# written by hand (stored zip/gzip, no deflate) so their bytes never depend on
# the local zlib build.

EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)
FILETIME_EPOCH = datetime(1601, 1, 1, tzinfo=timezone.utc)


def ms_of(t):
    return (t - EPOCH) // timedelta(milliseconds=1)


def ns_of(t):
    return (t - EPOCH) // timedelta(microseconds=1) * 1000


def iso_ms(t):
    return t.strftime("%Y-%m-%dT%H:%M:%S.") + f"{t.microsecond // 1000:03d}Z"


def rtimes(r, start, end, mean_gap):
    """`times` with its own generator."""
    t = start
    while True:
        t += timedelta(seconds=r.expovariate(1 / mean_gap))
        if t >= end:
            return
        yield t


def write_bytes(data, *parts):
    with open(path(*parts), "wb") as f:
        f.write(data)


def write_text(text, *parts, newline="\n", encoding="utf-8"):
    write_bytes(text.replace("\n", newline).encode(encoding), *parts)


def guid(r):
    return "%08x-%04x-%04x-%04x-%012x" % (r.getrandbits(32), r.getrandbits(16), r.getrandbits(16), r.getrandbits(16), r.getrandbits(48))


# --------------------------------------------------------------------------
# Windows event logs (.evtx). A small writer for the documented format
# (MS-EVEN6): file header, 64 KiB chunks with CRC32 checksums, and records
# whose BinXML spells the event out element by element (no templates), with
# typed values (UInt16 EventID, UInt8 Level, FILETIME SystemTime, ...).

def _crc(b):
    return zlib.crc32(bytes(b)) & 0xFFFFFFFF


def V_STR(s):
    u = s.encode("utf-16-le")
    return (0x01, struct.pack("<H", len(u) // 2) + u)


def V_U8(x):
    return (0x04, struct.pack("<B", x))


def V_U16(x):
    return (0x06, struct.pack("<H", x))


def V_U32(x):
    return (0x08, struct.pack("<I", x))


def V_U64(x):
    return (0x0A, struct.pack("<Q", x))


def V_HEX64(x):
    return (0x15, struct.pack("<Q", x))


def filetime(t):
    return (t - FILETIME_EPOCH) // timedelta(microseconds=1) * 10


def V_TIME(t):
    return (0x11, struct.pack("<Q", filetime(t)))


class BinXml:
    """BinXML of one record; `base` is the chunk offset of its first byte (names are inline)."""

    def __init__(self, base):
        self.b = bytearray()
        self.base = base

    def name(self, n):
        units = n.encode("utf-16-le")
        h = 0
        for i in range(0, len(units), 2):
            h = (h * 65599 + int.from_bytes(units[i:i + 2], "little")) & 0xFFFFFFFF
        self.b += struct.pack("<I", self.base + len(self.b) + 4)
        self.b += struct.pack("<IHH", 0, h & 0xFFFF, len(units) // 2) + units + b"\0\0"

    def value(self, v):
        self.b += bytes([0x05, v[0]]) + v[1]

    def element(self, el):
        name, attrs, content = el
        self.b.append(0x41 if attrs else 0x01)
        size_at = len(self.b)
        self.b += b"\0\0\0\0"
        self.name(name)
        if attrs:
            list_at = len(self.b)
            self.b += b"\0\0\0\0"
            for i, (an, av) in enumerate(attrs):
                self.b.append(0x46 if i < len(attrs) - 1 else 0x06)
                self.name(an)
                self.value(av)
            self.b[list_at:list_at + 4] = struct.pack("<I", len(self.b) - list_at - 4)
        if content is None:
            self.b.append(0x03)
        else:
            self.b.append(0x02)
            if isinstance(content, list):
                for child in content:
                    self.element(child)
            else:
                self.value(content)
            self.b.append(0x04)
        self.b[size_at:size_at + 4] = struct.pack("<I", len(self.b) - size_at - 4)


def event_xml(e, rid):
    prov = [("Name", V_STR(e["provider"]))]
    if e.get("guid"):
        prov.append(("Guid", V_STR("{" + e["guid"] + "}")))
    if e.get("source"):
        prov.append(("EventSourceName", V_STR(e["source"])))
    eid_attrs = [("Qualifiers", V_U16(e["qualifiers"]))] if e.get("qualifiers") is not None else []
    system = [
        ("Provider", prov, None),
        ("EventID", eid_attrs, V_U16(e["id"])),
        ("Version", [], V_U8(e.get("version", 0))),
        ("Level", [], V_U8(e["level"])),
        ("Task", [], V_U16(e.get("task", 0))),
        ("Opcode", [], V_U8(e.get("opcode", 0))),
        ("Keywords", [], V_HEX64(e.get("keywords", 0x80000000000000))),
        ("TimeCreated", [("SystemTime", V_TIME(e["t"]))], None),
        ("EventRecordID", [], V_U64(rid)),
        ("Correlation", [], None),
        ("Execution", [("ProcessID", V_U32(e.get("pid", 0))), ("ThreadID", V_U32(e.get("tid", 0)))], None),
        ("Channel", [], V_STR(e["channel"])),
        ("Computer", [], V_STR(e["computer"])),
        ("Security", [("UserID", V_STR(e["sid"]))] if e.get("sid") else [], None),
    ]
    data = [("Data", [("Name", V_STR(n))] if n else [], V_STR(v) if v else None) for n, v in e.get("data", [])]
    body = [("System", [], system)]
    if data:
        body.append(("EventData", [], data))
    return ("Event", [("xmlns", V_STR("http://schemas.microsoft.com/win/2004/08/events/event"))], body)


def evtx(events, first_id):
    """The bytes of an .evtx file holding `events` (sorted by time) with record ids from `first_id`."""
    chunks = []
    i = 0
    rid = first_id
    events = sorted(events, key=lambda e: e["t"])
    while i < len(events) or not chunks:
        data = bytearray(512)
        first, last, last_off = rid, rid - 1, 0
        while i < len(events):
            off = len(data)
            x = BinXml(off + 24)
            x.b += b"\x0f\x01\x01\x00"
            x.element(event_xml(events[i], rid))
            x.b.append(0x00)
            size = 24 + len(x.b) + 4
            if off + size > 65536:
                break
            data += b"\x2a\x2a\x00\x00" + struct.pack("<IQQ", size, rid, filetime(events[i]["t"])) + x.b + struct.pack("<I", size)
            last, last_off = rid, off
            rid += 1
            i += 1
        free = len(data)
        data += bytes(65536 - len(data))
        h = b"ElfChnk\0" + struct.pack("<QQQQIIII", first, last, first, last, 128, last_off, free, _crc(data[512:free]))
        data[0:len(h)] = h
        data[124:128] = struct.pack("<I", _crc(data[:120] + data[128:512]))
        chunks.append(bytes(data))
    head = bytearray(4096)
    head[0:44] = b"ElfFile\0" + struct.pack("<QQQIHHHH", 0, len(chunks) - 1, rid, 128, 1, 3, 4096, len(chunks))
    head[124:128] = struct.pack("<I", _crc(head[:120]))
    return bytes(head) + b"".join(chunks)


SCM_GUID = "555908d1-a6d7-4695-8e1e-26931d2012f4"


def scm(t, computer, eid, level, data, qualifiers=16384):
    return {"provider": "Service Control Manager", "guid": SCM_GUID, "source": "Service Control Manager", "id": eid,
            "qualifiers": qualifiers, "level": level, "keywords": 0x8080000000000000, "t": t, "pid": 708, "tid": 1880,
            "channel": "System", "computer": computer, "data": data}


def service_state(t, computer, service, state):
    return scm(t, computer, 7036, 4, [("param1", service), ("param2", state)])


def background_services(r, computer, start, end):
    """Windows Modules Installer / WinHTTP auto-proxy starting and stopping now and then."""
    out = []
    for t in rtimes(r, start, end, 1500.0):
        name = r.choice(["Windows Modules Installer", "WinHTTP Web Proxy Auto-Discovery Service", "Windows Update"])
        out.append(service_state(t, computer, name, "running"))
        out.append(service_state(t + timedelta(seconds=r.randint(60, 240)), computer, name, "stopped"))
    return [e for e in out if e["t"] < end]


wrnd = random.Random(1014)
WEB02_FQDN = "WEB02.contoso.local"
web02_system = background_services(wrnd, WEB02_FQDN, START, END)
web02_system.append(service_state(START + timedelta(milliseconds=750), WEB02_FQDN, "OrderService", "running"))
for h in (9, 10, 11):
    web02_system.append({"provider": "Microsoft-Windows-Time-Service", "guid": "06edcfeb-0fd0-4e53-acca-a6f8bbf81bcb", "id": 129,
                         "level": 3, "keywords": 0x8000000000000000, "t": START.replace(hour=h, minute=12, second=31), "pid": 1340,
                         "tid": 2116, "channel": "System", "computer": WEB02_FQDN, "sid": "S-1-5-19",
                         "data": [("ErrorMessage", "The entry is not found. (0x800706E1)"), ("RetryMinutes", "15")]})
web02_system.append(scm(CRASH + timedelta(milliseconds=350), WEB02_FQDN, 7031, 2,
                        [("param1", "OrderService"), ("param2", "1"), ("param3", "45000"), ("param4", "1"), ("param5", "Restart the service")],
                        qualifiers=49152))
web02_system.append(service_state(RESTART + timedelta(milliseconds=700), WEB02_FQDN, "OrderService", "running"))
web02_system.append({"provider": "Microsoft-Windows-WindowsUpdateClient", "guid": "945a8954-c147-4acd-923f-40c45405a658", "id": 19,
                     "version": 1, "level": 4, "task": 1, "opcode": 13, "keywords": 0x8000000000000018, "t": START.replace(hour=11, minute=20, second=5),
                     "pid": 3988, "tid": 4410, "channel": "System", "computer": WEB02_FQDN, "sid": "S-1-5-18",
                     "data": [("updateTitle", "Security Intelligence Update for Microsoft Defender Antivirus - KB2267602 (Version 1.419.212.0) - Current Channel (Broad)"),
                              ("updateGuid", "{" + guid(wrnd) + "}"), ("updateRevisionNumber", "200"), ("serviceGuid", "{" + guid(wrnd) + "}")]})
write_bytes(evtx(web02_system, 48211), "windows", "WEB02", "System.evtx")

APP_STACK = "\n".join(TIMEOUT_STACK.splitlines())
web02_app = []
for h in (9, 10, 11):
    web02_app.append({"provider": "Microsoft-Windows-Security-SPP", "guid": "e23b33b0-c8c9-472c-a5f9-f2bdfea0f156", "source": "Software Protection Platform Service",
                      "id": 16384, "qualifiers": 16384, "level": 4, "t": START.replace(hour=h, minute=3, second=wrnd.randint(0, 59)),
                      "channel": "Application", "computer": WEB02_FQDN,
                      "data": [(None, "2026-10-15T%02d:03:00Z" % h), (None, "RulesEngine")]})
web02_app.append({"provider": ".NET Runtime", "id": 1026, "qualifiers": 0, "level": 2, "t": CRASH - timedelta(milliseconds=40),
                  "channel": "Application", "computer": WEB02_FQDN,
                  "data": [(None, "Application: OrderService.exe\nCoreCLR Version: 9.0.124.61010\n.NET Version: 9.0.1\n"
                                  "Description: The process was terminated due to an unhandled exception.\n"
                                  "Exception Info: " + APP_STACK + "\n")]})
web02_app.append({"provider": "Application Error", "id": 1000, "qualifiers": 0, "level": 2, "task": 100, "t": CRASH + timedelta(milliseconds=120),
                  "pid": 6644, "tid": 6650, "channel": "Application", "computer": WEB02_FQDN,
                  "data": [("AppName", "OrderService.exe"), ("AppVersion", "3.14.2.0"), ("AppTimeStamp", "66f2a1b0"), ("ModuleName", "KERNELBASE.dll"),
                           ("ModuleVersion", "10.0.20348.2762"), ("ModuleTimeStamp", "8b1f0c3a"), ("ExceptionCode", "e0434352"),
                           ("FaultingOffset", "000000000003ea8c"), ("ProcessId", "0x1018"), ("ProcessCreationTime", "0x1db3e1a2c4d0000"),
                           ("AppPath", "C:\\Program Files\\Contoso\\OrderService\\OrderService.exe"), ("ModulePath", "C:\\Windows\\System32\\KERNELBASE.dll"),
                           ("IntegratorReportId", guid(wrnd)), ("PackageFullName", ""), ("PackageRelativeAppId", "")]})
web02_app.append({"provider": "Windows Error Reporting", "guid": "0ead09bd-2157-539a-8d6d-c87f95b64d70", "id": 1001, "level": 4,
                  "keywords": 0x8000000000000000, "t": CRASH + timedelta(seconds=1, milliseconds=900), "pid": 2432, "tid": 2276,
                  "channel": "Application", "computer": WEB02_FQDN, "sid": "S-1-5-18",
                  "data": [("Bucket", "2d1f4e7c9a8b3c51e8f0a6b7d4c2e913"), ("BucketType", "5"), ("EventName", "CLR20r3"), ("Response", "Not available"),
                           ("CabId", "0"), ("P1", "OrderService.exe"), ("P2", "3.14.2.0"), ("P3", "66f2a1b0"), ("P4", "System.Private.CoreLib"),
                           ("P5", "9.0.124.61010"), ("P6", "c2c3a8f1"), ("P7", "1a2c"), ("P8", "3e"), ("P9", "System.TimeoutException"), ("P10", ""),
                           ("AttachedFiles", "\\\\?\\C:\\ProgramData\\OrderService\\dumps\\OrderService.exe.4120.dmp"), ("StorePath", ""),
                           ("AnalysisSymbol", ""), ("Rechecking", "0"), ("ReportId", guid(wrnd)), ("ReportStatus", "268435456"),
                           ("HashedBucket", "2d1f4e7c9a8b3c51e8f0a6b7d4c2e913"), ("CabGuid", "0")]})
write_bytes(evtx(web02_app, 91873), "windows", "WEB02", "Application.evtx")

# BATCH01 loses power at 10:52:14 (the report service's run is cut off, see
# runs/batch01) and boots again at 10:54:57: Kernel-General 12, EventLog
# 6005/6009, the unexpected-shutdown 6008 and Kernel-Power 41.
BATCH01_FQDN = "BATCH01.contoso.local"
POWER_LOSS = datetime(2026, 10, 14, 10, 52, 14, 380000, tzinfo=timezone.utc)
BOOT = datetime(2026, 10, 14, 10, 54, 57, 500000, tzinfo=timezone.utc)
REPORT_UP = datetime(2026, 10, 14, 10, 56, 40, 120000, tzinfo=timezone.utc)
brnd = random.Random(1015)
batch_system = background_services(brnd, BATCH01_FQDN, START, POWER_LOSS) + background_services(brnd, BATCH01_FQDN, REPORT_UP, END)
batch_system = [e for e in batch_system if e["t"] < POWER_LOSS or e["t"] > REPORT_UP]
# The last event before the power goes (the downtime starts here).
batch_system.append({"provider": "Microsoft-Windows-Time-Service", "guid": "06edcfeb-0fd0-4e53-acca-a6f8bbf81bcb", "id": 37, "level": 4,
                     "keywords": 0x8000000000000000, "t": POWER_LOSS - timedelta(milliseconds=380), "pid": 1340, "tid": 2116, "channel": "System",
                     "computer": BATCH01_FQDN, "sid": "S-1-5-19",
                     "data": [("TimeSource", "DC01.contoso.local (ntp.d|10.20.0.10:123->10.20.4.21:123)")]})
batch_system.append({"provider": "Microsoft-Windows-Kernel-General", "guid": "a68ca8b7-004f-d7b6-a698-07e2de0f1f5d", "id": 12, "version": 2,
                     "level": 4, "task": 1, "keywords": 0x8000000000000080, "t": BOOT, "pid": 4, "tid": 8, "channel": "System",
                     "computer": BATCH01_FQDN, "sid": "S-1-5-18",
                     "data": [("MajorVersion", "10"), ("MinorVersion", "0"), ("BuildVersion", "20348"), ("QfeVersion", "2762"),
                              ("ServiceVersion", "0"), ("BootMode", "0"), ("StartTime", "2026-10-14T10:54:57.200000000Z")]})
for eid, delay, data in [(6009, 4100, [(None, "10.00."), (None, "20348"), (None, ""), (None, "Multiprocessor Free"), (None, "0")]),
                         (6005, 4120, []),
                         (6008, 4400, [(None, "10:52:14 AM"), (None, "10/14/2026"), (None, ""), (None, ""), (None, "312"), (None, ""), (None, "")])]:
    batch_system.append({"provider": "EventLog", "id": eid, "qualifiers": 32768, "level": 2 if eid == 6008 else 4,
                         "t": BOOT + timedelta(milliseconds=delay), "channel": "System", "computer": BATCH01_FQDN, "data": data})
batch_system.append({"provider": "Microsoft-Windows-Kernel-Power", "guid": "331c3b3a-2005-44c2-ac5e-77220c37d6b4", "id": 41, "version": 8,
                     "level": 1, "task": 63, "keywords": 0x8000400000000002, "t": BOOT + timedelta(seconds=6, milliseconds=210), "pid": 4, "tid": 8,
                     "channel": "System", "computer": BATCH01_FQDN, "sid": "S-1-5-18",
                     "data": [("BugcheckCode", "0"), ("BugcheckParameter1", "0x0"), ("BugcheckParameter2", "0x0"), ("BugcheckParameter3", "0x0"),
                              ("BugcheckParameter4", "0x0"), ("SleepInProgress", "0"), ("PowerButtonTimestamp", "0"), ("BootAppStatus", "0"),
                              ("Checkpoint", "0"), ("ConnectedStandbyInProgress", "false"), ("SystemSleepTransitionsToOn", "0"),
                              ("CsEntryScenarioInstanceId", "0"), ("BugcheckInfoFromEFI", "false"), ("CheckpointStatus", "0"),
                              ("LongPowerButtonPressDetected", "false")]})
batch_system.append(service_state(START + timedelta(minutes=4, seconds=58), BATCH01_FQDN, "ReportService", "running"))
batch_system.append(service_state(REPORT_UP - timedelta(milliseconds=300), BATCH01_FQDN, "ReportService", "running"))
write_bytes(evtx(batch_system, 30551), "windows", "BATCH01", "System.evtx")

# --------------------------------------------------------------------------
# Process Monitor "Save as CSV" on WEB02 around the crash (UTF-8 with BOM,
# CRLF, every cell quoted, as ProcMon writes it), with the optional
# "Date & Time", "TID" and "Duration" columns so the date is in the file.

prnd = random.Random(1016)


def pm_time(t):
    h = t.hour % 12 or 12
    return f"{h}:{t.minute:02d}:{t.second:02d}.{t.microsecond * 10:07d} {'AM' if t.hour < 12 else 'PM'}"


def pm_date(t):
    h = t.hour % 12 or 12
    return f"{t.month}/{t.day}/{t.year} {h}:{t.minute:02d}:{t.second:02d} {'AM' if t.hour < 12 else 'PM'}"


PM_FROM = CRASH - timedelta(seconds=55)
PM_TO = RESTART + timedelta(seconds=8)
pm_rows = []


def pm(t, proc, pid, tid, op, p, result, detail):
    pm_rows.append((t, proc, pid, op, p, result, detail, tid, f"0.{prnd.randint(1, 900000):07d}"))


OS_DIR = "C:\\Program Files\\Contoso\\OrderService"
for t in rtimes(prnd, PM_FROM, CRASH - timedelta(milliseconds=60), 0.045):
    s = stress(t)
    tid = prnd.choice([6412, 6420, 7104, 7110, 7388])
    r = prnd.random()
    port = prnd.randint(49800, 52000)
    if r < 0.30:
        pm(t, "OrderService.exe", 4120, tid, "TCP Send", f"WEB02.contoso.local:{port} -> SQL01.contoso.local:ms-sql-s", "SUCCESS",
           f"Length: {prnd.randint(80, 1400)}, startime: 0, endtime: 0, seqnum: 0, connid: 0")
    elif r < 0.52:
        pm(t, "OrderService.exe", 4120, tid, "TCP Receive", f"WEB02.contoso.local:{port} -> SQL01.contoso.local:ms-sql-s", "SUCCESS",
           f"Length: {prnd.randint(40, 8000)}, seqnum: 0, connid: 0")
    elif r < 0.56:
        pm(t, "OrderService.exe", 4120, tid, "TCP Reconnect", f"WEB02.contoso.local:{port} -> PAY01.contoso.local:https", "SUCCESS",
           "Length: 0, seqnum: 0, connid: 0")
    elif r < 0.70:
        pm(t, "OrderService.exe", 4120, tid, "WriteFile", "C:\\logs\\OrderService\\orders-20261014.clef", "SUCCESS",
           f"Offset: {prnd.randint(10_000_000, 20_000_000):,}, Length: {prnd.randint(200, 900)}, Priority: Normal")
    elif r < 0.78:
        pm(t, "OrderService.exe", 4120, tid, "RegQueryValue", "HKLM\\SOFTWARE\\Contoso\\OrderService\\PaymentTimeoutMs", "SUCCESS",
           "Type: REG_DWORD, Length: 4, Data: 5000")
    elif r < 0.80 + s * 0.05:
        pm(t, "OrderService.exe", 4120, tid, "CreateFile", "C:\\ProgramData\\OrderService\\cache\\orders.idx", "ACCESS DENIED",
           "Desired Access: Generic Read/Write, Disposition: OpenIf, Options: Synchronous IO Non-Alert, Non-Directory File, Attributes: N, "
           "ShareMode: Read, AllocationSize: 0")
    elif r < 0.86:
        pm(t, "OrderService.exe", 4120, tid, "CreateFile", f"{OS_DIR}\\appsettings.Production.json", "SUCCESS",
           "Desired Access: Generic Read, Disposition: Open, Options: Synchronous IO Non-Alert, Non-Directory File, Attributes: N, "
           "ShareMode: Read, Delete, AllocationSize: n/a, OpenResult: Opened")
    elif r < 0.90:
        pm(t, "OrderService.exe", 4120, tid, "QueryOpen", f"{OS_DIR}\\libSkiaSharp.dll", "NAME NOT FOUND", "")
    elif r < 0.95:
        pm(t, "filebeat.exe", 3020, 3104, "ReadFile", "C:\\logs\\OrderService\\orders-20261014.clef", "SUCCESS",
           f"Offset: {prnd.randint(10_000_000, 20_000_000):,}, Length: 16,384, Priority: Normal")
    elif r < 0.955:
        pm(t, "filebeat.exe", 3020, 3104, "CreateFile", "C:\\logs\\OrderService\\orders-20261014.clef", "SHARING VIOLATION",
           "Desired Access: Generic Read, Disposition: Open, Options: Synchronous IO Non-Alert, Non-Directory File, Attributes: n/a, "
           "ShareMode: Read, AllocationSize: n/a")
    else:
        pm(t, "OrderService.exe", 4120, tid, "RegOpenKey", "HKLM\\System\\CurrentControlSet\\Services\\Tcpip\\Parameters", "REPARSE",
           "Desired Access: Read")
pm(CRASH - timedelta(milliseconds=30), "OrderService.exe", 4120, 6412, "Thread Exit", "", "SUCCESS", "Thread ID: 6412, User Time: 41.2031250, Kernel Time: 3.8125000")
pm(CRASH, "OrderService.exe", 4120, 6412, "Process Exit", "", "SUCCESS",
   "Exit Status: -532462766, User Time: 1,612.4531250 seconds, Kernel Time: 98.0156250 seconds, Private Bytes: 2,431,422,464, "
   "Peak Private Bytes: 2,456,961,024, Working Set: 2,398,617,600, Peak Working Set: 2,412,888,064")
pm(CRASH + timedelta(milliseconds=60), "WerFault.exe", 6644, 6650, "Process Start", "", "SUCCESS",
   "Parent PID: 4120, Command line: C:\\Windows\\system32\\WerFault.exe -u -p 4120 -s 1212, Current directory: C:\\Windows\\system32\\, "
   "Environment: =::=::\\;ALLUSERSPROFILE=C:\\ProgramData")
pm(CRASH + timedelta(milliseconds=95), "WerFault.exe", 6644, 6650, "CreateFile", "C:\\ProgramData\\OrderService\\dumps\\OrderService.exe.4120.dmp", "SUCCESS",
   "Desired Access: Generic Write, Read Attributes, Disposition: OverwriteIf, Options: Synchronous IO Non-Alert, Non-Directory File, "
   "Attributes: N, ShareMode: None, AllocationSize: 0, OpenResult: Created")
for i in range(40):
    pm(CRASH + timedelta(milliseconds=110 + i * 21), "WerFault.exe", 6644, 6650, "WriteFile", "C:\\ProgramData\\OrderService\\dumps\\OrderService.exe.4120.dmp",
       "SUCCESS", f"Offset: {i * 1_048_576:,}, Length: 1,048,576, Priority: Normal")
pm(CRASH + timedelta(seconds=1, milliseconds=20), "WerFault.exe", 6644, 6650, "Process Exit", "", "SUCCESS",
   "Exit Status: 0, User Time: 0.1250000 seconds, Kernel Time: 0.4062500 seconds, Private Bytes: 9,814,016, Peak Private Bytes: 10,293,248, "
   "Working Set: 21,331,968, Peak Working Set: 21,782,528")
pm(CRASH + timedelta(milliseconds=380), "services.exe", 708, 1880, "RegQueryValue", "HKLM\\System\\CurrentControlSet\\Services\\OrderService\\FailureActions",
   "SUCCESS", "Type: REG_BINARY, Length: 44, Data: 80 51 01 00 00 00 00 00 00 00 00 00 01 00 00 00")
pm(RESTART - timedelta(milliseconds=210), "OrderService.exe", 5532, 5540, "Process Start", "", "SUCCESS",
   f"Parent PID: 708, Command line: \"{OS_DIR}\\OrderService.exe\" --urls http://0.0.0.0:8080, Current directory: {OS_DIR}\\, "
   "Environment: =::=::\\;ASPNETCORE_ENVIRONMENT=Production")
pm(RESTART - timedelta(milliseconds=200), "OrderService.exe", 5532, 5540, "Load Image", f"{OS_DIR}\\OrderService.exe", "SUCCESS",
   "Image Base: 0x7ff6a0000000, Image Size: 0x52000")
pm(RESTART - timedelta(milliseconds=150), "OrderService.exe", 5532, 5540, "Load Image", "C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App\\9.0.1\\coreclr.dll",
   "SUCCESS", "Image Base: 0x7ffd4a200000, Image Size: 0x4e5000")
for t in rtimes(prnd, RESTART, PM_TO, 0.08):
    tid = prnd.choice([5540, 5560, 5588])
    if prnd.random() < 0.5:
        pm(t, "OrderService.exe", 5532, tid, "CreateFile", f"{OS_DIR}\\{prnd.choice(['OrderService.dll', 'Microsoft.Data.SqlClient.dll', 'Serilog.dll', 'appsettings.json'])}",
           "SUCCESS", "Desired Access: Generic Read, Disposition: Open, Options: Synchronous IO Non-Alert, Non-Directory File, Attributes: N, "
           "ShareMode: Read, Delete, AllocationSize: n/a, OpenResult: Opened")
    else:
        pm(t, "OrderService.exe", 5532, tid, "TCP Connect", f"WEB02.contoso.local:{prnd.randint(52000, 53000)} -> SQL01.contoso.local:ms-sql-s", "SUCCESS",
           "Length: 0, mss: 1460, sackopt: 1, tsopt: 0, wsopt: 1, rcvwin: 65535, rcvwinscale: 8, sndwinscale: 8, seqnum: 0, connid: 0")
pm_rows.sort(key=lambda x: x[0])
lines = ['"Time of Day","Process Name","PID","Operation","Path","Result","Detail","Date & Time","TID","Duration"']
for t, proc, pid, op, p, result, detail, tid, dur in pm_rows:
    cells = [pm_time(t), proc, str(pid), op, p, result, detail, pm_date(t), str(tid), dur]
    lines.append(",".join('"' + c.replace('"', '""') + '"' for c in cells))
write_bytes(b"\xef\xbb\xbf" + ("\r\n".join(lines) + "\r\n").encode("utf-8"), "windows", "WEB02", "procmon-OrderService.csv")

# --------------------------------------------------------------------------
# Performance Monitor PDH-CSV (a Data Collector Set saved as CSV, as relog
# writes a .blg): WEB02's OrderService process every 15 s. The process
# counters are blank between the crash and the restart, then follow the new PID.

qrnd = random.Random(1017)
PDH = ["(PDH-CSV 4.0) (Coordinated Universal Time)(0)"] + ["\\\\WEB02\\" + c for c in [
    "Process(OrderService)\\% Processor Time", "Process(OrderService)\\Private Bytes", "Process(OrderService)\\ID Process",
    "Process(OrderService)\\Handle Count", "Process(OrderService)\\Thread Count", "Memory\\Available MBytes",
    "Processor(_Total)\\% Processor Time", "TCPv4\\Connections Established"]]
rows = [",".join(f'"{h}"' for h in PDH)]
t = START
while t <= END:
    s = stress(t) if t < CRASH else 0.0
    up = not (CRASH <= t < RESTART + timedelta(seconds=2))
    minutes = (t - (START if t < CRASH else RESTART)).total_seconds() / 60
    pid = 4120 if t < CRASH else 5532
    cpu = min(99.0, 31 + qrnd.uniform(-5, 7) + s * 62) if up else None
    private = (560 + minutes * (17.5 if t < CRASH else 0.6) + qrnd.uniform(0, 12)) * 1048576 if up else None
    proc = [f"{cpu * 8:.6f}" if up else " ", f"{private:.6f}" if up else " ", str(pid) if up else " ",
            f"{900 + int(minutes * 1.4) + qrnd.randint(0, 20)}" if up else " ", f"{60 + int(cpu * 1.6)}" if up else " "]
    machine = [f"{6100 - s * 4200 + qrnd.uniform(-60, 60):.6f}", f"{(cpu if up else 2.0) * 0.8 + qrnd.uniform(1, 6):.6f}",
               f"{120 + int(s * 600) + qrnd.randint(0, 25)}"]
    rows.append(",".join(f'"{c}"' for c in [t.strftime("%m/%d/%Y %H:%M:%S.000")] + proc + machine))
    t += timedelta(seconds=15)
write_bytes(("\r\n".join(rows) + "\r\n").encode("utf-8"), "windows", "WEB02", "perfmon-OrderService_000001.csv")

# --------------------------------------------------------------------------
# Debrief process monitor (debrief-monitor) output on BILL01: CLEF written by
# ClefFileSink, "Process stats" every 15 s with the .NET Framework runtime
# counters (Dotnet*) of the legacy billing service and the IIS worker process
# of the billing API. The app pool recycles at 11:00 (exit + restart detected).

mrnd = random.Random(1018)
MON_HOST = "BILL01"
mon = []


def mon_event(t, level, template, message, name, pid, **fields):
    e = {"@t": iso_ms(t)}
    if level != "Information":
        e["@l"] = level
    e["@m"] = message
    e["@mt"] = template
    e.update({"ProcessName": name, "ProcessId": pid, "MachineName": MON_HOST, "SourceContext": "Debrief.Monitor"})
    e.update(fields)
    mon.append(e)


class MonProc:
    def __init__(self, name, pid, exe, started, ws, heap):
        self.name, self.pid, self.exe, self.started = name, pid, exe, started
        self.ws, self.heap = ws, heap
        self.gc = [0, 0, 0]
        self.read = 0


procs = [MonProc("Billing.Service.exe", 3044, "C:\\Program Files\\Contoso\\Billing\\Billing.Service.exe", START - timedelta(days=3, hours=2), 410.0, 180.0),
         MonProc("w3wp.exe", 6120, "C:\\Windows\\System32\\inetsrv\\w3wp.exe", START - timedelta(hours=2, minutes=3), 520.0, 260.0)]
RECYCLE = datetime(2026, 10, 14, 11, 0, 3, 250000, tzinfo=timezone.utc)
t = START + timedelta(seconds=5)
first = True
while t < END:
    s = stress(t) if t < CRASH else max(0.0, 0.6 - (t - CRASH).total_seconds() / 1800)
    for p in procs:
        up = int((t - p.started).total_seconds())
        if first:
            mon_event(t, "Information", "Now monitoring {ProcessName} ({ProcessId}), running for {UptimeSec}s",
                      f"Now monitoring {p.name} ({p.pid}), running for {up}s", p.name, p.pid, Exe=p.exe, UptimeSec=up, AlreadyRunning=True)
    if t >= RECYCLE and procs[1].pid == 6120:
        old = procs[1]
        procs[1] = MonProc("w3wp.exe", 7044, old.exe, RECYCLE, 140.0, 40.0)
        mon_event(t, "Warning", "Application exited: {ProcessName} ({ProcessId}) after {UptimeSec}s",
                  f"Application exited: w3wp.exe (6120) after {int((RECYCLE - old.started).total_seconds())}s", "w3wp.exe", 6120,
                  UptimeSec=int((RECYCLE - old.started).total_seconds()), RestartedAs=7044)
        mon_event(t, "Information", "Application started: {ProcessName} ({ProcessId})", "Application started: w3wp.exe (7044)", "w3wp.exe", 7044,
                  Exe=old.exe, Restarted=True)
    for p in procs:
        busy = s if p.name == "Billing.Service.exe" else s * 0.5
        cpu = round(max(0.1, 4 + mrnd.uniform(-2, 3) + busy * 30), 1)
        p.ws = min(p.ws + mrnd.uniform(-1.5, 2.5) + busy * 3.2, 1900)
        p.heap = min(p.heap + mrnd.uniform(-1.0, 1.6) + busy * 2.6, 1500)
        gen0 = mrnd.randint(1, 4) + int(busy * 8)
        p.gc[0] += gen0
        p.gc[1] += gen0 // 3
        p.gc[2] += 1 if mrnd.random() < 0.08 + busy * 0.3 else 0
        p.read += mrnd.randint(10_000, 90_000)
        heap = int(p.heap * 1048576)
        loh = int(heap * (0.12 + busy * 0.1))
        gen2 = int(heap * 0.62)
        threads = 38 + int(busy * 40) + mrnd.randint(0, 4)
        up = int((t - p.started).total_seconds())
        ws = round(p.ws, 1)
        mon_event(t, "Information", "Process stats {ProcessName} CPU {CpuPercent}% · WorkingSet {WorkingSetMb} MB",
                  f"Process stats {p.name} CPU {cpu:.1f}% · WorkingSet {ws:.1f} MB", p.name, p.pid,
                  Exe=p.exe, CpuPercent=cpu, WorkingSetMb=ws, VirtualMb=round(ws * 2.9 + 4096, 1), PrivateMb=round(ws * 1.08, 1),
                  Threads=threads + 6, Handles=640 + threads * 4 + mrnd.randint(0, 30),
                  ReadBytesPerSec=mrnd.randint(2_000, 40_000) + int(busy * 120_000), WriteBytesPerSec=mrnd.randint(1_000, 12_000), UptimeSec=up,
                  DotnetRuntime="framework", DotnetGen0Collections=p.gc[0], DotnetGen1Collections=p.gc[1], DotnetGen2Collections=p.gc[2],
                  DotnetGen0HeapBytes=6_291_456, DotnetGen1HeapBytes=heap - gen2 - loh - 6_291_456 if heap - gen2 - loh > 6_291_456 else 1_048_576,
                  DotnetGen2HeapBytes=gen2, DotnetLohBytes=loh, DotnetHeapBytes=heap,
                  DotnetTimeInGcPercent=round(min(60.0, 1.5 + mrnd.uniform(0, 2) + busy * 24), 1), DotnetPinnedObjects=mrnd.randint(0, 12),
                  DotnetExceptionsPerSec=round(mrnd.uniform(0, 0.2) + busy * 6, 2), DotnetContentionPerSec=round(mrnd.uniform(0, 0.5) + busy * 14, 2),
                  DotnetQueueLengthPerSec=round(busy * 3 * mrnd.random(), 2), DotnetThreads=threads, DotnetPhysicalThreads=threads - 3,
                  DotnetAssemblies=112 if p.name == "Billing.Service.exe" else 186)
    first = False
    t += timedelta(seconds=15)
with open(path("monitor", "BILL01", "process-monitor.clef"), "w", encoding="utf-8", newline="\n") as f:
    for e in mon:
        f.write(json.dumps(e, ensure_ascii=False) + "\n")

# --------------------------------------------------------------------------
# Web: IIS (W3C extended) logs of the site in front of OrderService on WEB01
# and WEB02, the nginx edge's combined access log with $request_time, and a
# browser HAR capture of a checkout that hangs on the payment capture.

irnd = random.Random(1019)
IIS_FIELDS = ("date time s-sitename s-computername s-ip cs-method cs-uri-stem cs-uri-query s-port cs-username c-ip cs-version "
              "cs(User-Agent) cs(Referer) sc-status sc-substatus sc-win32-status sc-bytes cs-bytes time-taken")
AGENTS = ["Mozilla/5.0+(Windows+NT+10.0;+Win64;+x64)+AppleWebKit/537.36+(KHTML,+like+Gecko)+Chrome/129.0.0.0+Safari/537.36",
          "Mozilla/5.0+(Macintosh;+Intel+Mac+OS+X+14_6)+AppleWebKit/605.1.15+(KHTML,+like+Gecko)+Version/18.0+Safari/605.1.15",
          "Mozilla/5.0+(iPhone;+CPU+iPhone+OS+18_0+like+Mac+OS+X)+AppleWebKit/605.1.15+(KHTML,+like+Gecko)+Mobile/15E148",
          "Yarp/2.2"]


def iis_header(t, machine):
    return [f"#Software: Microsoft Internet Information Services 10.0", "#Version: 1.0", f"#Date: {t.strftime('%Y-%m-%d %H:%M:%S')}",
            f"#Fields: {IIS_FIELDS}"]


def iis_log(machine, ip, leaky):
    out = iis_header(START, machine)
    order = 88000 if machine == "WEB01" else 88001
    restarted = False
    for t in rtimes(irnd, START, END, 2.4):
        if leaky and not restarted and t >= RESTART:
            # W3SVC was restarted with the app: IIS starts a new header block.
            out += iis_header(t, machine)
            restarted = True
        s = stress(t) if (leaky and t < CRASH) else stress(t) * 0.3 if t < CRASH else 0.0
        order += 2
        stem, query, method = irnd.choice([("/api/orders", "page=1", "GET"), (f"/api/orders/{order}", "-", "GET"),
                                           (f"/api/orders/{order}/capture", "-", "POST"), ("/api/cart", "-", "GET"),
                                           ("/health", "-", "GET"), ("/static/app.js", "v=3.14.2", "GET")])
        status, sub, win32 = 200, 0, 0
        taken = irnd.randint(2, 60) if stem.startswith("/static") or stem == "/health" else int(irnd.uniform(20, 160) + s * irnd.uniform(800, 5200))
        if leaky and CRASH <= t < RESTART + timedelta(seconds=2):
            status, sub, win32, taken = 502, 3, 12029, irnd.randint(1, 8)
        elif stem.endswith("/capture") and irnd.random() < s * 0.6:
            status, sub, win32, taken = 500, 0, 0, taken + 5000
        elif irnd.random() < 0.01:
            status, sub, win32 = 404, 0, 2
        elif irnd.random() < 0.004:
            status, sub, win32 = 401, 2, 5
        if method == "POST" and status == 200:
            status = 201
        user = "-" if irnd.random() < 0.7 else f"CONTOSO\\svc-shop{irnd.randint(1, 3)}"
        agent = AGENTS[3] if stem.startswith("/api") else irnd.choice(AGENTS[:3])
        out.append(" ".join([t.strftime("%Y-%m-%d %H:%M:%S"), "W3SVC1", machine, ip, method, stem, query, "443", user,
                             "10.20.1.5" if agent == AGENTS[3] else f"203.0.113.{irnd.randint(2, 250)}", "HTTP/1.1", agent,
                             "https://shop.contoso.com/checkout" if irnd.random() < 0.4 else "-", str(status), str(sub), str(win32),
                             str(irnd.randint(300, 9000)), str(irnd.randint(200, 1800)), str(taken)]))
    return "\r\n".join(out) + "\r\n"


write_bytes(iis_log("WEB01", "10.20.1.11", leaky=False).encode("ascii"), "web", "iis", "WEB01", "W3SVC1", "u_ex261014.log")
write_bytes(iis_log("WEB02", "10.20.1.12", leaky=True).encode("ascii"), "web", "iis", "WEB02", "W3SVC1", "u_ex261014.log")

nrnd = random.Random(1020)
nginx = []
for t in rtimes(nrnd, START, END, 1.9):
    s = stress(t) if t < CRASH else (0.5 if t < RESTART + timedelta(minutes=2) else 0.0)
    method, p, ok = nrnd.choice([("GET", "/", 200), ("GET", "/checkout", 200), ("GET", "/api/orders?page=1", 200), ("POST", "/api/orders", 201),
                                 ("POST", f"/api/orders/{nrnd.randint(88000, 89000)}/capture", 200), ("GET", "/static/app.js?v=3.14.2", 200),
                                 ("GET", "/static/app.css?v=3.14.2", 304), ("GET", "/favicon.ico", 200)])
    status, rt, size = ok, nrnd.uniform(0.004, 0.18), nrnd.randint(200, 24000)
    if p.startswith("/api") and nrnd.random() < s * 0.45:
        status, rt, size = 504, nrnd.uniform(29.8, 30.1), 167
    elif p.startswith("/api"):
        rt += s * nrnd.uniform(0.5, 5.0)
    elif nrnd.random() < 0.006:
        status, size = 404, 153
    if status == 304:
        size = 0
    ip = f"198.51.100.{nrnd.randint(2, 254)}" if nrnd.random() < 0.6 else f"203.0.113.{nrnd.randint(2, 250)}"
    ua = nrnd.choice([a.replace("+", " ") for a in AGENTS[:3]] + ["curl/8.9.1", "Googlebot/2.1 (+http://www.google.com/bot.html)"])
    ref = "https://shop.contoso.com/checkout" if p.startswith("/api") else "-"
    stamp = t.strftime("%d/%b/%Y:%H:%M:%S +0000")
    nginx.append(f'{ip} - - [{stamp}] "{method} {p} HTTP/2.0" {status} {size} "{ref}" "{ua}" {rt:.3f}')
write_text("\n".join(nginx) + "\n", "web", "nginx", "edge01", "access.log")

hrnd = random.Random(1021)
HAR_START = datetime(2026, 10, 14, 10, 38, 12, 204000, tzinfo=timezone.utc)
har_entries = []


def har_entry(t, method, url, status, text, mime, wait_ms, size, ip="20.50.2.14", post=None):
    blocked, dns, connect, send, receive = 1.2, -1, -1, 0.1, round(hrnd.uniform(0.4, 9.0), 3)
    total = round(blocked + send + wait_ms + receive, 3)
    req = {"method": method, "url": url, "httpVersion": "h2", "cookies": [], "headers": [{"name": "accept", "value": "*/*"}],
           "queryString": [], "headersSize": -1, "bodySize": len(post) if post else 0}
    if post:
        req["postData"] = {"mimeType": "application/json", "text": post}
    har_entries.append({
        "startedDateTime": iso_ms(t), "time": total, "request": req,
        "response": {"status": status, "statusText": text, "httpVersion": "h2", "cookies": [], "headers": [{"name": "content-type", "value": mime}],
                     "content": {"size": size, "mimeType": mime}, "redirectURL": "", "headersSize": -1, "bodySize": size, "_transferSize": size + 320},
        "cache": {}, "timings": {"blocked": blocked, "dns": dns, "ssl": -1, "connect": connect, "send": send, "wait": wait_ms, "receive": receive},
        "serverIPAddress": ip, "connection": "443", "pageref": "page_1"})
    return t + timedelta(milliseconds=total)


SHOP = "https://shop.contoso.com"
t = har_entry(HAR_START, "GET", f"{SHOP}/checkout", 200, "", "text/html", 142.6, 18342)
for asset, mime in [("app.js?v=3.14.2", "application/javascript"), ("app.css?v=3.14.2", "text/css"), ("logo.svg", "image/svg+xml")]:
    har_entry(t + timedelta(milliseconds=hrnd.randint(2, 9)), "GET", f"{SHOP}/static/{asset}", 200, "", mime, round(hrnd.uniform(8, 40), 3), hrnd.randint(2000, 180000))
t = har_entry(t + timedelta(milliseconds=180), "GET", f"{SHOP}/api/cart", 200, "", "application/json", 388.1, 1290)
t = har_entry(t + timedelta(milliseconds=2600), "POST", f"{SHOP}/api/orders", 201, "", "application/json", 1912.4, 512,
              post='{"cartId":"c-77120","shipping":"standard"}')
t = har_entry(t + timedelta(milliseconds=40), "POST", f"{SHOP}/api/orders/88412/capture", 504, "", "text/html", 30011.7, 167,
              post='{"paymentMethod":"card","token":"tok_redacted"}')
har_entry(t + timedelta(milliseconds=15), "POST", "https://telemetry.contoso.com/v1/rum", 0, "", "", 0.0, 0, ip="")
t = har_entry(t + timedelta(milliseconds=3200), "GET", f"{SHOP}/api/orders/88412", 200, "", "application/json", 2410.9, 702)
har_entry(t + timedelta(milliseconds=900), "POST", f"{SHOP}/api/orders/88412/capture", 500, "", "application/problem+json", 5204.3, 288,
          post='{"paymentMethod":"card","token":"tok_redacted"}')
har = {"log": {"version": "1.2", "creator": {"name": "WebInspector", "version": "537.36"},
               "pages": [{"startedDateTime": iso_ms(HAR_START), "id": "page_1", "title": "Checkout – Contoso Shop",
                          "pageTimings": {"onContentLoad": 611.2, "onLoad": 948.5}}],
               "entries": sorted(har_entries, key=lambda e: e["startedDateTime"])}}
write_text(json.dumps(har, indent=2, ensure_ascii=False) + "\n", "web", "har", "checkout-slow.har")

# --------------------------------------------------------------------------
# Linux and cloud: /var/log/syslog of the Docker host (RFC 3164 / BSD lines
# as rsyslog writes them), RFC 5424 lines a Kubernetes node forwards, and a
# `journalctl -o json` export of the SQL Server on Linux host.

srnd = random.Random(1022)
syslog = []


def bsd(t, host, app, pid, msg):
    syslog.append((t, f"{t.strftime('%b')} {t.day:2d} {t.strftime('%H:%M:%S')} {host} {app}{f'[{pid}]' if pid else ''}: {msg}"))


t = START.replace(minute=5)
while t < END:
    bsd(t, "docker01", "CRON", 40000 + srnd.randint(0, 999), "(root) CMD (command -v debian-sa1 > /dev/null && debian-sa1 1 1)")
    t += timedelta(minutes=10)
for t in rtimes(srnd, START, END, 45.0):
    s = stress(t)
    r = srnd.random()
    if r < s * 0.5:
        bsd(t, "docker01", "dockerd", 1022, f'time="{t.strftime("%Y-%m-%dT%H:%M:%S")}.{t.microsecond:06d}000Z" level=warning '
            f'msg="Health check for container {CID[:12]} failing: context deadline exceeded" container={CID[:12]}')
    elif r < 0.25:
        bsd(t, "docker01", "containerd", 914, f'time="{t.strftime("%Y-%m-%dT%H:%M:%S")}.{t.microsecond:06d}000Z" level=info '
            f'msg="loading plugin \\"io.containerd.event.v1.publisher\\"..." runtime=io.containerd.runc.v2 type=io.containerd.event.v1')
    elif r < 0.45:
        bsd(t, "docker01", "systemd", 1, f"Started session-{srnd.randint(100, 999)}.scope - Session of User deploy.")
    elif r < 0.55:
        bsd(t, "docker01", "sshd", srnd.randint(30000, 39999), f"Accepted publickey for deploy from 10.20.9.{srnd.randint(2, 40)} port {srnd.randint(40000, 60000)} ssh2: ED25519 SHA256:3qBv7RkqVZb1")
    elif r < 0.6:
        bsd(t, "docker01", "kernel", None, f"[{int((t - START).total_seconds()) + 860412}.{srnd.randint(0, 999999):06d}] "
            f"[UFW BLOCK] IN=eth0 OUT= MAC= SRC=192.0.2.{srnd.randint(2, 250)} DST=10.20.3.4 LEN=60 PROTO=TCP SPT={srnd.randint(1024, 65000)} DPT=22")
    elif srnd.random() < 0.05:
        bsd(t, "docker01", "systemd-resolved", 611, "Clock change detected. Flushing caches.")
    else:
        bsd(t, "docker01", "rsyslogd", 702, "-- MARK --")
for t in rtimes(srnd, INCIDENT + timedelta(minutes=4), CRASH, 70.0):
    bsd(t, "docker01", "kernel", None, f"[{int((t - START).total_seconds()) + 860412}.{srnd.randint(0, 999999):06d}] "
        "nf_conntrack: nf_conntrack: table full, dropping packet")
syslog.sort(key=lambda x: x[0])
write_text("\n".join(l for _, l in syslog) + "\n", "linux", "docker01", "syslog")

krnd = random.Random(1023)
rfc = []
POD = "order-events-7c9d8b5f4-x2k4p"


def r5424(t, pri, app, procid, msgid, sd, msg):
    rfc.append((t, f"<{pri}>1 {iso_ms(t)} k8s-node-2 {app} {procid} {msgid} {sd} {msg}"))


def sd(**kv):
    return "[k8s@32473 " + " ".join(f'{k}="{v}"' for k, v in kv.items()) + "]"


for t in rtimes(krnd, START, END, 30.0):
    s = stress(t)
    if krnd.random() < 0.15 + s * 0.4:
        r5424(t, 28, "kubelet", 1380, "PROBE", sd(namespace="shop", pod=POD, container="consumer", probe="readiness"),
              'Readiness probe failed: Get "http://10.244.2.17:8081/ready": context deadline exceeded')
    else:
        r5424(t, 30, "kubelet", 1380, "SYNC", sd(namespace="shop", pod=POD), "SyncLoop (PLEG): event for pod; ContainerStarted")
for t in rtimes(krnd, START, END, 120.0):
    r5424(t, 30, "containerd", 902, "-", "-", "shim disconnected id=%016x namespace=k8s.io" % krnd.getrandbits(64))
r5424(CRASH + timedelta(milliseconds=180), 27, "kubelet", 1380, "CRASH", sd(namespace="shop", pod=POD, container="consumer", exitCode="2"),
      'Container "consumer" terminated: reason=Error exitCode=2 (panic: runtime error: invalid memory address or nil pointer dereference)')
r5424(CRASH + timedelta(seconds=1), 28, "kubelet", 1380, "BACKOFF", sd(namespace="shop", pod=POD, container="consumer"),
      'Back-off restarting failed container consumer in pod order-events-7c9d8b5f4-x2k4p_shop')
r5424(RESTART + timedelta(seconds=2, milliseconds=640), 30, "kubelet", 1380, "START", sd(namespace="shop", pod=POD, container="consumer", restartCount="1"),
      'Started container consumer')
r5424(CRASH - timedelta(minutes=6), 26, "kernel", "-", "-", "-", "Memory cgroup out of memory: Killed process 88213 (log-shipper) total-vm:812044kB, anon-rss:498212kB")
rfc.sort(key=lambda x: x[0])
write_text("\n".join(l for _, l in rfc) + "\n", "linux", "k8s-node-2", "rsyslog-rfc5424.log")

jrnd = random.Random(1024)
journal = []
BOOT_ID = "%032x" % jrnd.getrandbits(128)


def jent(t, prio, ident, pid, unit, msg, transport="syslog", **extra):
    us = (t - EPOCH) // timedelta(microseconds=1)
    e = {"__CURSOR": f"s=7f3b9c0e2a1d4e5f8a6b0c1d2e3f4a5b;i={len(journal) + 4_412_201:x};b={BOOT_ID};m={us - 1_791_000_000_000_000:x};t={us:x};x={jrnd.getrandbits(32):x}",
         "__REALTIME_TIMESTAMP": str(us), "__MONOTONIC_TIMESTAMP": str(us - 1_790_900_000_000_000), "_BOOT_ID": BOOT_ID,
         "PRIORITY": str(prio), "SYSLOG_FACILITY": "3", "SYSLOG_IDENTIFIER": ident, "_PID": str(pid), "_UID": "999" if ident == "sqlservr" else "0",
         "_COMM": ident, "_HOSTNAME": "sqlrpt01", "_TRANSPORT": transport, "MESSAGE": msg}
    if unit:
        e["_SYSTEMD_UNIT"] = unit
    e.update(extra)
    journal.append((t, e))


jent(START, 6, "systemd", 1, "init.scope", "Started mssql-server.service - Microsoft SQL Server Database Engine.", transport="journal")
jent(START + timedelta(milliseconds=900), 6, "sqlservr", 412, "mssql-server.service", "This is an evaluation version.  There are [148] days left in the evaluation period.", transport="stdout")
for t in rtimes(jrnd, START, END, 90.0):
    r = jrnd.random()
    if r < 0.4:
        jent(t, 6, "CRON", 50000 + jrnd.randint(0, 9999), "cron.service", "pam_unix(cron:session): session opened for user root(uid=0) by (uid=0)")
    elif r < 0.7:
        jent(t, 6, "systemd", 1, "init.scope", f"Started session-{jrnd.randint(100, 999)}.scope - Session of User mssql.", transport="journal")
    else:
        jent(t, 6, "sshd", jrnd.randint(20000, 29999), "ssh.service", f"Accepted publickey for dba from 10.20.9.{jrnd.randint(2, 40)} port {jrnd.randint(40000, 60000)} ssh2")
IOERR = CRASH - timedelta(minutes=5, seconds=2)
for i in range(6):
    jent(IOERR + timedelta(milliseconds=i * 37), 3, "kernel", 0, None,
         f"blk_update_request: I/O error, dev sdb, sector {7_841_312 + i * 8} op 0x0:(READ) flags 0x80700 phys_seg 1 prio class 0", transport="kernel",
         SYSLOG_FACILITY="0")
jent(IOERR + timedelta(milliseconds=260), 3, "kernel", 0, None, "Buffer I/O error on dev sdb1, logical block 980164, async page read", transport="kernel", SYSLOG_FACILITY="0")
jent(IOERR + timedelta(milliseconds=410), 4, "kernel", 0, None, "sd 2:0:0:1: [sdb] tag#18 timing out command, waited 180s", transport="kernel", SYSLOG_FACILITY="0")
jent(CRASH - timedelta(minutes=5) + timedelta(milliseconds=20), 3, "sqlservr", 412, "mssql-server.service",
     "Error: 823, Severity: 24, State: 2. The operating system returned error 21(The device is not ready.) to SQL Server during a read at offset "
     "0x0000000a4e2000 in file '/var/opt/mssql/data/Reporting.mdf'.", transport="stdout")
journal.sort(key=lambda x: x[0])
write_text("".join(json.dumps(e, ensure_ascii=False) + "\n" for _, e in journal), "linux", "sqlrpt01", "journal.json")

# OpenTelemetry Collector `file` exporter output (one OTLP/JSON export per
# line): traces of checkout captures across gateway → order-service →
# SQL and payments-adapter, and the payments adapter's logs linked by trace id.

ornd = random.Random(1025)


def attr(k, v):
    if isinstance(v, bool):
        return {"key": k, "value": {"boolValue": v}}
    if isinstance(v, int):
        return {"key": k, "value": {"intValue": str(v)}}
    if isinstance(v, float):
        return {"key": k, "value": {"doubleValue": v}}
    return {"key": k, "value": {"stringValue": v}}


def resource(service, host, version):
    return {"attributes": [attr("service.name", service), attr("service.version", version), attr("host.name", host),
                           attr("deployment.environment", "production")]}


trace_lines = []
log_lines = []
for t in rtimes(ornd, START, END, 25.0):
    s = stress(t) if t < CRASH else 0.0
    host = ornd.choice(["web01", "web02"]) if not (CRASH <= t < RESTART) else "web01"
    hs = s if host == "web02" else s * 0.3
    tid = "%032x" % ornd.getrandbits(128)
    ids = ["%016x" % ornd.getrandbits(64) for _ in range(6)]
    order = ornd.randint(88000, 89000)
    t0 = ns_of(t)
    db_ms = ornd.uniform(3, 25) + hs * ornd.uniform(200, 4000)
    deadlock = ornd.random() < hs * 0.25
    pay_ms = ornd.uniform(40, 180) + hs * ornd.uniform(1500, 14000)
    timeout = pay_ms > 5000
    pay_ms = min(pay_ms, 5000.0)
    total = db_ms + pay_ms + ornd.uniform(5, 20)
    err = timeout or deadlock

    def span(sid, parent, name, kind, start_ns, dur_ms, attrs, error=None, events=None):
        sp = {"traceId": tid, "spanId": sid, "name": name, "kind": kind, "startTimeUnixNano": str(start_ns),
              "endTimeUnixNano": str(start_ns + int(dur_ms * 1e6)), "attributes": attrs, "status": {"code": 2, "message": error} if error else {"code": 1 if kind == 2 else 0}}
        if parent:
            sp["parentSpanId"] = parent
        if events:
            sp["events"] = events
        return sp

    def exc(at_ns, typ, msg):
        return {"timeUnixNano": str(at_ns), "name": "exception",
                "attributes": [attr("exception.type", typ), attr("exception.message", msg), attr("exception.stacktrace", HTTP_STACK if "Http" in typ else TIMEOUT_STACK)]}

    gw = span(ids[0], None, "POST /api/orders/{id}/capture", 2, t0, total + 3,
              [attr("http.request.method", "POST"), attr("url.path", f"/api/orders/{order}/capture"), attr("http.response.status_code", 504 if timeout else 500 if err else 200),
               attr("server.address", "shop.contoso.com")], "upstream error" if err else None)
    gwc = span(ids[1], ids[0], "POST", 3, t0 + 1_000_000, total + 1, [attr("server.address", f"{host}:8080"), attr("http.response.status_code", 500 if err else 200)])
    srv = span(ids[2], ids[1], "POST /api/orders/{id}/capture", 2, t0 + 1_500_000, total,
               [attr("http.route", "/api/orders/{id}/capture"), attr("order.id", order), attr("http.response.status_code", 500 if err else 200)],
               "payment capture failed" if err else None,
               [exc(t0 + int((total - 1) * 1e6), "System.Net.Http.HttpRequestException", "Response status code does not indicate success: 504 (Gateway Timeout).")] if timeout else None)
    db = span(ids[3], ids[2], "SELECT Orders", 3, t0 + 2_000_000, db_ms,
              [attr("db.system", "mssql"), attr("db.namespace", "Orders"), attr("server.address", "SQL01")],
              "deadlock victim" if deadlock else None,
              [exc(t0 + 2_000_000 + int(db_ms * 1e6), "Microsoft.Data.SqlClient.SqlException", "Transaction was deadlocked on lock resources with another process")] if deadlock else None)
    payc = span(ids[4], ids[2], "POST", 3, t0 + 2_500_000 + int(db_ms * 1e6), pay_ms, [attr("server.address", "payments-adapter:9090")],
                "timeout" if timeout else None)
    pay = span(ids[5], ids[4], "capture", 2, t0 + 3_000_000 + int(db_ms * 1e6), pay_ms - 1, [attr("order.id", order), attr("payment.provider", "contoso-pay")],
               "context deadline exceeded" if timeout else None)
    scope = lambda name, spans: {"scope": {"name": name, "version": "1.9.0"}, "spans": spans}
    trace_lines.append({"resourceSpans": [
        {"resource": resource("gateway", "GATEWAY", "2.2.0"), "scopeSpans": [scope("Yarp.ReverseProxy", [gw, gwc])]},
        {"resource": resource("order-service", host.upper(), "3.14.2"), "scopeSpans": [scope("OrderService", [srv, db, payc])]},
        {"resource": resource("payments-adapter", "docker01", "1.7.3"), "scopeSpans": [scope("payments", [pay])]}]})
    sev = (17, "ERROR") if timeout else (13, "WARN") if pay_ms > 1000 else (9, "INFO")
    log_lines.append({"resourceLogs": [{"resource": resource("payments-adapter", "docker01", "1.7.3"), "scopeLogs": [{"scope": {"name": "payments"}, "logRecords": [{
        "timeUnixNano": str(t0 + 3_000_000 + int((db_ms + pay_ms) * 1e6)), "observedTimeUnixNano": str(t0 + 4_000_000 + int((db_ms + pay_ms) * 1e6)),
        "severityNumber": sev[0], "severityText": sev[1],
        "body": {"stringValue": "gateway capture failed: context deadline exceeded" if timeout else "capture ok"},
        "attributes": [attr("order.id", order), attr("duration.ms", round(pay_ms, 1))], "traceId": tid, "spanId": ids[5]}]}]}]})
write_text("".join(json.dumps(x, separators=(",", ":")) + "\n" for x in trace_lines), "otel", "collector", "traces-20261014.jsonl")
write_text("".join(json.dumps(x, separators=(",", ":")) + "\n" for x in log_lines), "otel", "collector", "logs-20261014.jsonl")

# --------------------------------------------------------------------------
# Configuration diff: the same files collected from WEB01 and WEB02 (a
# "snapshot" folder per machine, paired by relative path). WEB02 runs a
# smaller SQL connection pool, debug logging and the new checkout flag.

APPSETTINGS = {
    "Serilog": {"MinimumLevel": {"Default": "Information", "Override": {"Microsoft": "Warning", "System": "Warning"}},
                "WriteTo": [{"Name": "File", "Args": {"path": "C:\\logs\\OrderService\\orders-.clef", "rollingInterval": "Day",
                                                       "formatter": "Serilog.Formatting.Compact.CompactJsonFormatter, Serilog.Formatting.Compact"}}]},
    "ConnectionStrings": {"Orders": "Server=SQL01;Database=Orders;Integrated Security=true;Max Pool Size=100;Connect Timeout=15"},
    "Payments": {"GatewayUrl": "https://payments-adapter:9090", "TimeoutMs": 5000, "MaxAttempts": 3},
    "Caching": {"Redis": {"Enabled": True, "Configuration": "redis01:6379"}},
    "Features": {"NewCheckout": False},
    "Kestrel": {"Endpoints": {"Http": {"Url": "http://0.0.0.0:8080"}}, "Limits": {"MaxConcurrentConnections": 500}},
    "HealthChecks": {"Path": "/health"},
}
web02_settings = json.loads(json.dumps(APPSETTINGS))
web02_settings["Serilog"]["MinimumLevel"]["Default"] = "Debug"
web02_settings["ConnectionStrings"]["Orders"] = "Server=SQL01;Database=Orders;Integrated Security=true;Max Pool Size=20;Connect Timeout=15"
web02_settings["Caching"]["Redis"] = {"Enabled": False}
web02_settings["Features"]["NewCheckout"] = True
web02_settings["Kestrel"]["Limits"]["MaxConcurrentConnections"] = 200
del web02_settings["HealthChecks"]
web02_settings["Diagnostics"] = {"DumpOnCrash": True, "DumpFolder": "C:\\ProgramData\\OrderService\\dumps"}

WEB_CONFIG = """<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <location path="." inheritInChildApplications="false">
    <system.webServer>
      <handlers>
        <add name="aspNetCore" path="*" verb="*" modules="AspNetCoreModuleV2" resourceType="Unspecified" />
      </handlers>
      <aspNetCore processPath="dotnet" arguments=".\\OrderService.Web.dll" stdoutLogEnabled="{stdout}" stdoutLogFile=".\\logs\\stdout" hostingModel="outofprocess" requestTimeout="{timeout}">
        <environmentVariables>
          <environmentVariable name="ASPNETCORE_ENVIRONMENT" value="Production" />{extra}
        </environmentVariables>
      </aspNetCore>
    </system.webServer>
  </location>
</configuration>
"""

INI = """; OrderService host settings
[service]
name=OrderService
port=8080
environment=Production

[payments]
timeout_ms=5000
retries={retries}

[cache]
size_mb={cache}
{extra}"""

COLLECTOR = """receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
processors:
  batch:
    timeout: {batch}
  probabilistic_sampler:
    sampling_percentage: {sampling}
exporters:
  file:
    path: /var/log/otel/traces-20261014.jsonl
service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch, probabilistic_sampler]
      exporters: [file]
"""

REG = """Windows Registry Editor Version 5.00

[HKEY_LOCAL_MACHINE\\SOFTWARE\\Contoso\\OrderService]
"Version"="3.14.2"
"InstallDir"="C:\\\\Program Files\\\\Contoso\\\\OrderService"
"PaymentTimeoutMs"=dword:00001388
"GcServer"=dword:{gc}
{extra}
[HKEY_LOCAL_MACHINE\\SOFTWARE\\Contoso\\OrderService\\Features]
"NewCheckout"=dword:{checkout}

[HKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Services\\OrderService]
"Start"=dword:00000002
"ObjectName"="CONTOSO\\\\svc-orders"
"FailureActions"=hex:80,51,01,00,00,00,00,00,00,00,00,00,03,00,00,00,14,00,00,00,01,00,00,00,{delay},00,00,01,00,00,00,\\
  60,ea,00,00,00,00,00,00,00,00,00,00

"""


def nfo(machine, memory, build, model):
    rows = [("OS Name", "Microsoft Windows Server 2022 Datacenter"), ("Version", f"10.0.20348 Build {build}"), ("System Name", machine),
            ("System Manufacturer", "Microsoft Corporation"), ("System Model", model), ("System Type", "x64-based PC"),
            ("Processor", "Intel(R) Xeon(R) Platinum 8370C CPU @ 2.80GHz, 2794 Mhz, 4 Core(s), 8 Logical Processor(s)"),
            ("Installed Physical Memory (RAM)", memory), ("Time Zone", "Coordinated Universal Time"), ("Page File", "D:\\pagefile.sys")]
    data = "".join(f"<Data><Item><![CDATA[{k}]]></Item><Value><![CDATA[{v}]]></Value></Data>\n" for k, v in rows)
    services = "".join(f"<Data><Item><![CDATA[{k}]]></Item><Value><![CDATA[{v}]]></Value></Data>\n"
                       for k, v in [("OrderService", "Running, Auto"), ("W3SVC", "Running, Auto"), ("WerSvc", "Stopped, Manual"),
                                    ("filebeat", "Running, Auto")])
    return ('<?xml version="1.0"?>\n<MsInfo><Metadata><Version>8.0</Version><CreationUTC>10/14/2026 08:55:00</CreationUTC></Metadata>\n'
            f'<Category name="System Summary">\n{data}<Category name="Software Environment"><Category name="Services">\n{services}'
            '</Category></Category></Category></MsInfo>\n')


for machine, cfg in [("WEB01", dict(settings=APPSETTINGS, stdout="false", timeout="00:00:30", envx="", retries=3, cache=512, inix="",
                                    batch="5s", sampling=10, gc="00000001", regx="", checkout="00000000", delay="88,13",
                                    memory="16.0 GB", build="20348.2762", model="Virtual Machine (Standard_D4s_v5)")),
                     ("WEB02", dict(settings=web02_settings, stdout="true", timeout="00:02:00",
                                    envx='\n          <environmentVariable name="DOTNET_gcServer" value="0" />', retries=5, cache=128,
                                    inix="\n[debug]\ndump_on_crash=true\n", batch="1s", sampling=100, gc="00000000",
                                    regx='"DumpFolder"="C:\\\\ProgramData\\\\OrderService\\\\dumps"\n', checkout="00000001", delay="c8,af",
                                    memory="8.00 GB", build="20348.2700", model="Virtual Machine (Standard_D2s_v5)"))]:
    write_text(json.dumps(cfg["settings"], indent=2) + "\n", "configs", machine, "appsettings.json")
    write_text(WEB_CONFIG.replace("{stdout}", cfg["stdout"]).replace("{timeout}", cfg["timeout"]).replace("{extra}", cfg["envx"]),
               "configs", machine, "web.config", newline="\r\n")
    write_text(INI.format(retries=cfg["retries"], cache=cfg["cache"], extra=cfg["inix"]), "configs", machine, "orderservice.ini", newline="\r\n")
    write_text(COLLECTOR.format(batch=cfg["batch"], sampling=cfg["sampling"]), "configs", machine, "otel-collector.yaml")
    write_text(REG.format(gc=cfg["gc"], extra=cfg["regx"], checkout=cfg["checkout"], delay=cfg["delay"]),
               "configs", machine, "OrderService.reg", newline="\r\n", encoding="utf-16")
    write_text(nfo(machine, cfg["memory"], cfg["build"], cfg["model"]), "configs", machine, "msinfo32.nfo", newline="\r\n", encoding="utf-16")

# --------------------------------------------------------------------------
# Clock skew: the gateway forwards requests to WEB02 and both log the
# request's CorrelationId. WEB02's clock runs 3.742 s behind (its time
# service cannot reach a domain peer, see windows/WEB02/System.evtx), so
# WEB02 seems to receive each request before the gateway sent it.

SKEW_MS = 3742
crnd = random.Random(1026)
gw_lines, web_lines = [], []
SKEW_FROM = datetime(2026, 10, 14, 10, 0, 0, tzinfo=timezone.utc)
for t in rtimes(crnd, SKEW_FROM, SKEW_FROM + timedelta(minutes=30), 1.6):
    cid = guid(crnd)
    method, rp = crnd.choice([("GET", "/api/orders"), ("GET", f"/api/orders/{crnd.randint(88000, 89000)}"), ("POST", "/api/cart")])
    gw_lines.append({"@t": iso(t), "@mt": "Forwarding {RequestMethod} {RequestPath} to {Upstream} ({CorrelationId})", "RequestMethod": method,
                     "RequestPath": rp, "Upstream": "web02:8080", "CorrelationId": cid, "SourceContext": "Yarp.ReverseProxy.Forwarder.HttpForwarder",
                     "MachineName": "GATEWAY", "ProcessId": 1880, "Application": "Gateway"})
    seen = t + timedelta(milliseconds=crnd.randint(3, 12)) - timedelta(milliseconds=SKEW_MS)
    web_lines.append({"@t": iso(seen), "@mt": "Received {RequestMethod} {RequestPath} ({CorrelationId})", "RequestMethod": method, "RequestPath": rp,
                      "CorrelationId": cid, "SourceContext": "OrderService.Diagnostics.RequestTracing", "MachineName": "WEB02", "ProcessId": 4120,
                      "Application": "OrderService"})
for name, rows in [("gateway", gw_lines), ("web02", web_lines)]:
    write_text("".join(json.dumps(e) + "\n" for e in rows), "clock-skew", name, "requests-20261014.clef")

# --------------------------------------------------------------------------
# Archives: the notification service's rotated logs (app.log is the newest,
# app.log.3 the oldest), as a support engineer receives them: a zip of the
# current set and logrotate's gzip of the oldest file. Both are stored
# (uncompressed) so the bytes do not depend on the zlib build.

arnd = random.Random(1027)
notify = []
for t in rtimes(arnd, START, END, 9.0):
    s = stress(t)
    order = arnd.randint(88000, 89000)
    who = f"customer{order}@example.com"
    if arnd.random() < s * 0.35:
        notify.append((t, "WRN", f"SMTP send to {who} timed out after 30000 ms; retry {arnd.randint(1, 3)}/3",
                       {"OrderId": order, "Recipient": who}))
    elif arnd.random() < 0.003 + s * 0.01:
        notify.append((t, "ERR", f"Could not send order confirmation for order {order}", {"OrderId": order, "Recipient": who}))
    else:
        notify.append((t, "INF", f"Sent order confirmation for order {order} to {who}", {"OrderId": order, "Recipient": who}))
notify.insert(0, (START, "INF", "Application started — NotificationService 1.12.0", {}))


def notify_lines(rows):
    out = []
    for t, lvl, msg, props in rows:
        props = dict(props, MachineName="NOTIFY01", ProcessId=1544, Application="NotificationService")
        out.append(f"{t.strftime('%Y-%m-%d %H:%M:%S.')}{t.microsecond // 1000:03d} +00:00 [{lvl}] {msg} {json.dumps(props)}")
        if lvl == "ERR":
            out.append("System.Net.Mail.SmtpException: Failure sending mail.\n ---> System.IO.IOException: Unable to read data from the transport connection.\n"
                       "   at NotificationService.Mail.SmtpSender.SendAsync(MailMessage message) in /src/Mail/SmtpSender.cs:line 57")
    return "\n".join(out) + "\n"


rot = [START + timedelta(minutes=45 * i) for i in range(1, 4)]
parts = [[r for r in notify if lo <= r[0] < hi] for lo, hi in zip([START] + rot, rot + [END + timedelta(seconds=1)])]
names = ["app.log.3", "app.log.2", "app.log.1", "app.log"]
texts = {n: notify_lines(p).encode("utf-8") for n, p in zip(names, parts)}


def stored_zip(entries):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_STORED) as z:
        for name, data in entries:
            zi = zipfile.ZipInfo(name, date_time=(2026, 10, 14, 12, 0, 0))
            zi.create_system = 3
            zi.external_attr = 0o100644 << 16
            zi.compress_type = zipfile.ZIP_STORED
            z.writestr(zi, data)
    return buf.getvalue()


def stored_gzip(name, data):
    """RFC 1952 member with stored deflate blocks (BTYPE 00), mtime 0."""
    out = bytearray(b"\x1f\x8b\x08\x08\x00\x00\x00\x00\x00\x03" + name.encode("latin-1") + b"\x00")
    for i in range(0, max(len(data), 1), 65535):
        block = data[i:i + 65535]
        final = 1 if i + 65535 >= len(data) else 0
        out += struct.pack("<BHH", final, len(block), len(block) ^ 0xFFFF) + block
    out += struct.pack("<II", zlib.crc32(data) & 0xFFFFFFFF, len(data) & 0xFFFFFFFF)
    return bytes(out)


write_bytes(stored_zip([("app.log", texts["app.log"]), ("app.log.1", texts["app.log.1"]), ("app.log.2", texts["app.log.2"])]),
            "archives", "notify01", "notify01-logs-20261014.zip")
write_bytes(stored_gzip("app.log.3", texts["app.log.3"]), "archives", "notify01", "app.log.3.gz")

# --------------------------------------------------------------------------
# Runs (start/stop pairs) for the Runs view and timeline bands.
#  * batch01: the report service's export runs every 30 minutes. Most stop
#    cleanly; the 10:35 run is cut off by BATCH01's power loss (no "stopped"
#    line, the next start follows the reboot) and the 11:50 run is still going
#    when the log ends.
#  * shared: two image workers (Worker A and B, separate processes) write one
#    shared file, their runs interleaving; worker B loses its lease once and
#    starts its next batch without a stop.

rrnd = random.Random(1028)
run_rows = []


def rr(t, lvl, msg, **props):
    run_rows.append((t, lvl, msg, props))


def serilog_text(rows):
    out = []
    for t, lvl, msg, props in sorted(rows, key=lambda x: x[0]):
        out.append(f"{t.strftime('%Y-%m-%d %H:%M:%S.')}{t.microsecond // 1000:03d} +00:00 [{lvl}] {msg} {json.dumps(props)}")
    return "\n".join(out) + "\n"


BATCH = {"MachineName": "BATCH01", "Application": "ReportService", "SourceContext": "Reports.ExportWorker"}
rr(START + timedelta(minutes=4, seconds=58, milliseconds=420), "INF", "ReportService 2.3.0 ready; export schedule */30 at :05 and :35", **BATCH)
run = 0
SLOTS = [START + timedelta(minutes=5 + 30 * i) for i in range(4)] + [REPORT_UP + timedelta(seconds=20), START.replace(hour=11, minute=35),
                                                                   START.replace(hour=11, minute=52)]
for slot in SLOTS:
    run += 1
    t = slot + timedelta(milliseconds=rrnd.randint(5, 900))
    resumed = slot > REPORT_UP and slot < REPORT_UP + timedelta(minutes=1)
    what = "sales-daily, resumed after restart" if resumed else "sales-weekly, requested by ops" if slot == SLOTS[-1] else "sales-daily"
    rr(t, "INF", f"Report worker started: run {run} ({what})", RunId=run, **BATCH)
    length = timedelta(seconds=rrnd.randint(560, 760))
    if slot < POWER_LOSS < slot + timedelta(minutes=30):
        length += timedelta(minutes=12)  # month-end backlog: still exporting when the power goes
    stop = t + length
    rows = 0
    while True:
        t += timedelta(seconds=rrnd.uniform(20, 45))
        if t >= stop or t >= POWER_LOSS > slot or t >= END:
            break
        rows += rrnd.randint(800, 2400)
        if rrnd.random() < 0.08:
            rr(t, "WRN", f"Slow query for {what.split(',')[0]} page {rows // 1000} took {rrnd.randint(4000, 9000)} ms", RunId=run, **BATCH)
        else:
            rr(t, "INF", f"Exported {rows:,} rows of {what.split(',')[0]}", RunId=run, Rows=rows, **BATCH)
    if slot < POWER_LOSS < stop:
        continue  # cut off: no stop line
    if stop >= END:
        continue  # still running at the end of the log
    rr(stop, "INF", f"Report worker stopped: run {run} exported {rows:,} rows in {int(length.total_seconds())} s", RunId=run, Rows=rows, **BATCH)
rr(REPORT_UP, "INF", "ReportService 2.3.0 ready; export schedule */30 at :05 and :35", **BATCH)
rr(REPORT_UP + timedelta(milliseconds=380), "WRN", "Run 4 has no completion record; it will be resumed", **BATCH)
write_text(serilog_text(run_rows), "runs", "batch01", "report-service-20261014.txt")

run_rows = []
for worker, pid, gap in [("A", 7101, 140), ("B", 7102, 170)]:
    base = {"MachineName": "WORKER02", "Application": "ImageWorker", "Worker": worker, "ProcessId": pid, "SourceContext": "Images.BatchWorker"}
    t = START + timedelta(seconds=rrnd.randint(5, 60))
    batch = 0
    while t < END - timedelta(minutes=4):
        batch += 1
        rr(t, "INF", f"Worker {worker} started: batch {batch}", Batch=batch, **base)
        images = 0
        stop = t + timedelta(seconds=rrnd.randint(240, 600))
        lost = worker == "B" and batch == 6
        while t < stop:
            t += timedelta(seconds=rrnd.uniform(8, 20))
            images += rrnd.randint(4, 30)
            rr(t, "INF", f"Worker {worker} resized {images} images", Batch=batch, Images=images, **base)
            if lost and images > 60:
                rr(t + timedelta(milliseconds=120), "ERR", f"Worker {worker} lost its lease on batch {batch}; abandoning it", Batch=batch, **base)
                break
        if not lost:
            rr(t + timedelta(milliseconds=40), "INF", f"Worker {worker} stopped: batch {batch} done ({images} images)", Batch=batch, Images=images, **base)
        t += timedelta(seconds=rrnd.randint(30, gap))
write_text(serilog_text(run_rows), "runs", "shared", "workers-20261014.log")

# --------------------------------------------------------------------------
# Mixed sources (ADR 0020): a log server (rsyslog relay) collects what several
# products write into one shared file, and syslog from several hosts.
#
# linux/logserver01/apps-20261014.log mixes, line by line:
#   - Checkout (Serilog file sink: "{Timestamp} [{Level:u3}] {Message}
#     {Properties}"), APP01 pid 3120 recycled at 10:12 and back as pid 3388,
#     APP02 pid 3410, errors with stack traces;
#   - Inventory (log4net "%date [%thread] %level %logger - %message") on SRV2,
#     two processes (pid 7001, 7002) named in the message ("host=SRV2 pid=7001");
#   - Payments writing CLEF JSON lines (PAY01, pid 6100);
#   - a few haproxy lines in BSD syslog format that no default format reads.
# Read with profiles/logserver.debriefprofile.toml, routes split it into logical
# sources per product and host.
#
# linux/logserver01/syslog-20261014.log: RFC 5424 lines from three hosts
# (web01, db01, edge01) and several apps and pids, as the relay stores them.

mrnd = random.Random(2020)
mixed = []
MIXED_START = datetime(2026, 10, 14, 10, 0, 0, tzinfo=timezone.utc)
MIXED_END = datetime(2026, 10, 14, 10, 30, 0, tzinfo=timezone.utc)
MIXED_RECYCLE = datetime(2026, 10, 14, 10, 12, 30, 250000, tzinfo=timezone.utc)
MIXED_RESTART = MIXED_RECYCLE + timedelta(seconds=41)


def serilog_line(t, lvl, msg, props):
    stamp = t.strftime("%Y-%m-%d %H:%M:%S.") + f"{t.microsecond // 1000:03d} +00:00"
    return f"{stamp} [{lvl}] {msg} {json.dumps(props, separators=(', ', ': '))}"


def log4net_line(t, thread, lvl, logger, msg):
    stamp = t.strftime("%Y-%m-%d %H:%M:%S,") + f"{t.microsecond // 1000:03d}"
    return f"{stamp} [{thread}] {lvl:<5} {logger} - {msg}"


for machine, pid, gap in [("APP01", 3120, 9.0), ("APP02", 3410, 12.0)]:
    props = {"Application": "Checkout", "MachineName": machine, "ProcessId": pid}
    for t in rtimes(mrnd, MIXED_START, MIXED_END, gap):
        if machine == "APP01" and MIXED_RECYCLE <= t < MIXED_RESTART:
            continue
        if machine == "APP01" and t >= MIXED_RESTART:
            props = dict(props, ProcessId=3388)
        order = mrnd.randint(88000, 89999)
        if mrnd.random() < 0.08:
            mixed.append((t, serilog_line(t, "ERR", f"Payment capture failed for order {order}", dict(props, OrderId=order)) + "\n" + TIMEOUT_STACK))
        else:
            mixed.append((t, serilog_line(t, "INF", f"Order {order} placed in {mrnd.randint(12, 240)} ms", dict(props, OrderId=order))))
    # Every process ends the window healthy.
    mixed.append((MIXED_END, serilog_line(MIXED_END, "INF", "Checkout heartbeat", props)))
mixed.append((MIXED_RECYCLE, serilog_line(MIXED_RECYCLE, "INF", "Checkout stopping: application pool recycle",
                                          {"Application": "Checkout", "MachineName": "APP01", "ProcessId": 3120})))
mixed.append((MIXED_RESTART, serilog_line(MIXED_RESTART, "INF", "Checkout 4.2.1 starting",
                                          {"Application": "Checkout", "MachineName": "APP01", "ProcessId": 3388})))
for pid, thread in [(7001, "sync-1"), (7002, "sync-2")]:
    for t in rtimes(mrnd, MIXED_START, MIXED_END, 20.0):
        r = mrnd.random()
        if r < 0.15:
            mixed.append((t, log4net_line(t, thread, "WARN", "Inventory.Sync", f"host=SRV2 pid={pid} Stock feed slow: {mrnd.randint(2, 9)} s")))
        elif r < 0.2:
            mixed.append((t, log4net_line(t, thread, "ERROR", "Inventory.Sync", f"host=SRV2 pid={pid} Stock feed failed") + "\n"
                          "java.io.IOException: Connection reset by peer\n\tat inventory.Feed.read(Feed.java:88)"))
        else:
            mixed.append((t, log4net_line(t, thread, "INFO", "Inventory.Sync", f"host=SRV2 pid={pid} Synced {mrnd.randint(10, 400)} items")))
    mixed.append((MIXED_END, log4net_line(MIXED_END, thread, "INFO", "Inventory.Sync", f"host=SRV2 pid={pid} Idle")))
for t in rtimes(mrnd, MIXED_START, MIXED_END, 60.0):
    clef = {"@t": iso_ms(t), "@mt": "Captured {PaymentId} for {Amount}", "PaymentId": f"p-{mrnd.randint(1000, 9999)}",
            "Amount": mrnd.randint(5, 900), "Application": "Payments", "MachineName": "PAY01", "ProcessId": 6100}
    if mrnd.random() < 0.1:
        clef["@l"] = "Warning"
    mixed.append((t, json.dumps(clef, separators=(",", ":"))))
for t in rtimes(mrnd, MIXED_START, MIXED_END, 600.0):
    mixed.append((t, f"{t.strftime('%b')} {t.day:2d} {t.strftime('%H:%M:%S')} edge01 haproxy[812]: backend orders has no server available!"))
mixed.sort(key=lambda x: x[0])
write_text("# logserver01: forwarded application logs (rsyslog omfile, raw messages)\n" + "\n".join(l for _, l in mixed) + "\n",
           "linux", "logserver01", "apps-20261014.log")

LOGSERVER_PROFILE = '''name = "Log server (logserver01)"

# Inventory names its host and process in the message.
[[extract]]
name = "inventory-identity"
from = "@message"
regex = 'host=(?P<Host>\\S+) pid=(?P<Pid>\\d+) '
when = [{ field = "Logger", op = "contains", value = "Inventory" }]

# One logical source per product and host.
[[route]]
when = [{ field = "Application", op = "exists" }]
source = "{Application}@{MachineName}"

[[route]]
when = [{ field = "Logger", op = "contains", value = "Inventory" }]
source = "Inventory@{Host}"
'''
write_text(LOGSERVER_PROFILE, "profiles", "logserver.debriefprofile.toml")

lrnd = random.Random(2021)
relay = []


def relay5424(t, sev, host, app, pid, msg):
    relay.append((t, f"<{8 * 3 + sev}>1 {iso_ms(t)} {host} {app} {pid} - - {msg}"))


apps = [("web01", "nginx", [2211, 2212]), ("web01", "sshd", [30412]), ("db01", "postgres", [1880, 1893]), ("edge01", "haproxy", [812])]
for host, app, pids in apps:
    for t in rtimes(lrnd, MIXED_START, MIXED_END, 25.0):
        pid = lrnd.choice(pids)
        sev = 4 if lrnd.random() < 0.1 else 6
        relay5424(t, sev, host, app, pid, f"{app} worker {pid}: {'slow request' if sev == 4 else 'request served'} in {lrnd.randint(1, 900)} ms")
relay.sort(key=lambda x: x[0])
write_text("\n".join(l for _, l in relay) + "\n", "linux", "logserver01", "syslog-20261014.log")

# --------------------------------------------------------------------------
# Share: an .debriefproj project over the samples (paths relative to it, the
# OrderService profile, WEB02's clock offset) and an .debriefz bundle of the
# clock-skew logs in the format Workspace::write_bundle writes (project,
# profile, dashboards, logs and a manifest with SHA-256 per file).

with open(path("profiles", "orderservice.debriefprofile.toml"), encoding="utf-8", newline="") as f:
    SAMPLE_PROFILE = f.read()
project = {"version": 2, "name": "OrderService incident 14 Oct 2026",
           "sources": ["../serilog-clef", "../serilog-json", "../serilog-text", "../log4net", "../csv", "../clock-skew"],
           "removed": [], "profile": SAMPLE_PROFILE, "ui": {},
           "clock": {"../clock-skew/web02/requests-20261014.clef": SKEW_MS}}
write_text(json.dumps(project, indent=2, ensure_ascii=False), "share", "orderservice-incident.debriefproj")

BUNDLE_PROFILE = 'name = "WEB02 clock skew (shared)"\n\n[instance]\nkey = ["MachineName", "ProcessId"]\n'
entries = []
files = []
for i, name in enumerate(["gateway", "web02"]):
    with open(path("clock-skew", name, "requests-20261014.clef"), "rb") as f:
        files.append((f"logs/{i}/requests-20261014.clef", "log", f.read()))
files.append(("profile.toml", "profile", BUNDLE_PROFILE.encode("utf-8")))
files.append(("dashboards.json", "dashboards", b"[]"))
bundle_project = {"version": 2, "name": "WEB02 clock skew", "sources": ["logs"], "removed": [], "profile": BUNDLE_PROFILE, "ui": {},
                  "clock": {"logs/1/requests-20261014.clef": SKEW_MS}}
files.append(("project.debriefproj", "project", json.dumps(bundle_project, indent=2).encode("utf-8")))
manifest = {"format": "debrief-share", "schema": 1, "name": "WEB02 clock skew", "createdAt": ms_of(END), "debriefVersion": "0.1.0", "content": "raw",
            "redacted": False, "redaction": {}, "events": None,
            "entries": [{"path": p, "kind": k, "bytes": len(d), "sha256": hashlib.sha256(d).hexdigest()} for p, k, d in files], "skipped": []}
write_bytes(stored_zip([(p, d) for p, _, d in files] + [("manifest.json", json.dumps(manifest, indent=2).encode("utf-8"))]),
            "share", "web02-clock-skew.debriefz")

# --------------------------------------------------------------------------
# UMDH: three heap snapshots of OrderService on WEB02 (pid 4120) taken while it
# leaks (`umdh -p:4120 -f:...` at 10:30, 10:35 and 10:40) and UMDH's own diff of
# the first and the last (`umdh snap-1030.txt snap-1040.txt`, hexadecimal). The
# order cache grows by about 10 MB per snapshot (the leak), payment retries
# grow a little, sessions stay level, a 32 MB report buffer is one large
# allocation, and PdfLib has no symbols (raw addresses placed in its module).

urnd = random.Random(1023)
UMDH_MODULES = [
    (0x7FF6A0000000, 0x40000, r"C:\Program Files\Shop\OrderService.exe"),
    (0x7FF6B0000000, 0x120000, r"C:\Program Files\Shop\Shop.Orders.dll"),
    (0x7FF6B2000000, 0x80000, r"C:\Program Files\Shop\Shop.Payments.dll"),
    (0x7FF6B4000000, 0x60000, r"C:\Program Files\Shop\Shop.Reports.dll"),
    (0x7FF6B6000000, 0x200000, r"C:\Program Files\Shop\PdfLib.dll"),
    (0x7FFB1C000000, 0x1F8000, r"C:\Windows\SYSTEM32\ntdll.dll"),
    (0x7FFB1A000000, 0x2C0000, r"C:\Windows\System32\KERNELBASE.dll"),
    (0x7FFB18000000, 0x120000, r"C:\Windows\System32\ucrtbase.dll"),
]
ALLOC = ["ntdll!RtlpAllocateHeapInternal+0000000E", "ntdll!RtlAllocateHeap+00000021", "ucrtbase!_malloc_base+00000036"]
UMDH_STACKS = [
    # back trace id, allocation size, counts per snapshot, frames below the allocator
    (0xA2C3, 0x10000, [160, 320, 480], ["Shop.Orders!OrderCache::Add+0000004A (c:\\src\\orders\\ordercache.cpp, 88)",
                                        "Shop.Orders!OrderService::Handle+00000112 (c:\\src\\orders\\orderservice.cpp, 214)",
                                        "OrderService!wmain+0000002B", "KERNEL32!BaseThreadInitThunk+00000014", "ntdll!RtlUserThreadStart+00000021"]),
    (0x77B1, 0x800, [50, 82, 121], ["Shop.Payments!Capture::Retry+0000003C (c:\\src\\payments\\capture.cpp, 57)",
                                    "Shop.Payments!Capture::Run+000000A0", "OrderService!wmain+0000002B"]),
    (0x1F04, 0x200, [400, 412, 396], ["Shop.Orders!Session::Create+00000019 (c:\\src\\orders\\session.cpp, 31)",
                                      "Shop.Orders!OrderService::Handle+00000040", "OrderService!wmain+0000002B"]),
    (0x0C9A, 0x2000000, [1, 1, 1], ["Shop.Reports!ReportBuffer::Allocate+00000011", "Shop.Reports!Report::Render+00000208",
                                    "OrderService!wmain+0000002B"]),
    (0x5E12, 0x1000, [30, 30, 30], ["7FF6B6012A40", "7FF6B6004410", "Shop.Reports!Report::Render+00000300", "OrderService!wmain+0000002B"]),
]
UMDH_HEAP = 0x000001E0A3C40000
UMDH_TIMES = ["1030", "1035", "1040"]


def umdh_header():
    lines = ["// Debug library initialized ...",
             "DBGHELP: OrderService - private symbols & lines ",
             "         C:\\Program Files\\Shop\\OrderService.pdb",
             "DBGHELP: ntdll - public symbols  ",
             "         c:\\symbols\\ntdll.pdb\\9C8A0B5D4E2F4A1B8C3D2E1F0A9B8C7D1\\ntdll.pdb",
             "DBGHELP: PdfLib - export symbols",
             "//                                                                          ",
             "// Loaded modules:",
             "//     Base Size Module"]
    lines += ["//      %12X %8X %s" % m for m in UMDH_MODULES]
    lines += ["//", "// Process modules enumerated.", ""]
    return lines


def umdh_snapshot(snap):
    lines = umdh_header()
    lines += ["*- - - - - - - - - - Start of data for heap @ %016X - - - - - - - - - -" % UMDH_HEAP, "",
              "REQUESTED bytes + OVERHEAD at ADDRESS by BackTraceID",
              "     STACK if not already dumped.", "",
              "*- - - - - - - - - - Heap %016X Hogs - - - - - - - - - -" % UMDH_HEAP, ""]
    address = UMDH_HEAP + 0x10000
    for trace, size, counts, frames in UMDH_STACKS:
        for i in range(counts[snap]):
            lines.append("%08X bytes + %08X at %016X by BackTrace%04X" % (size, 0x30 if size >= 0x10000 else 0x10, address, trace))
            address += size + 0x40 + urnd.randrange(0, 4) * 0x10
            if i == 0:
                lines += ["\t" + f for f in ALLOC + frames]
            lines.append("")
    lines += ["*- - - - - - - - - - End of data for heap @ %016X - - - - - - - - - -" % UMDH_HEAP, ""]
    return "\n".join(lines)


def umdh_diff(old, new):
    lines = ["//", "// Each log entry has the following syntax:", "//",
             "// + BYTES_DELTA (NEW_BYTES - OLD_BYTES) NEW_COUNT allocs BackTrace TRACEID",
             "// + COUNT_DELTA (NEW_COUNT - OLD_COUNT) BackTrace TRACEID allocations",
             "//     ... stack trace ...", "//", "// where:", "//",
             "//     BYTES_DELTA - increase in bytes between before and after log",
             "//     NEW_BYTES - bytes in after log", "//     OLD_BYTES - bytes in before log",
             "//     COUNT_DELTA - increase in allocations between before and after log",
             "//     NEW_COUNT - number of allocations in after log",
             "//     OLD_COUNT - number of allocations in before log",
             "//     TRACEID - decimal index of the stack trace in the trace database",
             "//         (can be used to search for allocation instances in the original",
             "//         UMDH logs).", "//", ""]
    total = 0
    for trace, size, counts, frames in sorted(UMDH_STACKS, key=lambda s: -(s[2][new] - s[2][old]) * s[1]):
        nb, ob = counts[new] * size, counts[old] * size
        if nb == ob:
            continue
        total += nb - ob
        sign = "+" if nb >= ob else "-"
        lines.append("%s %7X ( %7X - %7X) %6X allocs\tBackTrace%04X" % (sign, abs(nb - ob), nb, ob, counts[new], trace))
        lines.append("%s %7X ( %7X - %7X)\tBackTrace%04X\tallocations" % (sign, abs(counts[new] - counts[old]), counts[new], counts[old], trace))
        lines.append("")
        lines += ["\t" + f for f in ALLOC + frames]
        lines.append("")
    overhead = 0x30 * 320
    lines.append("Total increase == %X requested + %X overhead = %X" % (total, overhead, total + overhead))
    lines.append("")
    return "\n".join(lines)


for i, hhmm in enumerate(UMDH_TIMES):
    write_text(umdh_snapshot(i), "dumps", "umdh", f"OrderService-4120-snap-{hhmm}.txt", newline="\r\n", encoding="ascii")
write_text(umdh_diff(0, 2), "dumps", "umdh", "OrderService-4120-diff-1030-1040.txt", newline="\r\n", encoding="ascii")

# --------------------------------------------------------------------------
# Built-in parsers without a sample of their own (ADR 0026): Android logcat,
# Spark/HDFS log4j layouts, the Apache httpd error log, NLog's default layout and
# the Microsoft.Extensions.Logging simple console formatter. Each tells its part
# of the incident, has several levels, multi-line stack traces and a few lines
# the parser's regexes do not match (kept with the event before them). Own random
# generators, so the files above do not change when this section does.

# Android logcat (threadtime) from the shop app on a QA phone: requests to the
# API slow down from 10:30, the app crashes on the failed payment at 10:41:24
# (pid 4211, every stack frame its own line as logcat writes them) and is
# started again as pid 4588. "--------- beginning of crash" is logcat's buffer
# divider, not an event. logcat writes no year: Debrief takes it from the file date.
grnd = random.Random(3001)


def logcat(t, pid, tid, lvl, tag, msg):
    return f"{t.strftime('%m-%d %H:%M:%S')}.{t.microsecond // 1000:03d} {pid:5d} {tid:5d} {lvl} {tag:<8}: {msg}"


lc = [logcat(START, 1702, 1702, "I", "ActivityManager", "Start proc 4211:com.contoso.shop/u0a77 for activity {com.contoso.shop/.MainActivity}")]
lc.append(logcat(START + timedelta(milliseconds=180), 4211, 4211, "V", "ShopApp", "onCreate: theme=Light, locale=en-GB"))
APP_CRASH = CRASH + timedelta(milliseconds=410)
crashed, cut_lc = False, False
for t in rtimes(grnd, START + timedelta(seconds=1), END, 40.0):
    if not crashed and t >= APP_CRASH:
        crashed = True
        c = APP_CRASH
        lc.append("--------- beginning of crash")
        lc.append(logcat(c, 4211, 4211, "E", "AndroidRuntime", "FATAL EXCEPTION: main"))
        lc.append(logcat(c, 4211, 4211, "E", "AndroidRuntime", "Process: com.contoso.shop, PID: 4211"))
        for frame in ["java.lang.IllegalStateException: Payment capture failed for order 88412",
                      "\tat com.contoso.shop.checkout.PayViewModel.capture(PayViewModel.kt:88)",
                      "\tat com.contoso.shop.checkout.PayFragment.onPayClicked(PayFragment.kt:51)",
                      "\tat android.view.View.performClick(View.java:7659)",
                      "Caused by: java.net.SocketTimeoutException: timeout",
                      "\tat okhttp3.internal.http2.Http2Stream.takeHeaders(Http2Stream.kt:149)",
                      "\t... 12 more"]:
            lc.append(logcat(c, 4211, 4211, "E", "AndroidRuntime", frame))
        lc.append(logcat(c + timedelta(milliseconds=6), 4211, 4211, "I", "Process", "Sending signal. PID: 4211 SIG: 9"))
        lc.append(logcat(c + timedelta(milliseconds=90), 1702, 1735, "W", "ActivityManager", "Process com.contoso.shop has crashed too many times: killing!"))
        lc.append(logcat(c + timedelta(milliseconds=95), 4211, 4249, "F", "libc", "Fatal signal 9 (SIGKILL), code 0 in tid 4249 (OkHttp Dispatch)"))
        lc.append(logcat(RESTART, 1702, 1702, "I", "ActivityManager", "Start proc 4588:com.contoso.shop/u0a77 for activity {com.contoso.shop/.MainActivity}"))
    pid, tid = (4211, 4230) if t < APP_CRASH else (4588, 4602)
    if APP_CRASH <= t < RESTART + timedelta(seconds=30):
        continue
    s = stress(t) if t < CRASH else 0.0
    r = grnd.random()
    if r < 0.02 + s * 0.15:
        lc.append(logcat(t, pid, tid, "E", "OkHttp", "<-- HTTP FAILED: java.net.SocketTimeoutException: timeout"))
        lc.append(logcat(t, pid, tid, "W", "System.err", "java.net.SocketTimeoutException: timeout"))
        lc.append(logcat(t, pid, tid, "W", "System.err", "\tat okhttp3.internal.http2.Http2Stream$StreamTimeout.newTimeoutException(Http2Stream.kt:675)"))
        lc.append(logcat(t, pid, tid, "W", "System.err", "\tat com.contoso.shop.api.OrdersApi.list(OrdersApi.kt:42)"))
    elif r < 0.05 + s * 0.4:
        lc.append(logcat(t, pid, tid, "W", "OkHttp", f"Slow response from /api/orders ({int(900 + s * 4000 + grnd.uniform(0, 400))} ms)"))
    elif r < 0.25:
        lc.append(logcat(t, pid, pid, "D", "ShopApp", f"Cart updated: {grnd.randint(1, 6)} items"))
    elif r < 0.3:
        lc.append(logcat(t, 1702, 1720, "D", "WindowManager", "Changing focus to Window{com.contoso.shop/.CartActivity}"))
    else:
        lc.append(logcat(t, pid, tid, "I", "OkHttp", f"<-- 200 https://api.contoso.example/api/orders ({grnd.randint(80, 400)}ms, {grnd.randint(1, 9)}.{grnd.randint(0, 9)}kB body)"))
    if not cut_lc and t > datetime(2026, 10, 14, 11, 15, tzinfo=timezone.utc):
        cut_lc = True
        # A line cut short when the log buffer was read while being written.
        lc.append(logcat(t, pid, tid, "I", "OkHttp", "<-- 200 https://api.contoso.example/api/ord")[:31])
write_text("\n".join(lc) + "\n", "android-logcat", "qa-pixel07", "logcat-20261014.txt")

# Spark driver log of the nightly order export (default log4j layout
# "yy/MM/dd HH:mm:ss LEVEL Logger: message"): an executor loses its connection
# to the order database during the incident, the task is retried and the stage
# finishes. The JVM's stdout ("Picked up JAVA_TOOL_OPTIONS") is not log4j.
hrnd2 = random.Random(3002)


def spark(t, lvl, logger, msg):
    return f"{t.strftime('%y/%m/%d %H:%M:%S')} {lvl} {logger}: {msg}"


sp = [spark(START + timedelta(seconds=2), "INFO", "SparkContext", "Running Spark version 3.5.1"),
      spark(START + timedelta(seconds=2), "INFO", "SparkContext", "Submitted application: orders-export"),
      spark(START + timedelta(seconds=4), "WARN", "NativeCodeLoader", "Unable to load native-hadoop library for your platform... using builtin-java classes where applicable"),
      "Picked up JAVA_TOOL_OPTIONS: -Dfile.encoding=UTF-8"]
stage, tid = 0, 0
for t in rtimes(hrnd2, START + timedelta(seconds=10), END, 60.0):
    s = stress(t) if t < CRASH else 0.0
    r = hrnd2.random()
    if r < 0.08:
        stage += 1
        sp.append(spark(t, "INFO", "DAGScheduler", f"Submitting {hrnd2.randint(4, 16)} missing tasks from ResultStage {stage} (MapPartitionsRDD[{stage * 3}] at save at OrdersExport.scala:57)"))
    elif r < 0.02 + s * 0.5:
        tid += 1
        sp.append(spark(t, "ERROR", "Executor", f"Exception in task 0.0 in stage {stage}.0 (TID {tid})"))
        sp += ["java.sql.SQLTransientConnectionException: HikariPool-1 - Connection is not available, request timed out after 30000ms.",
               "\tat com.zaxxer.hikari.pool.HikariPool.createTimeoutException(HikariPool.java:696)",
               "\tat com.contoso.export.OrdersReader.next(OrdersReader.scala:88)",
               "\tat org.apache.spark.scheduler.ResultTask.runTask(ResultTask.scala:93)",
               "Caused by: com.microsoft.sqlserver.jdbc.SQLServerException: Transaction (Process ID 61) was deadlocked on lock resources with another process and has been chosen as the deadlock victim.",
               "\tat com.microsoft.sqlserver.jdbc.SQLServerException.makeFromDatabaseError(SQLServerException.java:265)",
               "\t... 14 more"]
        sp.append(spark(t + timedelta(seconds=1), "WARN", "TaskSetManager", f"Lost task 0.0 in stage {stage}.0 (TID {tid}) (10.20.0.17 executor 2): java.sql.SQLTransientConnectionException"))
    elif r < 0.25:
        sp.append(spark(t, "INFO", "Executor", f"Running task {hrnd2.randint(0, 15)}.0 in stage {stage}.0 (TID {tid + 1})"))
    elif r < 0.4:
        tid += 1
        sp.append(spark(t, "INFO", "TaskSetManager", f"Finished task {hrnd2.randint(0, 15)}.0 in stage {stage}.0 (TID {tid}) in {hrnd2.randint(800, 9000)} ms on 10.20.0.{hrnd2.randint(11, 19)} (executor {hrnd2.randint(1, 4)})"))
    elif r < 0.5:
        sp.append(spark(t, "DEBUG", "BlockManager", f"Putting block rdd_{stage * 3}_{hrnd2.randint(0, 15)} with replication 1"))
    else:
        sp.append(spark(t, "INFO", "MemoryStore", f"Block broadcast_{stage} stored as values in memory (estimated size {hrnd2.randint(20, 400)}.{hrnd2.randint(0, 9)} KiB, free 2.1 GiB)"))
sp.append(spark(END - timedelta(seconds=5), "INFO", "SparkContext", "Successfully stopped SparkContext"))
write_text("\n".join(sp) + "\n", "hadoop-spark", "analytics01", "spark-orders-export.log")

# HDFS DataNode in the older layout "yyMMdd HHmmss <thread> LEVEL Logger: message".
drnd = random.Random(3003)


def hdfs(t, thread, lvl, logger, msg):
    return f"{t.strftime('%y%m%d %H%M%S')} {thread} {lvl} {logger}: {msg}"


dn = [hdfs(START, 1, "INFO", "dfs.DataNode", "STARTUP_MSG: Starting DataNode, host = dn01.contoso.example/10.20.0.31")]
for t in rtimes(drnd, START + timedelta(seconds=5), END, 100.0):
    r = drnd.random()
    blk = f"blk_{drnd.randint(10**17, 10**18)}"
    if r < 0.04:
        dn.append(hdfs(t, drnd.randint(100, 300), "ERROR", "dfs.DataNode", f"DatanodeRegistration(10.20.0.31:50010):DataXceiver: Got exception while serving {blk} to /10.20.0.17"))
        dn += ["java.net.SocketTimeoutException: 480000 millis timeout while waiting for channel to be ready for write.",
               "\tat org.apache.hadoop.net.SocketIOWithTimeout.waitForIO(SocketIOWithTimeout.java:246)",
               "\tat org.apache.hadoop.dfs.DataNode$BlockSender.sendBlock(DataNode.java:1873)"]
    elif r < 0.12:
        dn.append(hdfs(t, drnd.randint(100, 300), "WARN", "dfs.DataNode", f"Slow BlockReceiver write packet to mirror took {drnd.randint(300, 2000)}ms"))
    elif r < 0.35:
        dn.append(hdfs(t, drnd.randint(100, 300), "INFO", "dfs.DataNode", f"Receiving block {blk} src: /10.20.0.{drnd.randint(11, 19)}:{drnd.randint(40000, 60000)} dest: /10.20.0.31:50010"))
    elif r < 0.6:
        dn.append(hdfs(t, drnd.randint(100, 300), "INFO", "dfs.DataNode$PacketResponder", f"PacketResponder {drnd.randint(0, 2)} for block {blk} terminating"))
    else:
        dn.append(hdfs(t, drnd.randint(100, 300), "INFO", "dfs.FSNamesystem", f"BLOCK* NameSystem.addStoredBlock: blockMap updated: 10.20.0.31:50010 is added to {blk} size {drnd.randint(10**6, 6 * 10**7)}"))
write_text("\n".join(dn) + "\n", "hadoop-spark", "dn01", "hadoop-hdfs-datanode-dn01.log")

# Apache httpd 2.4 error log of a second edge in front of the web farm: proxy
# timeouts to WEB02 from 10:30, refused connections while it is down (10:41:24
# to 10:42:10), a CGI report script whose Python traceback reaches the log
# without a prefix (stderr of the script), and one line cut off by a full disk.
erand = random.Random(3004)


def apache(t, module, lvl, pid, tid, msg, client=None):
    c = f" [client {client}]" if client else ""
    return f"[{t.strftime('%a %b %d %H:%M:%S')}.{t.microsecond:06d} {t.year}] [{module}:{lvl}] [pid {pid}:tid {tid}]{c} {msg}"


ap = [apache(START, "mpm_event", "notice", 812, 140245, "AH00489: Apache/2.4.58 (Ubuntu) OpenSSL/3.0.13 configured -- resuming normal operations"),
      apache(START, "core", "notice", 812, 140245, "AH00094: Command line: '/usr/sbin/apache2'")]
cgi_done, cut = False, False
for t in rtimes(erand, START + timedelta(seconds=3), END, 25.0):
    pid, tid = erand.choice([(9041, 140112), (9042, 140118), (9077, 140131)])
    client = f"10.0.{erand.randint(1, 4)}.{erand.randint(2, 250)}:{erand.randint(40000, 65000)}"
    s = stress(t) if t < CRASH else 0.0
    down = CRASH <= t < RESTART
    r = erand.random()
    if down:
        ap.append(apache(t, "proxy", "error", pid, tid, "(111)Connection refused: AH00957: http: attempt to connect to 10.0.0.12:8080 (web02.contoso.example:8080) failed", client))
        ap.append(apache(t, "proxy_http", "error", pid, tid, "AH01114: HTTP: failed to make connection to backend: web02.contoso.example", client))
    elif r < s * 0.6:
        ap.append(apache(t, "proxy_http", "error", pid, tid, "(70007)The timeout specified has expired: AH01102: error reading status line from remote server web02.contoso.example:8080", client))
    elif r < 0.03 + s * 0.3:
        ap.append(apache(t, "proxy", "warn", pid, tid, "AH01144: No protocol handler was valid for the URL /favicon.ico (scheme 'h2c'). If you are using a DSO version of mod_proxy, make sure the proxy submodules are included in the configuration using LoadModule.", client))
    elif r < 0.1:
        ap.append(apache(t, "ssl", "info", pid, tid, "AH01964: Connection to child 3 established (server shop.contoso.example:443)", client))
    elif r < 0.13 and not cgi_done and t > INCIDENT:
        cgi_done = True
        ap += ["Traceback (most recent call last):",
               '  File "/usr/lib/cgi-bin/order-report.cgi", line 42, in <module>',
               "    rows = fetch_orders(since)",
               "ConnectionRefusedError: [Errno 111] Connection refused"]
        ap.append(apache(t, "cgid", "error", pid, tid, "End of script output before headers: order-report.cgi", client))
    elif r < 0.2:
        ap.append(apache(t, "authz_core", "debug", pid, tid, "mod_authz_core.c(815): AH01626: authorization result of Require all granted: granted", client))
    elif r < 0.25:
        ap.append(apache(t, "core", "info", pid, tid, "AH00128: File does not exist: /var/www/shop/robots.txt", client))
    else:
        continue
    if not cut and t > datetime(2026, 10, 14, 11, 15, tzinfo=timezone.utc):
        cut = True
        ap.append(apache(t, "core", "warn", pid, tid, "AH00045: child process 9077 still did not exit, sending a SIGTERM")[:40])
ap.append(apache(END, "mpm_event", "notice", 812, 140245, "AH00491: caught SIGTERM, shutting down"))
write_text("\n".join(ap) + "\n", "apache-error", "edge02", "error.log")

# NLog's default file layout "${longdate}|${level:uppercase=true}|${logger}|
# ${message:withexception=true}" from the notification scheduler: the exception
# follows the message after a "|" and continues on the next lines, as NLog 6
# writes it (see samples/real-libraries). One line was written by a
# Console.WriteLine left in a job.
nrnd2 = random.Random(3005)


def nlog(t, lvl, logger, msg):
    return f"{t.strftime('%Y-%m-%d %H:%M:%S')}.{t.microsecond // 100:04d}|{lvl}|{logger}|{msg}"


NLOG_SMTP = ("|System.Net.Mail.SmtpException: Failure sending mail.\n"
             " ---> System.IO.IOException: Unable to read data from the transport connection: An existing connection was forcibly closed by the remote host.\n"
             "   at System.Net.Mail.SmtpConnection.GetConnection(String host, Int32 port)\n"
             "   --- End of inner exception stack trace ---\n"
             "   at System.Net.Mail.SmtpClient.Send(MailMessage message)\n"
             "   at Contoso.Notify.Jobs.SendOrderMails.Execute(IJobExecutionContext context) in D:\\build\\Notify\\Jobs\\SendOrderMails.cs:line 64")
nl = [nlog(START, "INFO", "Contoso.Notify.Program", "Scheduler 2.4.0 started; 3 jobs registered")]
paused = stray = False
for t in rtimes(nrnd2, START + timedelta(seconds=2), END, 60.0):
    if not paused and t > CRASH:
        paused = True
        nl.append(nlog(CRASH + timedelta(milliseconds=900), "FATAL", "Contoso.Notify.Jobs.SendOrderMails", "Job SendOrderMails failed 5 times in a row and is paused" + NLOG_SMTP))
    s = stress(t) if t < CRASH else 0.0
    r = nrnd2.random()
    if r < 0.01 + s * 0.2:
        nl.append(nlog(t, "ERROR", "Contoso.Notify.Jobs.SendOrderMails", f"Could not send the confirmation for order {nrnd2.randint(88000, 89000)}" + NLOG_SMTP))
    elif r < 0.04 + s * 0.3:
        nl.append(nlog(t, "WARN", "Contoso.Notify.Jobs.SendOrderMails", f"Mail queue is backing up: {int(20 + s * 800)} messages"))
    elif r < 0.1:
        nl.append(nlog(t, "TRACE", "Quartz.Core.QuartzSchedulerThread", f"Batch acquisition of {nrnd2.randint(0, 2)} triggers"))
    elif r < 0.3:
        nl.append(nlog(t, "DEBUG", "Contoso.Notify.Templates", f"Rendered template order-confirmation.de-DE in {nrnd2.randint(2, 40)} ms"))
    else:
        nl.append(nlog(t, "INFO", "Contoso.Notify.Jobs.SendOrderMails", f"Sent {nrnd2.randint(1, 30)} order confirmations"))
    if not stray and t > datetime(2026, 10, 14, 10, 0, tzinfo=timezone.utc):
        stray = True
        nl.append(f"DEBUG queue={nrnd2.randint(0, 900)}")
write_text("\n".join(nl) + "\n", "nlog", "notify01", "notify-scheduler-20261014.log", newline="\r\n")

# Microsoft.Extensions.Logging simple console output (TimestampFormat
# "yyyy-MM-dd HH:mm:ss.fff ") of the payments adapter container: "level:
# Category[EventId]" with the message indented below and the exception after
# it, as .NET 10 writes it (see samples/real-libraries). Capture timeouts from
# 10:30, a critical circuit-breaker event, and one line a library wrote with
# Console.WriteLine.
prnd2 = random.Random(3006)
MEL_INDENT = "      "


def mel(t, lvl, category, event_id, msg, exc=None):
    lines = [f"{t.strftime('%Y-%m-%d %H:%M:%S')}.{t.microsecond // 1000:03d} {lvl}: {category}[{event_id}]"]
    lines += [MEL_INDENT + m for m in msg.split("\n")]
    if exc:
        lines += [MEL_INDENT + x for x in exc.split("\n")]
    return "\n".join(lines)


MEL_CAPTURE = ("Polly.Timeout.TimeoutRejectedException: The delegate executed asynchronously through TimeoutPolicy did not complete within the timeout.\n"
               " ---> System.Threading.Tasks.TaskCanceledException: A task was canceled.\n"
               "   at System.Net.Http.HttpClient.HandleFailure(Exception e, Boolean telemetryStarted, HttpResponseMessage response)\n"
               "   --- End of inner exception stack trace ---\n"
               "   at Polly.Timeout.AsyncTimeoutEngine.ImplementationAsync[TResult](Func`3 action, Context context)\n"
               "   at Contoso.Payments.Adapter.CaptureClient.CaptureAsync(CaptureRequest request, CancellationToken ct) in /src/Adapter/CaptureClient.cs:line 57")
mp = [mel(START, "info", "Microsoft.Hosting.Lifetime", 14, "Now listening on: http://[::]:8080"),
      mel(START, "info", "Microsoft.Hosting.Lifetime", 0, "Application started. Press Ctrl+C to shut down."),
      mel(START, "info", "Microsoft.Hosting.Lifetime", 0, "Hosting environment: Production\nContent root path: /app")]
breaker = stray = False
later = []  # the circuit breaker's own follow-ups, written when their time comes
for t in rtimes(prnd2, START + timedelta(seconds=1), END, 50.0):
    while later and later[0][0] <= t:
        at, lvl, eid, msg = later.pop(0)
        mp.append(mel(at, lvl, "Contoso.Payments.Adapter.CircuitBreaker", eid, msg))
    s = stress(t) if t < CRASH else 0.0
    r = prnd2.random()
    order = prnd2.randint(88000, 89000)
    if not breaker and s > 0.9:
        breaker = True
        mp.append(mel(t, "crit", "Contoso.Payments.Adapter.CircuitBreaker", 9001, "Circuit to the payment provider opened after 20 consecutive failures; captures are rejected for 30 s"))
        later += [(t + timedelta(seconds=30), "warn", 9002, "Circuit to the payment provider is half-open; trying one capture"),
                  (t + timedelta(seconds=31, milliseconds=204), "info", 9003, "Circuit to the payment provider closed")]
    elif r < 0.005 + s * 0.25:
        mp.append(mel(t, "fail", "Contoso.Payments.Adapter.CaptureClient", 1001, f"Capture for order {order} failed", MEL_CAPTURE))
    elif r < 0.03 + s * 0.4:
        mp.append(mel(t, "warn", "Contoso.Payments.Adapter.CaptureClient", 1002, f"Capture for order {order} took {int(1500 + s * 25000)} ms"))
    elif r < 0.15:
        mp.append(mel(t, "dbug", "Microsoft.EntityFrameworkCore.Database.Command", 20101, f"Executed DbCommand ({prnd2.randint(1, 80)}ms) [Parameters=[@p0='?'], CommandType='Text', CommandTimeout='30']\nSELECT TOP(1) [c].[Id] FROM [Captures] AS [c] WHERE [c].[OrderNo] = @p0"))
    elif r < 0.2:
        mp.append(mel(t, "trce", "Contoso.Payments.Adapter.Health", 3, "Health probe ok"))
    else:
        mp.append(mel(t, "info", "Contoso.Payments.Adapter.CaptureClient", 1000, f"Captured order {order} ({prnd2.randint(5, 400)}.{prnd2.randint(0, 99):02d} EUR)"))
    if not stray and t > CRASH:
        stray = True
        mp.append(f"Polly retry {prnd2.randint(1, 3)}/3 for capture {order}")
write_text("\n".join(mp) + "\n", "mel-console", "payments01", "payments-adapter-20261014.log")

# Text encodings (invented messages, one file each): the same short story told in the encodings Debrief detects at
# import. The text is made up for these files; the ASCII timestamps and levels are plain Serilog text lines.
# `encodings/utf16le-bom.log` has a byte-order mark, `encodings/utf16be-nomark.log` has none.
ENC_LINES = {
    "shift-jis": ("shift_jis", ["サービスを起動しました", "設定ファイルを読み込んでいます", "データベースへの接続に失敗しました", "再試行します", "注文の処理が完了しました", "サービスを停止します"]),
    "gbk": ("gbk", ["服务已启动", "正在读取配置文件", "数据库连接失败", "正在重试", "订单处理完成", "服务即将停止"]),
    "euc-kr": ("euc_kr", ["서비스를 시작했습니다", "설정 파일을 읽는 중입니다", "데이터베이스 연결에 실패했습니다", "다시 시도합니다", "주문 처리가 완료되었습니다", "서비스를 중지합니다"]),
    "big5": ("big5", ["服務已啟動", "正在讀取設定檔", "資料庫連線失敗", "正在重試", "訂單處理完成", "服務即將停止"]),
    "windows-1251": ("cp1251", ["Служба запущена", "Чтение файла настроек", "Не удалось подключиться к базе данных", "Повторная попытка", "Заказ обработан", "Служба останавливается"]),
    "windows-1252": ("cp1252", ["Dienst gestartet", "Konfigurationsdatei wird gelesen", "Verbindung zur Datenbank fehlgeschlagen: Zeitüberschreitung", "Neuer Versuch in 5 Sekunden", "Bestellung für Müller & Söhne abgeschlossen", "Dienst wird beendet"]),
    "utf16le-bom": ("utf-16-le", ["Service started", "Reading the configuration file", "Database connection failed", "Retrying", "Order completed", "Service stopping"]),
    "utf16be-nomark": ("utf-16-be", ["Service started", "Reading the configuration file", "Database connection failed", "Retrying", "Order completed", "Service stopping"]),
}
ENC_LEVELS = ["INF", "INF", "ERR", "WRN", "INF", "INF"]
for enc_name, (codec, messages) in ENC_LINES.items():
    rows = []
    for i, msg in enumerate(messages):
        t = START + timedelta(minutes=i * 7, seconds=i)
        rows.append(f"{t.strftime('%Y-%m-%d %H:%M:%S')}.{i * 111 % 1000:03d} +00:00 [{ENC_LEVELS[i]}] {msg}")
    data = ("\n".join(rows) + "\n").encode(codec)
    if enc_name == "utf16le-bom":
        data = b"\xff\xfe" + data
    write_bytes(data, "encodings", f"{enc_name}.log")

# Video frames (ADR 0033 §4): what `debrief-video` writes for a screen recording of the checkout page on WEB02 while it
# struggles. The header and one JSON line per kept frame; the file name is `<video name>.frames`. Invented content.
vrnd = random.Random(3007)
VIDEO_START = datetime(2026, 10, 14, 10, 36, 40, tzinfo=timezone.utc)
VIDEO_DESCRIPTIONS = [
    "The checkout page shows the cart with three items and a Pay now button.",
    "A spinner covers the payment form; the button is greyed out.",
    "A banner reads: We could not reach the payment service. Retrying...",
    "The same banner is shown with a second retry counter.",
    "An error dialog says the order could not be placed, with an OK button.",
    "The cart page is shown again; the cart is empty.",
]
VIDEO_NOTES = {2: "the retry banner first appears", 4: "dialog covers the form"}
frames_lines = [json.dumps({"videoFrames": 1, "video": "checkout-web02.mp4", "sha256": "".join(f"{vrnd.randrange(256):02x}" for _ in range(32)), "durationMs": 75000}, separators=(",", ":"))]
for i, text in enumerate(VIDEO_DESCRIPTIONS):
    video_ms = i * 13000 + vrnd.randrange(0, 900)
    frame = {"frame": i, "videoMs": video_ms, "time": ms_of(VIDEO_START) + video_ms, "text": text}
    if i in VIDEO_NOTES:
        frame["note"] = VIDEO_NOTES[i]
    frame["alignedBy"] = "fileName"
    frame["confidence"] = "high" if i < 4 else "medium"
    frames_lines.append(json.dumps(frame, separators=(",", ":")))
write_text("\n".join(frames_lines) + "\n", "video", "checkout-web02.mp4.frames")

print("samples written to", ROOT)
