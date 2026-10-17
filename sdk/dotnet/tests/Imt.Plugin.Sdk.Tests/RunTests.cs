using System.IO.Pipelines;
using System.Text;
using Imt.Plugin.Hosting;
using Imt.Plugin.Testing;
using KitchenSink;

namespace Imt.Plugin.Tests;

/// <summary>The process entry point: handshake line, lifeline, shutdown, arguments.</summary>
public class RunTests
{
    [Fact]
    public void Handshake_lines_are_parsed_strictly()
    {
        var info = LaunchInfo.Read(new MemoryStream("{\"address\":\"/tmp/a.sock\",\"token\":\"t0k\"}\nrest"u8.ToArray()));
        Assert.Equal(("/tmp/a.sock", "t0k", false), (info.Address, info.Token, info.IsNamedPipe));
        Assert.Equal("/tmp/a.sock", info.PipeName);
        var pipe = LaunchInfo.Parse("{\"address\":\"\\\\\\\\.\\\\pipe\\\\imt-plugin-ab\",\"token\":\"t\"}");
        Assert.True(pipe.IsNamedPipe);
        Assert.Equal("imt-plugin-ab", pipe.PipeName);
        foreach (var bad in new[] { "nope", "[]", "{\"address\":1,\"token\":\"t\"}", "{\"address\":\"a\"}", "{\"address\":\"\",\"token\":\"t\"}", "{\"address\":\"a\",\"token\":\"\"}", "{\"address\":\"a\",\"token\":\"bad token\"}" })
        {
            Assert.Throws<InvalidDataException>(() => LaunchInfo.Parse(bad));
        }

        Assert.Throws<InvalidDataException>(() => LaunchInfo.Read(new MemoryStream()));
        var longLine = Encoding.UTF8.GetBytes(new string('a', 70_000) + "\n");
        Assert.Contains("64 KiB", Assert.Throws<InvalidDataException>(() => LaunchInfo.Read(new MemoryStream(longLine))).Message, StringComparison.Ordinal);
    }

    [Fact]
    public async Task The_lifeline_fires_when_input_closes()
    {
        var pipe = new Pipe();
        var fired = new TaskCompletionSource();
        var thread = Lifeline.Watch(pipe.Reader.AsStream(), () => fired.SetResult());
        Assert.True(thread.IsBackground);
        await pipe.Writer.WriteAsync("keep alive"u8.ToArray());
        await Task.Delay(50);
        Assert.False(fired.Task.IsCompleted);
        await pipe.Writer.CompleteAsync();
        await fired.Task.WaitAsync(TimeSpan.FromSeconds(10));

        var failing = new TaskCompletionSource();
        var broken = new FailingStream();
        Lifeline.Watch(broken, () => failing.SetResult());
        await failing.Task.WaitAsync(TimeSpan.FromSeconds(10));
    }

    private sealed class FailingStream : MemoryStream
    {
        public override int Read(byte[] buffer, int offset, int count) => throw new IOException("gone");
    }

    [Fact]
    public async Task Run_serves_until_the_lifeline_closes()
    {
        var (launch, dir) = PluginTestHost.NewAddress();
        var stdin = new Pipe();
        var errors = new StringWriter();
        var run = Task.Run(() => ImtPlugin.RunAsync([], Kitchen.Build(), new ImtPluginOptions { Input = stdin.Reader.AsStream(), Error = errors, KeepConsoleOut = true }));
        await stdin.Writer.WriteAsync(Encoding.UTF8.GetBytes($"{{\"address\":\"{launch.Address}\",\"token\":\"{launch.Token}\"}}\n"));
        using var channel = PluginTestHost.Connect(launch.Address);
        var control = new Imt.Ipc.V1.Control.ControlClient(channel);
        var headers = new Grpc.Core.Metadata { { Contract.TokenHeader, launch.Token } };
        Imt.Ipc.V1.StatusReply? status = null;
        for (int i = 0; i < 200 && status is null; i++)
        {
            try
            {
                status = await control.StatusAsync(new Imt.Ipc.V1.StatusRequest(), headers);
            }
            catch (Grpc.Core.RpcException)
            {
                await Task.Delay(25);
            }
        }

        Assert.NotNull(status);
        Assert.False(run.IsCompleted);
        await stdin.Writer.CompleteAsync();
        Assert.Equal(0, await run.WaitAsync(TimeSpan.FromSeconds(20)));
        Directory.Delete(dir!, true);
    }

