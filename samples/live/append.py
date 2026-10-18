#!/usr/bin/env python3
"""Appends realistic OrderService lines to a growing log, for demos of live
tail, rotation and alerts. Not run by the tests; nothing here is generated.

    python3 samples/live/append.py [folder] [--rate 5] [--rotate-kb 256] [--seed 7]

Open the folder in IMT (File > Open folder, then Live tail), then start this
script. It writes `orders-live.log` in the Serilog text format of
`serilog-text/` (timestamp, level, message, properties as JSON) at about
`--rate` lines per second. Every `--rotate-kb` KiB the file is rotated the way
Serilog's rolling file sink and logrotate do it (`orders-live.log` becomes
`orders-live.log.1`, `.1` becomes `.2`, ...; three are kept) and a new file is
started, so the tail has to follow the rename. Roughly once a minute a burst of
payment timeouts and errors arrives (alerts on error rate fire), now and then a
slow request, and every 30 s a "Process stats" line (charts move).

Stop with Ctrl+C. The default folder is `imt-live-demo` in the system temp
folder; the path is printed at start.
"""

import argparse
import json
import os
import random
import sys
import tempfile
import time
from datetime import datetime, timezone

KEEP = 3


def stamp(now):
    return now.strftime("%Y-%m-%d %H:%M:%S.") + f"{now.microsecond // 1000:03d} +00:00"


def line(now, level, message, **props):
    props = dict(props, MachineName="WEB03", ProcessId=os.getpid(), Application="OrderService")
    return f"{stamp(now)} [{level}] {message} {json.dumps(props)}\n"


def rotate(path):
    for i in range(KEEP - 1, 0, -1):
        older = f"{path}.{i}"
        if os.path.exists(older):
            os.replace(older, f"{path}.{i + 1}")
    if os.path.exists(path):
        os.replace(path, f"{path}.1")


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("folder", nargs="?", default=os.path.join(tempfile.gettempdir(), "imt-live-demo"))
    ap.add_argument("--rate", type=float, default=5.0, help="lines per second (default 5)")
    ap.add_argument("--rotate-kb", type=int, default=256, help="rotate after this many KiB (default 256)")
    ap.add_argument("--seed", type=int, default=7)
    args = ap.parse_args()
    if args.rate <= 0 or args.rotate_kb <= 0:
        sys.exit("--rate and --rotate-kb must be positive")

    os.makedirs(args.folder, exist_ok=True)
    path = os.path.join(args.folder, "orders-live.log")
    rnd = random.Random(args.seed)
    print(f"appending to {path} (Ctrl+C to stop)", flush=True)

    out = open(path, "a", encoding="utf-8", newline="\n")
    out.write(line(datetime.now(timezone.utc), "INF", "Application started — OrderService 3.14.2 (live demo)", SourceContext="Program"))
    order = 90000
    burst_until = 0.0
    next_burst = time.monotonic() + rnd.uniform(40, 80)
    next_stats = time.monotonic()
    try:
        while True:
            now_m = time.monotonic()
            now = datetime.now(timezone.utc)
            if now_m >= next_burst:
                burst_until, next_burst = now_m + rnd.uniform(5, 12), now_m + rnd.uniform(50, 90)
            order += 1
            path_ = rnd.choice(["/api/orders", f"/api/orders/{order}", f"/api/orders/{order}/capture", "/api/cart"])
            if now_m < burst_until and path_.endswith("/capture") or now_m < burst_until and rnd.random() < 0.3:
                out.write(line(now, "WRN", f"Payment gateway call exceeded 5000 ms (attempt 3/3) for order {order}",
                               OrderId=order, SourceContext="OrderService.Payments"))
                out.write(line(now, "ERR", f"Failed to capture payment for order {order}: request timed out",
                               OrderId=order, SourceContext="OrderService.Payments"))
                out.write("System.Net.Http.HttpRequestException: Response status code does not indicate success: 504 (Gateway Timeout).\n"
                          "   at OrderService.Payments.GatewayClient.CaptureAsync(Guid orderId) in /src/Payments/GatewayClient.cs:line 41\n")
            elif rnd.random() < 0.03:
                ms = rnd.randint(1500, 6000)
                out.write(line(now, "WRN", f"Slow request {path_} took {ms} ms", RequestPath=path_, ElapsedMs=ms,
                               SourceContext="OrderService.Diagnostics"))
            else:
                ms = round(rnd.uniform(20, 160), 1)
                out.write(line(now, "INF", f"HTTP GET {path_} responded 200 in {ms} ms", RequestPath=path_, StatusCode=200, Elapsed=ms,
                               SourceContext="Serilog.AspNetCore.RequestLoggingMiddleware"))
            if now_m >= next_stats:
                cpu = round(rnd.uniform(30, 45) + (40 if now_m < burst_until else 0), 1)
                ws = round(rnd.uniform(600, 640), 1)
                out.write(line(now, "INF", f"Process stats CPU {cpu}% · WorkingSet {ws} MB · Threads {60 + int(cpu)}",
                               CpuPercent=cpu, WorkingSetMb=ws, Threads=60 + int(cpu), SourceContext="Diagnostics.ProcessMonitor"))
                next_stats = now_m + 30
            out.flush()
            if out.tell() >= args.rotate_kb * 1024:
                out.close()
                rotate(path)
                out = open(path, "a", encoding="utf-8", newline="\n")
                print(f"rotated {path}", flush=True)
            time.sleep(rnd.expovariate(args.rate))
    except KeyboardInterrupt:
        out.write(line(datetime.now(timezone.utc), "INF", "Application is shutting down...", SourceContext="Microsoft.Hosting.Lifetime"))
    finally:
        out.close()


if __name__ == "__main__":
    main()
