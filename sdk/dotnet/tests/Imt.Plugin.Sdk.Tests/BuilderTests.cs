using System.Text.Json.Nodes;
using Imt.Plugin.Hosting;
using KitchenSink;

namespace Imt.Plugin.Tests;

public class BuilderTests
{
    private sealed class Importer(string id, bool binary = false) : IImporter
    {
        public string Id => id;

        public bool Binary => binary;

        public ValueTask<int> SniffAsync(ImportContext context, ReadOnlyMemory<byte> head, CancellationToken cancellationToken) => ValueTask.FromResult(0);

        public IAsyncEnumerable<Record> ImportAsync(ImportContext context, ImportInput input, CancellationToken cancellationToken) => AsyncEnumerable.Empty<Record>();
    }

    private sealed class BinaryTail : ITailImporter
    {
        public string Id => "bt";

        public bool Binary => true;

        public ValueTask<int> SniffAsync(ImportContext context, ReadOnlyMemory<byte> head, CancellationToken cancellationToken) => ValueTask.FromResult(0);

        public IAsyncEnumerable<Record> ImportAsync(ImportContext context, ImportInput input, CancellationToken cancellationToken) => AsyncEnumerable.Empty<Record>();

        public IAsyncEnumerable<Record>? ImportTail(ImportContext context, string head, string tail, CancellationToken cancellationToken) => null;
    }

    private sealed class Exporter(string ext) : IExporter
    {
        public string Id => "e";

        public string Name => "E";

        public string Extension => ext;

        public ValueTask<long> ExportAsync(IAsyncEnumerable<ExportRow> rows, Stream output, CancellationToken cancellationToken) => ValueTask.FromResult(0L);
    }

    private sealed class Markers(params MarkerKind[] kinds) : IMarkerKinds
    {
        public IReadOnlyList<MarkerKind> MarkerKinds => kinds;
    }

    private sealed class Annotator(int limit, params Condition[] interest) : IAnnotator
    {
        public string Id => "a";

        public IReadOnlyList<Condition> Interest => interest;

        public int Limit => limit;

        public ValueTask<IReadOnlyList<Annotation>> AnnotateAsync(AnnotationInput input, CancellationToken cancellationToken) => ValueTask.FromResult<IReadOnlyList<Annotation>>([]);
    }

    private sealed class Settings(SettingsSection section) : ISettingsSection
    {
        public SettingsSection Section => section;

        public ValueTask ApplyAsync(SettingsValues values, CancellationToken cancellationToken) => ValueTask.CompletedTask;
    }

    private sealed class Events(int n) : IEventSubscriber
    {
        public IReadOnlyList<string> EventKinds => [.. Enumerable.Range(0, n).Select(i => $"k{i}")];

        public ValueTask OnEventAsync(PluginEvent e, CancellationToken cancellationToken) => ValueTask.CompletedTask;
    }

    private static string Fails(Func<PluginBuilder, PluginBuilder> f) =>
        Assert.Throws<InvalidOperationException>(() => f(PluginBuilder.Create("com.example.x")).Build()).Message;

    [Fact]
    public void Capabilities_come_from_the_interfaces_implemented()
    {
        var d = Kitchen.Build();
        Assert.Equal(("test.kitchen", "Kitchen sink", "1.2.3-beta.1"), (d.Manifest.Id, d.Manifest.Name, d.Manifest.Version));
        Assert.Equal(2, d.Importers.Count);
        Assert.Single(d.Transforms);
        Assert.Single(d.Exporters);
        Assert.Equal(4, d.MarkerKinds.Count);
        Assert.Single(d.Annotators);
        Assert.NotNull(d.Settings);
        Assert.Single(d.Services);
        Assert.Equal(new ServiceDeclaration("kitchen.echo", "1.0.0"), d.Manifest.Provides.Single());
        Assert.Equal(["recordsAppended", "workspaceCleared"], d.EventKinds);
        var plain = PluginBuilder.Create("com.example.p").Add(new Importer("i")).Build();
        Assert.Equal(("com.example.p", "0.1.0"), (plain.Manifest.Name, plain.Manifest.Version));
        Assert.Null(plain.Settings);
        var explicitProvides = PluginBuilder.Create("p").Provides("kitchen.echo", "2.0.0").Add(new EchoService()).Build();
        Assert.Equal("2.0.0", explicitProvides.Manifest.Provides.Single().Version);
    }

