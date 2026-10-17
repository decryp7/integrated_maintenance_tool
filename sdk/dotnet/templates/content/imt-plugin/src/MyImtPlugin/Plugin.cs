using Imt.Plugin;

namespace MyImtPlugin;

/// <summary>The plugin: its identity and contributions. Capabilities come from the interfaces each one implements.</summary>
public static class Plugin
{
    /// <summary>The plugin id (reverse DNS).</summary>
    public const string Id = "com.example.myimtplugin";

    /// <summary>Builds the plugin.</summary>
    /// <returns>The plugin.</returns>
    public static PluginDefinition Build() =>
        PluginBuilder.Create(Id)
            .Name("MyImtPlugin")
            .Version("0.1.0")
            .Add(new LineImporter())
            .Build();
}
