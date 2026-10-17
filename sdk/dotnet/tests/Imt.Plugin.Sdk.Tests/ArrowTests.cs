using System.Text.Json.Nodes;
using Apache.Arrow;
using Apache.Arrow.Ipc;
using Apache.Arrow.Types;
using Imt.Plugin.Arrow;

namespace Imt.Plugin.Tests;

public class ArrowTests
{
    internal static Record Sample()
    {
        var r = new Record(7)
        {
            Timestamp = -5,
            Level = Level.Fatal,
            Message = "héllo\nworld ☃",
            Template = "t {x}",
            Exception = "boom",
            Id = 3,
            Source = 4,
            Markers = ulong.MaxValue,
        };
        r.Fields["z"] = 1;
        r.Fields["a"] = JsonNode.Parse("""{"n":[1,2.5,null,true,"s"]}""");
        r.Fields["s"] = "text";
        r.Fields["f"] = 2.5;
        r.Fields["b"] = false;
        r.Fields["big"] = ulong.MaxValue;
        r.Fields["nul"] = null;
        return r;
    }

    [Fact]
    public void Records_round_trip_with_extra_columns_in_the_columnar_layout()
    {
        var a = Sample();
        var b = new Record(0);
        var bytes = new RecordBatchBuilder(RecordSchema.SourceName, RecordSchema.Group).Append(a, "s1", "g1").Append(b, "s2").Build();
        var schema = Schema(bytes);
        Assert.Contains("f:z", schema);
        Assert.Contains("j:a", schema);
        Assert.Contains("f:big", schema);
        Assert.Contains("j:nul", schema);
        Assert.DoesNotContain("fields", schema);
        var d = RecordBatchReader.Read(bytes, RecordSchema.SourceName, RecordSchema.Group);
        Assert.Equal(2, d.Records.Count);
        var r = d.Records[0];
        Assert.Equal((-5L, Level.Fatal, "héllo\nworld ☃", 7u, 3u, 4u, ulong.MaxValue), (r.Timestamp!.Value, r.Level, r.Message, r.Line, r.Id, r.Source, r.Markers));
        Assert.Equal(("t {x}", "boom"), (r.Template, r.Exception));
        Assert.Equal(["z", "a", "s", "f", "b", "big", "nul"], r.Fields.Select(f => f.Key));
        Assert.Equal(a.Fields.ToJsonString(), r.Fields.ToJsonString());
        Assert.Empty(d.Records[1].Fields);
        Assert.Null(d.Records[1].Timestamp);
        Assert.Equal(["s1", "s2"], d.Extra[0]);
        Assert.Equal(["g1", ""], d.Extra[1]);
    }

    [Fact]
    public void Decoded_values_keep_their_types_when_encoded_again()
    {
        var once = RecordBatchReader.Read(RecordBatchBuilder.Encode([Sample()])).Records[0];
        var bytes = RecordBatchBuilder.Encode([once]);
        var schema = Schema(bytes);
        Assert.Contains("f:f", schema);
        Assert.Equal(new DoubleType().TypeId, Type(bytes, "f:f"));
        Assert.Equal(new Int64Type().TypeId, Type(bytes, "f:z"));
        Assert.Equal(new UInt64Type().TypeId, Type(bytes, "f:big"));
        Assert.Equal(new BooleanType().TypeId, Type(bytes, "f:b"));
        Assert.Equal(Sample().Fields.ToJsonString(), RecordBatchReader.Read(bytes).Records[0].Fields.ToJsonString());
    }

    [Fact]
    public void Mixed_types_become_json_columns_and_field_order_is_kept()
    {
        var a = new Record(1);
        a.Fields["x"] = 1;
        a.Fields["y"] = "s";
        var b = new Record(2);
        b.Fields["y"] = "t";
        b.Fields["x"] = "now text";
        var c = new Record(3);
        c.Fields["y"] = "only y";
        var bytes = RecordBatchBuilder.Encode([a, b, c]);
        var schema = Schema(bytes);
        Assert.Contains("j:x", schema);
        Assert.Contains("f:y", schema);
        Assert.Contains("field_order", schema);
        var d = RecordBatchReader.Read(bytes).Records;
        Assert.Equal(["x", "y"], d[0].Fields.Select(f => f.Key));
        Assert.Equal(["y", "x"], d[1].Fields.Select(f => f.Key));
        Assert.Equal("now text", d[1].FieldText("x"));
        Assert.Equal(1, d[0].Fields["x"]!.GetValue<long>());
        Assert.Equal(["y"], d[2].Fields.Select(f => f.Key));
    }

