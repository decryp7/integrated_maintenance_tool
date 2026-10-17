using Imt.Plugin.Hosting;
using Wire = Imt.Plugin.V1;

namespace Imt.Plugin;

/// <summary>Options of <see cref="ImtPlugin.RunAsync(string[], PluginDefinition, ImtPluginOptions?)"/> (mostly for tests).</summary>
public sealed class ImtPluginOptions
{
    /// <summary>Where the handshake line and the lifeline come from (default: standard input).</summary>
    public Stream? Input { get; init; }

    /// <summary>Where <c>--print-manifest</c> writes (default: standard output).</summary>
    public TextWriter? ManifestOutput { get; init; }

    /// <summary>Where errors are reported (default: standard error).</summary>
    public TextWriter? Error { get; init; }

    /// <summary>Server options.</summary>
    public PluginServerOptions Server { get; init; } = new();

    /// <summary>Keep <see cref="Console.Out"/> as it is (by default it is redirected to standard error).</summary>
    public bool KeepConsoleOut { get; init; }
}

/// <summary>
/// The entry point of a plugin process: <c>return await ImtPlugin.RunAsync(args, plugin);</c>.
/// <para>
/// IMT starts the executable with an empty environment and a temporary working directory, writes one JSON line
/// <c>{"address","token"}</c> to its standard input and calls it over gRPC. This method reads that line, serves
/// the plugin on the private address (Unix socket / owner-only named pipe; never TCP), checks the token on every
/// call, and returns when IMT closes standard input (the lifeline), calls <c>Control.Shutdown</c>, or the
/// process gets a termination signal. Logs go to standard error; standard output is never written while serving.
/// </para>
/// <para>
/// Arguments: <c>--print-manifest</c> writes the plugin's manifest as JSON (for packaging: the
/// <c>plugin.json</c> of the plugin folder) and exits without serving.
/// </para>
/// </summary>
public static class ImtPlugin
{
    /// <summary>Runs the plugin until IMT lets it go.</summary>
    /// <param name="args">The process arguments.</param>
    /// <param name="plugin">The plugin.</param>
    /// <param name="options">Options, or null.</param>
    /// <returns>The process exit code: 0 after a normal stop, 1 when the plugin could not start, 2 on a usage error.</returns>
    public static async Task<int> RunAsync(string[] args, PluginDefinition plugin, ImtPluginOptions? options = null)
    {
        ArgumentNullException.ThrowIfNull(args);
        ArgumentNullException.ThrowIfNull(plugin);
        options ??= new ImtPluginOptions();
        var error = options.Error ?? Console.Error;
        if (args.Contains("--print-manifest", StringComparer.Ordinal))
        {
            var output = options.ManifestOutput ?? Console.Out;
            await output.WriteLineAsync(PluginPackage.ManifestJson(plugin, PluginPackage.ExecutableName())).ConfigureAwait(false);
            await output.FlushAsync().ConfigureAwait(false);
            return 0;
        }

        if (args.FirstOrDefault(a => a.StartsWith('-')) is { } unknown && unknown != "--")
        {
            await error.WriteLineAsync($"{plugin.Manifest.Id}: unknown option \"{unknown}\" (IMT starts plugins without options; use --print-manifest for packaging)").ConfigureAwait(false);
            return 2;
        }

        if (!options.KeepConsoleOut)
        {
            // Nothing may reach standard output while serving: IMT does not read it.
            Console.SetOut(Console.Error);
        }

        var input = options.Input ?? Console.OpenStandardInput();
        LaunchInfo launch;
        try
        {
            launch = LaunchInfo.Read(input);
        }
        catch (Exception e) when (e is InvalidDataException or IOException)
        {
            await error.WriteLineAsync($"{plugin.Manifest.Id}: {e.Message}. This program is an IMT plugin: install it in IMT, or test it with imt-plugin-check.").ConfigureAwait(false);
            return 1;
        }

        PluginServer server;
        try
        {
            server = await PluginServer.StartAsync(plugin, launch, options.Server).ConfigureAwait(false);
        }
        catch (Exception e) when (e is IOException or InvalidOperationException or PlatformNotSupportedException or UnauthorizedAccessException or System.Net.Sockets.SocketException)
        {
            await error.WriteLineAsync($"{plugin.Manifest.Id}: cannot listen on {launch.Address}: {e.Message}").ConfigureAwait(false);
            return 1;
        }

        await using (server.ConfigureAwait(false))
        {
            Lifeline.Watch(input, server.RequestStop);
            await server.WaitForShutdownAsync().ConfigureAwait(false);
        }

        return 0;
    }

    /// <summary>Synchronous <see cref="RunAsync(string[], PluginDefinition, ImtPluginOptions?)"/> for a plain <c>Main</c>.</summary>
    /// <param name="args">The process arguments.</param>
    /// <param name="plugin">The plugin.</param>
    /// <returns>The process exit code.</returns>
    public static int Run(string[] args, PluginDefinition plugin) => RunAsync(args, plugin).GetAwaiter().GetResult();

    /// <summary>The handshake reply the plugin sends (for tests and tools).</summary>
    /// <param name="plugin">The plugin.</param>
    /// <returns>The manifest message.</returns>
    public static Wire.Manifest ManifestMessage(PluginDefinition plugin) => ProtoMapping.ToProto(plugin);
}
