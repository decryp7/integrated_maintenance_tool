using System.Globalization;
using System.Text.Json.Nodes;
using Imt.Plugin;

namespace OrderAudit;

/// <summary>
/// One span per order, from <c>OrderStarted</c> to <c>OrderCompleted</c> or <c>OrderFailed</c> (an open order spans
/// to the newest record). Failed orders are critical, slow ones warnings. Each span links to its records
/// (<c>OrderId</c>) and carries a query that shows them.
/// </summary>
public sealed class OrderSpanAnnotator : IAnnotator
{
    /// <summary>The annotator id.</summary>
    public const string AnnotatorId = "orderaudit.orders";

    private readonly OrderAuditSettings settings;

    /// <summary>Creates the annotator.</summary>
    /// <param name="settings">The settings (slow threshold).</param>
    public OrderSpanAnnotator(OrderAuditSettings settings) => this.settings = settings;

    /// <inheritdoc/>
    public string Id => AnnotatorId;

    /// <inheritdoc/>
    public IReadOnlyList<Condition> Interest { get; } = [Condition.In("Event", "OrderStarted", "OrderCompleted", "OrderFailed")];

    /// <inheritdoc/>
    public int Limit => 100_000;

    /// <inheritdoc/>
    public ValueTask<IReadOnlyList<Annotation>> AnnotateAsync(AnnotationInput input, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(input);
        var open = new Dictionary<string, (long Start, string Instance)>(StringComparer.Ordinal);
        var spans = new List<Annotation>();
        foreach (var (r, _, instance) in input.Records)
        {
            if (r.FieldText("OrderId") is not { } id || r.Timestamp is not { } ts)
            {
                continue;
            }

            switch (r.FieldText("Event"))
            {
                case "OrderStarted":
                    open[id] = (ts, instance);
                    break;
                case "OrderCompleted" or "OrderFailed" when open.Remove(id, out var s):
                    spans.Add(Span(id, s.Start, ts, s.Instance, r.FieldText("Event") == "OrderFailed"));
                    break;
            }
        }

        long newest = input.TimeRange?.To ?? input.Records.Select(r => r.Record.Timestamp ?? long.MinValue).DefaultIfEmpty(0).Max();
        foreach (var (id, s) in open)
        {
            spans.Add(new Annotation("order", s.Start, Math.Max(s.Start, newest), $"Order {id} (open)")
            {
                Instance = s.Instance.Length > 0 ? s.Instance : null,
                OnField = "OrderId",
                OnValue = id,
                Severity = Severity.Info,
                EvidenceQuery = Evidence(id, s.Start, Math.Max(s.Start, newest)),
            });
        }

        return ValueTask.FromResult<IReadOnlyList<Annotation>>(spans);
    }

    private Annotation Span(string id, long from, long to, string instance, bool failed)
    {
        long ms = to - from;
        bool slow = ms > settings.SlowOrderMs;
        string seconds = (ms / 1000.0).ToString("0.0", CultureInfo.InvariantCulture);
        return new Annotation("order", from, to, $"Order {id} {(failed ? "failed" : "completed")} in {seconds} s{(slow ? " (slow)" : string.Empty)}")
        {
            Instance = instance.Length > 0 ? instance : null,
            OnField = "OrderId",
            OnValue = id,
            Severity = failed ? Severity.Critical : slow ? Severity.Warning : Severity.Info,
            EvidenceQuery = Evidence(id, from, to),
        };
    }

    private static JsonObject Evidence(string id, long from, long to) => new()
    {
        ["from"] = from,
        ["to"] = to,
        ["filters"] = new JsonArray(new JsonObject { ["field"] = "OrderId", ["op"] = "eq", ["value"] = id }),
    };
}