    [Fact]
    public async Task Run_stops_on_control_shutdown()
    {
        var (launch, dir) = PluginTestHost.NewAddress();
        var stdin = new Pipe();
        var run = Task.Run(() => ImtPlugin.RunAsync([], Kitchen.Build(), new ImtPluginOptions { Input = stdin.Reader.AsStream(), Error = new StringWriter(), KeepConsoleOut = true }));
        await stdin.Writer.WriteAsync(Encoding.UTF8.GetBytes($"{{\"address\":\"{launch.Address}\",\"token\":\"{launch.Token}\"}}\n"));
        using var channel = PluginTestHost.Connect(launch.Address);
        var control = new Imt.Ipc.V1.Control.ControlClient(channel);
        var headers = new Grpc.Core.Metadata { { Contract.TokenHeader, launch.Token } };
        bool done = false;
        for (int i = 0; i < 200 && !done; i++)
        {
            try
            {
                await control.ShutdownAsync(new Imt.Ipc.V1.ShutdownRequest(), headers);
                done = true;
            }
            catch (Grpc.Core.RpcException)
            {
                await Task.Delay(25);
            }
        }

        Assert.True(done);
        Assert.Equal(0, await run.WaitAsync(TimeSpan.FromSeconds(20)));
        await stdin.Writer.CompleteAsync();
        Directory.Delete(dir!, true);
    }

    [Fact]
    public async Task Bad_starts_are_reported_on_stderr()
    {
        var errors = new StringWriter();
        Assert.Equal(1, await ImtPlugin.RunAsync([], Kitchen.Build(), new ImtPluginOptions { Input = new MemoryStream("garbage\n"u8.ToArray()), Error = errors, KeepConsoleOut = true }));
        Assert.Contains("This program is an IMT plugin", errors.ToString(), StringComparison.Ordinal);

        errors = new StringWriter();
        var line = "{\"address\":\"/nonexistent/dir/w.sock\",\"token\":\"t\"}\n"u8.ToArray();
        Assert.Equal(1, await ImtPlugin.RunAsync([], Kitchen.Build(), new ImtPluginOptions { Input = new MemoryStream(line), Error = errors, KeepConsoleOut = true }));
        Assert.Contains("cannot listen", errors.ToString(), StringComparison.Ordinal);

        errors = new StringWriter();
        Assert.Equal(2, await ImtPlugin.RunAsync(["--verbose"], Kitchen.Build(), new ImtPluginOptions { Error = errors, KeepConsoleOut = true }));
        Assert.Contains("unknown option", errors.ToString(), StringComparison.Ordinal);

        if (!OperatingSystem.IsWindows())
        {
            errors = new StringWriter();
            var pipe = "{\"address\":\"\\\\\\\\.\\\\pipe\\\\x\",\"token\":\"t\"}\n"u8.ToArray();
            Assert.Equal(1, await ImtPlugin.RunAsync([], Kitchen.Build(), new ImtPluginOptions { Input = new MemoryStream(pipe), Error = errors, KeepConsoleOut = true }));
            Assert.Contains("only used on Windows", errors.ToString(), StringComparison.Ordinal);
        }
    }

    [Fact]
    public async Task Print_manifest_writes_the_plugin_json()
    {
        var output = new StringWriter();
        Assert.Equal(0, await ImtPlugin.RunAsync(["--print-manifest"], Kitchen.Build(), new ImtPluginOptions { ManifestOutput = output }));
        Assert.Contains("\"id\": \"test.kitchen\"", output.ToString(), StringComparison.Ordinal);
    }
}
