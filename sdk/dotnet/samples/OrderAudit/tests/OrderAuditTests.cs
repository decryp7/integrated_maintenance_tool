using System.Text;
using System.Text.Json.Nodes;
using Imt.Plugin;
using Imt.Plugin.Testing;
using Wire = Imt.Plugin.V1;

namespace OrderAudit.Tests;

/// <summary>The sample over the real transport, against the generated sample data.</summary>
public sealed class OrderAuditTests : IAsyncLifetime
{
    private static readonly string Data = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "data", "orders-audit.log"));
    private static readonly JsonNode Expected = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "data", "expected.json")))!;
    private readonly OrderAuditSettings settings = new();
    private readonly OrderStats stats = new();
    private PluginTestHost host = null!;

    private static int Count(string key) => Expected[key]!.GetValue<int>();

    public async ValueTask InitializeAsync()
    {
        host = await PluginTestHost.StartAsync(OrderAuditPlugin.Build(settings, stats));
        await host.HandshakeAsync();
    }

    public async ValueTask DisposeAsync() => await host.DisposeAsync();

    [Fact]
    public async Task The_manifest_declares_every_capability_and_no_permission()
    {
        var m = await host.HandshakeAsync();
        Assert.Equal(("com.example.orderaudit", "1.0.0"), (m.Id, m.Version));
        Assert.Empty(m.Permissions);
        var dep = Assert.Single(m.Requires);
        Assert.Equal(("net.decryptology.otel", true), (dep.Id, dep.Optional));
        var imp = m.Capabilities.Single(c => c.Importer is not null).Importer;
        Assert.True(imp.Tail && imp.Multiline && !imp.Binary);
        Assert.Contains(m.Capabilities, c => c.Transform?.Id == "orderaudit.enrich");
        Assert.Contains(m.Capabilities, c => c.Exporter?.Extension == "csv");
        Assert.Contains(m.Capabilities, c => c.Annotator?.Id == "orderaudit.orders");
        Assert.Contains(m.Capabilities, c => c.Settings is not null);
        Assert.Contains(m.Capabilities, c => c.MarkerKinds is not null);
        Assert.Contains(m.Capabilities, c => c.Service?.Id == "com.example.orderaudit.stats");
        Assert.Equal("com.example.orderaudit.stats", m.Provides.Single().Id);
    }

    [Fact]
    public async Task Sniff_recognises_the_format()
    {
        Assert.Equal(100u, await host.SniffAsync(AuditLogImporter.ImporterId, "orders-audit.log", Data[..4000]));
        Assert.Equal(95u, await host.SniffAsync(AuditLogImporter.ImporterId, "service.log", Data[..4000]));
        Assert.Equal(60u, await host.SniffAsync(AuditLogImporter.ImporterId, "x.log", "2026-10-17 08:00:00.001 +00:00 [DBG] Health: Heartbeat\n"));
        Assert.Equal(0u, await host.SniffAsync(AuditLogImporter.ImporterId, "x.log", "2024-01-01 INF hello\n"));
        Assert.Equal(0u, await host.SniffAsync(AuditLogImporter.ImporterId, "x.log", string.Empty));
    }

    [Fact]
    public async Task The_sample_imports_with_multi_line_records()
    {
        var r = await host.ImportAsync(AuditLogImporter.ImporterId, "orders-audit.log", Data);
        Assert.Equal(Count("records"), r.Records.Count);
        Assert.Equal((ulong)Count("skipped"), r.Skipped);
        var started = r.Records.Where(x => x.FieldText("Event") == "OrderStarted").ToList();
        Assert.Equal(Count("orders"), started.Count);
        var first = started[0];
        Assert.Equal(("OrderService", "ORD-10001", "OrderStarted"), (first.FieldText("Component"), first.FieldText("OrderId"), first.Template));
        Assert.Equal(["Component", "Event", "OrderId", "Customer", "Amount", "Channel"], first.Fields.Select(f => f.Key));
        Assert.Equal(new DateTimeOffset(2026, 10, 17, 8, 0, 8, 31, TimeSpan.Zero), first.Time);
        var failed = r.Records.Where(x => x.FieldText("Event") == "OrderFailed").ToList();
        Assert.Equal(Count("failed"), failed.Count);
        Assert.All(failed, f => Assert.Equal(Level.Error, f.Level));
        Assert.Equal(Count("withException"), r.Records.Count(x => x.Exception is not null));
        var ex = failed.First(f => f.Exception is not null).Exception!;
        Assert.StartsWith("Shop.Orders.OrderException:", ex, StringComparison.Ordinal);
        Assert.Contains("\n   at Shop.Orders.Worker.ExecuteAsync", ex, StringComparison.Ordinal);
        Assert.Equal(Count("paymentRetries"), r.Records.Count(x => x.FieldText("Event") == "PaymentRetry"));
        Assert.Equal("PaymentRetry for ORD-10001", r.Records.First(x => x.FieldText("Event") == "PaymentRetry").Message);
    }

    [Fact]
    public async Task Live_tail_parses_appended_records()
    {
        const string tail = "2026-10-17 09:00:00.000 +00:00 [INF] OrderService: OrderStarted\n    OrderId = ORD-99999\n2026-10-17 09:00:01.000 +00:00 [INF] OrderService: OrderCompleted\n    OrderId = ORD-99999\n";
        var r = await host.ImportTailAsync(AuditLogImporter.ImporterId, "orders-audit.log", Data[..2000], tail);
        Assert.False(r.TailUnsupported);
        Assert.Equal(2, r.Records.Count);
        Assert.Equal((1u, 3u), (r.Records[0].Line, r.Records[1].Line));
        Assert.Equal("ORD-99999", r.Records[1].FieldText("OrderId"));
    }

    [Fact]
    public async Task The_transform_enriches_marks_slow_orders_and_drops_heartbeats()
    {
        var records = AuditLogImporter.ParseAll(Data, out _);
        var results = await host.TransformAsync(OrderEnrichTransform.TransformId, "orders-audit.log", records);
        var t = Assert.Single(results);
        Assert.Equal(Count("heartbeats"), t.Verdicts.Count(v => v == Verdict.Drop));
        Assert.Equal(Count("slow"), t.Marks.Count);
        Assert.All(t.Marks.Values, m => Assert.Equal([OrderMarkers.SlowOrder], m));
        var ends = t.Records.Where(r => r.FieldText("Event") is "OrderCompleted" or "OrderFailed").ToList();
        Assert.Equal(Count("completed") + Count("failed"), ends.Count);
        Assert.All(ends, r => Assert.True(r.Fields["DurationMs"]!.GetValue<long>() > 0));
        var start = t.Records.First(r => r.FieldText("Event") == "OrderStarted" && r.FieldText("Amount")!.EndsWith("EUR", StringComparison.Ordinal));
        Assert.IsType<double>(start.Fields["AmountValue"]!.GetValue<double>());
        Assert.Equal("EUR", start.FieldText("Currency"));
        // Amounts without a currency get the default; retries get the order id from the message.
        Assert.Contains(t.Records, r => r.FieldText("Amount") is { } a && !a.Contains(' ', StringComparison.Ordinal) && r.FieldText("Currency") == "EUR");
        Assert.All(t.Records.Where(r => r.FieldText("Event") == "PaymentRetry"), r => Assert.StartsWith("ORD-", r.FieldText("OrderId"), StringComparison.Ordinal));
        Assert.Equal(Count("orders"), stats.Started);
        Assert.Equal(Count("slow"), stats.Slow);
    }

    [Fact]
    public async Task Settings_change_the_transform()
    {
        await host.ApplySettingsAsync(new JsonObject { ["slowOrderMs"] = 100_000, ["dropHeartbeats"] = false, ["defaultCurrency"] = "USD" });
        Assert.Equal((100_000L, false, "USD"), (settings.SlowOrderMs, settings.DropHeartbeats, settings.DefaultCurrency));
        var t = (await host.TransformAsync(OrderEnrichTransform.TransformId, "x", AuditLogImporter.ParseAll(Data, out _))).Single();
        Assert.Empty(t.Marks);
        Assert.DoesNotContain(Verdict.Drop, t.Verdicts);
        Assert.Contains(t.Records, r => r.FieldText("Currency") == "USD");
        var bad = await Assert.ThrowsAsync<Grpc.Core.RpcException>(() => host.ApplySettingsAsync(new JsonObject { ["defaultCurrency"] = "euro" }));
        Assert.Contains("defaultCurrency", bad.Status.Detail, StringComparison.Ordinal);
    }

    [Fact]
    public async Task The_annotator_produces_one_span_per_order()
    {
        var records = AuditLogImporter.ParseAll(Data, out _);
        var enriched = (await host.TransformAsync(OrderEnrichTransform.TransformId, "orders-audit.log", records)).Single().Records;
        var interest = enriched.Where(r => r.FieldText("Event") is "OrderStarted" or "OrderCompleted" or "OrderFailed")
            .Select(r => new AnnotatedRecord(r, "orders-audit.log", "orders-audit.log")).ToList();
        var spans = await host.AnnotateAsync(OrderSpanAnnotator.AnnotatorId, interest);
        Assert.Equal(Count("orders"), spans.Count);
        Assert.Equal(Count("failed"), spans.Count(s => s.Severity == Wire.Severity.Critical));
        Assert.Equal(Count("open"), spans.Count(s => s.Label.EndsWith("(open)", StringComparison.Ordinal)));
        var slowCompleted = spans.Count(s => s.Severity == Wire.Severity.Warning);
        Assert.True(slowCompleted > 0 && slowCompleted <= Count("slow"));
        var s0 = spans.First(s => s.OnValue == "ORD-10002");
        Assert.Equal(("order", "OrderId", "orders-audit.log"), (s0.Kind, s0.OnField, s0.Instance));
        Assert.True(s0.To > s0.From);
        var q = JsonNode.Parse(s0.EvidenceQueryJson)!;
        Assert.Equal("ORD-10002", q["filters"]![0]!["value"]!.GetValue<string>());
    }

    [Fact]
    public async Task The_exporter_writes_one_line_per_order()
    {
        var enriched = (await host.TransformAsync(OrderEnrichTransform.TransformId, "orders-audit.log", AuditLogImporter.ParseAll(Data, out _))).Single().Records;
        var (output, rows) = await host.ExportAsync("orderaudit.summary", [.. enriched.Select(r => new ExportRow(r, "orders-audit.log", string.Empty))]);
        Assert.Equal((ulong)enriched.Count, rows);
        var lines = Encoding.UTF8.GetString(output).TrimEnd('\n').Split('\n');
        Assert.Equal(OrderSummaryExporter.Header, lines[0]);
        Assert.Equal(Count("orders"), lines.Length - 1);
        Assert.StartsWith("ORD-10001,", lines[1], StringComparison.Ordinal);
        Assert.Equal(Count("failed"), lines.Count(l => l.Contains(",failed,", StringComparison.Ordinal)));
        Assert.Equal(Count("open"), lines.Count(l => l.Contains(",open,", StringComparison.Ordinal)));
    }

    [Fact]
    public void Csv_cells_are_quoted_and_defused()
    {
        Assert.Equal(string.Empty, OrderSummaryExporter.Csv(null));
        Assert.Equal("plain", OrderSummaryExporter.Csv("plain"));
        Assert.Equal("\"a,b\"", OrderSummaryExporter.Csv("a,b"));
        Assert.Equal("\"say \"\"hi\"\"\"", OrderSummaryExporter.Csv("say \"hi\""));
        Assert.Equal("'=cmd()", OrderSummaryExporter.Csv("=cmd()"));
        Assert.Equal("-5", OrderSummaryExporter.Csv("-5"));
        Assert.Equal("'-x", OrderSummaryExporter.Csv("-x"));
    }

    [Fact]
    public async Task Marker_kinds_service_and_events()
    {
        var kinds = (await host.Client.MarkerKindsAsync(new Wire.MarkerKindsRequest())).Kinds;
        Assert.Equal(["order-failed", "payment-retry", "slow-order"], kinds.Select(k => k.Kind));
        Assert.Equal(("Event", "eq", "\"OrderFailed\""), (kinds[0].When[0].Field, kinds[0].When[0].Op, kinds[0].When[0].ValueJson));
        Assert.Empty(kinds[2].When);

        stats.Count("OrderStarted");
        var reply = await host.Client.CallServiceAsync(new Wire.ServiceCall { ServiceId = "com.example.orderaudit.stats", Method = "summary" });
        Assert.Equal(1, JsonNode.Parse(reply.Payload.ToStringUtf8())!["started"]!.GetValue<long>());
        await Assert.ThrowsAsync<Grpc.Core.RpcException>(async () => await host.Client.CallServiceAsync(new Wire.ServiceCall { ServiceId = "com.example.orderaudit.stats", Method = "nope" }));

        using var call = host.Client.Events();
        await call.RequestStream.WriteAsync(new Wire.HostEvent { Kind = "recordsAppended", PayloadJson = """{"count":7,"sources":[1]}""" });
        await call.RequestStream.CompleteAsync();
        await call.ResponseAsync;
        Assert.Equal(7, stats.Appended);
        using var clear = host.Client.Events();
        await clear.RequestStream.WriteAsync(new Wire.HostEvent { Kind = "workspaceCleared", PayloadJson = "{}" });
        await clear.RequestStream.CompleteAsync();
        await clear.ResponseAsync;
        Assert.Equal((0L, 0L), (stats.Started, stats.Appended));
    }

    [Fact]
    public void Lines_that_belong_to_no_record_are_skipped_and_huge_lines_cut()
    {
        var text = "junk first\n    OrderId = x\n2026-10-17 08:00:00.000 +00:00 [INF] A: E\n    not a detail\n    Exception:\n      boom\n    Key = v\n" + new string('x', 100_000) + "\n";
        var records = AuditLogImporter.ParseAll(text, out long skipped);
        var r = Assert.Single(records);
        Assert.Equal(("boom", "v"), (r.Exception, r.FieldText("Key")));
        Assert.Equal(4, skipped);
    }
}
