using System.Text;
using System.Text.Json.Nodes;
using Google.Protobuf;
using Grpc.Core;
using Grpc.Core.Interceptors;
using Imt.Plugin.Arrow;
using Imt.Plugin.Hosting;
using Imt.Plugin.Testing;
using KitchenSink;
using Wire = Imt.Plugin.V1;

namespace Imt.Plugin.Tests;

/// <summary>The plugin served over the real transport (socket, token, gRPC, Arrow), as IMT sees it.</summary>
public sealed class HostTests : IAsyncLifetime
{
    private readonly KitchenEvents events = new();
    private readonly KitchenSettings settings = new();
    private PluginTestHost host = null!;

    public async ValueTask InitializeAsync() => host = await PluginTestHost.StartAsync(Kitchen.Build(events, settings));

    public async ValueTask DisposeAsync() => await host.DisposeAsync();

    private static RemoteError Typed(RpcException e) => RemoteError.TryRead(e) ?? throw new InvalidOperationException($"no typed error in {e}");

    [Fact]
    public async Task Calls_before_the_handshake_are_refused_with_a_typed_error()
    {
        var e = await Assert.ThrowsAsync<RpcException>(async () => await host.Client.MarkerKindsAsync(new Wire.MarkerKindsRequest()));
        Assert.Equal(StatusCode.InvalidArgument, e.StatusCode);
        Assert.Equal((int)ErrorCode.NoHandshake, Typed(e).Code);
        await Assert.ThrowsAsync<RpcException>(async () => await host.SniffAsync("kitchen.lines", "a", "x"));
    }

    [Fact]
    public async Task The_handshake_checks_the_contract_and_returns_the_manifest()
    {
        var e = await Assert.ThrowsAsync<RpcException>(async () => await host.Client.HandshakeAsync(new Wire.HandshakeRequest { ContractMajor = 99 }));
        Assert.Equal(StatusCode.FailedPrecondition, e.StatusCode);
        Assert.Equal((int)ErrorCode.Version, Typed(e).Code);
        Assert.Contains("v99", Typed(e).Message, StringComparison.Ordinal);

        var m = await host.HandshakeAsync();
        Assert.Equal(("test.kitchen", "Kitchen sink", "1.2.3-beta.1", 1u, 0u), (m.Id, m.Name, m.Version, m.ContractMajor, m.ContractMinor));
        Assert.Equal(2, m.Requires.Count);
        Assert.True(m.Requires[0].Optional);
        Assert.Equal("kitchen.echo", m.Provides.Single().Id);
        Assert.Equal(Wire.PermissionKind.ReadOpenedFiles, m.Permissions.Single().Kind);
        var caps = m.Capabilities;
        var lines = caps.Single(c => c.Importer?.Id == "kitchen.lines").Importer;
        Assert.True(lines.Tail && !lines.Binary && !lines.Multiline);
        var bin = caps.Single(c => c.Importer?.Id == "kitchen.bin").Importer;
        Assert.True(bin.Binary && !bin.Tail);
        Assert.Equal("kitchen.upper", caps.Single(c => c.Transform is not null).Transform.Id);
        Assert.Equal(("Kitchen lines", "txt"), (caps.Single(c => c.Exporter is not null).Exporter.Name, caps.Single(c => c.Exporter is not null).Exporter.Extension));
        Assert.Single(caps, c => c.MarkerKinds is not null);
        var ann = caps.Single(c => c.Annotator is not null).Annotator;
        Assert.Equal((500u, "@level", "eq", "\"error\""), (ann.Limit, ann.Interest[0].Field, ann.Interest[0].Op, ann.Interest[0].ValueJson));
        Assert.Single(caps, c => c.Settings is not null);
        Assert.Equal("1.0.0", caps.Single(c => c.Service is not null).Service.Version);
        Assert.Equal(["recordsAppended", "workspaceCleared"], caps.Single(c => c.Events is not null).Events.Kinds);
    }

