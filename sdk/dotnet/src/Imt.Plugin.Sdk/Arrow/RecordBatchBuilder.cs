using System.Runtime.CompilerServices;
using System.Text.Json.Nodes;
using Apache.Arrow;
using Apache.Arrow.Ipc;
using Apache.Arrow.Types;

namespace Imt.Plugin.Arrow;

/// <summary>Builds one Arrow record batch in IMT's record schema (see <see cref="RecordSchema"/>).</summary>
public sealed class RecordBatchBuilder
{
    private readonly string[] extra;
    private readonly List<(Record Record, string[]? Extra)> rows = [];

    /// <summary>Creates a builder.</summary>
    /// <param name="extraColumns">Extra Utf8 columns after the record and field columns.</param>
    public RecordBatchBuilder(params string[] extraColumns) => extra = extraColumns ?? [];

    /// <summary>Rows appended so far.</summary>
    public int Count => rows.Count;

    /// <summary>Estimated text bytes appended so far.</summary>
    public long TextBytes { get; private set; }

    /// <summary>Compress buffers with LZ4 frames (default true, like IMT: batches shrink about 4x).</summary>
    public bool Compress { get; init; } = true;

    /// <summary>Use the JSON fields layout even when the columnar one fits (for tests and odd consumers).</summary>
    public bool ForceJsonFields { get; init; }

    /// <summary>Appends a record and the values of the extra columns (missing values are empty).</summary>
    /// <param name="record">The record.</param>
    /// <param name="extraValues">One value per extra column.</param>
    /// <returns>The builder.</returns>
    public RecordBatchBuilder Append(Record record, params string[] extraValues)
    {
        ArgumentNullException.ThrowIfNull(record);
        rows.Add((record, extraValues));
        TextBytes += RecordSchema.EstimateSize(record);
        return this;
    }

    /// <summary>Encodes the rows as Arrow IPC stream bytes and clears the builder.</summary>
    /// <returns>The bytes.</returns>
    public byte[] Build()
    {
        int n = rows.Count;
        var ts = new Int64Array.Builder().Reserve(n);
        var level = new UInt8Array.Builder().Reserve(n);
        var message = new StringArray.Builder();
        var template = new StringArray.Builder();
        var exception = new StringArray.Builder();
        var line = new UInt32Array.Builder().Reserve(n);
        var id = new UInt32Array.Builder().Reserve(n);
        var source = new UInt32Array.Builder().Reserve(n);
        var markers = new UInt64Array.Builder().Reserve(n);
        foreach (var (r, _) in rows)
        {
            if (r.Timestamp is { } t)
            {
                ts.Append(t);
            }
            else
            {
                ts.AppendNull();
            }

            level.Append((byte)r.Level <= (byte)Level.Fatal ? (byte)r.Level : (byte)Level.Information);
            message.Append(r.Message ?? string.Empty);
            AppendOptional(template, r.Template);
            AppendOptional(exception, r.Exception);
            line.Append(r.Line);
            id.Append(r.Id);
            source.Append(r.Source);
            markers.Append(r.Markers);
        }

        var fields = RecordSchema.RecordFields().ToList();
        var columns = new List<IArrowArray>
        {
            ts.Build(), level.Build(), message.Build(), template.Build(), exception.Build(),
            line.Build(), id.Build(), source.Build(), markers.Build(),
        };
        var fieldColumns = ForceJsonFields ? null : FieldColumns();
        if (fieldColumns is null)
        {
            var json = new StringArray.Builder();
            foreach (var (r, _) in rows)
            {
                AppendOptional(json, r.Fields is { Count: > 0 } f ? FieldValues.Json(f) : null);
            }

            fields.Add(new Field(RecordSchema.FieldsColumn, StringType.Default, true));
            columns.Add(json.Build());
        }
        else
        {
            foreach (var (f, c) in fieldColumns)
            {
                fields.Add(f);
                columns.Add(c);
            }
        }

        for (int i = 0; i < extra.Length; i++)
        {
            var b = new StringArray.Builder();
            foreach (var (_, ex) in rows)
            {
                b.Append(ex is not null && i < ex.Length ? ex[i] ?? string.Empty : string.Empty);
            }

            fields.Add(new Field(extra[i], StringType.Default, false));
            columns.Add(b.Build());
        }

        var schema = new Schema(fields, null);
        using var batch = new RecordBatch(schema, columns, n);
        using var ms = new MemoryStream();
        var options = Compress
            ? new IpcOptions { CompressionCodec = CompressionCodecType.Lz4Frame, CompressionCodecFactory = RecordSchema.Codecs }
            : new IpcOptions();
        using (var w = new ArrowStreamWriter(ms, schema, leaveOpen: true, options))
        {
            w.WriteRecordBatch(batch);
            w.WriteEnd();
        }

        rows.Clear();
        TextBytes = 0;
        return ms.ToArray();
    }

