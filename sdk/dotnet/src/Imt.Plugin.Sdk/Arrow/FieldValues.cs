using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace Imt.Plugin.Arrow;

/// <summary>The Arrow type of a field column.</summary>
internal enum FieldKind
{
    Str,
    I64,
    U64,
    F64,
    Bool,
    Json,
}

/// <summary>Classifies JSON values the way <c>serde_json</c> does, so both SDKs pick the same column types.</summary>
internal static class FieldValues
{
    public static FieldKind KindOf(JsonNode? node)
    {
        if (node is not JsonValue v)
        {
            return FieldKind.Json;
        }

        if (v.TryGetValue(out JsonElement e))
        {
            return e.ValueKind switch
            {
                JsonValueKind.String => FieldKind.Str,
                JsonValueKind.True or JsonValueKind.False => FieldKind.Bool,
                JsonValueKind.Number when e.TryGetInt64(out _) => FieldKind.I64,
                JsonValueKind.Number when e.TryGetUInt64(out _) => FieldKind.U64,
                JsonValueKind.Number => double.IsFinite(e.GetDouble()) ? FieldKind.F64 : FieldKind.Json,
                _ => FieldKind.Json,
            };
        }

        if (v.TryGetValue(out string? _) || v.TryGetValue(out char _))
        {
            return FieldKind.Str;
        }

        if (v.TryGetValue(out bool _))
        {
            return FieldKind.Bool;
        }

        if (v.TryGetValue(out long _) || v.TryGetValue(out int _) || v.TryGetValue(out short _) || v.TryGetValue(out sbyte _)
            || v.TryGetValue(out byte _) || v.TryGetValue(out ushort _) || v.TryGetValue(out uint _))
        {
            return FieldKind.I64;
        }

        if (v.TryGetValue(out ulong u))
        {
            return u <= long.MaxValue ? FieldKind.I64 : FieldKind.U64;
        }

        if (v.TryGetValue(out double d))
        {
            return double.IsFinite(d) ? FieldKind.F64 : FieldKind.Json;
        }

        if (v.TryGetValue(out float f))
        {
            return float.IsFinite(f) ? FieldKind.F64 : FieldKind.Json;
        }

        if (v.TryGetValue(out decimal _))
        {
            return FieldKind.F64;
        }

        // Other CLR values (dates, GUIDs, ...) serialise as JSON strings or numbers.
        return v.GetValueKind() == JsonValueKind.String ? FieldKind.Str : FieldKind.Json;
    }

    public static string Text(JsonValue v) =>
        v.TryGetValue(out JsonElement e) ? e.GetString() ?? string.Empty
        : v.TryGetValue(out string? s) ? s ?? string.Empty
        : JsonSerializer.Deserialize<string>(v.ToJsonString()) ?? string.Empty;

    public static long Int64(JsonValue v) => long.Parse(v.ToJsonString(), NumberStyles.Integer, CultureInfo.InvariantCulture);

    public static ulong UInt64(JsonValue v) => ulong.Parse(v.ToJsonString(), NumberStyles.Integer, CultureInfo.InvariantCulture);

    public static double Float64(JsonValue v) =>
        v.TryGetValue(out JsonElement e) ? e.GetDouble()
        : v.TryGetValue(out double d) ? d
        : v.TryGetValue(out float f) ? f
        : double.Parse(v.ToJsonString(), NumberStyles.Float, CultureInfo.InvariantCulture);

    public static bool Bool(JsonValue v) => v.GetValue<bool>();

    /// <summary>JSON text of any value (non-finite numbers become <c>null</c>, as in serde_json).</summary>
    public static string Json(JsonNode? node)
    {
        if (node is null)
        {
            return "null";
        }

        try
        {
            return node.ToJsonString(RecordSchema.Json);
        }
        catch (ArgumentException)
        {
            return "null";
        }
    }
}
