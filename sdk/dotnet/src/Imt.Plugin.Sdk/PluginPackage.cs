using System.Text.Json;
using System.Text.Json.Nodes;
using Google.Protobuf;
using Imt.Plugin.Hosting;

namespace Imt.Plugin;

/// <summary>
/// The plugin folder layout IMT's plugin manager installs (proposed in <c>sdk/README.md</c>):
/// <code>
/// &lt;id&gt;/
///   plugin.json            the manifest (this class writes it)
///   bin/&lt;rid&gt;/&lt;executable&gt;  one self-contained executable per OS/architecture (osx-arm64, linux-x64, win-x64, ...)
/// </code>
/// </summary>
public static class PluginPackage
{
    /// <summary>The version of the <c>plugin.json</c> format.</summary>
    public const int FormatVersion = 1;

    /// <summary>The manifest file name in the plugin folder.</summary>
    public const string ManifestFileName = "plugin.json";

    /// <summary>
    /// The <c>plugin.json</c> content: the format version, the contract (<c>grpc</c>, <c>imt.plugin.v1</c>), the
    /// executable's base name, and the handshake manifest in protobuf's canonical JSON form.
    /// </summary>
    /// <param name="plugin">The plugin.</param>
    /// <param name="executable">The executable's base name (without <c>.exe</c>).</param>
    /// <returns>The JSON text (indented).</returns>
    public static string ManifestJson(PluginDefinition plugin, string executable)
    {
        ArgumentNullException.ThrowIfNull(plugin);
        var manifest = JsonNode.Parse(new JsonFormatter(JsonFormatter.Settings.Default.WithFormatDefaultValues(true)).Format(ProtoMapping.ToProto(plugin)));
        var doc = new JsonObject
        {
            ["format"] = FormatVersion,
            ["contract"] = new JsonObject { ["kind"] = "grpc", ["package"] = "imt.plugin.v1", ["major"] = Contract.Major, ["minor"] = Contract.Minor },
            ["executable"] = executable,
            ["manifest"] = manifest,
        };
        return doc.ToJsonString(new JsonSerializerOptions { WriteIndented = true });
    }

    /// <summary>The running executable's base name, without directory and <c>.exe</c>.</summary>
    /// <returns>The name.</returns>
    public static string ExecutableName()
    {
        var path = Environment.ProcessPath ?? AppDomain.CurrentDomain.FriendlyName;
        var name = Path.GetFileNameWithoutExtension(path);
        return string.Equals(name, "dotnet", StringComparison.OrdinalIgnoreCase) ? AppDomain.CurrentDomain.FriendlyName : name;
    }
}
