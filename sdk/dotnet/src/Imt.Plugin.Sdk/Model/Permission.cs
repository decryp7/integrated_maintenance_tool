namespace Imt.Plugin;

/// <summary>Something a plugin may do beyond reading the bytes IMT gives it.</summary>
public enum PermissionKind
{
    /// <summary>Receive the paths of files the user opened (and read them directly).</summary>
    ReadOpenedFiles = 1,

    /// <summary>Connect to the network (<see cref="Permission.Target"/> names the host).</summary>
    Network = 2,
}

/// <summary>A permission: its kind and, for <see cref="PermissionKind.Network"/>, the host (empty for any).</summary>
/// <param name="Kind">The kind.</param>
/// <param name="Target">The host for network access; empty otherwise.</param>
[System.Diagnostics.CodeAnalysis.SuppressMessage("Naming", "CA1711", Justification = "The contract calls it a permission.")]
public sealed record Permission(PermissionKind Kind, string Target = "")
{
    /// <summary>Receive the paths of opened files.</summary>
    public static Permission ReadOpenedFiles { get; } = new(PermissionKind.ReadOpenedFiles);

    /// <summary>Connect to <paramref name="host"/>.</summary>
    /// <param name="host">The host name, e.g. <c>example.com</c>; empty for any.</param>
    /// <returns>The permission.</returns>
    public static Permission Network(string host) => new(PermissionKind.Network, host);
}

/// <summary>A permission a plugin asks the user for, with the reason shown at install.</summary>
/// <param name="Permission">The permission.</param>
/// <param name="Reason">Why the plugin needs it.</param>
public sealed record PermissionRequest(Permission Permission, string Reason);
