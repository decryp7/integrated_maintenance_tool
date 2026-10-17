using System.Text.Json.Nodes;

namespace Imt.Plugin;

/// <summary>
/// One log record, as IMT stores it. Importers create records; transforms change them; annotators and
/// exporters read them. <see cref="Id"/>, <see cref="Source"/> and <see cref="Markers"/> are assigned by IMT.
/// </summary>
public sealed class Record
{
    /// <summary>Creates an empty record.</summary>
    public Record()
    {
    }

    /// <summary>Creates a record at a line of its file.</summary>
    /// <param name="line">The 1-based line number where the record starts.</param>
    public Record(uint line) => Line = line;

    /// <summary>Time in milliseconds since the Unix epoch (UTC), or null when the record has none.</summary>
    public long? Timestamp { get; set; }

    /// <summary>The time as a <see cref="DateTimeOffset"/> (UTC), or null.</summary>
    public DateTimeOffset? Time
    {
        get => Timestamp is { } ms ? DateTimeOffset.FromUnixTimeMilliseconds(ms) : null;
        set => Timestamp = value?.ToUnixTimeMilliseconds();
    }

    /// <summary>The severity (default <see cref="Level.Information"/>).</summary>
    public Level Level { get; set; } = Level.Information;

    /// <summary>The rendered message.</summary>
    public string Message { get; set; } = string.Empty;

    /// <summary>The message template, for structured logs (e.g. <c>"Order {OrderId} failed"</c>).</summary>
    public string? Template { get; set; }

    /// <summary>An exception or stack trace attached to the record.</summary>
    public string? Exception { get; set; }

    /// <summary>Named values (any JSON: strings, numbers, booleans, objects, arrays). Order is kept.</summary>
    public JsonObject Fields { get; set; } = [];

    /// <summary>The 1-based line where the record starts (relative to the tail for live tail).</summary>
    public uint Line { get; set; }

    /// <summary>The record's id in the workspace (assigned by IMT; 0 from importers).</summary>
    public uint Id { get; set; }

    /// <summary>The file the record came from (assigned by IMT; 0 from importers).</summary>
    public uint Source { get; set; }

    /// <summary>Marker bits (assigned by IMT; plugins mark through <see cref="TransformBatch.Mark"/>).</summary>
    public ulong Markers { get; set; }

    /// <summary>A field's value as text (strings unquoted), or null when it is missing.</summary>
    /// <param name="name">The field name.</param>
    /// <returns>The text, or null.</returns>
    public string? FieldText(string name) => Fields.TryGetPropertyValue(name, out JsonNode? v) && v is not null
        ? (v is JsonValue jv && jv.TryGetValue(out string? s) ? s : v.ToJsonString())
        : null;

    /// <summary>A deep copy of the record.</summary>
    /// <returns>The copy.</returns>
    public Record Clone() => new()
    {
        Timestamp = Timestamp,
        Level = Level,
        Message = Message,
        Template = Template,
        Exception = Exception,
        Fields = (JsonObject)Fields.DeepClone(),
        Line = Line,
        Id = Id,
        Source = Source,
        Markers = Markers,
    };
}
