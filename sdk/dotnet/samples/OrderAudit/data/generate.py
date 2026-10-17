#!/usr/bin/env python3
"""Generates the OrderAudit sample data: data/orders-audit.log and data/expected.json.

The log is the audit log of a fictitious .NET order service (one header line per event, indented
`Key = Value` details, optional `Exception:` blocks). expected.json holds the counts the tests and the
end-to-end check assert. Deterministic: the same seed always writes the same files. Don't hand-edit them;
run `python3 data/generate.py`.
"""

import json
import random
from datetime import datetime, timedelta, timezone
from pathlib import Path

SEED = 20261017
ORDERS = 240
SLOW_MS = 2000  # the plugin's default slow order threshold

HERE = Path(__file__).resolve().parent


def ts(t: datetime) -> str:
    return t.strftime("%Y-%m-%d %H:%M:%S.") + f"{t.microsecond // 1000:03d} +00:00"


def main() -> None:
    rnd = random.Random(SEED)
    start = datetime(2026, 10, 17, 8, 0, 0, tzinfo=timezone.utc)
    events = []  # (time, sort key, lines)
    expected = {"orders": ORDERS, "completed": 0, "failed": 0, "open": 0, "slow": 0, "paymentRetries": 0, "heartbeats": 0,
                "records": 0, "skipped": 0, "withException": 0}
    seq = 0

    def add(t, level, component, event, text="", details=(), exception=None):
        nonlocal seq
        seq += 1
        header = f"{ts(t)} [{level}] {component}: {event}" + (f" {text}" if text else "")
        lines = [header] + [f"    {k} = {v}" for k, v in details]
        if exception:
            lines.append("    Exception:")
            lines += ["      " + l for l in exception.split("\n")]
            expected["withException"] += 1
        events.append((t, seq, lines))

    for n in range(ORDERS):
        oid = f"ORD-{10001 + n}"
        t0 = start + timedelta(seconds=n * 30 + rnd.randint(0, 20), milliseconds=rnd.randint(0, 999))
        amount = f"{rnd.randint(5, 2500)}.{rnd.randint(0, 99):02d}"
        currency = rnd.choice(["EUR", "EUR", "EUR", "USD", "GBP", ""])
        details = [("OrderId", oid), ("Customer", f"C-{rnd.randint(1, 120):03d}"), ("Amount", f"{amount} {currency}".strip()),
                   ("Channel", rnd.choice(["web", "app", "store"]))]
        add(t0, "INF", "OrderService", "OrderStarted", details=details)
        t = t0
        retries = rnd.choices([0, 1, 2, 3], weights=[70, 18, 8, 4])[0]
        for a in range(retries):
            t += timedelta(milliseconds=rnd.randint(200, 900))
            add(t, "WRN", "PaymentGateway", "PaymentRetry", f"for {oid}",
                [("Attempt", str(a + 2)), ("Reason", rnd.choice(["Gateway timeout", "HTTP 503", "Connection reset"]))])
            expected["paymentRetries"] += 1
        outcome = rnd.choices(["completed", "failed", "open"], weights=[86, 11, 3])[0]
        if n == ORDERS - 1:
            outcome = "open"
        if outcome == "open":
            expected["open"] += 1
            continue
        slow = rnd.random() < 0.12
        t += timedelta(milliseconds=rnd.randint(2500, 9000) if slow else rnd.randint(150, 1500))
        duration_ms = int((t - t0).total_seconds() * 1000)
        if duration_ms > SLOW_MS:
            expected["slow"] += 1
        if outcome == "completed":
            expected["completed"] += 1
            add(t, "INF", "OrderService", "OrderCompleted", details=[("OrderId", oid), ("Items", str(rnd.randint(1, 9)))])
        else:
            expected["failed"] += 1
            error = rnd.choice(["InsufficientStock", "PaymentDeclined", "AddressInvalid"])
            exc = None
            if error != "AddressInvalid":
                exc = (f"Shop.Orders.OrderException: {error} for {oid}\n"
                       f"   at Shop.Orders.OrderService.CompleteAsync(String orderId) in OrderService.cs:line {rnd.randint(40, 400)}\n"
                       "   at Shop.Orders.Worker.ExecuteAsync(CancellationToken stoppingToken)")
            add(t, "ERR", "OrderService", "OrderFailed", details=[("OrderId", oid), ("Error", error)], exception=exc)

    # Heartbeats every two minutes (dropped by default by the enrich transform).
    for m in range(0, ORDERS * 30 // 60 + 2, 2):
        add(start + timedelta(minutes=m, milliseconds=1), "DBG", "Health", "Heartbeat", details=[("Uptime", f"{m * 60}s")])
        expected["heartbeats"] += 1

    events.sort(key=lambda e: (e[0], e[1]))
    out = []
    for i, (_, _, lines) in enumerate(events):
        out += lines
        if i == 5:
            # A line that belongs to no record (the importer counts it as skipped).
            out.append("--- log rotated by logrotate ---")
            expected["skipped"] += 1
    out.insert(0, "# order-service audit log")  # before the first record: skipped too
    expected["skipped"] += 1
    expected["records"] = len(events)

    (HERE / "orders-audit.log").write_text("\n".join(out) + "\n", encoding="utf-8")
    (HERE / "expected.json").write_text(json.dumps(expected, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(expected))


if __name__ == "__main__":
    main()
