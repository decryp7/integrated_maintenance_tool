using Imt.Plugin;

namespace ToyFormat;

/// <summary>The "hello world" IMT plugin: an importer for the toy line format, with marker kinds and settings.</summary>
public static class ToyPlugin
{
    /// <summary>The plugin id.</summary>
    public const string Id = "com.example.toyformat";

    /// <summary>Builds the plugin.</summary>
    /// <returns>The plugin.</returns>
    public static PluginDefinition Build()
    {
        var settings = new ToySettings();
        return PluginBuilder.Create(Id)
            .Name("Toy format")
            .Version("1.0.0")
            .Add(new ToyImporter(settings))
            .Add(new ToyMarkers())
            .Add(settings)
            .Build();
    }
}
