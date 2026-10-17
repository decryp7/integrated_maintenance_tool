using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json.Nodes;
using Imt.Plugin;

namespace KitchenSink;

/// <summary>Every capability, with triggers for the error paths.</summary>
public static class Kitchen
{
    /// <summary>The plugin id.</summary>
    public const string Id = "test.kitchen";

    /// <summary>Builds the plugin.</summary>
    /// <param name="events">Receives the events (optional).</param>
    /// <param name="settings">The settings section (optional).</param>
    /// <returns>The plugin.</returns>
    public static PluginDefinition Build(KitchenEvents? events = null, KitchenSettings? settings = null) =>
        PluginBuilder.Create(Id)
            .Name("Kitchen sink")
            .Version("1.2.3-beta.1")
            .Optionally("net.decryptology.otel", "^0.1")
            .Requires("test.other", ">=1.0")
            .Permission(Permission.ReadOpenedFiles, "to test grants")
            .Add(new LinesImporter())
            .Add(new BytesImporter())
            .Add(new UpperTransform())
            .Add(new LinesExporter())
            .Add(new KitchenMarkers())
            .Add(new ErrorAnnotator())
            .Add(settings ?? new KitchenSettings())
            .Add(new EchoService())
            .Add(events ?? new KitchenEvents())
            .Build();

    /// <summary>Kinds of the kitchen sink.</summary>
    public sealed class KitchenMarkers : IMarkerKinds
    {
        /// <inheritdoc/>
        public IReadOnlyList<MarkerKind> MarkerKinds { get; } =
        [
            new MarkerKind("kitchen-mark", "Marked by code") { Role = MarkerRole.None, Badge = "M", Shape = "circle" },
            new MarkerKind("kitchen-error", "Error") { Color = "#F00", Icon = "x-circle", Role = MarkerRole.Crash, When = [Condition.Eq("@level", "error")] },
            new MarkerKind("kitchen-start", "Start") { Role = MarkerRole.Start, When = [Condition.Contains("@message", "started")] },
            new MarkerKind("kitchen-stop", "Stop") { Role = MarkerRole.Stop, When = [Condition.In("Event", "Stop", "Exit")] },
        ];
    }
}

/// <summary>Text importer: <c>LEVEL|message|k=v…</c>; <c>ERROR</c> fails, <c>CRASH</c> throws a bug, <c>SLOW</c> hangs.</summary>
public sealed class LinesImporter : ITailImporter
{
    /// <inheritdoc/>
    public string Id => "kitchen.lines";

    /// <inheritdoc/>
    public ValueTask<int> SniffAsync(ImportContext context, ReadOnlyMemory<byte> head, CancellationToken cancellationToken) =>
        ValueTask.FromResult(context.FileName.EndsWith(".kitchen", StringComparison.Ordinal) ? 150 : (head.Length > 0 ? 10 : -5));

    /// <inheritdoc/>
    public IAsyncEnumerable<Record> ImportAsync(ImportContext context, ImportInput input, CancellationToken cancellationToken) =>
        Parse(context, input.Text, cancellationToken);

    /// <inheritdoc/>
    public IAsyncEnumerable<Record>? ImportTail(ImportContext context, string head, string tail, CancellationToken cancellationToken) =>
        tail.StartsWith("NOTAIL", StringComparison.Ordinal) ? null : Parse(context, tail, cancellationToken);

    private static async IAsyncEnumerable<Record> Parse(ImportContext context, string text, [EnumeratorCancellation] CancellationToken ct)
    {
        uint n = 0;
        foreach (var line in text.Split('\n'))
        {
            n++;
            switch (line)
            {
                case "":
                    continue;
                case "ERROR":
                    throw new PluginException("this file is not valid");
                case "CRASH":
                    throw new InvalidOperationException("a bug");
                case "SLOW":
                    await Task.Delay(Timeout.Infinite, ct);
                    break;
            }

            var parts = line.Split('|');
            if (parts.Length < 2 || !Levels.TryParse(parts[0], out var level))
            {
                context.Skip();
                continue;
            }

            var r = new Record(n) { Level = level, Message = parts[1], Timestamp = 1_700_000_000_000 + n };
            foreach (var kv in parts.Skip(2))
            {
                var i = kv.IndexOf('=', StringComparison.Ordinal);
                if (i > 0)
                {
                    r.Fields[kv[..i]] = long.TryParse(kv[(i + 1)..], out var l) ? l : kv[(i + 1)..];
                }
            }

            if (context.Path is { } p)
            {
                r.Fields["path"] = p;
            }

            yield return r;
        }
    }
}

/// <summary>Binary importer: one record with the byte count.</summary>
public sealed class BytesImporter : IImporter
{
    /// <inheritdoc/>
    public string Id => "kitchen.bin";

    /// <inheritdoc/>
    public bool Binary => true;

    /// <inheritdoc/>
    public ValueTask<int> SniffAsync(ImportContext context, ReadOnlyMemory<byte> head, CancellationToken cancellationToken) =>
        ValueTask.FromResult(head.Length > 4 && head.Span[0] == 0 ? 60 : 0);

    /// <inheritdoc/>
    public async IAsyncEnumerable<Record> ImportAsync(ImportContext context, ImportInput input, [EnumeratorCancellation] CancellationToken cancellationToken)
    {
        await Task.Yield();
        yield return new Record(1) { Message = $"{input.Bytes.Length} bytes", Fields = new JsonObject { ["bytes"] = input.Bytes.Length } };
    }
}

/// <summary>Upper-cases messages, drops "please drop", marks "mark me".</summary>
public sealed class UpperTransform : RecordTransform
{
    /// <inheritdoc/>
    public override string Id => "kitchen.upper";

