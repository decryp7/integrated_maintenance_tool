using Imt.Plugin;

namespace ToyFormat;

/// <summary>Marker kinds of the toy format: errors and slow operations.</summary>
public sealed class ToyMarkers : IMarkerKinds
{
    /// <inheritdoc/>
    public IReadOnlyList<MarkerKind> MarkerKinds { get; } =
    [
        new MarkerKind("toy-error", "Toy error")
        {
            Description = "A record of level Error.",
            Color = "#F2545B",
            Icon = "x-circle",
            When = [Condition.Eq("@level", "error")],
        },
        new MarkerKind("toy-slow", "Slow operation")
        {
            Description = "An operation that took more than a second (ms > 1000).",
            Color = "#F5A623",
            Icon = "clock",
            When = [Condition.Gt("ms", 1000)],
        },
    ];
}
