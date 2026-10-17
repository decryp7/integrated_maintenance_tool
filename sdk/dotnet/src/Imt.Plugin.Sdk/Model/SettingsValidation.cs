using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;

namespace Imt.Plugin;

/// <summary>A setting that failed validation.</summary>
/// <param name="Key">The setting's key (empty for the whole object).</param>
/// <param name="Message">What is wrong.</param>
public sealed record SettingError(string Key, string Message);

/// <summary>
/// Validation of settings schemas and values, ported from <c>imt_core::settings</c> so a plugin re-validates
/// what IMT already checked (defence in depth) and authors get the same errors at build time.
/// </summary>
public static class SettingsValidation
{
    /// <summary>Most fields per section.</summary>
    public const int MaxFields = 200;

    /// <summary>Longest text value.</summary>
    public const int MaxTextLength = 64 * 1024;

    /// <summary>Most items of a list.</summary>
    public const int MaxListItems = 1000;

    /// <summary>Longest list item.</summary>
    public const int MaxItemLength = 4096;

    /// <summary>Longest path.</summary>
    public const int MaxPathLength = 4096;

    private const int MaxIdLength = 128;

    /// <summary>Checks a section's schema (ids, kinds, defaults).</summary>
    /// <param name="section">The section.</param>
    /// <returns>Null when valid, otherwise the problem.</returns>
    public static string? CheckSchema(SettingsSection section)
    {
        ArgumentNullException.ThrowIfNull(section);
        if (section.Fields.Count > MaxFields)
        {
            return $"the section has {section.Fields.Count} fields; the limit is {MaxFields}";
        }

        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var f in section.Fields)
        {
            if (!IsId(f.Key))
            {
                return $"field key \"{f.Key}\" must be 1-{MaxIdLength} characters of A-Z, a-z, 0-9, '.', '_' or '-'";
            }

            if (!seen.Add(f.Key))
            {
                return $"field \"{f.Key}\" is declared twice";
            }

            if (CheckKind(f.Kind) is { } kindError)
            {
                return $"field \"{f.Key}\": {kindError}";
            }

            if (ValidateField(f, f.Default) is { Error: { } e })
            {
                return $"field \"{f.Key}\": default {f.Default?.ToJsonString() ?? "null"}: {e}";
            }
        }

