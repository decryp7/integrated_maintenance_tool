using Imt.Plugin;

namespace OrderAudit;

/// <summary>
/// A complete IMT plugin for the audit log of a .NET order service: importer (multi-line, live tail),
/// enrichment transform, marker kinds, order spans, a summary exporter, settings, a service and events.
/// </summary>
public static class OrderAuditPlugin
{
    /// <summary>The plugin id.</summary>
    public const string Id = "com.example.orderaudit";

    /// <summary>Builds the plugin. Every contribution shares one <see cref="OrderAuditSettings"/> and <see cref="OrderStats"/>.</summary>
    /// <param name="settings">The settings (a fresh one by default).</param>
    /// <param name="stats">The statistics (a fresh one by default).</param>
    /// <returns>The plugin.</returns>
    public static PluginDefinition Build(OrderAuditSettings? settings = null, OrderStats? stats = null)
    {
        settings ??= new OrderAuditSettings();
        stats ??= new OrderStats();
        return PluginBuilder.Create(Id)
            .Name("Order audit")
            .Version("1.0.0")
            // Optional: when IMT's OpenTelemetry plugin is there, its traces sit next to the orders.
            .Optionally("net.decryptology.otel", "^0.1")
            // No permissions: the plugin only reads the bytes IMT gives it (no paths, no network).
            .Add(new AuditLogImporter())
            .Add(new OrderEnrichTransform(settings, stats))
            .Add(new OrderMarkers())
            .Add(new OrderSpanAnnotator(settings))
            .Add(new OrderSummaryExporter())
            .Add(settings)
            .Add(new OrderStatsService(stats))
            .Add(stats)
            .Build();
    }
}
