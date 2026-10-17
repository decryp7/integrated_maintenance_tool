using System.Diagnostics;
using KitchenSink;

namespace Imt.Plugin.Tests;

/// <summary>
/// Runs the Rust conformance kit (<c>imt-plugin-check</c>) against the kitchen-sink plugin as a real process: the
/// Rust host's Arrow encoder and decoder exchange batches with this SDK (imports decoded by Rust; transform,
/// export and annotator input encoded by Rust and decoded here). Skipped when the checker is not built; set
/// <c>IMT_PLUGIN_CHECK</c> to its path (CI does).
/// </summary>
public class ConformanceKitTests
{
    private static string? Checker()
    {
        var exe = OperatingSystem.IsWindows() ? "imt-plugin-check.exe" : "imt-plugin-check";
        var root = Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "../../../../.."));
        string?[] candidates =
        [
            Environment.GetEnvironmentVariable("IMT_PLUGIN_CHECK"),
            Path.Combine(root, "artifacts/cargo-target/debug", exe),
            Path.Combine(root, "../../target/debug", exe),
        ];
        return candidates.FirstOrDefault(c => !string.IsNullOrEmpty(c) && File.Exists(c));
    }

    private static string DotnetHost()
    {
        if (Path.GetFileNameWithoutExtension(Environment.ProcessPath) == "dotnet")
        {
            return Environment.ProcessPath!;
        }

        var fromEnv = Environment.GetEnvironmentVariable("DOTNET_HOST_PATH");
        if (!string.IsNullOrEmpty(fromEnv) && File.Exists(fromEnv))
        {
            return fromEnv;
        }

        var root = Environment.GetEnvironmentVariable("DOTNET_ROOT");
        var local = Path.Combine(root ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".dotnet"), OperatingSystem.IsWindows() ? "dotnet.exe" : "dotnet");
        return File.Exists(local) ? local : "dotnet";
    }

    [Fact]
    public async Task The_kitchen_sink_passes_imt_plugin_check()
    {
        var checker = Checker();
        if (checker is null)
        {
            Assert.Skip("imt-plugin-check is not built (cargo build -p imt-plugin-sdk --bin imt-plugin-check, or set IMT_PLUGIN_CHECK)");
        }

        var plugin = typeof(Kitchen).Assembly.Location;
        var psi = new ProcessStartInfo(checker!)
        {
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };
        // imt-plugin-check starts "<dotnet> <plugin.dll>" with an empty environment, like IMT starts a plugin.
        foreach (var a in new[] { DotnetHost(), plugin, "--grant", "read-opened-files", "--fuzz", "24" })
        {
            psi.ArgumentList.Add(a);
        }

        using var p = Process.Start(psi)!;
        var stdout = p.StandardOutput.ReadToEndAsync();
        var stderr = p.StandardError.ReadToEndAsync();
        await p.WaitForExitAsync().WaitAsync(TimeSpan.FromMinutes(3));
        var report = await stdout;
        Assert.True(p.ExitCode == 0, report + await stderr);
        Assert.Contains("transform \"kitchen.upper\": answers every batch with as many rows", report, StringComparison.Ordinal);
        Assert.Contains("exporter \"kitchen.lines\": writes every row", report, StringComparison.Ordinal);
        Assert.Contains("0 failed", report, StringComparison.Ordinal);
        Assert.DoesNotContain("refused with a typed error", report, StringComparison.Ordinal);
    }
}