    /// <inheritdoc/>
    public override Verdict Apply(Record record, Action<string> mark, TransformContext context)
    {
        ArgumentNullException.ThrowIfNull(record);
        ArgumentNullException.ThrowIfNull(mark);
        ArgumentNullException.ThrowIfNull(context);
        if (record.Message == "mark me")
        {
            mark("kitchen-mark");
            mark("kitchen-mark");
            mark("not-mine");
        }

        if (record.Message == "boom")
        {
            throw new InvalidOperationException("transform bug");
        }

        var count = (int)context.Items.AddOrUpdate("count", 1, (_, c) => (int)c + 1);
        record.Message = record.Message.ToUpperInvariant();
        record.Fields["upper"] = true;
        record.Fields["seen"] = count;
        record.Fields["source"] = context.SourceName;
        record.Markers = 999;
        return record.Message.Contains("PLEASE DROP", StringComparison.Ordinal) ? Verdict.Drop : Verdict.Keep;
    }
}

/// <summary>Writes <c>source|group|message</c> lines.</summary>
public sealed class LinesExporter : IExporter
{
    /// <inheritdoc/>
    public string Id => "kitchen.lines";

    /// <inheritdoc/>
    public string Name => "Kitchen lines";

    /// <inheritdoc/>
    public string Extension => "txt";

    /// <inheritdoc/>
    public async ValueTask<long> ExportAsync(IAsyncEnumerable<ExportRow> rows, Stream output, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(rows);
        ArgumentNullException.ThrowIfNull(output);
        long n = 0;
        await foreach (var r in rows.WithCancellation(cancellationToken))
        {
            var bytes = Encoding.UTF8.GetBytes($"{r.SourceName}|{r.Group}|{r.Record.Message}\n");
            if (n % 2 == 0)
            {
                await output.WriteAsync(bytes, cancellationToken);
            }
            else
            {
                output.Write(bytes, 0, bytes.Length);
            }

            n++;
        }

        output.Flush();
        return n;
    }
}

/// <summary>One annotation per error record.</summary>
public sealed class ErrorAnnotator : IAnnotator
{
    /// <inheritdoc/>
    public string Id => "kitchen.errors";

    /// <inheritdoc/>
    public IReadOnlyList<Condition> Interest { get; } = [Condition.Eq("@level", "error")];

    /// <inheritdoc/>
    public int Limit => 500;

    /// <inheritdoc/>
    public ValueTask<IReadOnlyList<Annotation>> AnnotateAsync(AnnotationInput input, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(input);
        IReadOnlyList<Annotation> list =
        [
            .. input.Records.Where(r => r.Record.Level >= Level.Error).Select(r => new Annotation("error", r.Record.Timestamp ?? 0, (r.Record.Timestamp ?? 0) + 1, $"{r.SourceName}: {r.Record.Message}")
            {
                Instance = r.Instance,
                OnField = "Computer",
                OnValue = "WEB01",
                Severity = Severity.Critical,
                EvidenceQuery = new JsonObject { ["from"] = r.Record.Timestamp },
            }),
        ];
        return ValueTask.FromResult(list);
    }
}

/// <summary>Every setting kind.</summary>
public sealed class KitchenSettings : ISettingsSection
{
    /// <summary>The last values applied.</summary>
    public SettingsValues? Applied { get; private set; }

    /// <inheritdoc/>
    public SettingsSection Section { get; } = new(
        "Kitchen",
        [
            new SettingField("b", "B", new BoolSetting(), true) { RestartRequired = true },
            new SettingField("i", "I", new IntSetting(0, 10, 2, "x"), 4) { Advanced = true, Description = "an int" },
            new SettingField("f", "F", new FloatSetting(0, 1), 0.5),
            new SettingField("t", "T", new TextSetting(10, "[a-z]*"), "ab"),
            new SettingField("e", "E", new EnumSetting([new EnumOption("a", "A"), new EnumOption("b", "B")]), "a"),
            new SettingField("p", "P", new PathSetting(true), string.Empty),
            new SettingField("d", "D", new DurationSetting(1, 1000), 10),
            new SettingField("s", "S", new SizeSetting(1, 100), 10),
            new SettingField("l", "L", new StringListSetting(3), new JsonArray("x")),
        ])
    {
        Description = "desc",
    };

    /// <inheritdoc/>
    public ValueTask ApplyAsync(SettingsValues values, CancellationToken cancellationToken)
    {
        Applied = values;
        return ValueTask.CompletedTask;
    }
}

/// <summary>Echoes payloads; <c>fail</c> fails.</summary>
public sealed class EchoService : IPluginService
{
    /// <inheritdoc/>
    public string ServiceId => "kitchen.echo";

    /// <inheritdoc/>
    public string ServiceVersion => "1.0.0";

    /// <inheritdoc/>
    public ValueTask<ReadOnlyMemory<byte>> CallAsync(string method, ReadOnlyMemory<byte> payload, CancellationToken cancellationToken) =>
        method == "fail" ? throw new PluginException(ErrorCode.Failed, "asked to fail", new JsonObject { ["why"] = "test" }) : ValueTask.FromResult(payload);
}

/// <summary>Collects events.</summary>
public sealed class KitchenEvents : IEventSubscriber
{
    /// <summary>The events received.</summary>
    public List<PluginEvent> Received { get; } = [];

    /// <inheritdoc/>
    public IReadOnlyList<string> EventKinds { get; } = [Imt.Plugin.EventKinds.RecordsAppended, Imt.Plugin.EventKinds.WorkspaceCleared];

    /// <inheritdoc/>
    public ValueTask OnEventAsync(PluginEvent e, CancellationToken cancellationToken)
    {
        lock (Received)
        {
            Received.Add(e);
        }

        return ValueTask.CompletedTask;
    }
}
