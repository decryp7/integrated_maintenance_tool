namespace Imt.Plugin;

/// <summary>
/// Constants of the <c>imt.plugin.v1</c> contract, identical to the Rust SDK (<c>crates/imt-plugin-sdk</c>).
/// </summary>
public static class Contract
{
    /// <summary>The contract major version this SDK implements (<c>imt.plugin.v1</c>).</summary>
    public const uint Major = 1;

    /// <summary>The contract minor version this SDK implements.</summary>
    public const uint Minor = 0;

    /// <summary>Metadata key carrying the per-launch token on every call.</summary>
    public const string TokenHeader = "x-imt-token";

    /// <summary>Binary trailer carrying a typed error (<c>{code, message, data?}</c> as JSON).</summary>
    public const string ErrorTrailer = "x-imt-error-bin";

    /// <summary>Largest gRPC message in either direction.</summary>
    public const int MaxMessageBytes = 64 * 1024 * 1024;

    /// <summary>File content and exporter output are streamed in chunks of this size.</summary>
    public const int ChunkBytes = 1024 * 1024;

    /// <summary>Largest file content one <c>Import</c> call accepts (the host's limit).</summary>
    public const ulong MaxImportBytes = 4UL * 1024 * 1024 * 1024;

    /// <summary>Rows per record batch the SDK sends (the host's <c>DEFAULT_BATCH_ROWS</c>).</summary>
    public const int DefaultBatchRows = 64 * 1024;

    /// <summary>A batch is closed early once its text reaches about this many bytes (the host's <c>BATCH_BYTES</c>).</summary>
    public const int BatchBytes = 24 * 1024 * 1024;

    /// <summary>Most rows a received batch may hold.</summary>
    public const int MaxBatchRows = 1 << 20;

    /// <summary>Most records an annotator run receives (the host caps <c>AnnotatorCap.limit</c> at this).</summary>
    public const int MaxViewRecords = 100_000;

    /// <summary>Longest handshake line read from standard input.</summary>
    public const int MaxHandshakeBytes = 64 * 1024;
}