    [Fact]
    public async Task Importers_stream_records_in_batches_with_skips_and_typed_fields()
    {
        await host.HandshakeAsync();
        Assert.Equal(100u, await host.SniffAsync("kitchen.lines", "x.kitchen", "anything"));
        Assert.Equal(0u, await host.SniffAsync("kitchen.lines", "x.log", string.Empty));
        Assert.Equal(10u, await host.SniffAsync("kitchen.lines", "x.log", "INF|a"));
        var r = await host.ImportAsync("kitchen.lines", "a.kitchen", "ERR|boom|n=5|s=x\nnot a record\nINF|fine\n");
        Assert.Equal(1ul, r.Skipped);
        Assert.Equal(2, r.Records.Count);
        Assert.Equal(Level.Error, r.Records[0].Level);
        Assert.Equal(5, r.Records[0].Fields["n"]!.GetValue<long>());
        Assert.Equal("x", r.Records[0].FieldText("s"));
        Assert.Equal(3u, r.Records[1].Line);
        Assert.False(r.Records[0].Fields.ContainsKey("path"), "no path without the permission");

        var big = string.Concat(Enumerable.Range(0, 150_000).Select(n => $"INF|line {n}|k={n}\n"));
        var many = await host.ImportAsync("kitchen.lines", "big.kitchen", big);
        Assert.Equal(150_000, many.Records.Count);
        Assert.Equal(3, many.Batches);
        Assert.Equal(149_999, many.Records[^1].Fields["k"]!.GetValue<long>());

        var b = await host.ImportAsync("kitchen.bin", "x.bin", "\0abcdef");
        Assert.Equal("7 bytes", b.Records.Single().Message);
    }

    [Fact]
    public async Task The_path_is_only_sent_with_the_permission()
    {
        await host.HandshakeAsync(Permission.ReadOpenedFiles, Permission.Network("example.com"));
        using var call = host.Client.Import();
        await call.RequestStream.WriteAsync(new Wire.ImportChunk { Start = new Wire.ImportStart { ImporterId = "kitchen.lines", Ctx = new Wire.ImportContext { FileName = "a", Path = "/x/a", FileDate = "bad" }, BatchRows = 1 } });
        await call.RequestStream.WriteAsync(new Wire.ImportChunk { Chunk = new Wire.Chunk { Data = ByteString.CopyFromUtf8("INF|a\nINF|b\n") } });
        await call.RequestStream.CompleteAsync();
        var batches = new List<Wire.RecordBatch>();
        while (await call.ResponseStream.MoveNext(CancellationToken.None))
        {
            if (call.ResponseStream.Current.Batch is { } batch)
            {
                batches.Add(batch);
            }
        }

        Assert.Equal(2, batches.Count);
        Assert.Equal("/x/a", RecordBatchReader.Read(batches[0].Arrow.Memory).Records[0].FieldText("path"));
    }

    [Fact]
    public async Task Typed_errors_crashes_and_malformed_streams()
    {
        await host.HandshakeAsync();
        var failed = await Assert.ThrowsAsync<RpcException>(() => host.ImportAsync("kitchen.lines", "a", "ERROR\n"));
        Assert.Equal(StatusCode.Aborted, failed.StatusCode);
        Assert.Equal(((int)ErrorCode.Failed, "this file is not valid"), (Typed(failed).Code, Typed(failed).Message));

        var crashed = await Assert.ThrowsAsync<RpcException>(() => host.ImportAsync("kitchen.lines", "a", "CRASH\n"));
        Assert.Equal(StatusCode.Internal, crashed.StatusCode);
        Assert.Equal((int)ErrorCode.Crashed, Typed(crashed).Code);
        Assert.Contains("InvalidOperationException: a bug", Typed(crashed).Message, StringComparison.Ordinal);

        var unknown = await Assert.ThrowsAsync<RpcException>(() => host.ImportAsync("nope", "a", "x"));
        Assert.Equal((int)ErrorCode.Invalid, Typed(unknown).Code);
        var unknownSniff = await Assert.ThrowsAsync<RpcException>(() => host.SniffAsync("nope", "a", "x"));
        Assert.Equal((int)ErrorCode.Invalid, Typed(unknownSniff).Code);
        var noCtx = await Assert.ThrowsAsync<RpcException>(async () => await host.Client.SniffAsync(new Wire.SniffRequest { ImporterId = "kitchen.lines" }));
        Assert.Contains("context is missing", Typed(noCtx).Message, StringComparison.Ordinal);

        // A stream that does not start with ImportStart, or mixes in another start.
        using (var call = host.Client.Import())
        {
            await call.RequestStream.WriteAsync(new Wire.ImportChunk { Chunk = new Wire.Chunk() });
            await call.RequestStream.CompleteAsync();
            var e = await Assert.ThrowsAsync<RpcException>(() => call.ResponseStream.MoveNext(CancellationToken.None));
            Assert.Equal((int)ErrorCode.Invalid, Typed(e).Code);
        }

        using (var call = host.Client.Import())
        {
            var start = new Wire.ImportChunk { Start = new Wire.ImportStart { ImporterId = "kitchen.lines", Ctx = new Wire.ImportContext() } };
            await call.RequestStream.WriteAsync(start);
            await call.RequestStream.WriteAsync(start);
            await call.RequestStream.CompleteAsync();
            var e = await Assert.ThrowsAsync<RpcException>(() => call.ResponseStream.MoveNext(CancellationToken.None));
            Assert.Contains("only chunks", Typed(e).Message, StringComparison.Ordinal);
        }

        using (var call = host.Client.Import())
        {
            await call.RequestStream.WriteAsync(new Wire.ImportChunk { Start = new Wire.ImportStart { ImporterId = "kitchen.lines", TotalBytes = ulong.MaxValue } });
            var e = await Assert.ThrowsAsync<RpcException>(() => call.ResponseStream.MoveNext(CancellationToken.None));
            Assert.Equal(StatusCode.ResourceExhausted, e.StatusCode);
            Assert.Equal((int)ErrorCode.TooLarge, Typed(e).Code);
        }
    }

