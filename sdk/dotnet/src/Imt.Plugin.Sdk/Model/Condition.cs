using System.Text.Json.Nodes;

namespace Imt.Plugin;

/// <summary>
/// A condition in IMT's profile condition language, used by marker rules and annotator interest. IMT evaluates
/// it; the SDK only carries it.
/// </summary>
/// <param name="Field">A field name or a pseudo-field (<c>@level</c>, <c>@message</c>, <c>@exception</c>, <c>@source</c>, ...).</param>
/// <param name="Op">The operator: <c>eq</c>, <c>ne</c>, <c>contains</c>, <c>notContains</c>, <c>regex</c>, <c>exists</c>, <c>missing</c>, <c>gt</c>, <c>lt</c>, <c>in</c>.</param>
/// <param name="Value">The value (a string, number, boolean or list of strings), or null for none.</param>
public sealed record Condition(string Field, string Op, JsonNode? Value = null)
{
    /// <summary><paramref name="field"/> equals <paramref name="value"/>.</summary>
    /// <param name="field">The field.</param>
    /// <param name="value">The value.</param>
    /// <returns>The condition.</returns>
    public static Condition Eq(string field, string value) => new(field, "eq", JsonValue.Create(value));

    /// <summary><paramref name="field"/> differs from <paramref name="value"/>.</summary>
    /// <param name="field">The field.</param>
    /// <param name="value">The value.</param>
    /// <returns>The condition.</returns>
    public static Condition Ne(string field, string value) => new(field, "ne", JsonValue.Create(value));

    /// <summary><paramref name="field"/> contains <paramref name="value"/>.</summary>
    /// <param name="field">The field.</param>
    /// <param name="value">The text to find.</param>
    /// <returns>The condition.</returns>
    public static Condition Contains(string field, string value) => new(field, "contains", JsonValue.Create(value));

    /// <summary><paramref name="field"/> matches the regular expression <paramref name="pattern"/>.</summary>
    /// <param name="field">The field.</param>
    /// <param name="pattern">The pattern (Rust regex syntax).</param>
    /// <returns>The condition.</returns>
    public static Condition Regex(string field, string pattern) => new(field, "regex", JsonValue.Create(pattern));

    /// <summary><paramref name="field"/> is greater than <paramref name="value"/>.</summary>
    /// <param name="field">The field.</param>
    /// <param name="value">The bound.</param>
    /// <returns>The condition.</returns>
    public static Condition Gt(string field, double value) => new(field, "gt", JsonValue.Create(value));

    /// <summary><paramref name="field"/> is less than <paramref name="value"/>.</summary>
    /// <param name="field">The field.</param>
    /// <param name="value">The bound.</param>
    /// <returns>The condition.</returns>
    public static Condition Lt(string field, double value) => new(field, "lt", JsonValue.Create(value));

    /// <summary><paramref name="field"/> is one of <paramref name="values"/>.</summary>
    /// <param name="field">The field.</param>
    /// <param name="values">The accepted values.</param>
    /// <returns>The condition.</returns>
    public static Condition In(string field, params string[] values)
    {
        ArgumentNullException.ThrowIfNull(values);
        return new(field, "in", new JsonArray([.. values.Select(v => (JsonNode?)JsonValue.Create(v))]));
    }

    /// <summary><paramref name="field"/> is present.</summary>
    /// <param name="field">The field.</param>
    /// <returns>The condition.</returns>
    public static Condition Exists(string field) => new(field, "exists");

    /// <summary><paramref name="field"/> is absent.</summary>
    /// <param name="field">The field.</param>
    /// <returns>The condition.</returns>
    public static Condition Missing(string field) => new(field, "missing");
}
