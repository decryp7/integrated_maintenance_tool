# OrderAudit: a complete IMT plugin in C#

A realistic plugin for the audit log of a .NET order service, using every capability a .NET team usually
needs. It is the reference for the C# SDK; `examples/ToyFormatImporter` is the minimal one.

| Capability | Class | What it does |
|---|---|---|
| Importer + live tail | `AuditLogImporter` (`orderaudit.log`) | Multi-line records: a header line plus indented `Key = Value` details and an optional `Exception:` block |
| Transform | `OrderEnrichTransform` (`orderaudit.enrich`) | `OrderId` from the message, `Amount` → `AmountValue` + `Currency`, `DurationMs` from start/end pairs; marks slow orders; drops heartbeats |
| Marker kinds | `OrderMarkers` | `order-failed` and `payment-retry` by rule; `slow-order` set by the transform's code |
| Annotator | `OrderSpanAnnotator` (`orderaudit.orders`) | One span per order (start → completed/failed), critical when failed, warning when slow |
| Exporter | `OrderSummaryExporter` (`orderaudit.summary`, `.csv`) | One CSV line per order (not per record), formula-safe |
| Settings | `OrderAuditSettings` | Slow-order threshold (duration), hide heartbeats (bool), default currency (text, `[A-Z]{3}`) |
| Service | `OrderStatsService` (`com.example.orderaudit.stats` 1.0.0) | `summary`: order statistics as JSON for other plugins |
| Events | `OrderStats` | `workspaceCleared` resets the statistics, `recordsAppended` counts records |
| Manifest | `OrderAuditPlugin` | Optional dependency on IMT's OpenTelemetry plugin (`net.decryptology.otel ^0.1`); **no permissions** |

## The format

```
2026-10-17 08:00:08.031 +00:00 [INF] OrderService: OrderStarted
    OrderId = ORD-10001
    Customer = C-016
    Amount = 1798.23 EUR
2026-10-17 08:00:08.767 +00:00 [WRN] PaymentGateway: PaymentRetry for ORD-10001
    Attempt = 2
    Reason = Connection reset
2026-10-17 08:00:10.853 +00:00 [ERR] OrderService: OrderFailed
    OrderId = ORD-10001
    Error = InsufficientStock
    Exception:
      Shop.Orders.OrderException: InsufficientStock for ORD-10001
         at Shop.Orders.OrderService.CompleteAsync(String orderId) in OrderService.cs:line 284
```

A record has the fields `Component` and `Event` (and `Template` = the event), then the details in order.
Lines that belong to no record (a banner, a rotation note) are counted as skipped. Because records span
several lines, the importer declares `Multiline`: on live tail IMT re-sends the text from the start of the last
record, so continuation lines that arrive later join it.

## Walkthrough

### 1. Sample data

`data/orders-audit.log` (240 orders over two hours, retries, failures with stack traces, slow orders,
heartbeats, two stray lines) and `data/expected.json` (the counts the tests assert) are generated, never
edited by hand:

```sh
python3 data/generate.py
```

### 2. Build and test

```sh
cd sdk/dotnet
dotnet test --project samples/OrderAudit/tests
```

The tests start the plugin on a private socket with `PluginTestHost` and call it exactly as IMT does.

### 3. Publish

```sh
dotnet publish samples/OrderAudit/src -c Release -r osx-arm64     # or linux-x64, win-x64, ...
```

This writes the plugin folder (see `sdk/README.md`, "Plugin folder layout"):

```
sdk/dotnet/artifacts/plugins/com.example.orderaudit/
  plugin.json
  bin/osx-arm64/order-audit
```

Check it with IMT's conformance kit:

```sh
cargo build -p imt-plugin-sdk --bin imt-plugin-check
target/debug/imt-plugin-check sdk/dotnet/artifacts/plugins/com.example.orderaudit/bin/osx-arm64/order-audit
```

### 4. Install into IMT

Copy the folder into IMT's plugins directory and grant nothing (the plugin asks for no permission). IMT starts
`bin/<rid>/order-audit` through its plugin host, shakes hands and lists **Order audit 1.0.0** in the Plugins
panel with its importer, transform, exporter, three marker kinds, annotator, service and settings. The
OpenTelemetry dependency is optional: without that plugin the panel shows a note, not an error.

> IMT's plugin manager does not install external plugins yet; until it does, `tests/e2e-rs` runs exactly this
> path (`imt_plugin_host::RemotePlugin` → `imt_core::Workspace`) and `tests/e2e/run.sh` runs it with one command.

### 5. Open the sample data

Open `data/orders-audit.log`. The importer wins the format detection (score 100 for a file named like an
audit log, 95 otherwise). To enrich the records, add the transform to your profile's pipeline:

```toml
[[pipeline]]
step = "plugin"
id = "orderaudit.enrich"
```

### 6. What you'll see

- **Grid:** 585 records (646 parsed, 61 heartbeats dropped by the transform, 2 stray lines skipped), with the
  fields `Component`, `Event`, `OrderId`, `Customer`, `Amount`, `AmountValue`, `Currency`, `Channel`,
  `DurationMs`; failed orders carry their stack trace.
- **Timeline markers:** `com.example.orderaudit/order-failed` (26, red), `payment-retry` (114, amber) from the
  plugin's rules, and `slow-order` (44, purple) set by the transform's code for orders over the threshold.
- **Spans:** one per order (240): "Order ORD-10002 completed in 1.3 s", critical for the 26 failed ones,
  warnings for the slow ones, "(open)" for the 9 that never finished. Each span's evidence query filters
  `OrderId`.
- **Settings → Order audit:** raise *Slow order threshold* and the slow markers disappear on the next import;
  turn *Hide heartbeats* off and they come back.
- **Export → Order summary (CSV):** `OrderId,Customer,Amount,Currency,Status,Started,DurationMs,PaymentRetries,Source`,
  one line per order.

The numbers above come from `data/expected.json`; the end-to-end test asserts every one of them.