    [Fact]
    public async Task Live_tail_parses_the_tail_or_asks_for_a_full_import()
    {
        await host.HandshakeAsync();
        var t = await host.ImportTailAsync("kitchen.lines", "a.kitchen", "INF|a\n", "WRN|b\n");
        Assert.Equal("b", t.Records.Single().Message);
        Assert.Equal(1u, t.Records[0].Line);
        Assert.False(t.TailUnsupported);
        var none = await host.ImportTailAsync("kitchen.lines", "a.kitchen", "INF|a\n", "NOTAIL\n");
        Assert.True(none.TailUnsupported);
        Assert.Empty(none.Records);
        var binary = await Assert.ThrowsAsync<RpcException>(() => host.ImportTailAsync("kitchen.bin", "a", "x", "y"));
        Assert.Contains("no text importer", Typed(binary).Message, StringComparison.Ordinal);
    }

    [Fact]
    public async Task Deadlines_cancel_the_call_and_the_plugin_stays_responsive()
    {
        await host.HandshakeAsync();
        var started = DateTime.UtcNow;
        var e = await Assert.ThrowsAsync<RpcException>(() => host.ImportAsync("kitchen.lines", "a", "INF|x\nSLOW\n", DateTime.UtcNow.AddMilliseconds(300)));
        Assert.Equal(StatusCode.DeadlineExceeded, e.StatusCode);
        Assert.True(DateTime.UtcNow - started < TimeSpan.FromSeconds(5));
        var status = await host.Control.StatusAsync(new Imt.Ipc.V1.StatusRequest());
        Assert.Equal((uint)Environment.ProcessId, status.Pid);
        Assert.Equal("test.kitchen 1.2.3-beta.1", status.Name);
        Assert.True(status.RssBytes > 0);
    }