    [Fact]
    public void Mistakes_fail_fast_with_a_message_that_says_what_to_fix()
    {
        Assert.Contains("not a valid id", Assert.Throws<InvalidOperationException>(() => PluginBuilder.Create("bad id").Add(new Importer("i")).Build()).Message, StringComparison.Ordinal);
        Assert.Contains("not semver", Fails(b => b.Version("1.0").Add(new Importer("i"))), StringComparison.Ordinal);
        Assert.Contains("contributes nothing", Fails(b => b), StringComparison.Ordinal);
        Assert.Contains("declared twice", Fails(b => b.Add(new Importer("i")).Add(new Importer("i"))), StringComparison.Ordinal);
        Assert.Contains("not a valid id", Fails(b => b.Add(new Importer(string.Empty))), StringComparison.Ordinal);
        Assert.Contains("requirement is empty", Fails(b => b.Requires("o", " ").Add(new Importer("i"))), StringComparison.Ordinal);
        Assert.Contains("dependency", Fails(b => b.Optionally("o o", "^1").Add(new Importer("i"))), StringComparison.Ordinal);
        Assert.Contains("not semver", Fails(b => b.Provides("s", "x").Add(new Importer("i"))), StringComparison.Ordinal);
        Assert.Contains("not a host name", Fails(b => b.Permission(Permission.Network("evil host/x"), "r").Add(new Importer("i"))), StringComparison.Ordinal);
        Assert.Contains("extension", Fails(b => b.Add(new Exporter(".csv"))), StringComparison.Ordinal);
        Assert.Contains("binary importers cannot live-tail", Fails(b => b.Add(new BinaryTail())), StringComparison.Ordinal);
        Assert.Contains("1–64 characters", Fails(b => b.Add(new Markers(new MarkerKind("bad kind", "B")))), StringComparison.Ordinal);
        Assert.Contains("declared twice", Fails(b => b.Add(new Markers(new MarkerKind("k", "K"), new MarkerKind("k", "K")))), StringComparison.Ordinal);
        Assert.Contains("at most 64", Fails(b => b.Add(new Markers([.. Enumerable.Range(0, 65).Select(i => new MarkerKind($"k{i}", "K"))]))), StringComparison.Ordinal);
        Assert.Contains("unknown icon", Fails(b => b.Add(new Markers(new MarkerKind("k", "K") { Icon = "nope" }))), StringComparison.Ordinal);
        Assert.Contains("badge", Fails(b => b.Add(new Markers(new MarkerKind("k", "K") { Badge = "LONG" }))), StringComparison.Ordinal);
        Assert.Contains("shape", Fails(b => b.Add(new Markers(new MarkerKind("k", "K") { Shape = "star" }))), StringComparison.Ordinal);
        Assert.Contains("#RGB", Fails(b => b.Add(new Markers(new MarkerKind("k", "K") { Color = "red" }))), StringComparison.Ordinal);
        Assert.Contains("unknown condition operator", Fails(b => b.Add(new Markers(new MarkerKind("k", "K") { When = [new Condition("f", "like")] }))), StringComparison.Ordinal);
        Assert.Contains("has no field", Fails(b => b.Add(new Markers(new MarkerKind("k", "K") { When = [new Condition(string.Empty, "eq")] }))), StringComparison.Ordinal);
        Assert.Contains("limit", Fails(b => b.Add(new Annotator(200_000))), StringComparison.Ordinal);
        Assert.Contains("operator", Fails(b => b.Add(new Annotator(0, new Condition("f", "??")))), StringComparison.Ordinal);
        Assert.Contains("invalid settings schema", Fails(b => b.Add(new Settings(new SettingsSection("S", [new SettingField("k", "K", new IntSetting(0, 10), 50)])))), StringComparison.Ordinal);
        Assert.Contains("no fields", Fails(b => b.Add(new Settings(new SettingsSection("S", [])))), StringComparison.Ordinal);
        Assert.Contains("64 event kinds", Fails(b => b.Add(new Events(65))), StringComparison.Ordinal);
        Assert.Throws<ArgumentException>(() => PluginBuilder.Create("p").Add("not a contribution"));
    }

    [Fact]
    public void The_manifest_json_for_the_plugin_folder()
    {
        var json = JsonNode.Parse(PluginPackage.ManifestJson(Kitchen.Build(), "kitchen"))!;
        Assert.Equal(1, json["format"]!.GetValue<int>());
        Assert.Equal("imt.plugin.v1", json["contract"]!["package"]!.GetValue<string>());
        Assert.Equal("kitchen", json["executable"]!.GetValue<string>());
        Assert.Equal("test.kitchen", json["manifest"]!["id"]!.GetValue<string>());
        Assert.Equal("READ_OPENED_FILES", json["manifest"]!["permissions"]![0]!["kind"]!.GetValue<string>());
        Assert.False(string.IsNullOrEmpty(PluginPackage.ExecutableName()));
        Assert.Equal("test.kitchen", ImtPlugin.ManifestMessage(Kitchen.Build()).Id);
    }

