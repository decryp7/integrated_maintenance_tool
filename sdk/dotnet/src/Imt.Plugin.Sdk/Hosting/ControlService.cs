using Grpc.Core;
using Imt.Ipc.V1;
using Microsoft.Extensions.Hosting;

namespace Imt.Plugin.Hosting;

/// <summary>
/// <c>imt.ipc.v1.Control</c>, which every IMT child process serves: status for the supervisor's health check
/// and a polite shutdown.
/// </summary>
public sealed class ControlService : Control.ControlBase
{
    private readonly PluginState state;
    private readonly IHostApplicationLifetime lifetime;

    /// <summary>Creates the service.</summary>
    /// <param name="state">The plugin state.</param>
    /// <param name="lifetime">Stops the process on <c>Shutdown</c>.</param>
    public ControlService(PluginState state, IHostApplicationLifetime lifetime)
    {
        this.state = state;
        this.lifetime = lifetime;
    }

    /// <inheritdoc/>
    public override Task<StatusReply> Status(StatusRequest request, ServerCallContext context) => Task.FromResult(new StatusReply
    {
        Pid = (uint)Environment.ProcessId,
        RssBytes = (ulong)Math.Max(0, Environment.WorkingSet),
        UptimeSecs = (ulong)(DateTime.UtcNow - state.Started).TotalSeconds,
        Name = $"{state.Plugin.Manifest.Id} {state.Plugin.Manifest.Version}",
    });

    /// <inheritdoc/>
    public override Task<ShutdownReply> Shutdown(ShutdownRequest request, ServerCallContext context)
    {
        // Reply first, then stop (the server drains for at most a second).
        _ = Task.Run(async () =>
        {
            await Task.Delay(50).ConfigureAwait(false);
            lifetime.StopApplication();
        });
        return Task.FromResult(new ShutdownReply());
    }
}
