using Imt.Plugin;

namespace OrderAudit;

/// <summary>Marker kinds for the timeline: failed orders and payment retries by rule, slow orders by code.</summary>
public sealed class OrderMarkers : IMarkerKinds
{
    /// <summary>A failed order.</summary>
    public const string OrderFailed = "order-failed";

    /// <summary>A payment retry.</summary>
    public const string PaymentRetry = "payment-retry";

    /// <summary>An order slower than the threshold (set by <see cref="OrderEnrichTransform"/>).</summary>
    public const string SlowOrder = "slow-order";

    /// <inheritdoc/>
    public IReadOnlyList<MarkerKind> MarkerKinds { get; } =
    [
        new MarkerKind(OrderFailed, "Order failed")
        {
            Description = "The order service gave up on an order.",
            Color = "#F2545B",
            Icon = "x-circle",
            When = [Condition.Eq("Event", "OrderFailed")],
        },
        new MarkerKind(PaymentRetry, "Payment retry")
        {
            Description = "The payment gateway did not answer and the payment was retried.",
            Color = "#F5A623",
            Icon = "restart",
            When = [Condition.Eq("Event", "PaymentRetry")],
        },
        new MarkerKind(SlowOrder, "Slow order")
        {
            Description = "The order took longer than the slow order threshold (Settings → Order audit).",
            Color = "#8E6CEF",
            Icon = "timer",
        },
    ];
}