    [Fact]
    public void Levels_parse_common_spellings()
    {
        foreach (var (text, level) in new[] { ("INF", Level.Information), ("warn", Level.Warning), ("ERR", Level.Error), ("Trace", Level.Verbose), ("DBG", Level.Debug), ("CRITICAL", Level.Fatal), (" info ", Level.Information) })
        {
            Assert.True(Levels.TryParse(text, out var l), text);
            Assert.Equal(level, l);
        }

        Assert.False(Levels.TryParse("nope", out var x));
        Assert.Equal(Level.Information, x);
        Assert.False(Levels.TryParse(string.Empty, out _));
        Assert.False(Levels.TryParse("much-too-long-level", out _));
    }

    [Fact]
    public void Records_and_conditions()
    {
        var r = new Record(3) { Time = DateTimeOffset.FromUnixTimeMilliseconds(1000) };
        Assert.Equal(1000, r.Timestamp);
        Assert.Equal(DateTimeOffset.FromUnixTimeMilliseconds(1000), r.Time);
        r.Time = null;
        Assert.Null(r.Time);
        r.Fields["n"] = 5;
        r.Fields["s"] = "x";
        r.Fields["null"] = null;
        Assert.Equal("5", r.FieldText("n"));
        Assert.Equal("x", r.FieldText("s"));
        Assert.Null(r.FieldText("null"));
        Assert.Null(r.FieldText("missing"));
        var c = r.Clone();
        c.Fields["n"] = 6;
        Assert.Equal("5", r.FieldText("n"));
        Assert.Equal(3u, c.Line);
        Assert.Equal("[\"a\",\"b\"]", Condition.In("f", "a", "b").Value!.ToJsonString());
        Assert.Equal("ne", Condition.Ne("f", "v").Op);
        Assert.Equal("regex", Condition.Regex("f", "v").Op);
        Assert.Equal("lt", Condition.Lt("f", 1).Op);
        Assert.Null(Condition.Exists("f").Value);
        Assert.Equal("missing", Condition.Missing("f").Op);
        Assert.Equal(Permission.ReadOpenedFiles, new Permission(PermissionKind.ReadOpenedFiles));
        Assert.Equal(0, new ImportContext("a").Skipped);
        Assert.Null(new ImportContext("a", profileJson: "{").Profile);
        Assert.NotNull(new ImportContext("a", profileJson: "{}").Profile);
        Assert.Null(new ImportContext("a").Profile);
        Assert.Equal("é", new ImportInput("é").Text);
    }

    [Fact]
    public void Remote_errors_map_codes_and_survive_odd_trailers()
    {
        foreach (var (code, status) in new[]
        {
            (ErrorCode.Invalid, Grpc.Core.StatusCode.InvalidArgument), (ErrorCode.NoHandshake, Grpc.Core.StatusCode.InvalidArgument),
            (ErrorCode.Unimplemented, Grpc.Core.StatusCode.Unimplemented), (ErrorCode.Version, Grpc.Core.StatusCode.FailedPrecondition),
            (ErrorCode.TooLarge, Grpc.Core.StatusCode.ResourceExhausted), (ErrorCode.Crashed, Grpc.Core.StatusCode.Internal),
            (ErrorCode.Failed, Grpc.Core.StatusCode.Aborted),
        })
        {
            var e = RemoteError.ToRpcException(code, "m — café", new JsonObject { ["limit"] = 5 });
            Assert.Equal(status, e.StatusCode);
            var back = RemoteError.TryRead(e)!;
            Assert.Equal(((int)code, "m — café", 5), (back.Code, back.Message, back.Data!["limit"]!.GetValue<int>()));
        }

        Assert.Null(RemoteError.TryRead(RemoteError.ToRpcException(ErrorCode.Failed, "x"))!.Data);
        Assert.Null(RemoteError.TryRead((Grpc.Core.Metadata?)null));
        Assert.Null(RemoteError.TryRead(new Grpc.Core.Metadata { { Contract.ErrorTrailer, "not json"u8.ToArray() } }));
        Assert.Null(RemoteError.TryRead(new Grpc.Core.Metadata { { Contract.ErrorTrailer, "[1]"u8.ToArray() } }));
        Assert.Null(RemoteError.TryRead(new Grpc.Core.Metadata { { "x-imt-error", "text" } }));
        Assert.Equal(ErrorCode.Invalid, PluginException.Invalid("x").Code);
        Assert.Equal(ErrorCode.TooLarge, PluginException.TooLarge("x").Code);
        Assert.Equal(ErrorCode.Failed, new PluginException().Code);
        Assert.Equal(ErrorCode.Failed, new PluginException("m", new IOException()).Code);
        Assert.Equal(ErrorCode.Failed, new PluginException("m").Code);
    }
}
