using System.Globalization;
using System.Runtime.CompilerServices;
using System.Text.Json.Nodes;
using Imt.Plugin;

namespace ToyFormat;

/// <summary>
/// Reads the toy format, one record per line: <c>TIME|LEVEL|MESSAGE|key=value|key=value…</c>, e.g.
/// <c>2024-05-01T12:00:00.250Z|WRN|Cache miss|key=user:42|ms=1830</c>. TIME is ISO 8601, or a time of day
/// (<c>12:00:00.250</c>) combined with the file's date. Values that look like numbers or booleans are typed.
/// </summary>
public sealed class ToyImporter : ITailImporter
{
    private readonly ToySettings settings;

    /// <summary>Creates the importer.</summary>
    /// <param name="settings">The plugin's settings.</param>
    public ToyImporter(ToySettings settings) => this.settings = settings;

    /// <inheritdoc/>
    public string Id => "toy";

    /// <inheritdoc/>
    public ValueTask<int> SniffAsync(ImportContext context, ReadOnlyMemory<byte> head, CancellationToken cancellationToken)
    {
        var lines = new ImportInput(head).Text.Split('\n').Select(l => l.TrimEnd('\r')).Where(l => l.Length > 0).Take(10).ToList();
        if (lines.Count == 0)
        {
            return ValueTask.FromResult(0);
        }

        var date = context.FileDate ?? DateOnly.FromDateTime(DateTime.UtcNow);
        int ok = lines.Count(l => Parse(l, 1, date, settings.MaxMessageLength) is not null);
        return ValueTask.FromResult(ok * 90 / lines.Count);
    }

    /// <inheritdoc/>
    public IAsyncEnumerable<Record> ImportAsync(ImportContext context, ImportInput input, CancellationToken cancellationToken) =>
        Lines(context, input.Text, cancellationToken);

    /// <inheritdoc/>
    public IAsyncEnumerable<Record>? ImportTail(ImportContext context, string head, string tail, CancellationToken cancellationToken) =>
        Lines(context, tail, cancellationToken);

    /// <summary>Parses one line, or returns null when it is not a toy record.</summary>
    /// <param name="line">The line.</param>
    /// <param name="number">Its 1-based line number.</param>
    /// <param name="fileDate">The file's date, for times of day.</param>
    /// <param name="maxMessage">Longest message kept.</param>
    /// <returns>The record, or null.</returns>
    public static Record? Parse(string line, uint number, DateOnly? fileDate, int maxMessage)
    {
        ArgumentNullException.ThrowIfNull(line);
        var parts = line.Split('|');
        if (parts.Length < 3 || !Levels.TryParse(parts[1], out var level))
        {
            return null;
        }

        long? ts = ParseTime(parts[0].Trim(), fileDate);
        if (ts is null)
        {
            return null;
        }

        var r = new Record(number)
        {
            Timestamp = ts,
            Level = level,
            Message = parts[2].Length > maxMessage ? parts[2][..maxMessage] : parts[2],
        };
        foreach (var kv in parts.Skip(3))
        {
            int eq = kv.IndexOf('=', StringComparison.Ordinal);
            if (eq > 0)
            {
                r.Fields[kv[..eq].Trim()] = Value(kv[(eq + 1)..].Trim());
            }
        }

        return r;
    }

    private async IAsyncEnumerable<Record> Lines(ImportContext context, string text, [EnumeratorCancellation] CancellationToken ct)
    {
        uint number = 0;
        int max = settings.MaxMessageLength;
        foreach (var raw in text.Split('\n'))
        {
            number++;
            if ((number & 4095) == 0)
            {
                ct.ThrowIfCancellationRequested();
                await Task.Yield();
            }

            var line = raw.TrimEnd('\r');
            if (line.Length == 0)
            {
                continue;
            }

            if (Parse(line, number, context.FileDate, max) is { } r)
            {
                yield return r;
            }
            else
            {
                context.Skip();
            }
        }
    }

    private static long? ParseTime(string text, DateOnly? fileDate)
    {
        if (DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal | DateTimeStyles.AdjustToUniversal, out var dto)
            && text.Contains('-', StringComparison.Ordinal))
        {
            return dto.ToUnixTimeMilliseconds();
        }

        if (fileDate is { } d && TimeOnly.TryParse(text, CultureInfo.InvariantCulture, out var t))
        {
            return new DateTimeOffset(d.ToDateTime(t), TimeSpan.Zero).ToUnixTimeMilliseconds();
        }

        return null;
    }

    private static JsonNode? Value(string text)
    {
        if (long.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out long l))
        {
            return l;
        }

        if (double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out double d) && double.IsFinite(d))
        {
            return d;
        }

        return text switch
        {
            "true" => true,
            "false" => false,
            _ => text,
        };
    }
}