    [Fact]
    public async Task Transforms_return_every_row_with_verdicts_marks_and_imt_values_kept()
    {
        await host.HandshakeAsync();
        var records = new[] { "hello", "please drop", "mark me", "x" }.Select((m, i) => new Record((uint)i) { Message = m, Id = 100 + (uint)i, Source = 7, Markers = 5 }).ToList();
        var results = await host.TransformAsync("kitchen.upper", "a.kitchen", records, [new Record(9) { Message = "second batch" }]);
        Assert.Equal(2, results.Count);
        var first = results[0];
        Assert.Equal([Verdict.Keep, Verdict.Drop, Verdict.Keep, Verdict.Keep], first.Verdicts);
        Assert.Equal("HELLO", first.Records[0].Message);
        Assert.True(first.Records[0].Fields["upper"]!.GetValue<bool>());
        Assert.Equal("a.kitchen", first.Records[0].FieldText("source"));
        Assert.Equal((100u, 7u, 5ul), (first.Records[0].Id, first.Records[0].Source, first.Records[0].Markers));
        Assert.Equal(["kitchen-mark", "not-mine"], first.Marks[2]);
        Assert.False(first.Marks.ContainsKey(0));
        // State lives across the batches of one call.
        Assert.Equal(5, results[1].Records[0].Fields["seen"]!.GetValue<long>());

        var crash = await Assert.ThrowsAsync<RpcException>(() => host.TransformAsync("kitchen.upper", "a", [new Record(1) { Message = "boom" }]));
        Assert.Equal((int)ErrorCode.Crashed, Typed(crash).Code);
        var unknown = await Assert.ThrowsAsync<RpcException>(() => host.TransformAsync("nope", "a", [new Record(1)]));
        Assert.Equal((int)ErrorCode.Invalid, Typed(unknown).Code);

        using var call = host.Client.Transform();
        await call.RequestStream.WriteAsync(new Wire.TransformIn { Start = new Wire.TransformStart { TransformId = "kitchen.upper" } });
        await call.RequestStream.WriteAsync(new Wire.TransformIn { Batch = new Wire.RecordBatch { Arrow = ByteString.CopyFromUtf8("garbage") } });
        var bad = await Assert.ThrowsAsync<RpcException>(() => call.ResponseStream.MoveNext(CancellationToken.None));
        Assert.Equal((int)ErrorCode.Invalid, Typed(bad).Code);
    }

    [Fact]
    public async Task Streams_must_start_with_their_start_message()
    {
        await host.HandshakeAsync();
        using (var t = host.Client.Transform())
        {
            await t.RequestStream.WriteAsync(new Wire.TransformIn { Batch = new Wire.RecordBatch() });
            Assert.Equal((int)ErrorCode.Invalid, Typed(await Assert.ThrowsAsync<RpcException>(() => t.ResponseStream.MoveNext(CancellationToken.None))).Code);
        }

        using (var t = host.Client.Transform())
        {
            var start = new Wire.TransformIn { Start = new Wire.TransformStart { TransformId = "kitchen.upper" } };
            await t.RequestStream.WriteAsync(start);
            await t.RequestStream.WriteAsync(start);
            Assert.Contains("only batches", Typed(await Assert.ThrowsAsync<RpcException>(() => t.ResponseStream.MoveNext(CancellationToken.None))).Message, StringComparison.Ordinal);
        }

        using (var a = host.Client.Annotate())
        {
            await a.RequestStream.CompleteAsync();
            Assert.Equal((int)ErrorCode.Invalid, Typed(await Assert.ThrowsAsync<RpcException>(() => a.ResponseAsync)).Code);
        }

        using (var a = host.Client.Annotate())
        {
            var start = new Wire.AnnotateIn { Start = new Wire.AnnotateStart { AnnotatorId = "kitchen.errors" } };
            await a.RequestStream.WriteAsync(start);
            await a.RequestStream.WriteAsync(start);
            await a.RequestStream.CompleteAsync();
            Assert.Contains("only batches", Typed(await Assert.ThrowsAsync<RpcException>(() => a.ResponseAsync)).Message, StringComparison.Ordinal);
        }

        using (var a = host.Client.Annotate())
        {
            await a.RequestStream.WriteAsync(new Wire.AnnotateIn { Start = new Wire.AnnotateStart { AnnotatorId = "nope" } });
            await a.RequestStream.CompleteAsync();
            Assert.Equal((int)ErrorCode.Invalid, Typed(await Assert.ThrowsAsync<RpcException>(() => a.ResponseAsync)).Code);
        }

        using (var x = host.Client.Export())
        {
            await x.RequestStream.WriteAsync(new Wire.ExportIn { Batch = new Wire.RecordBatch() });
            Assert.Equal((int)ErrorCode.Invalid, Typed(await Assert.ThrowsAsync<RpcException>(() => x.ResponseStream.MoveNext(CancellationToken.None))).Code);
        }

        using (var x = host.Client.Export())
        {
            await x.RequestStream.WriteAsync(new Wire.ExportIn { Start = new Wire.ExportStart { ExporterId = "nope" } });
            Assert.Equal((int)ErrorCode.Invalid, Typed(await Assert.ThrowsAsync<RpcException>(() => x.ResponseStream.MoveNext(CancellationToken.None))).Code);
        }

        using (var x = host.Client.Export())
        {
            var start = new Wire.ExportIn { Start = new Wire.ExportStart { ExporterId = "kitchen.lines" } };
            await x.RequestStream.WriteAsync(start);
            await x.RequestStream.WriteAsync(start);
            await x.RequestStream.CompleteAsync();
            var e = await Assert.ThrowsAsync<RpcException>(async () =>
            {
                while (await x.ResponseStream.MoveNext(CancellationToken.None))
                {
                }
            });
            Assert.Contains("only batches", Typed(e).Message, StringComparison.Ordinal);
        }
    }

