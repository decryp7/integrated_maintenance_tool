using System.Text.Json.Nodes;

namespace Imt.Plugin;

/// <summary>The kind of a setting: its type and limits. One subclass per kind, so a field cannot mix them.</summary>
public abstract record SettingKind
{
    private protected SettingKind()
    {
    }
}

/// <summary>On or off.</summary>
public sealed record BoolSetting : SettingKind;

/// <summary>A whole number in <c>[Min, Max]</c>, <c>Min + k * Step</c>.</summary>
/// <param name="Min">Smallest value.</param>
/// <param name="Max">Largest value.</param>
/// <param name="Step">Step (at least 1).</param>
/// <param name="Unit">Unit shown next to the value.</param>
public sealed record IntSetting(long Min, long Max, long Step = 1, string? Unit = null) : SettingKind;

/// <summary>A number in <c>[Min, Max]</c>.</summary>
/// <param name="Min">Smallest value.</param>
/// <param name="Max">Largest value.</param>
/// <param name="Step">Step of the input control (greater than 0).</param>
/// <param name="Unit">Unit shown next to the value.</param>
public sealed record FloatSetting(double Min, double Max, double Step = 0.1, string? Unit = null) : SettingKind;

/// <summary>Text of at most <c>MaxLen</c> characters, optionally matching <c>Pattern</c> (whole value).</summary>
/// <param name="MaxLen">Longest value (1–65536).</param>
/// <param name="Pattern">A regular expression the whole value must match.</param>
public sealed record TextSetting(int MaxLen = 256, string? Pattern = null) : SettingKind;

/// <summary>One of a list of options.</summary>
/// <param name="Options">The options (value and label); values are unique.</param>
public sealed record EnumSetting(IReadOnlyList<EnumOption> Options) : SettingKind;

/// <summary>An option of an <see cref="EnumSetting"/>.</summary>
/// <param name="Value">The stored value.</param>
/// <param name="Label">The text shown.</param>
public sealed record EnumOption(string Value, string Label);

/// <summary>An absolute path (or empty).</summary>
/// <param name="Dir">A directory rather than a file.</param>
public sealed record PathSetting(bool Dir = false) : SettingKind;

/// <summary>A duration in whole milliseconds.</summary>
/// <param name="MinMs">Shortest value.</param>
/// <param name="MaxMs">Longest value.</param>
public sealed record DurationSetting(ulong MinMs, ulong MaxMs) : SettingKind;

/// <summary>A size in whole bytes.</summary>
/// <param name="MinBytes">Smallest value.</param>
/// <param name="MaxBytes">Largest value.</param>
public sealed record SizeSetting(ulong MinBytes, ulong MaxBytes) : SettingKind;

/// <summary>A list of text items.</summary>
/// <param name="MaxItems">Most items (1–1000).</param>
public sealed record StringListSetting(int MaxItems = 100) : SettingKind;

/// <summary>One setting of a plugin's section.</summary>
public sealed class SettingField
{
    /// <summary>Creates a setting.</summary>
    /// <param name="key">The key (1–128 characters of A–Z, a–z, 0–9, '.', '_', '-').</param>
    /// <param name="label">The label shown to the user.</param>
    /// <param name="kind">The kind and its limits.</param>
    /// <param name="defaultValue">The default value (must be valid for the kind).</param>
    public SettingField(string key, string label, SettingKind kind, JsonNode? defaultValue)
    {
        Key = key;
        Label = label;
        Kind = kind;
        Default = defaultValue;
    }

    /// <summary>The key.</summary>
    public string Key { get; }

    /// <summary>The label.</summary>
    public string Label { get; }

    /// <summary>The kind and its limits.</summary>
    public SettingKind Kind { get; }

    /// <summary>The default value.</summary>
    public JsonNode? Default { get; }

    /// <summary>A longer explanation.</summary>
    public string Description { get; init; } = string.Empty;

    /// <summary>A change takes effect only after IMT restarts.</summary>
    public bool RestartRequired { get; init; }

    /// <summary>Shown only in the advanced view.</summary>
    public bool Advanced { get; init; }
}

/// <summary>A plugin's settings section: IMT renders, validates and stores it, and hands the plugin the values.</summary>
public sealed class SettingsSection
{
    /// <summary>Creates a section.</summary>
    /// <param name="title">The section's title.</param>
    /// <param name="fields">The settings.</param>
    public SettingsSection(string title, IReadOnlyList<SettingField> fields)
    {
        Title = title;
        Fields = fields;
    }

    /// <summary>The title.</summary>
    public string Title { get; }

    /// <summary>A longer explanation.</summary>
    public string Description { get; init; } = string.Empty;

    /// <summary>The settings.</summary>
    public IReadOnlyList<SettingField> Fields { get; }
}

/// <summary>Settings values that passed validation: every field of the section is present and valid.</summary>
public sealed class SettingsValues
{
    private readonly IReadOnlyDictionary<string, JsonNode?> values;

    internal SettingsValues(IReadOnlyDictionary<string, JsonNode?> values) => this.values = values;

    /// <summary>The keys.</summary>
    public IEnumerable<string> Keys => values.Keys;

    /// <summary>The raw JSON value of <paramref name="key"/>, or null.</summary>
    /// <param name="key">The key.</param>
    /// <returns>The value.</returns>
    public JsonNode? this[string key] => values.TryGetValue(key, out JsonNode? v) ? v : null;

    /// <summary>A boolean setting.</summary>
    /// <param name="key">The key.</param>
    /// <returns>The value.</returns>
    public bool GetBool(string key) => Value(key).GetValue<bool>();

    /// <summary>An integer, duration (ms) or size (bytes) setting.</summary>
    /// <param name="key">The key.</param>
    /// <returns>The value.</returns>
    public long GetInt64(string key) => Value(key).GetValue<long>();

    /// <summary>A number setting.</summary>
    /// <param name="key">The key.</param>
    /// <returns>The value.</returns>
    public double GetDouble(string key) => Value(key).GetValue<double>();

    /// <summary>A text, enum or path setting.</summary>
    /// <param name="key">The key.</param>
    /// <returns>The value.</returns>
    public string GetString(string key) => Value(key).GetValue<string>();

    /// <summary>A duration setting.</summary>
    /// <param name="key">The key.</param>
    /// <returns>The value.</returns>
    public TimeSpan GetDuration(string key) => TimeSpan.FromMilliseconds(GetInt64(key));

    /// <summary>A string-list setting.</summary>
    /// <param name="key">The key.</param>
    /// <returns>The items.</returns>
    public IReadOnlyList<string> GetStringList(string key) =>
        this[key] is JsonArray a ? [.. a.Select(i => i?.GetValue<string>() ?? string.Empty)] : [];

    /// <summary>The values as a JSON object.</summary>
    /// <returns>The object.</returns>
    public JsonObject ToJson()
    {
        var o = new JsonObject();
        foreach (var (k, v) in values)
        {
            o[k] = v?.DeepClone();
        }

        return o;
    }

    private JsonValue Value(string key) => this[key] as JsonValue
        ?? throw new KeyNotFoundException($"setting \"{key}\" has no value");
}
