using System.Runtime.CompilerServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using Google.Protobuf;
using Grpc.Core;
using Imt.Plugin.Arrow;
using Wire = Imt.Plugin.V1;

namespace Imt.Plugin.Hosting;

/// <summary>
/// <c>imt.plugin.v1.Plugin</c> over one <see cref="PluginDefinition"/>: the same semantics as the Rust SDK's
/// <c>PluginService</c> (gate on the handshake, typed errors for unknown ids and malformed streams, limits,
/// Arrow batches of the host's size).
/// </summary>
public sealed class PluginService : Wire.Plugin.PluginBase
{
    private readonly PluginState state;

    /// <summary>Creates the service.</summary>
    /// <param name="state">The plugin state.</param>
    public PluginService(PluginState state) => this.state = state;

    private PluginDefinition Plugin => state.Plugin;

    /// <inheritdoc/>
    public override Task<Wire.Manifest> Handshake(Wire.HandshakeRequest request, ServerCallContext context)
    {
        ArgumentNullException.ThrowIfNull(request);
        if (request.ContractMajor != Contract.Major)
        {
            throw new PluginException(
                ErrorCode.Version,
                $"this plugin implements imt.plugin.v{Contract.Major}, IMT speaks v{request.ContractMajor}");
        }

        state.Handshake([.. request.Granted.Select(ProtoMapping.FromProto).OfType<Permission>()]);
        return Task.FromResult(ProtoMapping.ToProto(Plugin));
    }

    /// <inheritdoc/>
    public override async Task<Wire.SniffReply> Sniff(Wire.SniffRequest request, ServerCallContext context)
    {
        ArgumentNullException.ThrowIfNull(request);
        ArgumentNullException.ThrowIfNull(context);
        state.Gate();
        var importer = Importer(request.ImporterId);
        var ctx = ProtoMapping.Context(request.Ctx, state.MayReadFiles);
        int score = await importer.SniffAsync(ctx, request.Head.Memory, context.CancellationToken).ConfigureAwait(false);
        return new Wire.SniffReply { Score = (uint)Math.Clamp(score, 0, 100) };
    }

    /// <inheritdoc/>
    public override async Task Import(IAsyncStreamReader<Wire.ImportChunk> requestStream, IServerStreamWriter<Wire.ImportReply> responseStream, ServerCallContext context)
    {
        ArgumentNullException.ThrowIfNull(context);
        state.Gate();
        var ct = context.CancellationToken;
        var (start, _, body) = await ReadImport(requestStream, ct).ConfigureAwait(false);
        var ctx = ProtoMapping.Context(start.Ctx, state.MayReadFiles);
        var importer = Importer(start.ImporterId);
        var records = importer.ImportAsync(ctx, new ImportInput(body), ct);
        await SendRecords(records, ctx, BatchRows(start), responseStream, ct).ConfigureAwait(false);
    }

    /// <inheritdoc/>
    public override async Task ImportTail(IAsyncStreamReader<Wire.ImportChunk> requestStream, IServerStreamWriter<Wire.ImportReply> responseStream, ServerCallContext context)
    {
        ArgumentNullException.ThrowIfNull(context);
        ArgumentNullException.ThrowIfNull(responseStream);
        state.Gate();
        var ct = context.CancellationToken;
        var (start, head, tail) = await ReadImport(requestStream, ct).ConfigureAwait(false);
        var ctx = ProtoMapping.Context(start.Ctx, state.MayReadFiles);
        var importer = Plugin.Importers.FirstOrDefault(i => i.Id == start.ImporterId && !i.Binary)
            ?? throw PluginException.Invalid($"no text importer \"{start.ImporterId}\"");
        var records = importer is ITailImporter t
            ? t.ImportTail(ctx, new ImportInput(head).Text, new ImportInput(tail).Text, ct)
            : null;
        if (records is null)
        {
            await responseStream.WriteAsync(new Wire.ImportReply { Done = new Wire.ImportDone { TailUnsupported = true } }, ct).ConfigureAwait(false);
            return;
        }

        await SendRecords(records, ctx, BatchRows(start), responseStream, ct).ConfigureAwait(false);
    }