    [Fact]
    public async Task Annotators_receive_their_records_with_source_and_instance()
    {
        await host.HandshakeAsync();
        var recs = new List<AnnotatedRecord>
        {
            new(new Record(1) { Level = Level.Error, Message = "bad", Timestamp = 10 }, "a.log", "i1"),
            new(new Record(2) { Level = Level.Information, Message = "fine", Timestamp = 20 }, "a.log", "i1"),
        };
        var list = await host.AnnotateAsync("kitchen.errors", recs);
        var a = Assert.Single(list);
        Assert.Equal(("error", 10L, 11L, "a.log: bad", "i1", "Computer", "WEB01", Wire.Severity.Critical), (a.Kind, a.From, a.To, a.Label, a.Instance, a.OnField, a.OnValue, a.Severity));
        Assert.Equal("""{"from":10}""", a.EvidenceQueryJson);

        using var call = host.Client.Annotate();
        await call.RequestStream.WriteAsync(new Wire.AnnotateIn { Start = new Wire.AnnotateStart { AnnotatorId = "kitchen.errors", HasSince = true, SinceId = 3, SinceHasTs = true, SinceTs = 5, HasRange = true, RangeFrom = 1, RangeTo = 2 } });
        await call.RequestStream.WriteAsync(new Wire.AnnotateIn { Batch = new Wire.RecordBatch { Arrow = ByteString.CopyFrom(RecordBatchBuilder.Encode([new Record(1)])) } });
        await call.RequestStream.CompleteAsync();
        var e = await Assert.ThrowsAsync<RpcException>(() => call.ResponseAsync);
        Assert.Contains("source_name", Typed(e).Message, StringComparison.Ordinal);
    }

    [Fact]
    public async Task Too_many_annotator_records_are_refused()
    {
        await host.HandshakeAsync();
        using var call = host.Client.Annotate();
        await call.RequestStream.WriteAsync(new Wire.AnnotateIn { Start = new Wire.AnnotateStart { AnnotatorId = "kitchen.errors" } });
        var b = new RecordBatchBuilder(RecordSchema.SourceName, RecordSchema.Instance);
        for (int i = 0; i < 60_000; i++)
        {
            b.Append(new Record((uint)i), "s", "i");
        }

        var bytes = ByteString.CopyFrom(b.Build());
        await call.RequestStream.WriteAsync(new Wire.AnnotateIn { Batch = new Wire.RecordBatch { Arrow = bytes, Rows = 60_000 } });
        await call.RequestStream.WriteAsync(new Wire.AnnotateIn { Batch = new Wire.RecordBatch { Arrow = bytes, Rows = 60_000 } });
        await call.RequestStream.CompleteAsync();
        var e = await Assert.ThrowsAsync<RpcException>(() => call.ResponseAsync);
        Assert.Equal((int)ErrorCode.TooLarge, Typed(e).Code);
    }

    [Fact]
    public async Task Exporters_stream_their_output_in_chunks()
    {
        await host.HandshakeAsync();
        var rows = Enumerable.Range(0, 120_000).Select(i => new ExportRow(new Record((uint)i) { Message = $"m{i}" }, "a.log", "g")).ToList();
        var (output, n) = await host.ExportAsync("kitchen.lines", rows);
        Assert.Equal(120_000ul, n);
        var text = Encoding.UTF8.GetString(output);
        Assert.StartsWith("a.log|g|m0\n", text, StringComparison.Ordinal);
        Assert.EndsWith("a.log|g|m119999\n", text, StringComparison.Ordinal);
        Assert.True(output.Length > Contract.ChunkBytes);
        var (empty, zero) = await host.ExportAsync("kitchen.lines", []);
        Assert.Empty(empty);
        Assert.Equal(0ul, zero);
    }

