using System.Globalization;
using System.Runtime.CompilerServices;
using System.Text.RegularExpressions;
using Imt.Plugin;

namespace MyImtPlugin;

/// <summary>
/// Reads lines like <c>2024-05-01T12:00:00.250Z WRN Cache miss key=user:42 ms=1830</c>: an ISO 8601 time, a
/// level, a message, and trailing <c>key=value</c> pairs that become fields. Change it to your format.
/// </summary>
public sealed partial class LineImporter : ITailImporter
{
    /// <inheritdoc/>
    public string Id => "myimtplugin.lines";

    /// <inheritdoc/>
    public ValueTask<int> SniffAsync(ImportContext context, ReadOnlyMemory<byte> head, CancellationToken cancellationToken)
    {
        var lines = new ImportInput(head).Text.Split('\n').Where(l => l.Length > 0).Take(10).ToList();
        int ok = lines.Count(l => Line().IsMatch(l.TrimEnd('\r')));
        return ValueTask.FromResult(lines.Count == 0 ? 0 : ok * 80 / lines.Count);
    }

    /// <inheritdoc/>
    public IAsyncEnumerable<Record> ImportAsync(ImportContext context, ImportInput input, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(input);
        return Parse(context, input.Text, cancellationToken);
    }

    /// <inheritdoc/>
    public IAsyncEnumerable<Record>? ImportTail(ImportContext context, string head, string tail, CancellationToken cancellationToken) =>
        Parse(context, tail, cancellationToken);

    private static async IAsyncEnumerable<Record> Parse(ImportContext context, string text, [EnumeratorCancellation] CancellationToken ct)
    {
        uint number = 0;
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

            var m = Line().Match(line);
            if (!m.Success || !DateTimeOffset.TryParse(m.Groups["ts"].Value, CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal, out var ts)
                || !Levels.TryParse(m.Groups["level"].Value, out var level))
            {
                context.Skip();
                continue;
            }

            var record = new Record(number) { Time = ts, Level = level };
            var words = new List<string>();
            foreach (var word in m.Groups["rest"].Value.Split(' ', StringSplitOptions.RemoveEmptyEntries))
            {
                int eq = word.IndexOf('=', StringComparison.Ordinal);
                if (eq > 0)
                {
                    var value = word[(eq + 1)..];
                    record.Fields[word[..eq]] = long.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out long n) ? n : value;
                }
                else
                {
                    words.Add(word);
                }
            }

            record.Message = string.Join(' ', words);
            yield return record;
        }
    }

    [GeneratedRegex(@"^(?<ts>\d{4}-\d{2}-\d{2}T\S+)\s+(?<level>[A-Za-z]{3,11})\s+(?<rest>.*)$", RegexOptions.CultureInvariant | RegexOptions.NonBacktracking)]
    private static partial Regex Line();
}
