namespace Imt.Plugin;

/// <summary>What a marker means for the timeline.</summary>
public enum MarkerRole
{
    /// <summary>An ordinary marker.</summary>
    None = 0,

    /// <summary>The application started.</summary>
    Start = 1,

    /// <summary>The application stopped.</summary>
    Stop = 2,

    /// <summary>The application crashed.</summary>
    Crash = 3,
}

/// <summary>
/// A marker kind a plugin contributes to the timeline: its style and its default rule (all
/// <see cref="When"/> conditions must match). IMT shows it as <c>&lt;plugin-id&gt;/&lt;kind&gt;</c>. A kind
/// without conditions is only set by the plugin's transforms (<see cref="TransformBatch.Mark"/>).
/// </summary>
public sealed class MarkerKind
{
    /// <summary>Creates a marker kind.</summary>
    /// <param name="kind">1–64 characters from A–Z, a–z, 0–9, '.', '_' and '-'.</param>
    /// <param name="label">The name shown to the user.</param>
    public MarkerKind(string kind, string label)
    {
        Kind = kind;
        Label = label;
    }

    /// <summary>The kind's short name (unique within the plugin).</summary>
    public string Kind { get; }

    /// <summary>The name shown to the user.</summary>
    public string Label { get; }

    /// <summary>A longer explanation.</summary>
    public string? Description { get; init; }

    /// <summary>A CSS colour, e.g. <c>#F2545B</c>.</summary>
    public string? Color { get; init; }

    /// <summary>An icon name, e.g. <c>x-circle</c>.</summary>
    public string? Icon { get; init; }

    /// <summary>A short badge text.</summary>
    public string? Badge { get; init; }

    /// <summary>The badge shape, e.g. <c>circle</c>.</summary>
    public string? Shape { get; init; }

    /// <summary>The kind's role on the timeline.</summary>
    public MarkerRole Role { get; init; }

    /// <summary>The default rule: every condition must match. Empty: only transforms set it.</summary>
    public IReadOnlyList<Condition> When { get; init; } = [];
}
