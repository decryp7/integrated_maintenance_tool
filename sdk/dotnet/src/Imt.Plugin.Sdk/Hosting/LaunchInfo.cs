using System.Text;
using System.Text.Json;

namespace Imt.Plugin.Hosting;

/// <summary>
/// Where to listen and the per-launch token. IMT writes them to the plugin's standard input as one JSON line,
/// <c>{"address": "...", "token": "..."}</c>, right after starting it (<c>imt_ipc::endpoint::Handshake</c>):
/// the address is a Unix socket path inside a private 0700 directory, or a Windows pipe name
/// <c>\\.\pipe\imt-plugin-…</c>.
/// </summary>
/// <param name="Address">The socket path or pipe name.</param>
/// <param name="Token">The token every call must carry in <c>x-imt-token</c>.</param>
public sealed record LaunchInfo(string Address, string Token)
{
    /// <summary>Reads the handshake line (at most 64 KiB) from <paramref name="input"/>, leaving the rest unread.</summary>
    /// <param name="input">Standard input.</param>
    /// <returns>The launch information.</returns>
    /// <exception cref="InvalidDataException">The line is missing or not a valid handshake.</exception>
    public static LaunchInfo Read(Stream input)
    {
        ArgumentNullException.ThrowIfNull(input);
        var line = new List<byte>(256);
        var one = new byte[1];
        while (line.Count < Contract.MaxHandshakeBytes)
        {
            int n = input.Read(one, 0, 1);
            if (n == 0 || one[0] == (byte)'\n')
            {
                break;
            }

            line.Add(one[0]);
        }

        if (line.Count >= Contract.MaxHandshakeBytes)
        {
            throw new InvalidDataException("bad handshake: longer than 64 KiB");
        }

        return Parse(Encoding.UTF8.GetString([.. line]));
    }

    /// <summary>Parses a handshake line.</summary>
    /// <param name="line">The JSON line.</param>
    /// <returns>The launch information.</returns>
    /// <exception cref="InvalidDataException">The line is not a valid handshake.</exception>
    public static LaunchInfo Parse(string line)
    {
        ArgumentNullException.ThrowIfNull(line);
        try
        {
            using var doc = JsonDocument.Parse(line.Trim());
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !root.TryGetProperty("address", out var a) || a.ValueKind != JsonValueKind.String
                || !root.TryGetProperty("token", out var t) || t.ValueKind != JsonValueKind.String)
            {
                throw new InvalidDataException("bad handshake: expected {\"address\": \"...\", \"token\": \"...\"}");
            }

            string address = a.GetString()!;
            string token = t.GetString()!;
            if (address.Length == 0 || address.Contains('\0', StringComparison.Ordinal))
            {
                throw new InvalidDataException("bad handshake: the address is empty or invalid");
            }

            if (token.Length == 0 || !token.All(c => c is >= '!' and <= '~'))
            {
                throw new InvalidDataException("bad handshake: the token is empty or not valid header text");
            }

            return new LaunchInfo(address, token);
        }
        catch (JsonException e)
        {
            throw new InvalidDataException($"bad handshake: {e.Message}", e);
        }
    }

    /// <summary>Whether the address names a Windows named pipe.</summary>
    public bool IsNamedPipe => Address.StartsWith(@"\\.\pipe\", StringComparison.OrdinalIgnoreCase);

    /// <summary>The pipe name without the <c>\\.\pipe\</c> prefix (Kestrel's form).</summary>
    public string PipeName => IsNamedPipe ? Address[@"\\.\pipe\".Length..] : Address;
}