    /// <inheritdoc/>
    public override async Task Transform(IAsyncStreamReader<Wire.TransformIn> requestStream, IServerStreamWriter<Wire.TransformOut> responseStream, ServerCallContext context)
    {
        ArgumentNullException.ThrowIfNull(requestStream);
        ArgumentNullException.ThrowIfNull(responseStream);
        ArgumentNullException.ThrowIfNull(context);
        state.Gate();
        var ct = context.CancellationToken;
        if (!await requestStream.MoveNext(ct).ConfigureAwait(false) || requestStream.Current.ItemCase != Wire.TransformIn.ItemOneofCase.Start)
        {
            throw PluginException.Invalid("a transform starts with TransformStart");
        }

        var start = requestStream.Current.Start;
        var transform = Plugin.Transforms.FirstOrDefault(t => t.Id == start.TransformId)
            ?? throw PluginException.Invalid($"no transform \"{start.TransformId}\"");
        var tctx = new TransformContext(start.SourceName);

        // Answer the call (headers) now: IMT waits for them before it streams the batches.
        await context.WriteResponseHeadersAsync([]).ConfigureAwait(false);
        while (await requestStream.MoveNext(ct).ConfigureAwait(false))
        {
            var m = requestStream.Current;
            if (m.ItemCase != Wire.TransformIn.ItemOneofCase.Batch)
            {
                throw PluginException.Invalid("only batches may follow TransformStart");
            }

            var records = Decode(m.Batch).Records;
            var original = records.Select(r => r.Markers).ToArray();
            foreach (var r in records)
            {
                r.Markers = 0;
            }

            var batch = new TransformBatch(records);
            await transform.ApplyAsync(batch, tctx, ct).ConfigureAwait(false);
            var reply = new Wire.TransformOut();
            for (int i = 0; i < records.Count; i++)
            {
                records[i].Markers = original[i];
                var kinds = batch.MarksOf(i);
                if (kinds.Count > 0)
                {
                    var rm = new Wire.RowMarks { Row = (uint)i };
                    rm.Kinds.AddRange(kinds.Take(64));
                    reply.Marks.Add(rm);
                }
            }

            reply.Verdicts.AddRange(batch.Verdicts.Select(v => v == Verdict.Drop ? Wire.Verdict.Drop : Wire.Verdict.Keep));
            reply.Batch = new Wire.RecordBatch { Arrow = ByteString.CopyFrom(RecordBatchBuilder.Encode(records)), Rows = (uint)records.Count };
            await responseStream.WriteAsync(reply, ct).ConfigureAwait(false);
        }
    }

    /// <inheritdoc/>
    public override Task<Wire.MarkerKindsReply> MarkerKinds(Wire.MarkerKindsRequest request, ServerCallContext context)
    {
        state.Gate();
        var reply = new Wire.MarkerKindsReply();
        reply.Kinds.AddRange(Plugin.MarkerKinds.Select(ProtoMapping.ToProto));
        return Task.FromResult(reply);
    }

    /// <inheritdoc/>
    public override async Task<Wire.AnnotateReply> Annotate(IAsyncStreamReader<Wire.AnnotateIn> requestStream, ServerCallContext context)
    {
        ArgumentNullException.ThrowIfNull(requestStream);
        ArgumentNullException.ThrowIfNull(context);
        state.Gate();
        var ct = context.CancellationToken;
        if (!await requestStream.MoveNext(ct).ConfigureAwait(false) || requestStream.Current.ItemCase != Wire.AnnotateIn.ItemOneofCase.Start)
        {
            throw PluginException.Invalid("an annotation run starts with AnnotateStart");
        }

        var start = requestStream.Current.Start;
        var annotator = Plugin.Annotators.FirstOrDefault(a => a.Id == start.AnnotatorId)
            ?? throw PluginException.Invalid($"no annotator \"{start.AnnotatorId}\"");
        var records = new List<AnnotatedRecord>();
        while (await requestStream.MoveNext(ct).ConfigureAwait(false))
        {
            var m = requestStream.Current;
            if (m.ItemCase != Wire.AnnotateIn.ItemOneofCase.Batch)
            {
                throw PluginException.Invalid("only batches may follow AnnotateStart");
            }

            var d = Decode(m.Batch, RecordSchema.SourceName, RecordSchema.Instance);
            if (records.Count + d.Records.Count > Contract.MaxViewRecords)
            {
                throw PluginException.TooLarge($"more than {Contract.MaxViewRecords} records");
            }

            for (int i = 0; i < d.Records.Count; i++)
            {
                records.Add(new AnnotatedRecord(d.Records[i], d.Extra[0][i], d.Extra[1][i]));
            }
        }

        var input = new AnnotationInput(records)
        {
            Since = start.HasSince ? new RecordCursor(start.SinceId, start.SinceHasTs ? start.SinceTs : null) : null,
            TimeRange = start.HasRange ? (start.RangeFrom, start.RangeTo) : null,
            Instances = [.. start.Instances],
        };
        var list = await annotator.AnnotateAsync(input, ct).ConfigureAwait(false);
        var reply = new Wire.AnnotateReply();
        reply.Annotations.AddRange((list ?? []).Select(ProtoMapping.ToProto));
        return reply;
    }

