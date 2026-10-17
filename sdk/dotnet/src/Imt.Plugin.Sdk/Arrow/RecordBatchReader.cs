using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;
using Apache.Arrow;
using Apache.Arrow.Ipc;

namespace Imt.Plugin.Arrow;

/// <summary>A decoded batch: the records and, per requested extra column, its values.</summary>
public sealed class DecodedBatch
{
    internal DecodedBatch(List<Record> records, List<string>[] extra)
    {
        Records = records;
        Extra = extra;
    }

    /// <summary>The records.</summary>
    public IReadOnlyList<Record> Records { get; }

    /// <summary>The values of each requested extra column (same order as requested), one per record.</summary>
    public IReadOnlyList<IReadOnlyList<string>> Extra { get; }
}

/// <summary>
/// Reads Arrow record batches in IMT's record schema (either fields layout), validating everything: column types,
/// level range, field order indexes and JSON text.
/// </summary>
public static class RecordBatchReader
{
    /// <summary>Decodes one Arrow IPC stream, requiring the <paramref name="extraColumns"/>.</summary>
    /// <param name="arrow">The Arrow IPC stream bytes.</param>
    /// <param name="extraColumns">Extra Utf8 columns that must be present.</param>
    /// <returns>The records and extra values.</returns>
    /// <exception cref="InvalidDataException">The bytes are not a valid batch of the schema.</exception>
    public static DecodedBatch Read(ReadOnlyMemory<byte> arrow, params string[] extraColumns)
    {
        ArgumentNullException.ThrowIfNull(extraColumns);
        var records = new List<Record>();
        var extra = extraColumns.Select(_ => new List<string>()).ToArray();
        if (arrow.IsEmpty)
        {
            return new DecodedBatch(records, extra);
        }

        try
        {
            using var reader = new ArrowStreamReader(arrow, RecordSchema.Codecs);
            while (reader.ReadNextRecordBatch() is { } batch)
            {
                using (batch)
                {
                    ReadBatch(batch, extraColumns, records, extra);
                }
            }
        }
        catch (InvalidDataException)
        {
            throw;
        }
        catch (Exception e) when (e is not OutOfMemoryException)
        {
            throw new InvalidDataException($"the record batch is not valid Arrow IPC ({e.Message})", e);
        }

        return new DecodedBatch(records, extra);
    }

    private static T Column<T>(RecordBatch b, string name)
        where T : class, IArrowArray
    {
        int i = b.Schema.GetFieldIndex(name);
        if (i < 0)
        {
            throw new InvalidDataException($"the batch has no \"{name}\" column");
        }

        return b.Column(i) as T ?? throw new InvalidDataException($"column \"{name}\" has the wrong type");
    }

    private static void ReadBatch(RecordBatch b, string[] extraColumns, List<Record> records, List<string>[] extra)
    {
        int n = b.Length;
        if (records.Count + n > Contract.MaxBatchRows)
        {
            throw new InvalidDataException($"the batch has more than {Contract.MaxBatchRows} rows");
        }

        var ts = Column<Int64Array>(b, "ts");
        var level = Column<UInt8Array>(b, "level");
        var message = Column<StringArray>(b, "message");
        var template = Column<StringArray>(b, "template");
        var exception = Column<StringArray>(b, "exception");
        var fields = FieldsReader.Of(b);
        var line = Column<UInt32Array>(b, "line");
        var id = Column<UInt32Array>(b, "id");
        var source = Column<UInt32Array>(b, "source");
        var markers = Column<UInt64Array>(b, "markers");
        var extras = extraColumns.Select(c => Column<StringArray>(b, c)).ToArray();
        for (int i = 0; i < n; i++)
        {
            byte lv = level.GetValue(i) ?? 0;
            if (lv > (byte)Level.Fatal)
            {
                throw new InvalidDataException($"row {i}: level {lv} is not 0..=5");
            }

            records.Add(new Record
            {
                Timestamp = ts.IsNull(i) ? null : ts.GetValue(i),
                Level = (Level)lv,
                Message = message.IsNull(i) ? string.Empty : message.GetString(i),
                Template = template.IsNull(i) ? null : template.GetString(i),
                Exception = exception.IsNull(i) ? null : exception.GetString(i),
                Fields = fields.Row(i),
                Line = line.GetValue(i) ?? 0,
                Id = id.GetValue(i) ?? 0,
                Source = source.GetValue(i) ?? 0,
                Markers = markers.GetValue(i) ?? 0,
            });
        }

        for (int c = 0; c < extras.Length; c++)
        {
            for (int i = 0; i < n; i++)
            {
                extra[c].Add(extras[c].IsNull(i) ? string.Empty : extras[c].GetString(i));
            }
        }
    }

