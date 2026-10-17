using System.Collections.Concurrent;
using System.Globalization;
using System.Text.RegularExpressions;
using Imt.Plugin;

namespace OrderAudit;

/// <summary>
/// Enriches order records (use it as a <c>plugin</c> step of the profile's pipeline: <c>id = "orderaudit.enrich"</c>):
/// <list type="bullet">
/// <item><c>OrderId</c> from the message when the record has no such field;</item>
/// <item><c>Amount</c> "129.95 EUR" split into <c>AmountValue</c> (number) and <c>Currency</c>;</item>
/// <item><c>DurationMs</c> on the end record of an order (completed or failed), from its start;
/// orders slower than the setting are marked <c>slow-order</c> (a mark no condition could express);</item>
/// <item><c>Heartbeat</c> records dropped when the setting says so.</item>
/// </list>
/// </summary>
public sealed partial class OrderEnrichTransform : RecordTransform
{
    /// <summary>The transform id.</summary>
    public const string TransformId = "orderaudit.enrich";

    private readonly OrderAuditSettings settings;
    private readonly OrderStats stats;

    /// <summary>Creates the transform.</summary>
    /// <param name="settings">The settings.</param>
    /// <param name="stats">Statistics to update.</param>
    public OrderEnrichTransform(OrderAuditSettings settings, OrderStats stats)
    {
        this.settings = settings;
        this.stats = stats;
    }

    /// <inheritdoc/>
    public override string Id => TransformId;

    /// <inheritdoc/>
    public override Verdict Apply(Record record, Action<string> mark, TransformContext context)
    {
        ArgumentNullException.ThrowIfNull(record);
        ArgumentNullException.ThrowIfNull(mark);
        ArgumentNullException.ThrowIfNull(context);
        string? evt = record.FieldText("Event");
        if (evt == "Heartbeat" && settings.DropHeartbeats)
        {
            return Verdict.Drop;
        }

        string? orderId = record.FieldText("OrderId");
        if (orderId is null && OrderIdPattern().Match(record.Message) is { Success: true } m)
        {
            orderId = m.Value;
            record.Fields["OrderId"] = orderId;
        }

        if (record.FieldText("Amount") is { } amount)
        {
            var parts = amount.Split(' ', 2, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
            if (parts.Length > 0 && decimal.TryParse(parts[0], NumberStyles.Number, CultureInfo.InvariantCulture, out var value))
            {
                record.Fields["AmountValue"] = (double)value;
                record.Fields["Currency"] = parts.Length > 1 ? parts[1] : settings.DefaultCurrency;
            }
        }

        if (evt is null || orderId is null || record.Timestamp is not { } ts)
        {
            return Verdict.Keep;
        }

        stats.Count(evt);
        var starts = (ConcurrentDictionary<string, long>)context.Items.GetOrAdd("starts", _ => new ConcurrentDictionary<string, long>(StringComparer.Ordinal));
        if (evt == "OrderStarted")
        {
            starts[orderId] = ts;
        }
        else if (evt is "OrderCompleted" or "OrderFailed" && starts.TryRemove(orderId, out long start))
        {
            long duration = ts - start;
            record.Fields["DurationMs"] = duration;
            if (duration > settings.SlowOrderMs)
            {
                mark(OrderMarkers.SlowOrder);
                stats.CountSlow();
            }
        }

        return Verdict.Keep;
    }

    [GeneratedRegex(@"\bORD-\d{1,12}\b", RegexOptions.CultureInvariant | RegexOptions.NonBacktracking)]
    private static partial Regex OrderIdPattern();
}