    [Fact]
    public void Many_or_sparse_names_use_the_json_layout()
    {
        var wide = new Record(1);
        for (int i = 0; i < 300; i++)
        {
            wide.Fields[$"k{i}"] = i;
        }

        var bytes = RecordBatchBuilder.Encode([wide]);
        Assert.Contains("fields", Schema(bytes));
        Assert.Equal(300, RecordBatchReader.Read(bytes).Records[0].Fields.Count);

        var sparse = Enumerable.Range(0, 40).Select(i =>
        {
            var r = new Record((uint)i);
            r.Fields[$"only{i}"] = "v";
            return r;
        }).ToList();
        var sb = RecordBatchBuilder.Encode(sparse);
        Assert.Contains("fields", Schema(sb));
        Assert.Equal("v", RecordBatchReader.Read(sb).Records[39].FieldText("only39"));

        var forced = new RecordBatchBuilder { ForceJsonFields = true }.Append(Sample()).Build();
        Assert.Contains("fields", Schema(forced));
        Assert.Equal(Sample().Fields.ToJsonString(), RecordBatchReader.Read(forced).Records[0].Fields.ToJsonString());
    }

    [Fact]
    public void Clr_values_are_classified_like_serde_json()
    {
        var r = new Record(1);
        r.Fields["i"] = (int)5;
        r.Fields["sh"] = (short)5;
        r.Fields["by"] = (byte)5;
        r.Fields["u"] = 5UL;
        r.Fields["fl"] = 1.5f;
        r.Fields["de"] = 1.25m;
        r.Fields["ch"] = 'c';
        r.Fields["nan"] = double.NaN;
        r.Fields["date"] = JsonValue.Create(new DateTime(2024, 1, 2, 0, 0, 0, DateTimeKind.Utc));
        r.Fields["guid"] = JsonValue.Create(Guid.Empty);
        r.Fields["arr"] = new JsonArray(1, 2);
        var bytes = RecordBatchBuilder.Encode([r]);
        Assert.Equal(new Int64Type().TypeId, Type(bytes, "f:i"));
        Assert.Equal(new Int64Type().TypeId, Type(bytes, "f:u"));
        Assert.Equal(new DoubleType().TypeId, Type(bytes, "f:fl"));
        Assert.Equal(new DoubleType().TypeId, Type(bytes, "f:de"));
        Assert.Equal(new StringType().TypeId, Type(bytes, "f:ch"));
        Assert.Equal(new StringType().TypeId, Type(bytes, "f:date"));
        var back = RecordBatchReader.Read(bytes).Records[0];
        Assert.Equal("null", back.Fields["nan"]?.ToJsonString() ?? "null");
        Assert.Equal("2024-01-02T00:00:00Z", back.FieldText("date"));
        Assert.Equal("[1,2]", back.Fields["arr"]!.ToJsonString());
        Assert.Equal(1.25, back.Fields["de"]!.GetValue<double>());
    }

    [Fact]
    public void Empty_batches_and_missing_columns()
    {
        Assert.Empty(RecordBatchReader.Read(RecordBatchBuilder.Encode([])).Records);
        Assert.Empty(RecordBatchReader.Read(ReadOnlyMemory<byte>.Empty).Records);
        var bytes = RecordBatchBuilder.Encode([new Record(1)]);
        var e = Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read(bytes, RecordSchema.Instance));
        Assert.Contains("instance", e.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void Garbage_is_an_error_not_a_crash()
    {
        Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read("not arrow at all"u8.ToArray()));
        var good = RecordBatchBuilder.Encode([Sample()]);
        foreach (var cut in new[] { 1, 8, good.Length / 2, good.Length - 9 })
        {
            Assert.ThrowsAny<Exception>(() => RecordBatchReader.Read(good.AsMemory(0, cut)));
        }
    }

    private static byte[] Custom(params (string Name, IArrowArray Array)[] cols)
    {
        var fields = cols.Select(c => new Field(c.Name, c.Array.Data.DataType, true)).ToList();
        var schema = new Apache.Arrow.Schema(fields, null);
        using var batch = new RecordBatch(schema, cols.Select(c => c.Array), cols[0].Array.Length);
        using var ms = new MemoryStream();
        using (var w = new ArrowStreamWriter(ms, schema, leaveOpen: true))
        {
            w.WriteRecordBatch(batch);
            w.WriteEnd();
        }

        return ms.ToArray();
    }

    private static List<(string, IArrowArray)> Valid(byte level, params (string, IArrowArray)[] fields)
    {
        var cols = new List<(string, IArrowArray)>
        {
            ("ts", new Int64Array.Builder().Append(1).Build()),
            ("level", new UInt8Array.Builder().Append(level).Build()),
            ("message", new StringArray.Builder().AppendNull().Build()),
            ("template", new StringArray.Builder().AppendNull().Build()),
            ("exception", new StringArray.Builder().AppendNull().Build()),
            ("line", new UInt32Array.Builder().Append(1).Build()),
            ("id", new UInt32Array.Builder().Append(0).Build()),
            ("source", new UInt32Array.Builder().Append(0).Build()),
            ("markers", new UInt64Array.Builder().Append(0).Build()),
        };
        cols.AddRange(fields);
        return cols;
    }