    /// <inheritdoc/>
    public override async Task Export(IAsyncStreamReader<Wire.ExportIn> requestStream, IServerStreamWriter<Wire.ExportOut> responseStream, ServerCallContext context)
    {
        ArgumentNullException.ThrowIfNull(requestStream);
        ArgumentNullException.ThrowIfNull(responseStream);
        ArgumentNullException.ThrowIfNull(context);
        state.Gate();
        var ct = context.CancellationToken;
        if (!await requestStream.MoveNext(ct).ConfigureAwait(false) || requestStream.Current.ItemCase != Wire.ExportIn.ItemOneofCase.Start)
        {
            throw PluginException.Invalid("an export starts with ExportStart");
        }

        var start = requestStream.Current.Start;
        var exporter = Plugin.Exporters.FirstOrDefault(e => e.Id == start.ExporterId)
            ?? throw PluginException.Invalid($"no exporter \"{start.ExporterId}\"");
        await context.WriteResponseHeadersAsync([]).ConfigureAwait(false);
        var output = new ExportOutputStream(responseStream, ct);
        await using (output.ConfigureAwait(false))
        {
            long rows = await exporter.ExportAsync(ExportRows(requestStream, ct), output, ct).ConfigureAwait(false);
            await output.FlushAsync(ct).ConfigureAwait(false);
            await responseStream.WriteAsync(new Wire.ExportOut { Done = new Wire.ExportDone { Rows = (ulong)Math.Max(0, rows) } }, ct).ConfigureAwait(false);
        }
    }

    /// <inheritdoc/>
    public override Task<Wire.SettingsSectionReply> SettingsSection(Wire.SettingsSectionRequest request, ServerCallContext context)
    {
        state.Gate();
        var s = Plugin.Settings ?? throw PluginException.Unimplemented("this plugin has no settings");
        return Task.FromResult(new Wire.SettingsSectionReply { Section = ProtoMapping.ToProto(s.Section) });
    }

    /// <inheritdoc/>
    public override async Task<Wire.ApplySettingsReply> ApplySettings(Wire.ApplySettingsRequest request, ServerCallContext context)
    {
        ArgumentNullException.ThrowIfNull(request);
        ArgumentNullException.ThrowIfNull(context);
        state.Gate();
        var s = Plugin.Settings ?? throw PluginException.Unimplemented("this plugin has no settings");
        JsonNode? values;
        try
        {
            values = JsonNode.Parse(request.ValuesJson);
        }
        catch (JsonException e)
        {
            throw PluginException.Invalid($"the values are not JSON: {e.Message}");
        }

        var errors = SettingsValidation.Validate(s.Section, values, out var valid);
        if (errors.Count > 0)
        {
            throw PluginException.Invalid(string.Join("; ", errors.Select(e => $"{e.Key}: {e.Message}")));
        }

        await state.SettingsLock.WaitAsync(context.CancellationToken).ConfigureAwait(false);
        try
        {
            await s.ApplyAsync(valid, context.CancellationToken).ConfigureAwait(false);
        }
        finally
        {
            state.SettingsLock.Release();
        }

        return new Wire.ApplySettingsReply();
    }

    /// <inheritdoc/>
    public override async Task<Wire.ServiceReply> CallService(Wire.ServiceCall request, ServerCallContext context)
    {
        ArgumentNullException.ThrowIfNull(request);
        ArgumentNullException.ThrowIfNull(context);
        state.Gate();
        var svc = Plugin.Services.FirstOrDefault(s => s.ServiceId == request.ServiceId)
            ?? throw PluginException.Unimplemented($"no service \"{request.ServiceId}\"");
        var payload = await svc.CallAsync(request.Method, request.Payload.Memory, context.CancellationToken).ConfigureAwait(false);
        return new Wire.ServiceReply { Payload = ByteString.CopyFrom(payload.Span) };
    }

