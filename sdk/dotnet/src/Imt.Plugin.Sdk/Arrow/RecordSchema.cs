using System.Text.Encodings.Web;
using System.Text.Json;
using Apache.Arrow;
using Apache.Arrow.Types;

namespace Imt.Plugin.Arrow;

/// <summary>
/// The Arrow record schema of <c>imt.plugin.v1</c>, identical to the Rust SDK's <c>imt_plugin_sdk::arrow</c>.
/// One batch is one Arrow IPC <i>stream</i>: a schema message, one record batch and the end-of-stream marker.
/// <para>Columns, in order: <c>ts</c> Int64 (nullable, ms since the Unix epoch), <c>level</c> UInt8 (0 Verbose …
/// 5 Fatal), <c>message</c> Utf8, <c>template</c> Utf8 (nullable), <c>exception</c> Utf8 (nullable), <c>line</c>
/// UInt32, <c>id</c> UInt32, <c>source</c> UInt32, <c>markers</c> UInt64; then the record's fields; then any extra
/// Utf8 columns a call names (<c>source_name</c>, <c>group</c>, <c>instance</c>).</para>
/// <para>Fields use one of two layouts, told apart by the column names:</para>
/// <list type="bullet">
/// <item><b>Columnar</b> (the usual case): one nullable column per field name used in the batch, in first-seen
/// order. <c>f:&lt;name&gt;</c> columns hold one type: Utf8 (strings), Int64, UInt64 (integers above
/// <c>long.MaxValue</c>), Float64 or Boolean. <c>j:&lt;name&gt;</c> columns hold any other value (JSON null, objects,
/// arrays, or mixed types) as JSON text. A null cell means the record has no such field. When a record's field order
/// differs from the column order, <c>field_order</c> (Utf8, nullable) lists its column indexes, comma-separated.</item>
/// <item><b>JSON</b> (more than 256 names, or sparse names): one <c>fields</c> column (Utf8, nullable) with each
/// record's fields as a JSON object.</item>
/// </list>
/// <para>Batches are written with LZ4-frame buffer compression (part of the Arrow IPC format), like the Rust SDK;
/// readers accept compressed and uncompressed batches.</para>
/// </summary>
public static class RecordSchema
{
    /// <summary>The record columns, in order (fields and extra columns follow).</summary>
    public static IReadOnlyList<string> RecordColumns { get; } = ["ts", "level", "message", "template", "exception", "line", "id", "source", "markers"];

    /// <summary>The <c>source_name</c> extra column (exports and annotators).</summary>
    public const string SourceName = "source_name";

    /// <summary>The <c>group</c> extra column (exports).</summary>
    public const string Group = "group";

    /// <summary>The <c>instance</c> extra column (annotators).</summary>
    public const string Instance = "instance";

    /// <summary>The fields column of the JSON layout.</summary>
    public const string FieldsColumn = "fields";

    /// <summary>The per-record field order column of the columnar layout.</summary>
    public const string FieldOrderColumn = "field_order";

    /// <summary>Prefix of a typed field column.</summary>
    public const string TypedPrefix = "f:";

    /// <summary>Prefix of a JSON-text field column.</summary>
    public const string JsonPrefix = "j:";

    /// <summary>Most field names a batch lays out as columns (more: the JSON layout).</summary>
    public const int MaxFieldColumns = 256;

    /// <summary>The record columns' Arrow fields.</summary>
    /// <returns>The fields, in order.</returns>
    public static IReadOnlyList<Field> RecordFields() =>
    [
        new Field("ts", Int64Type.Default, true),
        new Field("level", UInt8Type.Default, false),
        new Field("message", StringType.Default, false),
        new Field("template", StringType.Default, true),
        new Field("exception", StringType.Default, true),
        new Field("line", UInt32Type.Default, false),
        new Field("id", UInt32Type.Default, false),
        new Field("source", UInt32Type.Default, false),
        new Field("markers", UInt64Type.Default, false),
    ];

    /// <summary>The schema of the JSON fields layout with <paramref name="extra"/> Utf8 columns.</summary>
    /// <param name="extra">The extra column names.</param>
    /// <returns>The schema.</returns>
    public static Schema JsonLayout(params string[] extra)
    {
        ArgumentNullException.ThrowIfNull(extra);
        var fields = RecordFields().ToList();
        fields.Add(new Field(FieldsColumn, StringType.Default, true));
        fields.AddRange(extra.Select(n => new Field(n, StringType.Default, false)));
        return new Schema(fields, null);
    }

    /// <summary>Text size estimate of a record, used to close batches early (the host's <c>record_size</c>).</summary>
    /// <param name="r">The record.</param>
    /// <returns>Approximate bytes.</returns>
    public static long EstimateSize(Record r)
    {
        ArgumentNullException.ThrowIfNull(r);
        return r.Message.Length + (r.Exception?.Length ?? 0) + (r.Template?.Length ?? 0) + (r.Fields.Count * 24L) + 32;
    }

    /// <summary>Codecs for compressed IPC buffers (IMT writes LZ4-frame; readers accept compressed and uncompressed).</summary>
    internal static readonly Apache.Arrow.Compression.CompressionCodecFactory Codecs = new();

    internal static readonly JsonSerializerOptions Json = new() { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping };
}