    [Fact]
    public void Invalid_values_are_rejected()
    {
        Assert.Equal(string.Empty, RecordBatchReader.Read(Custom([.. Valid(2)])).Records[0].Message);
        Assert.Contains("level 9", Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read(Custom([.. Valid(9)]))).Message, StringComparison.Ordinal);
        var json = new StringArray.Builder().Append("[1]").Build();
        Assert.Contains("not a JSON object", Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read(Custom([.. Valid(2, ("fields", json))]))).Message, StringComparison.Ordinal);
        var bad = new StringArray.Builder().Append("{").Build();
        Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read(Custom([.. Valid(2, ("fields", bad))])));
        var badJ = new StringArray.Builder().Append("{").Build();
        Assert.Contains("not valid JSON", Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read(Custom([.. Valid(2, ("j:x", badJ))]))).Message, StringComparison.Ordinal);
        var wrongType = new Int64Array.Builder().Append(1).Build();
        Assert.Contains("unsupported type", Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read(Custom([.. Valid(2, ("j:x", wrongType))]))).Message, StringComparison.Ordinal);
        var order = new StringArray.Builder().Append("5").Build();
        var x = new StringArray.Builder().Append("v").Build();
        Assert.Contains("field order", Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read(Custom([.. Valid(2, ("f:x", x), ("field_order", order))]))).Message, StringComparison.Ordinal);
        var cols = Valid(2);
        cols[1] = ("level", new UInt32Array.Builder().Append(2).Build());
        Assert.Contains("wrong type", Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read(Custom([.. cols]))).Message, StringComparison.Ordinal);
        var nullFields = new StringArray.Builder().AppendNull().Build();
        Assert.Empty(RecordBatchReader.Read(Custom([.. Valid(2, ("fields", nullFields), ("instance", new StringArray.Builder().AppendNull().Build()))]), "instance").Records[0].Fields);
        var many = Enumerable.Range(0, 257).Select(i => ($"f:k{i}", (IArrowArray)new StringArray.Builder().Append("v").Build())).ToArray();
        Assert.Contains("more than 256", Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read(Custom([.. Valid(2, many)]))).Message, StringComparison.Ordinal);
        var other = Valid(2, ("f:t", new UInt32Array.Builder().Append(1).Build()));
        Assert.Throws<InvalidDataException>(() => RecordBatchReader.Read(Custom([.. other])));
    }

    [Fact]
    public async Task Batches_are_split_by_rows_and_bytes()
    {
        static async IAsyncEnumerable<Record> Many(int n, int size)
        {
            for (int i = 0; i < n; i++)
            {
                await Task.Yield();
                yield return new Record((uint)i) { Message = new string('x', size) };
            }
        }

        var counts = new List<int>();
        await foreach (var (_, rows) in RecordBatchBuilder.Batches(Many(5, 1), 2))
        {
            counts.Add(rows);
        }

        Assert.Equal([2, 2, 1], counts);
        counts.Clear();
        await foreach (var (_, rows) in RecordBatchBuilder.Batches(Many(3, Contract.BatchBytes), 100))
        {
            counts.Add(rows);
        }

        Assert.Equal([1, 1, 1], counts);
        counts.Clear();
        await foreach (var (_, rows) in RecordBatchBuilder.Batches(Many(2, 1), 0))
        {
            counts.Add(rows);
        }

        Assert.Equal([1, 1], counts);
        Assert.True(RecordSchema.EstimateSize(Sample()) > 32);
        Assert.Equal(11, RecordSchema.JsonLayout("x").FieldsList.Count);
        Assert.Equal(9, RecordSchema.RecordColumns.Count);
    }

    [Fact]
    public void Compressed_and_uncompressed_batches_both_decode()
    {
        var records = Enumerable.Range(0, 2000).Select(i => new Record((uint)i) { Message = "the same message over and over again " + (i % 3) }).ToList();
        var compressed = RecordBatchBuilder.Encode(records);
        var plain = new RecordBatchBuilder { Compress = false };
        records.ForEach(r => plain.Append(r));
        var uncompressed = plain.Build();
        Assert.True(compressed.Length * 2 < uncompressed.Length, $"{compressed.Length} vs {uncompressed.Length}");
        Assert.Equal(records[1999].Message, RecordBatchReader.Read(compressed).Records[1999].Message);
        Assert.Equal(records[1999].Message, RecordBatchReader.Read(uncompressed).Records[1999].Message);
    }

    [Fact]
    public void Out_of_range_levels_are_written_as_information()
    {
        var bytes = RecordBatchBuilder.Encode([new Record(1) { Level = (Level)200 }]);
        Assert.Equal(Level.Information, RecordBatchReader.Read(bytes).Records[0].Level);
    }

    internal static List<string> Schema(byte[] bytes)
    {
        using var r = new ArrowStreamReader(bytes);
        return [.. r.Schema.FieldsList.Select(f => f.Name)];
    }

    private static ArrowTypeId Type(byte[] bytes, string name)
    {
        using var r = new ArrowStreamReader(bytes);
        return r.Schema.GetFieldByName(name).DataType.TypeId;
    }
}
