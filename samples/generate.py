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

print("samples written to", ROOT)
