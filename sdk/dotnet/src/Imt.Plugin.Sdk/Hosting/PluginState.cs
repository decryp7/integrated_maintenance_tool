namespace Imt.Plugin.Hosting;

/// <summary>What every call of one plugin process shares: the plugin, whether IMT has shaken hands, the grants.</summary>
public sealed class PluginState
{
    private volatile bool handshaken;
    private volatile IReadOnlyList<Permission> granted = [];

    /// <summary>Creates the state.</summary>
    /// <param name="plugin">The plugin.</param>
    public PluginState(PluginDefinition plugin) => Plugin = plugin;

    /// <summary>The plugin.</summary>
    public PluginDefinition Plugin { get; }

    /// <summary>When the process started serving.</summary>
    public DateTime Started { get; } = DateTime.UtcNow;

    /// <summary>Whether <c>Handshake</c> was called.</summary>
    public bool Handshaken => handshaken;

    /// <summary>The permissions the user granted.</summary>
    public IReadOnlyList<Permission> Granted => granted;

    /// <summary>Whether the user granted <see cref="PermissionKind.ReadOpenedFiles"/>.</summary>
    public bool MayReadFiles => granted.Any(g => g.Kind == PermissionKind.ReadOpenedFiles);

    /// <summary>Serialises <see cref="ISettingsSection.ApplyAsync"/> calls.</summary>
    internal SemaphoreSlim SettingsLock { get; } = new(1, 1);

    internal void Handshake(IReadOnlyList<Permission> grants)
    {
        granted = grants;
        handshaken = true;
    }

    internal void Gate()
    {
        if (!handshaken)
        {
            throw new PluginException(ErrorCode.NoHandshake, "call Handshake first");
        }
    }
}