    /// <inheritdoc/>
    public override async Task<Wire.EventsAck> Events(IAsyncStreamReader<Wire.HostEvent> requestStream, ServerCallContext context)
    {
        ArgumentNullException.ThrowIfNull(requestStream);
        ArgumentNullException.ThrowIfNull(context);
        state.Gate();
        ulong received = 0;
        while (await requestStream.MoveNext(context.CancellationToken).ConfigureAwait(false))
        {
            var e = requestStream.Current;
            received++;
            var ev = new PluginEvent(e.Kind, ProtoMapping.ParsePayload(e.PayloadJson));
            foreach (var s in Plugin.EventSubscribers.Where(s => s.EventKinds.Contains(e.Kind, StringComparer.Ordinal)))
            {
                await s.OnEventAsync(ev, context.CancellationToken).ConfigureAwait(false);
            }
        }

        return new Wire.EventsAck { Received = received };
    }

    private IImporter Importer(string id) =>
        Plugin.Importers.FirstOrDefault(i => i.Id == id) ?? throw PluginException.Invalid($"no importer \"{id}\"");

    private static DecodedBatch Decode(Wire.RecordBatch? batch, params string[] extra)
    {
        try
        {
            return RecordBatchReader.Read(batch?.Arrow.Memory ?? ReadOnlyMemory<byte>.Empty, extra);
        }
        catch (InvalidDataException e)
        {
            throw PluginException.Invalid(e.Message);
        }
    }

    private static async IAsyncEnumerable<ExportRow> ExportRows(IAsyncStreamReader<Wire.ExportIn> requestStream, [EnumeratorCancellation] CancellationToken ct)
    {
        while (await requestStream.MoveNext(ct).ConfigureAwait(false))
        {
            var m = requestStream.Current;
            if (m.ItemCase != Wire.ExportIn.ItemOneofCase.Batch)
            {
                throw PluginException.Invalid("only batches may follow ExportStart");
            }

            var d = Decode(m.Batch, RecordSchema.SourceName, RecordSchema.Group);
            for (int i = 0; i < d.Records.Count; i++)
            {
                yield return new ExportRow(d.Records[i], d.Extra[0][i], d.Extra[1][i]);
            }
        }
    }

    /// <summary>Reads an import request: the start message and the content (or head and tail).</summary>
    private static async Task<(Wire.ImportStart Start, byte[] Head, byte[] Body)> ReadImport(IAsyncStreamReader<Wire.ImportChunk> s, CancellationToken ct)
    {
        if (!await s.MoveNext(ct).ConfigureAwait(false) || s.Current.ItemCase != Wire.ImportChunk.ItemOneofCase.Start)
        {
            throw PluginException.Invalid("an import starts with ImportStart");
        }

        var start = s.Current.Start;
        ulong limit = Math.Min(Contract.MaxImportBytes, (ulong)Array.MaxLength);
        if (start.TotalBytes > limit)
        {
            throw PluginException.TooLarge($"the content is larger than {limit} bytes");
        }

        using var head = new MemoryStream();
        using var body = new MemoryStream((int)Math.Min(start.TotalBytes, 256UL << 20));
        ulong total = 0;
        while (await s.MoveNext(ct).ConfigureAwait(false))
        {
            var m = s.Current;
            if (m.ItemCase != Wire.ImportChunk.ItemOneofCase.Chunk)
            {
                throw PluginException.Invalid("only chunks may follow ImportStart");
            }

            total += (ulong)m.Chunk.Data.Length;
            if (total > limit)
            {
                throw PluginException.TooLarge($"the content is larger than {limit} bytes");
            }

            var target = m.Chunk.Part == Wire.ChunkPart.Head ? head : body;
            target.Write(m.Chunk.Data.Span);
        }

        return (start, head.ToArray(), body.ToArray());
    }

    private static int BatchRows(Wire.ImportStart start) =>
        start.BatchRows == 0 ? Contract.DefaultBatchRows : (int)Math.Min(start.BatchRows, (uint)Contract.MaxBatchRows);

    private static async Task SendRecords(IAsyncEnumerable<Record> records, ImportContext ctx, int batchRows, IServerStreamWriter<Wire.ImportReply> output, CancellationToken ct)
    {
        await foreach (var (arrow, rows) in RecordBatchBuilder.Batches(records, batchRows, ct).ConfigureAwait(false))
        {
            var reply = new Wire.ImportReply { Batch = new Wire.RecordBatch { Arrow = UnsafeByteOperations.UnsafeWrap(arrow), Rows = (uint)rows } };
            await output.WriteAsync(reply, ct).ConfigureAwait(false);
        }

        await output.WriteAsync(new Wire.ImportReply { Done = new Wire.ImportDone { Skipped = (ulong)Math.Max(0, ctx.Skipped) } }, ct).ConfigureAwait(false);
    }
}