    /// <summary>Encodes <paramref name="records"/> as one batch without extra columns.</summary>
    /// <param name="records">The records.</param>
    /// <returns>The Arrow IPC stream bytes.</returns>
    public static byte[] Encode(IEnumerable<Record> records)
    {
        ArgumentNullException.ThrowIfNull(records);
        var b = new RecordBatchBuilder();
        foreach (var r in records)
        {
            b.Append(r);
        }

        return b.Build();
    }

    /// <summary>
    /// Groups records into batches of at most <paramref name="rowsPerBatch"/> rows and about
    /// <see cref="Contract.BatchBytes"/> of text, encoded as they fill (the host's batching).
    /// </summary>
    /// <param name="records">The records.</param>
    /// <param name="rowsPerBatch">Most rows per batch.</param>
    /// <param name="cancellationToken">Stops the enumeration.</param>
    /// <returns>Each batch's bytes and row count.</returns>
    public static async IAsyncEnumerable<(byte[] Arrow, int Rows)> Batches(
        IAsyncEnumerable<Record> records,
        int rowsPerBatch = Contract.DefaultBatchRows,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(records);
        rowsPerBatch = Math.Clamp(rowsPerBatch, 1, Contract.MaxBatchRows);
        var b = new RecordBatchBuilder();
        await foreach (var r in records.WithCancellation(cancellationToken).ConfigureAwait(false))
        {
            b.Append(r);
            if (b.Count >= rowsPerBatch || b.TextBytes >= Contract.BatchBytes)
            {
                int rows = b.Count;
                yield return (b.Build(), rows);
            }
        }

        if (b.Count > 0)
        {
            int rows = b.Count;
            yield return (b.Build(), rows);
        }
    }

    /// <summary>The field columns (columnar layout), or null for the JSON layout (the Rust <c>field_columns</c>).</summary>
    private List<(Field Field, IArrowArray Column)>? FieldColumns()
    {
        int n = rows.Count;
        var index = new Dictionary<string, int>(StringComparer.Ordinal);
        var keys = new List<(string Name, FieldKind Kind)>();
        long present = 0;
        bool reordered = false;
        foreach (var (r, _) in rows)
        {
            int last = -1;
            foreach (var (k, v) in r.Fields)
            {
                if (!index.TryGetValue(k, out int i))
                {
                    if (keys.Count == RecordSchema.MaxFieldColumns)
                    {
                        return null;
                    }

                    i = keys.Count;
                    index[k] = i;
                    keys.Add((k, FieldValues.KindOf(v)));
                }
                else if (keys[i].Kind != FieldValues.KindOf(v))
                {
                    keys[i] = (k, FieldKind.Json);
                }

                reordered |= last >= i;
                last = i;
                present++;
            }
        }

        if (keys.Count == 0)
        {
            return [];
        }

        // Sparse names (each record has few of many): one JSON object per record is smaller.
        if (keys.Count > 16 && present * 4 < (long)n * keys.Count)
        {
            return null;
        }

        var slots = new JsonNode?[keys.Count];
        var has = new bool[keys.Count];
        var builders = keys.Select(k => NewBuilder(k.Kind)).ToArray();
        var order = new StringArray.Builder();
        var seq = new List<int>();
        foreach (var (r, _) in rows)
        {
            System.Array.Clear(has);
            System.Array.Clear(slots);
            seq.Clear();
            foreach (var (k, v) in r.Fields)
            {
                int i = index[k];
                slots[i] = v;
                has[i] = true;
                seq.Add(i);
            }

            for (int i = 0; i < keys.Count; i++)
            {
                Append(builders[i], keys[i].Kind, has[i], slots[i]);
            }

            if (reordered)
            {
                bool increasing = true;
                for (int i = 1; i < seq.Count; i++)
                {
                    increasing &= seq[i - 1] < seq[i];
                }

                AppendOptional(order, increasing ? null : string.Join(',', seq));
            }
        }

        var outColumns = new List<(Field, IArrowArray)>(keys.Count + 1);
        for (int i = 0; i < keys.Count; i++)
        {
            var (name, kind) = keys[i];
            outColumns.Add(Finish(builders[i], kind, name));
        }

        if (reordered)
        {
            outColumns.Add((new Field(RecordSchema.FieldOrderColumn, StringType.Default, true), order.Build()));
        }

        return outColumns;
    }