        return null;
    }

    /// <summary>
    /// Validates values (a JSON object) against a section: missing or null keys take the default, unknown
    /// keys are ignored.
    /// </summary>
    /// <param name="section">The section.</param>
    /// <param name="values">The values.</param>
    /// <param name="valid">The valid values (invalid ones replaced by defaults).</param>
    /// <returns>The problems; empty when every value is valid.</returns>
    public static IReadOnlyList<SettingError> Validate(SettingsSection section, JsonNode? values, out SettingsValues valid)
    {
        ArgumentNullException.ThrowIfNull(section);
        var errors = new List<SettingError>();
        JsonObject? obj = values as JsonObject;
        if (values is not null and not JsonObject)
        {
            errors.Add(new SettingError(string.Empty, "Settings must be an object of key/value pairs."));
        }

        var outValues = new Dictionary<string, JsonNode?>(StringComparer.Ordinal);
        foreach (var f in section.Fields)
        {
            JsonNode? v = null;
            obj?.TryGetPropertyValue(f.Key, out v);
            if (v is null)
            {
                outValues[f.Key] = Normalize(f.Default);
                continue;
            }

            var (value, error) = ValidateField(f, v);
            if (error is not null)
            {
                errors.Add(new SettingError(f.Key, error));
                outValues[f.Key] = Normalize(f.Default);
            }
            else
            {
                outValues[f.Key] = value;
            }
        }

        valid = new SettingsValues(outValues);
        return errors;
    }

    private static JsonNode? Normalize(JsonNode? v) => v is null ? null : JsonNode.Parse(v.ToJsonString());

    private static bool IsId(string id) =>
        id.Length is > 0 and <= MaxIdLength && id.All(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '_' or '-');

    private static string? CheckKind(SettingKind kind) => kind switch
    {
        IntSetting i when i.Min > i.Max || i.Step < 1 => "needs min <= max and step >= 1",
        FloatSetting f when !(double.IsFinite(f.Min) && double.IsFinite(f.Max) && double.IsFinite(f.Step)) || f.Min > f.Max || f.Step <= 0 =>
            "needs finite min <= max and step > 0",
        TextSetting t when t.MaxLen is <= 0 or > MaxTextLength => $"max_len must be 1-{MaxTextLength}",
        TextSetting { Pattern: { } p } when Anchored(p) is null => "pattern: not a valid regular expression",
        EnumSetting e when e.Options.Count == 0 || e.Options.Select(o => o.Value).Distinct(StringComparer.Ordinal).Count() != e.Options.Count =>
            "needs at least one option and unique values",
        DurationSetting d when d.MinMs > d.MaxMs => "needs min_ms <= max_ms",
        SizeSetting s when s.MinBytes > s.MaxBytes => "needs min_bytes <= max_bytes",
        StringListSetting l when l.MaxItems is <= 0 or > MaxListItems => $"max_items must be 1-{MaxListItems}",
        _ => null,
    };

    private static Regex? Anchored(string pattern)
    {
        try
        {
            return new Regex($"^(?:{pattern})$", RegexOptions.CultureInvariant, TimeSpan.FromMilliseconds(250));
        }
        catch (ArgumentException)
        {
            return null;
        }
    }

    private static string WithUnit<T>(T n, string? unit)
        where T : IFormattable =>
        string.IsNullOrEmpty(unit) ? n.ToString(null, CultureInfo.InvariantCulture) : $"{n.ToString(null, CultureInfo.InvariantCulture)} {unit}";

    /// <summary>An integer, also from a float without a fractional part (JavaScript numbers).</summary>
    private static long? Whole(JsonNode v)
    {
        if (v is not JsonValue jv || jv.GetValueKind() != JsonValueKind.Number)
        {
            return null;
        }

        if (jv.TryGetValue(out long l))
        {
            return l;
        }

        if (jv.TryGetValue(out double d) && Math.Abs(d) < 9.0e15 && Math.Floor(d) == d)
        {
            return (long)d;
        }

        return null;
    }

    private static string? Text(JsonNode v) =>
        v is JsonValue jv && jv.GetValueKind() == JsonValueKind.String ? jv.GetValue<string>() : null;

    private static int CharCount(string s) => s.EnumerateRunes().Count();

    internal static (JsonNode? Value, string? Error) ValidateField(SettingField field, JsonNode? value)
    {
        if (value is null)
        {
            return (null, "Must have a value.");
        }

        // Normalise values built in code (e.g. a JsonValue of int) to their JSON form, like values from IMT.
        value = JsonNode.Parse(value.ToJsonString())!;

        switch (field.Kind)
        {
            case BoolSetting:
                return value is JsonValue b && b.GetValueKind() is JsonValueKind.True or JsonValueKind.False
                    ? (JsonValue.Create(b.GetValue<bool>()), null)
                    : (null, "Must be on or off (true or false).");
            case IntSetting i:
                {
                    if (Whole(value) is not { } n)
                    {
                        return (null, "Must be a whole number.");
                    }

                    if (n < i.Min || n > i.Max)
                    {
                        return (null, $"Must be between {WithUnit(i.Min, i.Unit)} and {WithUnit(i.Max, i.Unit)}.");
                    }

                    return (n - i.Min) % i.Step != 0 ? (null, $"Must be {i.Min} plus a multiple of {i.Step}.") : (JsonValue.Create(n), null);
                }

            case FloatSetting f:
                {
                    if (value is not JsonValue fv || fv.GetValueKind() != JsonValueKind.Number || !fv.TryGetValue(out double d) || !double.IsFinite(d))
                    {
                        return (null, "Must be a number.");
                    }

                    return d < f.Min || d > f.Max
                        ? (null, $"Must be between {WithUnit(f.Min, f.Unit)} and {WithUnit(f.Max, f.Unit)}.")
                        : (JsonValue.Create(d), null);
                }

            case TextSetting t:
                {
                    if (Text(value) is not { } s)
                    {
                        return (null, "Must be text.");
                    }

                    if (CharCount(s) > t.MaxLen)
                    {
                        return (null, $"Must be at most {t.MaxLen} characters.");
                    }

                    if (t.Pattern is { } p)
                    {
                        if (Anchored(p) is not { } re)
                        {
                            return (null, "The setting's pattern is invalid.");
                        }

                        try
                        {
                            if (!re.IsMatch(s))
                            {
                                return (null, $"Does not have the expected format ({p}).");
                            }
                        }
                        catch (RegexMatchTimeoutException)
                        {
                            return (null, $"Does not have the expected format ({p}).");
                        }
                    }

                    return (JsonValue.Create(s), null);
                }

            case EnumSetting e:
                {
                    var s = Text(value);
                    return s is not null && e.Options.Any(o => o.Value == s)
                        ? (JsonValue.Create(s), null)
                        : (null, $"Must be one of: {string.Join(", ", e.Options.Select(o => o.Value))}.");
                }

            case PathSetting:
                {
                    if (Text(value) is not { } s)
                    {
                        return (null, "Must be a path.");
                    }

                    if (CharCount(s) > MaxPathLength)
                    {
                        return (null, $"Must be at most {MaxPathLength} characters.");
                    }

                    if (s.Contains('\0', StringComparison.Ordinal))
                    {
                        return (null, "Must not contain NUL characters.");
                    }

                    return s.Length > 0 && !Path.IsPathFullyQualified(s) ? (null, "Must be an absolute path (or empty).") : (JsonValue.Create(s), null);
                }

            case DurationSetting dur:
                {
                    if (Whole(value) is not { } n || n < 0)
                    {
                        return (null, "Must be a duration in whole milliseconds.");
                    }

                    return (ulong)n < dur.MinMs || (ulong)n > dur.MaxMs
                        ? (null, $"Must be between {dur.MinMs} ms and {dur.MaxMs} ms.")
                        : (JsonValue.Create(n), null);
                }

            case SizeSetting size:
                {
                    if (Whole(value) is not { } n || n < 0)
                    {
                        return (null, "Must be a size in whole bytes.");
                    }

                    return (ulong)n < size.MinBytes || (ulong)n > size.MaxBytes
                        ? (null, $"Must be between {size.MinBytes} bytes and {size.MaxBytes} bytes.")
                        : (JsonValue.Create(n), null);
                }

            case StringListSetting list:
                {
                    if (value is not JsonArray items)
                    {
                        return (null, "Must be a list of text.");
                    }

                    if (items.Count > list.MaxItems)
                    {
                        return (null, $"Must have at most {list.MaxItems} items.");
                    }

                    var outList = new JsonArray();
                    for (int i = 0; i < items.Count; i++)
                    {
                        if (items[i] is not { } item || Text(item) is not { } s)
                        {
                            return (null, $"Item {i + 1} must be text.");
                        }

                        if (CharCount(s) > MaxItemLength)
                        {
                            return (null, $"Item {i + 1} must be at most {MaxItemLength} characters.");
                        }

                        outList.Add(JsonValue.Create(s));
                    }

                    return (outList, null);
                }

            default:
                return (null, "Unknown setting kind.");
        }
    }
}
