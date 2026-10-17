using System.IO.Pipes;
using System.Runtime.Versioning;
using System.Security.AccessControl;
using System.Security.Principal;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Server.Kestrel.Core;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;

namespace Imt.Plugin.Hosting;

/// <summary>Options of a plugin server.</summary>
public sealed class PluginServerOptions
{
    /// <summary>The lowest level written to standard error (default <see cref="LogLevel.Information"/> for the plugin, warnings for the framework).</summary>
    public LogLevel MinimumLogLevel { get; init; } = LogLevel.Information;

    /// <summary>How long connections may drain on shutdown before the server stops anyway (the Rust SDK uses one second).</summary>
    public TimeSpan ShutdownTimeout { get; init; } = TimeSpan.FromSeconds(1);
}

/// <summary>
/// A Kestrel gRPC server for one plugin on IMT's private local transport: a Unix domain socket (made 0600
/// inside IMT's 0700 directory) or a Windows named pipe only the current user can open; HTTP/2 only; never TCP.
/// Serves <c>imt.plugin.v1.Plugin</c> and <c>imt.ipc.v1.Control</c>, with the token checked on every call.
/// </summary>
public sealed class PluginServer : IAsyncDisposable
{
    private readonly WebApplication app;
    private int disposed;

    private PluginServer(WebApplication app, LaunchInfo launch)
    {
        this.app = app;
        Launch = launch;
    }

    /// <summary>Where the server listens and its token.</summary>
    public LaunchInfo Launch { get; }

    /// <summary>Completes when the server was asked to stop (lifeline, <c>Control.Shutdown</c>, a signal).</summary>
    public CancellationToken Stopping => app.Lifetime.ApplicationStopping;

    /// <summary>Builds and starts a server.</summary>
    /// <param name="plugin">The plugin.</param>
    /// <param name="launch">The address and token.</param>
    /// <param name="options">Options, or null for the defaults.</param>
    /// <param name="cancellationToken">Cancels the start.</param>
    /// <returns>The running server.</returns>
    public static async Task<PluginServer> StartAsync(PluginDefinition plugin, LaunchInfo launch, PluginServerOptions? options = null, CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(plugin);
        ArgumentNullException.ThrowIfNull(launch);
        options ??= new PluginServerOptions();
        var builder = WebApplication.CreateEmptyBuilder(new WebApplicationOptions { ApplicationName = plugin.Manifest.Id });
        builder.Logging.AddConsole(o => o.LogToStandardErrorThreshold = LogLevel.Trace);
        builder.Logging.SetMinimumLevel(options.MinimumLogLevel);
        builder.Logging.AddFilter("Microsoft", LogLevel.Warning);
        builder.Logging.AddFilter("Grpc", LogLevel.Warning);
        builder.Services.Configure<HostOptions>(o => o.ShutdownTimeout = options.ShutdownTimeout);
        builder.Services.AddSingleton(new PluginState(plugin));
        builder.Services.AddRoutingCore();
        builder.Services.AddGrpc(o =>
        {
            o.MaxReceiveMessageSize = Contract.MaxMessageBytes;
            o.MaxSendMessageSize = Contract.MaxMessageBytes;
            o.EnableDetailedErrors = false;
            o.Interceptors.Add<ImtInterceptor>(launch.Token);
        });
        builder.WebHost.UseKestrelCore();
        if (launch.IsNamedPipe)
        {
            if (!OperatingSystem.IsWindows())
            {
                throw new PlatformNotSupportedException("named pipe addresses are only used on Windows");
            }

            ConfigureNamedPipes(builder);
        }

        builder.WebHost.ConfigureKestrel(k =>
        {
            k.AddServerHeader = false;
            k.Limits.MaxRequestBodySize = null;
            if (launch.IsNamedPipe)
            {
                k.ListenNamedPipe(launch.PipeName, l => l.Protocols = HttpProtocols.Http2);
            }
            else
            {
                k.ListenUnixSocket(launch.Address, l => l.Protocols = HttpProtocols.Http2);
            }
        });
        var app = builder.Build();
        app.UseRouting();
        app.MapGrpcService<PluginService>();
        app.MapGrpcService<ControlService>();
        await app.StartAsync(cancellationToken).ConfigureAwait(false);
        if (!launch.IsNamedPipe && !OperatingSystem.IsWindows())
        {
            // Defence in depth: IMT's directory is 0700; the socket itself is 0600.
            File.SetUnixFileMode(launch.Address, UnixFileMode.UserRead | UnixFileMode.UserWrite);
        }

        return new PluginServer(app, launch);
    }

    /// <summary>Asks the server to stop (as the lifeline does). Does nothing once the server is disposed.</summary>
    public void RequestStop()
    {
        if (Volatile.Read(ref disposed) == 0)
        {
            try
            {
                app.Lifetime.StopApplication();
            }
            catch (ObjectDisposedException)
            {
                // Stopped concurrently: nothing left to do.
            }
        }
    }

    /// <summary>Completes when the server has stopped.</summary>
    /// <returns>A task.</returns>
    public Task WaitForShutdownAsync() => app.WaitForShutdownAsync();

    /// <summary>Stops the server (connections drain for at most the shutdown timeout).</summary>
    /// <returns>A task.</returns>
    public async ValueTask DisposeAsync()
    {
        if (Interlocked.Exchange(ref disposed, 1) == 1)
        {
            return;
        }

        try
        {
            await app.StopAsync().ConfigureAwait(false);
        }
        finally
        {
            await app.DisposeAsync().ConfigureAwait(false);
        }
    }

    [SupportedOSPlatform("windows")]
    private static void ConfigureNamedPipes(WebApplicationBuilder builder)
    {
        builder.WebHost.UseNamedPipes(o =>
        {
            // Owner-only ACL like imt-ipc's "D:P(A;;GA;;;OW)", and no access over the network.
            using var me = WindowsIdentity.GetCurrent();
            var security = new PipeSecurity();
            var owner = me.User ?? throw new InvalidOperationException("the current user has no SID");
            security.SetOwner(owner);
            security.SetAccessRuleProtection(isProtected: true, preserveInheritance: false);
            security.AddAccessRule(new PipeAccessRule(owner, PipeAccessRights.FullControl, AccessControlType.Allow));
            security.AddAccessRule(new PipeAccessRule(new SecurityIdentifier(WellKnownSidType.NetworkSid, null), PipeAccessRights.FullControl, AccessControlType.Deny));
            o.PipeSecurity = security;
            o.CurrentUserOnly = false;
        });
    }
}
