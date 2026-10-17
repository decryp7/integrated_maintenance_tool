using System.Text.Json.Nodes;

namespace Imt.Plugin;

/// <summary>How serious an annotation is.</summary>
public enum Severity
{
    /// <summary>Informational.</summary>
    Info = 0,

    /// <summary>Worth a look.</summary>
    Warning = 1,

    /// <summary>A problem.</summary>
    Critical = 2,
}

/// <summary>A range (or a point, when <see cref="From"/> equals <see cref="To"/>) an annotator found across records.</summary>
public sealed class Annotation
{
    /// <summary>Creates an annotation.</summary>
    /// <param name="kind">What it is, e.g. <c>order</c>.</param>
    /// <param name="from">Start, milliseconds since the Unix epoch.</param>
    /// <param name="to">End, milliseconds since the Unix epoch.</param>
    /// <param name="label">The text shown to the user.</param>
    public Annotation(string kind, long from, long to, string label)
    {
        Kind = kind;
        From = from;
        To = to;
        Label = label;
    }

    /// <summary>What it is, e.g. <c>order</c>.</summary>
    public string Kind { get; }

    /// <summary>Start, milliseconds since the Unix epoch.</summary>
    public long From { get; }

    /// <summary>End, milliseconds since the Unix epoch.</summary>
    public long To { get; }

    /// <summary>The text shown to the user.</summary>
    public string Label { get; }

    /// <summary>The instance it belongs to; null for every instance.</summary>
    public string? Instance { get; init; }

    /// <summary>Applies to records whose field <see cref="OnField"/> equals <see cref="OnValue"/>.</summary>
    public string? OnField { get; init; }

    /// <summary>The value of <see cref="OnField"/>.</summary>
    public string? OnValue { get; init; }

    /// <summary>How serious it is.</summary>
    public Severity Severity { get; init; }

    /// <summary>The query that shows the evidence, in IMT's query schema (e.g. <c>{"from":1,"to":2,"filters":[...]}</c>).</summary>
    public JsonObject? EvidenceQuery { get; init; }
}