    [Fact]
    public async Task Marker_kinds_and_settings_cross_the_contract()
    {
        await host.HandshakeAsync();
        var kinds = (await host.Client.MarkerKindsAsync(new Wire.MarkerKindsRequest())).Kinds;
        Assert.Equal(["kitchen-mark", "kitchen-error", "kitchen-start", "kitchen-stop"], kinds.Select(k => k.Kind));
        Assert.Equal((Wire.MarkerRole.RoleNone, "M", "circle"), (kinds[0].Role, kinds[0].Badge, kinds[0].Shape));
        Assert.Equal((Wire.MarkerRole.RoleCrash, "#F00", "x-circle", "@level", "\"error\""), (kinds[1].Role, kinds[1].Color, kinds[1].Icon, kinds[1].When[0].Field, kinds[1].When[0].ValueJson));
        Assert.Equal(Wire.MarkerRole.RoleStart, kinds[2].Role);
        Assert.Equal((Wire.MarkerRole.RoleStop, "[\"Stop\",\"Exit\"]"), (kinds[3].Role, kinds[3].When[0].ValueJson));

        var section = (await host.Client.SettingsSectionAsync(new Wire.SettingsSectionRequest())).Section;
        Assert.Equal(("Kitchen", "desc", 9), (section.Title, section.Description, section.Fields.Count));
        Assert.True(section.Fields[0].RestartRequired && section.Fields[0].Kind.Bool is not null);
        Assert.Equal((0L, 10L, 2L, "x", true), (section.Fields[1].Kind.Int.Min, section.Fields[1].Kind.Int.Max, section.Fields[1].Kind.Int.Step, section.Fields[1].Kind.Int.Unit, section.Fields[1].Advanced));
        Assert.Equal(1.0, section.Fields[2].Kind.Float.Max);
        Assert.Equal(("[a-z]*", 10ul), (section.Fields[3].Kind.Text.Pattern, section.Fields[3].Kind.Text.MaxLen));
        Assert.Equal(2, section.Fields[4].Kind.Enum.Options.Count);
        Assert.True(section.Fields[5].Kind.Path.Dir);
        Assert.Equal(1000ul, section.Fields[6].Kind.Duration.MaxMs);
        Assert.Equal(100ul, section.Fields[7].Kind.Size.MaxBytes);
        Assert.Equal((3ul, "[\"x\"]"), (section.Fields[8].Kind.StringList.MaxItems, section.Fields[8].DefaultJson));

        await host.ApplySettingsAsync(new JsonObject { ["i"] = 6.0, ["t"] = "abc", ["l"] = new JsonArray("a", "b") });
        Assert.Equal(6, settings.Applied!.GetInt64("i"));
        Assert.True(settings.Applied.GetBool("b"));
        Assert.Equal(["a", "b"], settings.Applied.GetStringList("l"));
        var e = await Assert.ThrowsAsync<RpcException>(() => host.ApplySettingsAsync(new JsonObject { ["i"] = 5, ["t"] = "ABC" }));
        Assert.Equal((int)ErrorCode.Invalid, Typed(e).Code);
        Assert.Contains("i: Must be 0 plus a multiple of 2.", Typed(e).Message, StringComparison.Ordinal);
        Assert.Contains("t: Does not have the expected format", Typed(e).Message, StringComparison.Ordinal);
        var notJson = await Assert.ThrowsAsync<RpcException>(async () => await host.Client.ApplySettingsAsync(new Wire.ApplySettingsRequest { ValuesJson = "{" }));
        Assert.Contains("not JSON", Typed(notJson).Message, StringComparison.Ordinal);
    }

