using System.Text.Json.Nodes;

namespace Imt.Plugin.Tests;

public class SettingsTests
{
    private static string? Error(SettingKind kind, JsonNode? value) =>
        SettingsValidation.ValidateField(new SettingField("k", "K", kind, null), value).Error;

    [Fact]
    public void Every_kind_validates_like_imt()
    {
        Assert.Null(Error(new BoolSetting(), true));
        Assert.Contains("on or off", Error(new BoolSetting(), "yes"), StringComparison.Ordinal);
        Assert.Null(Error(new IntSetting(0, 10, 2), 6.0));
        Assert.Contains("whole number", Error(new IntSetting(0, 10), 5.5), StringComparison.Ordinal);
        Assert.Contains("whole number", Error(new IntSetting(0, 10), 1e300), StringComparison.Ordinal);
        Assert.Contains("between 0 x and 10 x", Error(new IntSetting(0, 10, 1, "x"), 11), StringComparison.Ordinal);
        Assert.Contains("multiple of 2", Error(new IntSetting(0, 10, 2), 3), StringComparison.Ordinal);
        Assert.Null(Error(new FloatSetting(0, 1), 0.5));
        Assert.Contains("a number", Error(new FloatSetting(0, 1), "0.5"), StringComparison.Ordinal);
        Assert.Contains("between", Error(new FloatSetting(0, 1, 0.1, "s"), 2), StringComparison.Ordinal);
        Assert.Null(Error(new TextSetting(3, "[a-z]+"), "abc"));
        Assert.Contains("text", Error(new TextSetting(3), 1), StringComparison.Ordinal);
        Assert.Contains("at most 3", Error(new TextSetting(3), "abcd"), StringComparison.Ordinal);
        Assert.Contains("format", Error(new TextSetting(3, "[a-z]+"), "AB"), StringComparison.Ordinal);
        Assert.Contains("pattern is invalid", Error(new TextSetting(3, "("), "a"), StringComparison.Ordinal);
        var e = new EnumSetting([new EnumOption("a", "A")]);
        Assert.Null(Error(e, "a"));
        Assert.Contains("one of: a", Error(e, "b"), StringComparison.Ordinal);
        Assert.Null(Error(new PathSetting(), string.Empty));
        Assert.Null(Error(new PathSetting(), Path.GetFullPath("/tmp")));
        Assert.Contains("absolute", Error(new PathSetting(), "relative/x"), StringComparison.Ordinal);
        Assert.Contains("NUL", Error(new PathSetting(), "/a\0b"), StringComparison.Ordinal);
        Assert.Contains("a path", Error(new PathSetting(), 3), StringComparison.Ordinal);
        Assert.Contains("at most", Error(new PathSetting(), "/" + new string('a', 5000)), StringComparison.Ordinal);
        Assert.Null(Error(new DurationSetting(1, 100), 50));
        Assert.Contains("whole milliseconds", Error(new DurationSetting(1, 100), -1), StringComparison.Ordinal);
        Assert.Contains("between", Error(new DurationSetting(1, 100), 500), StringComparison.Ordinal);
        Assert.Null(Error(new SizeSetting(1, 100), 50));
        Assert.Contains("whole bytes", Error(new SizeSetting(1, 100), "x"), StringComparison.Ordinal);
        Assert.Contains("between", Error(new SizeSetting(1, 100), 0), StringComparison.Ordinal);
        Assert.Null(Error(new StringListSetting(2), new JsonArray("a")));
        Assert.Contains("list of text", Error(new StringListSetting(2), "a"), StringComparison.Ordinal);
        Assert.Contains("at most 2", Error(new StringListSetting(2), new JsonArray("a", "b", "c")), StringComparison.Ordinal);
        Assert.Contains("Item 1 must be text", Error(new StringListSetting(2), new JsonArray(1)), StringComparison.Ordinal);
        Assert.Contains("Item 1 must be at most", Error(new StringListSetting(2), new JsonArray(new string('a', 5000))), StringComparison.Ordinal);
        Assert.Contains("Must have a value", Error(new BoolSetting(), null), StringComparison.Ordinal);
    }

