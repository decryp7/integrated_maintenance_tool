using System.Text.Json.Nodes;
using Imt.Plugin;
using Imt.Plugin.Testing;

namespace ToyFormat.Tests;

public sealed class ToyImporterTests : IAsyncLifetime
{
    private PluginTestHost host = null!;

    public async ValueTask InitializeAsync()
    {
        host = await PluginTestHost.StartAsync(ToyPlugin.Build());
        await host.HandshakeAsync();
    }

    public async ValueTask DisposeAsync() => await host.DisposeAsync();

    [Fact]
    public async Task Reads_typed_fields_times_of_day_and_skips_garbage()
    {
        var r = await host.ImportAsync("toy", "app.toy", "2024-05-01T12:00:00.250Z|WRN|Cache miss|key=user:42|ms=1830|hit=false|ratio=0.5\n12:00:01|INF|time of day\nnonsense\n");
        Assert.Equal(2, r.Records.Count);
        Assert.Equal(1UL, r.Skipped);
        var a = r.Records[0];
        Assert.Equal((Level.Warning, "Cache miss", 1830L, false, 0.5), (a.Level, a.Message, a.Fields["ms"]!.GetValue<long>(), a.Fields["hit"]!.GetValue<bool>(), a.Fields["ratio"]!.GetValue<double>()));
        Assert.Equal(new DateTimeOffset(2024, 1, 1, 12, 0, 1, TimeSpan.Zero), r.Records[1].Time);
        Assert.Equal(90u, await host.SniffAsync("toy", "x", "12:00:01|INF|a\n"));
        Assert.Equal(0u, await host.SniffAsync("toy", "x", string.Empty));
        Assert.Single((await host.ImportTailAsync("toy", "x", "12:00:01|INF|a\n", "12:00:02|ERR|b\n")).Records);
    }

    [Fact]
    public async Task The_message_length_setting_cuts_messages()
    {
        await host.ApplySettingsAsync(new JsonObject { ["maxMessageLength"] = 100 });
        var r = await host.ImportAsync("toy", "a", $"12:00:00|INF|{new string('x', 500)}\n");
        Assert.Equal(100, r.Records[0].Message.Length);
    }
}
