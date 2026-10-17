using System.IO.Pipes;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json.Nodes;
using Google.Protobuf;
using Grpc.Core;
using Grpc.Core.Interceptors;
using Grpc.Net.Client;
using Imt.Plugin.Arrow;
using Imt.Plugin.Hosting;
using Wire = Imt.Plugin.V1;

namespace Imt.Plugin.Testing;

/// <summary>Records an import returned.</summary>
/// <param name="Records">The records.</param>
/// <param name="Skipped">Lines the importer skipped.</param>
/// <param name="TailUnsupported">For live tail: the importer asked for a full re-import.</param>
/// <param name="Batches">How many Arrow batches carried the records.</param>
public sealed record ImportResult(IReadOnlyList<Record> Records, ulong Skipped, bool TailUnsupported, int Batches);

/// <summary>What a transform returned for one batch.</summary>
/// <param name="Records">The transformed records.</param>
/// <param name="Verdicts">One verdict per row.</param>
/// <param name="Marks">The marks per row (row index to kinds).</param>
public sealed record TransformResult(IReadOnlyList<Record> Records, IReadOnlyList<Verdict> Verdicts, IReadOnlyDictionary<int, IReadOnlyList<string>> Marks);

/// <summary>
/// Runs a plugin in-process on a private socket (or named pipe), exactly as IMT would talk to it: gRPC over the
/// local transport, the token on every call, Arrow batches. For unit tests of plugins:
/// <code>
/// await using var host = await PluginTestHost.StartAsync(MyPlugin.Build());
/// await host.HandshakeAsync();
/// var result = await host.ImportAsync("my.importer", "app.log", "...");
/// </code>
/// </summary>
public sealed class PluginTestHost : IAsyncDisposable
{
    private readonly PluginServer server;
    private readonly GrpcChannel channel;
    private readonly string? directory;

    private PluginTestHost(PluginServer server, GrpcChannel channel, CallInvoker invoker, string? directory)
    {
        this.server = server;
        this.channel = channel;
        this.directory = directory;
        Client = new Wire.Plugin.PluginClient(invoker);
        Control = new Imt.Ipc.V1.Control.ControlClient(invoker);
    }

    /// <summary>The generated <c>imt.plugin.v1.Plugin</c> client (token attached).</summary>
    public Wire.Plugin.PluginClient Client { get; }

    /// <summary>The generated <c>imt.ipc.v1.Control</c> client (token attached).</summary>
    public Imt.Ipc.V1.Control.ControlClient Control { get; }

    /// <summary>The address and token the plugin listens on.</summary>
    public LaunchInfo Launch => server.Launch;

    /// <summary>Starts the plugin on a fresh private address with a fresh token.</summary>
    /// <param name="plugin">The plugin.</param>
    /// <param name="options">Server options.</param>
    /// <returns>The running host.</returns>
    public static async Task<PluginTestHost> StartAsync(PluginDefinition plugin, PluginServerOptions? options = null)
    {
        var (launch, dir) = NewAddress();
        var server = await PluginServer.StartAsync(plugin, launch, options).ConfigureAwait(false);
        var channel = Connect(launch.Address);
        var invoker = channel.Intercept(m =>
        {
            m.Add(Contract.TokenHeader, launch.Token);
            return m;
        });
        return new PluginTestHost(server, channel, invoker, dir);
    }