    private static object NewBuilder(FieldKind kind) => kind switch
    {
        FieldKind.I64 => new Int64Array.Builder(),
        FieldKind.U64 => new UInt64Array.Builder(),
        FieldKind.F64 => new DoubleArray.Builder(),
        FieldKind.Bool => new BooleanArray.Builder(),
        _ => new StringArray.Builder(),
    };

    private static void Append(object builder, FieldKind kind, bool has, JsonNode? value)
    {
        // A JSON null value is only representable in a j: column; elsewhere the cell is null (no field).
        var v = value as JsonValue;
        switch (kind)
        {
            case FieldKind.Str when has && v is not null:
                ((StringArray.Builder)builder).Append(FieldValues.Text(v));
                break;
            case FieldKind.I64 when has && v is not null:
                ((Int64Array.Builder)builder).Append(FieldValues.Int64(v));
                break;
            case FieldKind.U64 when has && v is not null:
                ((UInt64Array.Builder)builder).Append(FieldValues.UInt64(v));
                break;
            case FieldKind.F64 when has && v is not null:
                ((DoubleArray.Builder)builder).Append(FieldValues.Float64(v));
                break;
            case FieldKind.Bool when has && v is not null:
                ((BooleanArray.Builder)builder).Append(FieldValues.Bool(v));
                break;
            case FieldKind.Json when has:
                ((StringArray.Builder)builder).Append(FieldValues.Json(value));
                break;
            case FieldKind.I64:
                ((Int64Array.Builder)builder).AppendNull();
                break;
            case FieldKind.U64:
                ((UInt64Array.Builder)builder).AppendNull();
                break;
            case FieldKind.F64:
                ((DoubleArray.Builder)builder).AppendNull();
                break;
            case FieldKind.Bool:
                ((BooleanArray.Builder)builder).AppendNull();
                break;
            default:
                ((StringArray.Builder)builder).AppendNull();
                break;
        }
    }

    private static (Field, IArrowArray) Finish(object builder, FieldKind kind, string name) => kind switch
    {
        FieldKind.I64 => (new Field(RecordSchema.TypedPrefix + name, Int64Type.Default, true), ((Int64Array.Builder)builder).Build()),
        FieldKind.U64 => (new Field(RecordSchema.TypedPrefix + name, UInt64Type.Default, true), ((UInt64Array.Builder)builder).Build()),
        FieldKind.F64 => (new Field(RecordSchema.TypedPrefix + name, DoubleType.Default, true), ((DoubleArray.Builder)builder).Build()),
        FieldKind.Bool => (new Field(RecordSchema.TypedPrefix + name, BooleanType.Default, true), ((BooleanArray.Builder)builder).Build()),
        FieldKind.Str => (new Field(RecordSchema.TypedPrefix + name, StringType.Default, true), ((StringArray.Builder)builder).Build()),
        _ => (new Field(RecordSchema.JsonPrefix + name, StringType.Default, true), ((StringArray.Builder)builder).Build()),
    };

    private static void AppendOptional(StringArray.Builder b, string? value)
    {
        if (value is null)
        {
            b.AppendNull();
        }
        else
        {
            b.Append(value);
        }
    }
}
