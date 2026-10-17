using System.Globalization;
using System.Text;
using Imt.Plugin;

namespace OrderAudit;

/// <summary>
/// Exports one CSV line per order (not per record): id, customer, amount, currency, status, start, duration and
/// payment retries, from the records of the current view.
/// </summary>
public sealed class OrderSummaryExporter : IExporter
{
    /// <summary>The CSV header.</summary>
    public const string Header = "OrderId,Customer,Amount,Currency,Status,Started,DurationMs,PaymentRetries,Source";

    /// <inheritdoc/>
    public string Id => "orderaudit.summary";

    /// <inheritdoc/>
    public string Name => "Order summary (CSV)";

    /// <inheritdoc/>
    public string Extension => "csv";

    /// <inheritdoc/>
    public async ValueTask<long> ExportAsync(IAsyncEnumerable<ExportRow> rows, Stream output, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(rows);
        ArgumentNullException.ThrowIfNull(output);
        var orders = new Dictionary<string, Order>(StringComparer.Ordinal);
        long count = 0;
        await foreach (var row in rows.WithCancellation(cancellationToken))
        {
            count++;
            var r = row.Record;
            if (r.FieldText("OrderId") is not { } id)
            {
                continue;
            }

            if (!orders.TryGetValue(id, out var o))
            {
                o = new Order { Id = id, Source = row.SourceName };
                orders.Add(id, o);
            }

            o.Customer ??= r.FieldText("Customer");
            o.Amount ??= r.FieldText("AmountValue") ?? r.FieldText("Amount");
            o.Currency ??= r.FieldText("Currency");
            switch (r.FieldText("Event"))
            {
                case "OrderStarted":
                    o.Started = r.Time;
                    o.Status ??= "open";
                    break;
                case "OrderCompleted":
                    o.Status = "completed";
                    o.DurationMs = r.FieldText("DurationMs");
                    break;
                case "OrderFailed":
                    o.Status = "failed";
                    o.DurationMs = r.FieldText("DurationMs");
                    break;
                case "PaymentRetry":
                    o.Retries++;
                    break;
            }
        }

        await using var writer = new StreamWriter(output, new UTF8Encoding(false), 64 * 1024, leaveOpen: true);
        await writer.WriteLineAsync(Header.AsMemory(), cancellationToken);
        foreach (var o in orders.Values.OrderBy(o => o.Started ?? DateTimeOffset.MaxValue).ThenBy(o => o.Id, StringComparer.Ordinal))
        {
            var line = string.Join(',', Csv(o.Id), Csv(o.Customer), Csv(o.Amount), Csv(o.Currency), Csv(o.Status ?? "unknown"),
                Csv(o.Started?.ToString("o", CultureInfo.InvariantCulture)), Csv(o.DurationMs), o.Retries.ToString(CultureInfo.InvariantCulture), Csv(o.Source));
            await writer.WriteLineAsync(line.AsMemory(), cancellationToken);
        }

        await writer.FlushAsync(cancellationToken);
        return count;
    }

    /// <summary>A CSV cell: quoted when it holds a comma, quote or line break; formula-like cells are defused.</summary>
    /// <param name="value">The value.</param>
    /// <returns>The cell.</returns>
    public static string Csv(string? value)
    {
        if (string.IsNullOrEmpty(value))
        {
            return string.Empty;
        }

        // Spreadsheets run cells starting with = + - @ as formulas; log text is untrusted.
        if (value[0] is '=' or '+' or '@' or '\t' or '\r' || (value[0] == '-' && !double.TryParse(value, NumberStyles.Float, CultureInfo.InvariantCulture, out _)))
        {
            value = "'" + value;
        }

        return value.AsSpan().IndexOfAny(",\"\r\n") >= 0 ? $"\"{value.Replace("\"", "\"\"", StringComparison.Ordinal)}\"" : value;
    }

    private sealed class Order
    {
        public required string Id { get; init; }

        public required string Source { get; init; }

        public string? Customer { get; set; }

        public string? Amount { get; set; }

        public string? Currency { get; set; }

        public string? Status { get; set; }

        public DateTimeOffset? Started { get; set; }

        public string? DurationMs { get; set; }

        public int Retries { get; set; }
    }
}