    [Fact]
    public void Schemas_are_checked()
    {
        static string? Check(SettingKind kind, JsonNode? def, string key = "k") =>
            SettingsValidation.CheckSchema(new SettingsSection("S", [new SettingField(key, "K", kind, def)]));

        Assert.Null(Check(new BoolSetting(), false));
        Assert.Contains("must be 1-128", Check(new BoolSetting(), false, "bad key"), StringComparison.Ordinal);
        Assert.Contains("min <= max", Check(new IntSetting(5, 1), 1), StringComparison.Ordinal);
        Assert.Contains("finite", Check(new FloatSetting(0, double.NaN), 0), StringComparison.Ordinal);
        Assert.Contains("max_len", Check(new TextSetting(0), string.Empty), StringComparison.Ordinal);
        Assert.Contains("pattern", Check(new TextSetting(5, "("), string.Empty), StringComparison.Ordinal);
        Assert.Contains("unique", Check(new EnumSetting([]), "a"), StringComparison.Ordinal);
        Assert.Contains("min_ms", Check(new DurationSetting(5, 1), 1), StringComparison.Ordinal);
        Assert.Contains("min_bytes", Check(new SizeSetting(5, 1), 1), StringComparison.Ordinal);
        Assert.Contains("max_items", Check(new StringListSetting(0), new JsonArray()), StringComparison.Ordinal);
        Assert.Contains("default", Check(new BoolSetting(), "x"), StringComparison.Ordinal);
        Assert.Contains("declared twice", SettingsValidation.CheckSchema(new SettingsSection("S", [new SettingField("k", "K", new BoolSetting(), true), new SettingField("k", "K", new BoolSetting(), true)])), StringComparison.Ordinal);
        var many = Enumerable.Range(0, 201).Select(i => new SettingField($"k{i}", "K", new BoolSetting(), true)).ToList();
        Assert.Contains("limit", SettingsValidation.CheckSchema(new SettingsSection("S", many)), StringComparison.Ordinal);
    }

    [Fact]
    public void Values_take_defaults_and_report_every_problem()
    {
        var s = new SettingsSection("S",
        [
            new SettingField("a", "A", new IntSetting(0, 10), 3),
            new SettingField("b", "B", new BoolSetting(), true),
            new SettingField("c", "C", new DurationSetting(0, 1000), 100),
            new SettingField("d", "D", new TextSetting(5), "x"),
            new SettingField("e", "E", new FloatSetting(0, 1), 0.5),
        ]);
        Assert.Empty(SettingsValidation.Validate(s, new JsonObject { ["a"] = 4, ["unknown"] = 1, ["b"] = null }, out var v));
        Assert.Equal((4L, true, TimeSpan.FromMilliseconds(100), "x", 0.5), (v.GetInt64("a"), v.GetBool("b"), v.GetDuration("c"), v.GetString("d"), v.GetDouble("e")));
        Assert.Equal(["a", "b", "c", "d", "e"], v.Keys);
        Assert.Equal(4, v.ToJson()["a"]!.GetValue<long>());
        Assert.Null(v["nope"]);
        Assert.Empty(v.GetStringList("nope"));
        Assert.Throws<KeyNotFoundException>(() => v.GetBool("nope"));
        var errors = SettingsValidation.Validate(s, new JsonObject { ["a"] = 40, ["b"] = "x" }, out var fixedValues);
        Assert.Equal(["a", "b"], errors.Select(e => e.Key));
        Assert.Equal(3, fixedValues.GetInt64("a"));
        Assert.Single(SettingsValidation.Validate(s, new JsonArray(), out _));
        Assert.Empty(SettingsValidation.Validate(s, null, out _));
    }
}
