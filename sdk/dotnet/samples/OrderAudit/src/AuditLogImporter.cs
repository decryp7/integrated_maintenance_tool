using System.Globalization;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.RegularExpressions;
using Imt.Plugin;

namespace OrderAudit;

/// <summary>
/// Reads the audit log of the order service. A record is a header line followed by indented details:
/// <code>
/// 2026-10-17 08:00:00.123 +00:00 [INF] OrderService: OrderStarted web checkout
///     OrderId = ORD-10001
///     Amount = 129.95 EUR
///     Exception:
///       System.TimeoutException: The gateway did not answer
///          at Shop.Payments.Charge()
/// </code>
/// The header gives the time, level, component and event (plus optional text); <c>Key = Value</c> details become
/// fields; an <c>Exception:</c> block becomes the record's exception. Lines that belong to no record are skipped.
/// </summary>
public sealed partial class AuditLogImporter : ITailImporter
{
    /// <summary>The importer id.</summary>
    public const string ImporterId = "orderaudit.log";

    /// <summary>Longest line the importer reads; longer lines are cut (IMT's MAX_LINE_LEN is similar).</summary>
    public const int MaxLine = 64 * 1024;

    /// <inheritdoc/>
    public string Id => ImporterId;

    /// <inheritdoc/>
    public bool Multiline => true;

    /// <inheritdoc/>
    public ValueTask<int> SniffAsync(ImportContext context, ReadOnlyMemory<byte> head, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(context);
        var lines = new ImportInput(head).Text.Split('\n').Select(l => l.TrimEnd('\r')).Where(l => l.Length > 0).Take(20).ToList();
        int headers = lines.Count(l => Header().IsMatch(Cut(l)));
        int details = lines.Count(l => Detail().IsMatch(Cut(l)));
        if (headers == 0)
        {
            return ValueTask.FromResult(0);
        }

        // Headers alone could be another format; details make it certain.
        int score = details > 0 ? 95 : 60;
        return ValueTask.FromResult(context.FileName.Contains("audit", StringComparison.OrdinalIgnoreCase) ? Math.Min(100, score + 5) : score);
    }

    /// <inheritdoc/>
    public IAsyncEnumerable<Record> ImportAsync(ImportContext context, ImportInput input, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(input);
        return Parse(context, input.Text, cancellationToken);
    }

    /// <inheritdoc/>
    /// <remarks>The importer is multi-line, so IMT re-sends the tail from the start of the last record; parsing it alone is right.</remarks>
    public IAsyncEnumerable<Record>? ImportTail(ImportContext context, string head, string tail, CancellationToken cancellationToken) =>
        Parse(context, tail, cancellationToken);

    /// <summary>Parses audit log text into records (synchronous, for tests and tools).</summary>
    /// <param name="text">The text.</param>
    /// <param name="skipped">Lines that belong to no record.</param>
    /// <returns>The records.</returns>
    public static List<Record> ParseAll(string text, out long skipped)
    {
        var ctx = new ImportContext("audit.log");
        var list = Parse(ctx, text, CancellationToken.None).ToBlockingEnumerable().ToList();
        skipped = ctx.Skipped;
        return list;
    }

    private static async IAsyncEnumerable<Record> Parse(ImportContext context, string text, [EnumeratorCancellation] CancellationToken ct)
    {
        Record? current = null;
        StringBuilder? exception = null;
        uint number = 0;
        foreach (var raw in text.Split('\n'))
        {
            number++;
            if ((number & 8191) == 0)
            {
                ct.ThrowIfCancellationRequested();
                await Task.Yield();
            }

            var line = Cut(raw.TrimEnd('\r'));
            if (line.Length == 0)
            {
                continue;
            }

            var h = Header().Match(line);
            if (h.Success)
            {
                if (current is not null)
                {
                    yield return Finish(current, exception);
                }

                current = FromHeader(h, number);
                exception = null;
                continue;
            }

            if (current is null)
            {
                context.Skip();
                continue;
            }

            if (exception is not null && line.StartsWith("      ", StringComparison.Ordinal))
            {
                exception.Append(exception.Length > 0 ? "\n" : string.Empty).Append(line[6..]);
                continue;
            }

            if (line == "    Exception:")
            {
                exception = new StringBuilder();
                continue;
            }

            var d = Detail().Match(line);
            if (d.Success)
            {
                // A detail ends the exception block.
                Finish(current, exception);
                exception = null;
                current.Fields[d.Groups["key"].Value] = d.Groups["value"].Value;
            }
            else
            {
                context.Skip();
            }
        }

        if (current is not null)
        {
            yield return Finish(current, exception);
        }
    }

    private static Record FromHeader(Match h, uint number)
    {
        var time = DateTimeOffset.ParseExact($"{h.Groups["ts"].Value} {h.Groups["tz"].Value}", "yyyy-MM-dd HH:mm:ss.fff zzz", CultureInfo.InvariantCulture);
        var level = Levels.TryParse(h.Groups["level"].Value, out var parsed) ? parsed : Level.Information;
        string evt = h.Groups["event"].Value;
        string rest = h.Groups["text"].Value;
        var r = new Record(number)
        {
            Time = time,
            Level = level,
            Template = evt,
            Message = rest.Length > 0 ? $"{evt} {rest}" : evt,
        };
        r.Fields["Component"] = h.Groups["component"].Value;
        r.Fields["Event"] = evt;
        return r;
    }

    private static Record Finish(Record r, StringBuilder? exception)
    {
        if (exception is { Length: > 0 })
        {
            r.Exception = exception.ToString();
        }

        return r;
    }

    private static string Cut(string line) => line.Length > MaxLine ? line[..MaxLine] : line;

    [GeneratedRegex(@"^(?<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) (?<tz>[+-]\d{2}:\d{2}) \[(?<level>VRB|DBG|INF|WRN|ERR|FTL)\] (?<component>[A-Za-z0-9_.]{1,64}): (?<event>[A-Za-z][A-Za-z0-9_]{0,63})(?: (?<text>.*))?$", RegexOptions.CultureInvariant | RegexOptions.NonBacktracking)]
    private static partial Regex Header();

    [GeneratedRegex(@"^    (?<key>[A-Za-z][A-Za-z0-9_]{0,63}) = (?<value>.*)$", RegexOptions.CultureInvariant | RegexOptions.NonBacktracking)]
    private static partial Regex Detail();
}