    /// <summary>Reads the fields of a batch in either layout.</summary>
    private sealed class FieldsReader
    {
        private readonly StringArray? json;
        private readonly List<(string Name, bool Json, IArrowArray Column)> columns = [];
        private readonly StringArray? order;

        private FieldsReader(StringArray? json, StringArray? order)
        {
            this.json = json;
            this.order = order;
        }

        public static FieldsReader Of(RecordBatch b)
        {
            if (b.Schema.GetFieldIndex(RecordSchema.FieldsColumn) >= 0)
            {
                return new FieldsReader(Column<StringArray>(b, RecordSchema.FieldsColumn), null);
            }

            var order = b.Schema.GetFieldIndex(RecordSchema.FieldOrderColumn) >= 0 ? Column<StringArray>(b, RecordSchema.FieldOrderColumn) : null;
            var reader = new FieldsReader(null, order);
            for (int i = 0; i < b.Schema.FieldsList.Count; i++)
            {
                var name = b.Schema.FieldsList[i].Name;
                bool isJson;
                if (name.StartsWith(RecordSchema.TypedPrefix, StringComparison.Ordinal))
                {
                    isJson = false;
                }
                else if (name.StartsWith(RecordSchema.JsonPrefix, StringComparison.Ordinal))
                {
                    isJson = true;
                }
                else
                {
                    continue;
                }

                var c = b.Column(i);
                bool ok = c is StringArray || (!isJson && c is Int64Array or UInt64Array or DoubleArray or BooleanArray);
                if (!ok)
                {
                    throw new InvalidDataException($"field column \"{name}\" has an unsupported type {c.Data.DataType.Name}");
                }

                reader.columns.Add((name[2..], isJson, c));
            }

            if (reader.columns.Count > RecordSchema.MaxFieldColumns)
            {
                throw new InvalidDataException($"the batch has more than {RecordSchema.MaxFieldColumns} field columns");
            }

            return reader;
        }

        public JsonObject Row(int i)
        {
            if (json is not null)
            {
                if (json.IsNull(i))
                {
                    return [];
                }

                try
                {
                    return JsonNode.Parse(json.GetString(i)) as JsonObject
                        ?? throw new InvalidDataException($"row {i}: \"fields\" is not a JSON object");
                }
                catch (Exception e) when (e is JsonException or ArgumentException)
                {
                    throw new InvalidDataException($"row {i}: \"fields\" is not a JSON object ({e.Message})", e);
                }
            }

            var f = new JsonObject();
            if (order is not null && !order.IsNull(i))
            {
                foreach (var part in order.GetString(i).Split(','))
                {
                    if (!int.TryParse(part, NumberStyles.None, CultureInfo.InvariantCulture, out int k) || k >= columns.Count)
                    {
                        throw new InvalidDataException($"row {i}: field order \"{part}\" is not a column index");
                    }

                    Put(f, k, i);
                }
            }
            else
            {
                for (int k = 0; k < columns.Count; k++)
                {
                    Put(f, k, i);
                }
            }

            return f;
        }

        private void Put(JsonObject f, int k, int i)
        {
            var (name, isJson, c) = columns[k];
            if (c.IsNull(i))
            {
                return;
            }

            JsonNode? v;
            switch (c)
            {
                case StringArray s when isJson:
                    try
                    {
                        v = JsonNode.Parse(s.GetString(i));
                    }
                    catch (JsonException e)
                    {
                        throw new InvalidDataException($"row {i}: field \"{name}\" is not valid JSON ({e.Message})", e);
                    }

                    break;
                case StringArray s:
                    v = JsonValue.Create(s.GetString(i));
                    break;
                case Int64Array a:
                    v = JsonValue.Create(a.GetValue(i)!.Value);
                    break;
                case UInt64Array a:
                    v = JsonValue.Create(a.GetValue(i)!.Value);
                    break;
                case DoubleArray a:
                    v = JsonValue.Create(a.GetValue(i)!.Value);
                    break;
                default:
                    v = JsonValue.Create(((BooleanArray)c).GetValue(i)!.Value);
                    break;
            }

            try
            {
                f[name] = v;
            }
            catch (ArgumentException e)
            {
                throw new InvalidDataException($"row {i}: field \"{name}\" appears twice ({e.Message})", e);
            }
        }
    }
}
