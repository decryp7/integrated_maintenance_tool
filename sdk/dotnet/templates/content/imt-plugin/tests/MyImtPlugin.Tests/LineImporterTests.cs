using Imt.Plugin;
using Imt.Plugin.Testing;

namespace MyImtPlugin.Tests;

/// <summary>Runs the plugin as IMT would talk to it: gRPC on a private socket, token, Arrow batches.</summary>
public sealed class LineImporterTests : IAsyncLifetime
{
    private PluginTestHost host = null!;

    public async ValueTask InitializeAsync()
    {
        host = await PluginTestHost.StartAsync(Plugin.Build());
        await host.HandshakeAsync();
    }

    public async ValueTask DisposeAsync() => await host.DisposeAsync();

    [Fact]
    public async Task Imports_lines_with_fields_and_counts_skipped_lines()
    {
        var result = await host.ImportAsync("myimtplugin.lines", "app.log",
            "2024-05-01T12:00:00.250Z WRN Cache miss key=user:42 ms=1830\nnot a log line\n2024-05-01T12:00:01Z INF Done\n");
        Assert.Equal(2, result.Records.Count);
        Assert.Equal(1UL, result.Skipped);
        var first = result.Records[0];
        Assert.Equal((Level.Warning, "Cache miss"), (first.Level, first.Message));
        Assert.Equal("user:42", first.FieldText("key"));
        Assert.Equal(1830, first.Fields["ms"]!.GetValue<long>());
        Assert.Equal(DateTimeOffset.Parse("2024-05-01T12:00:00.250Z", System.Globalization.CultureInfo.InvariantCulture), first.Time);
    }

    [Fact]
    public async Task Sniffs_its_format()
    {
        Assert.Equal(80u, await host.SniffAsync("myimtplugin.lines", "app.log", "2024-05-01T12:00:00Z INF hello\n"));
        Assert.Equal(0u, await host.SniffAsync("myimtplugin.lines", "app.log", "{\"json\": true}\n"));
    }

    [Fact]
    public async Task Live_tail_parses_appended_lines()
    {
        var result = await host.ImportTailAsync("myimtplugin.lines", "app.log", "2024-05-01T12:00:00Z INF a\n", "2024-05-01T12:00:02Z ERR b\n");
        Assert.Equal(Level.Error, Assert.Single(result.Records).Level);
    }
}