    /// <summary>A channel to a plugin at <paramref name="address"/> (Unix socket path or Windows pipe name), without the token.</summary>
    /// <param name="address">The address.</param>
    /// <returns>The channel.</returns>
    public static GrpcChannel Connect(string address)
    {
        ArgumentNullException.ThrowIfNull(address);
        var handler = new SocketsHttpHandler
        {
            ConnectCallback = async (_, ct) =>
            {
                if (address.StartsWith(@"\\.\pipe\", StringComparison.OrdinalIgnoreCase))
                {
                    var pipe = new NamedPipeClientStream(".", address[@"\\.\pipe\".Length..], PipeDirection.InOut, PipeOptions.Asynchronous);
                    await pipe.ConnectAsync(ct).ConfigureAwait(false);
                    return pipe;
                }

                var socket = new Socket(AddressFamily.Unix, SocketType.Stream, ProtocolType.Unspecified);
                try
                {
                    await socket.ConnectAsync(new UnixDomainSocketEndPoint(address), ct).ConfigureAwait(false);
                    return new NetworkStream(socket, ownsSocket: true);
                }
                catch
                {
                    socket.Dispose();
                    throw;
                }
            },
        };
        return GrpcChannel.ForAddress("http://localhost", new GrpcChannelOptions
        {
            HttpHandler = handler,
            MaxReceiveMessageSize = Contract.MaxMessageBytes,
            MaxSendMessageSize = Contract.MaxMessageBytes,
            DisposeHttpClient = true,
        });
    }

    /// <summary>A fresh private address like IMT's: a socket in a new 0700 directory, or a random pipe name on Windows.</summary>
    /// <returns>The launch information and the directory to delete afterwards (null for a pipe).</returns>
    public static (LaunchInfo Launch, string? Directory) NewAddress()
    {
        string token = Convert.ToHexString(RandomNumberGenerator.GetBytes(32)).ToLowerInvariant();
        string id = Convert.ToHexString(RandomNumberGenerator.GetBytes(6)).ToLowerInvariant();
        if (OperatingSystem.IsWindows())
        {
            return (new LaunchInfo($@"\\.\pipe\imt-test-{id}", token), null);
        }

        foreach (var root in new[] { Path.GetTempPath(), "/tmp" })
        {
            var dir = Path.Combine(root, $"imt-test-{id}");
            var sock = Path.Combine(dir, "w.sock");
            if (sock.Length > 96)
            {
                continue;
            }

            Directory.CreateDirectory(dir, UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute);
            return (new LaunchInfo(sock, token), dir);
        }

        throw new IOException("no place for a private socket");
    }

    /// <summary>Calls <c>Handshake</c> as IMT does.</summary>
    /// <param name="granted">Permissions to grant.</param>
    /// <returns>The manifest.</returns>
    public async Task<Wire.Manifest> HandshakeAsync(params Permission[] granted)
    {
        var req = new Wire.HandshakeRequest { ContractMajor = Contract.Major, ContractMinor = Contract.Minor, HostVersion = "test" };
        req.Granted.AddRange(granted.Select(g => ProtoMapping.ToProto(new PermissionRequest(g, string.Empty))));
        return await Client.HandshakeAsync(req).ConfigureAwait(false);
    }

    /// <summary>Calls <c>Sniff</c>.</summary>
    /// <param name="importerId">The importer.</param>
    /// <param name="fileName">The file name.</param>
    /// <param name="head">The start of the file.</param>
    /// <returns>The score.</returns>
    public async Task<uint> SniffAsync(string importerId, string fileName, string head)
    {
        var reply = await Client.SniffAsync(new Wire.SniffRequest { ImporterId = importerId, Ctx = Context(fileName), Head = ByteString.CopyFromUtf8(head) }).ConfigureAwait(false);
        return reply.Score;
    }

    /// <summary>Calls <c>Import</c> (content split into 1 MiB chunks) and decodes every batch.</summary>
    /// <param name="importerId">The importer.</param>
    /// <param name="fileName">The file name.</param>
    /// <param name="content">The content.</param>
    /// <param name="deadline">An optional deadline.</param>
    /// <returns>The records.</returns>
    public Task<ImportResult> ImportAsync(string importerId, string fileName, string content, DateTime? deadline = null) =>
        ImportCoreAsync(false, importerId, fileName, [(Wire.ChunkPart.Content, Encoding.UTF8.GetBytes(content))], deadline);

    /// <summary>Calls <c>ImportTail</c> with a head and an appended tail.</summary>
    /// <param name="importerId">The importer.</param>
    /// <param name="fileName">The file name.</param>
    /// <param name="head">The start of the file.</param>
    /// <param name="tail">The appended text.</param>
    /// <returns>The records.</returns>
    public Task<ImportResult> ImportTailAsync(string importerId, string fileName, string head, string tail) =>
        ImportCoreAsync(true, importerId, fileName, [(Wire.ChunkPart.Head, Encoding.UTF8.GetBytes(head)), (Wire.ChunkPart.Tail, Encoding.UTF8.GetBytes(tail))], null);

    /// <summary>Calls <c>Transform</c> with one batch per list.</summary>
    /// <param name="transformId">The transform.</param>
    /// <param name="sourceName">The file name.</param>
    /// <param name="batches">The batches.</param>
    /// <returns>One result per batch.</returns>
    public async Task<IReadOnlyList<TransformResult>> TransformAsync(string transformId, string sourceName, params IReadOnlyList<Record>[] batches)
    {
        using var call = Client.Transform();
        await call.RequestStream.WriteAsync(new Wire.TransformIn { Start = new Wire.TransformStart { TransformId = transformId, SourceName = sourceName } }).ConfigureAwait(false);
        var results = new List<TransformResult>();
        foreach (var b in batches)
        {
            await call.RequestStream.WriteAsync(new Wire.TransformIn { Batch = Batch(RecordBatchBuilder.Encode(b), b.Count) }).ConfigureAwait(false);
            if (!await call.ResponseStream.MoveNext(CancellationToken.None).ConfigureAwait(false))
            {
                throw new InvalidOperationException("the transform answered fewer batches than it was sent");
            }

            var m = call.ResponseStream.Current;
            var recs = RecordBatchReader.Read(m.Batch.Arrow.Memory).Records;
            var marks = m.Marks.ToDictionary(x => (int)x.Row, x => (IReadOnlyList<string>)[.. x.Kinds]);
            var verdicts = Enumerable.Range(0, recs.Count).Select(i => i < m.Verdicts.Count && m.Verdicts[i] == Wire.Verdict.Drop ? Verdict.Drop : Verdict.Keep).ToList();
            results.Add(new TransformResult(recs, verdicts, marks));
        }

        await call.RequestStream.CompleteAsync().ConfigureAwait(false);
        while (await call.ResponseStream.MoveNext(CancellationToken.None).ConfigureAwait(false))
        {
        }

        return results;
    }

    /// <summary>Calls <c>Annotate</c> with the records (source name and instance per record).</summary>
    /// <param name="annotatorId">The annotator.</param>
    /// <param name="records">The records.</param>
    /// <returns>The annotations as sent on the wire.</returns>
    public async Task<IReadOnlyList<Wire.Annotation>> AnnotateAsync(string annotatorId, IReadOnlyList<AnnotatedRecord> records)
    {
        ArgumentNullException.ThrowIfNull(records);
        using var call = Client.Annotate();
        var start = new Wire.AnnotateStart { AnnotatorId = annotatorId };
        start.Instances.AddRange(records.Select(r => r.Instance).Distinct());
        await call.RequestStream.WriteAsync(new Wire.AnnotateIn { Start = start }).ConfigureAwait(false);
        var b = new RecordBatchBuilder(RecordSchema.SourceName, RecordSchema.Instance);
        foreach (var r in records)
        {
            b.Append(r.Record, r.SourceName, r.Instance);
        }

        await call.RequestStream.WriteAsync(new Wire.AnnotateIn { Batch = Batch(b.Build(), records.Count) }).ConfigureAwait(false);
        await call.RequestStream.CompleteAsync().ConfigureAwait(false);
        var reply = await call.ResponseAsync.ConfigureAwait(false);
        return [.. reply.Annotations];
    }

    /// <summary>Calls <c>Export</c> and returns the output and the reported row count.</summary>
    /// <param name="exporterId">The exporter.</param>
    /// <param name="rows">The rows.</param>
    /// <returns>The output bytes and rows.</returns>
    public async Task<(byte[] Output, ulong Rows)> ExportAsync(string exporterId, IReadOnlyList<ExportRow> rows)
    {
        ArgumentNullException.ThrowIfNull(rows);
        using var call = Client.Export();
        await call.RequestStream.WriteAsync(new Wire.ExportIn { Start = new Wire.ExportStart { ExporterId = exporterId } }).ConfigureAwait(false);
        var b = new RecordBatchBuilder(RecordSchema.SourceName, RecordSchema.Group);
        foreach (var r in rows)
        {
            b.Append(r.Record, r.SourceName, r.Group);
        }

        await call.RequestStream.WriteAsync(new Wire.ExportIn { Batch = Batch(b.Build(), rows.Count) }).ConfigureAwait(false);
        await call.RequestStream.CompleteAsync().ConfigureAwait(false);
        using var output = new MemoryStream();
        while (await call.ResponseStream.MoveNext(CancellationToken.None).ConfigureAwait(false))
        {
            var m = call.ResponseStream.Current;
            if (m.ItemCase == Wire.ExportOut.ItemOneofCase.Data)
            {
                m.Data.WriteTo(output);
            }
            else if (m.ItemCase == Wire.ExportOut.ItemOneofCase.Done)
            {
                return (output.ToArray(), m.Done.Rows);
            }
        }

        throw new InvalidOperationException("the export ended without ExportDone");
    }

    /// <summary>Calls <c>ApplySettings</c>.</summary>
    /// <param name="values">The values (a JSON object).</param>
    /// <returns>A task.</returns>
    public async Task ApplySettingsAsync(JsonObject values)
    {
        ArgumentNullException.ThrowIfNull(values);
        await Client.ApplySettingsAsync(new Wire.ApplySettingsRequest { ValuesJson = values.ToJsonString() }).ConfigureAwait(false);
    }

    /// <summary>Stops the plugin and removes the private directory.</summary>
    /// <returns>A task.</returns>
    public async ValueTask DisposeAsync()
    {
        channel.Dispose();
        await server.DisposeAsync().ConfigureAwait(false);
        if (directory is not null)
        {
            try
            {
                Directory.Delete(directory, recursive: true);
            }
            catch (IOException)
            {
            }
        }
    }

    private static Wire.ImportContext Context(string fileName) => new() { FileName = fileName, FileDate = "2024-01-01" };

    private static Wire.RecordBatch Batch(byte[] arrow, int rows) => new() { Arrow = ByteString.CopyFrom(arrow), Rows = (uint)rows };

    private async Task<ImportResult> ImportCoreAsync(bool tail, string importerId, string fileName, (Wire.ChunkPart Part, byte[] Data)[] parts, DateTime? deadline)
    {
        using var call = tail ? Client.ImportTail(deadline: deadline) : Client.Import(deadline: deadline);
        await call.RequestStream.WriteAsync(new Wire.ImportChunk
        {
            Start = new Wire.ImportStart { ImporterId = importerId, Ctx = Context(fileName), TotalBytes = (ulong)parts.Sum(p => (long)p.Data.Length) },
        }).ConfigureAwait(false);
        foreach (var (part, data) in parts)
        {
            for (int off = 0; off < data.Length; off += Contract.ChunkBytes)
            {
                var chunk = new Wire.Chunk { Part = part, Data = ByteString.CopyFrom(data, off, Math.Min(Contract.ChunkBytes, data.Length - off)) };
                await call.RequestStream.WriteAsync(new Wire.ImportChunk { Chunk = chunk }).ConfigureAwait(false);
            }
        }

        await call.RequestStream.CompleteAsync().ConfigureAwait(false);
        var records = new List<Record>();
        int batches = 0;
        while (await call.ResponseStream.MoveNext(CancellationToken.None).ConfigureAwait(false))
        {
            var m = call.ResponseStream.Current;
            if (m.ItemCase == Wire.ImportReply.ItemOneofCase.Batch)
            {
                var d = RecordBatchReader.Read(m.Batch.Arrow.Memory);
                if (d.Records.Count != m.Batch.Rows)
                {
                    throw new InvalidDataException($"batch says {m.Batch.Rows} rows, holds {d.Records.Count}");
                }

                records.AddRange(d.Records);
                batches++;
            }
            else if (m.ItemCase == Wire.ImportReply.ItemOneofCase.Done)
            {
                return new ImportResult(records, m.Done.Skipped, m.Done.TailUnsupported, batches);
            }
        }

        throw new InvalidOperationException("the import ended without ImportDone");
    }
}