    [Fact]
    public async Task Services_and_events()
    {
        await host.HandshakeAsync();
        var reply = await host.Client.CallServiceAsync(new Wire.ServiceCall { ServiceId = "kitchen.echo", Method = "echo", Payload = ByteString.CopyFromUtf8("hi") });
        Assert.Equal("hi", reply.Payload.ToStringUtf8());
        var fail = await Assert.ThrowsAsync<RpcException>(async () => await host.Client.CallServiceAsync(new Wire.ServiceCall { ServiceId = "kitchen.echo", Method = "fail" }));
        Assert.Equal("test", Typed(fail).Data!["why"]!.GetValue<string>());
        var none = await Assert.ThrowsAsync<RpcException>(async () => await host.Client.CallServiceAsync(new Wire.ServiceCall { ServiceId = "nope" }));
        Assert.Equal(StatusCode.Unimplemented, none.StatusCode);

        using var call = host.Client.Events();
        await call.RequestStream.WriteAsync(new Wire.HostEvent { Kind = "recordsAppended", PayloadJson = """{"count":3,"sources":[1]}""" });
        await call.RequestStream.WriteAsync(new Wire.HostEvent { Kind = "profileChanged", PayloadJson = """{"name":"x"}""" });
        await call.RequestStream.WriteAsync(new Wire.HostEvent { Kind = "workspaceCleared", PayloadJson = "not json" });
        await call.RequestStream.WriteAsync(new Wire.HostEvent { Kind = "workspaceCleared" });
        await call.RequestStream.CompleteAsync();
        Assert.Equal(4ul, (await call.ResponseAsync).Received);
        Assert.Equal(["recordsAppended", "workspaceCleared", "workspaceCleared"], events.Received.Select(e => e.Kind));
        Assert.Equal(3, events.Received[0].Payload["count"]!.GetValue<int>());
        Assert.Empty(events.Received[1].Payload.AsObject());
    }

    [Fact]
    public async Task Plugins_without_settings_answer_unimplemented()
    {
        await using var bare = await PluginTestHost.StartAsync(PluginBuilder.Create("test.bare").Add(new LinesImporter()).Build());
        await bare.HandshakeAsync();
        var e = await Assert.ThrowsAsync<RpcException>(async () => await bare.Client.SettingsSectionAsync(new Wire.SettingsSectionRequest()));
        Assert.Equal(StatusCode.Unimplemented, e.StatusCode);
        e = await Assert.ThrowsAsync<RpcException>(() => bare.ApplySettingsAsync([]));
        Assert.Equal((int)ErrorCode.Unimplemented, Typed(e).Code);
        Assert.Empty((await bare.Client.MarkerKindsAsync(new Wire.MarkerKindsRequest())).Kinds);
    }

    [Fact]
    public async Task Calls_without_the_right_token_are_refused()
    {
        using var channel = PluginTestHost.Connect(host.Launch.Address);
        var noToken = new Wire.Plugin.PluginClient(channel);
        var e = await Assert.ThrowsAsync<RpcException>(async () => await noToken.HandshakeAsync(new Wire.HandshakeRequest { ContractMajor = 1 }));
        Assert.Equal(StatusCode.Unauthenticated, e.StatusCode);
        var wrong = new Imt.Ipc.V1.Control.ControlClient(channel.Intercept(m =>
        {
            m.Add(Contract.TokenHeader, host.Launch.Token[..^1] + "x");
            return m;
        }));
        e = await Assert.ThrowsAsync<RpcException>(async () => await wrong.StatusAsync(new Imt.Ipc.V1.StatusRequest()));
        Assert.Equal(StatusCode.Unauthenticated, e.StatusCode);
        var shorter = new Wire.Plugin.PluginClient(channel.Intercept(m =>
        {
            m.Add(Contract.TokenHeader, "abc");
            return m;
        }));
        await Assert.ThrowsAsync<RpcException>(async () => await shorter.SniffAsync(new Wire.SniffRequest()));
        using var stream = noToken.Import();
        await Assert.ThrowsAsync<RpcException>(() => stream.ResponseStream.MoveNext(CancellationToken.None));
        using var duplex = noToken.Transform();
        await Assert.ThrowsAsync<RpcException>(() => duplex.ResponseStream.MoveNext(CancellationToken.None));
        using var client = noToken.Annotate();
        await Assert.ThrowsAsync<RpcException>(() => client.ResponseAsync);
    }

    [Fact]
    public async Task The_socket_is_private()
    {
        if (OperatingSystem.IsWindows())
        {
            return;
        }

        var mode = File.GetUnixFileMode(host.Launch.Address);
        Assert.Equal(UnixFileMode.UserRead | UnixFileMode.UserWrite, mode);
        var dir = File.GetUnixFileMode(Path.GetDirectoryName(host.Launch.Address)!);
        Assert.Equal(UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute, dir);
        await Task.CompletedTask;
    }
}
