using Google.Protobuf;
using Grpc.Core;
using Wire = Imt.Plugin.V1;

namespace Imt.Plugin.Hosting;

/// <summary>
/// The output file of an export, streamed to IMT in <see cref="Contract.ChunkBytes"/> chunks. Async writes send
/// as the buffer fills; synchronous writes only buffer (they never block a thread on the network) and are sent
/// on the next async write, flush or at the end.
/// </summary>
internal sealed class ExportOutputStream : Stream
{
    private readonly IServerStreamWriter<Wire.ExportOut> writer;
    private readonly CancellationToken cancellationToken;
    private MemoryStream buffer = new();

    public ExportOutputStream(IServerStreamWriter<Wire.ExportOut> writer, CancellationToken cancellationToken)
    {
        this.writer = writer;
        this.cancellationToken = cancellationToken;
    }

    public long Written { get; private set; }

    public override bool CanRead => false;

    public override bool CanSeek => false;

    public override bool CanWrite => true;

    public override long Length => Written;

    public override long Position
    {
        get => Written;
        set => throw new NotSupportedException();
    }

    public override void Flush()
    {
        // Synchronous flushes keep the data buffered; it is sent by FlushAsync or at the end of the export.
    }

    public override async Task FlushAsync(CancellationToken cancellationToken)
    {
        while (buffer.Length > 0)
        {
            await SendAsync(Math.Min((int)buffer.Length, Contract.ChunkBytes), cancellationToken).ConfigureAwait(false);
        }
    }

    public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException();

    public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();

    public override void SetLength(long value) => throw new NotSupportedException();

    public override void Write(byte[] buffer, int offset, int count) => Write(buffer.AsSpan(offset, count));

    public override void Write(ReadOnlySpan<byte> data)
    {
        buffer.Write(data);
        Written += data.Length;
    }

    public override Task WriteAsync(byte[] buffer, int offset, int count, CancellationToken cancellationToken) =>
        WriteAsync(buffer.AsMemory(offset, count), cancellationToken).AsTask();

    public override async ValueTask WriteAsync(ReadOnlyMemory<byte> data, CancellationToken cancellationToken = default)
    {
        buffer.Write(data.Span);
        Written += data.Length;
        while (buffer.Length >= Contract.ChunkBytes)
        {
            await SendAsync(Contract.ChunkBytes, cancellationToken).ConfigureAwait(false);
        }
    }

    private async Task SendAsync(int count, CancellationToken token)
    {
        var all = buffer.GetBuffer();
        int length = (int)buffer.Length;
        var chunk = ByteString.CopyFrom(all, 0, count);
        var rest = new MemoryStream();
        rest.Write(all, count, length - count);
        buffer = rest;
        using var linked = CancellationTokenSource.CreateLinkedTokenSource(token, cancellationToken);
        await writer.WriteAsync(new Wire.ExportOut { Data = chunk }, linked.Token).ConfigureAwait(false);
    }
}
