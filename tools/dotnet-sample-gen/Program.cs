// Writes invented log events through the real .NET logging libraries so IMT's parsers are tested against what
// the libraries actually produce (samples/real-libraries/). A developer tool: not part of IMT, not built by the
// app or CI. See README.md.
//
//   dotnet run --project tools/dotnet-sample-gen -- samples/real-libraries
//
// Serilog, log4net and NLog are given fixed instants, so their files only change when a library version (or,
// for log4net, the machine's time zone) changes. Microsoft.Extensions.Logging's console formatter takes the
// clock's time, so its file has the time of the run.

using System.Text;
using log4net.Appender;
using log4net.Core;
using log4net.Layout;
using log4net.Repository.Hierarchy;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Console;
using NLog.Targets;
using Serilog;
using Serilog.Events;
using Serilog.Formatting.Compact;
using Serilog.Formatting.Json;

var outDir = Path.GetFullPath(args.Length > 0 ? args[0] : Path.Combine("samples", "real-libraries"));
Directory.CreateDirectory(outDir);
foreach (var f in Directory.GetFiles(outDir).Where(f => !f.EndsWith("README.md", StringComparison.Ordinal)))
{
    File.Delete(f);
}

var events = Scenario.Events();
SerilogOutput.Write(outDir, events);
Log4NetOutput.Write(outDir, events);
NLogOutput.Write(outDir, events);
MelOutput.Write(outDir, events);
Console.WriteLine($"wrote {Directory.GetFiles(outDir).Length - 1} files to {outDir}");

/// <summary>One invented event: when, how severe, a message template with its values, and an optional exception.</summary>
record Ev(DateTimeOffset At, string Level, string Logger, int Thread, string Template, object?[] Values, Exception? Error = null);

static class Scenario
{
    /// <summary>The events every library logs: Tokyo (+09:00) wall-clock times on 14 Oct 2026, 19:30-19:42 (10:30-10:42 UTC).</summary>
    public static List<Ev> Events()
    {
        var jst = TimeSpan.FromHours(9);
        DateTimeOffset T(int m, int s, int ms) => new(2026, 10, 14, 19, m, s, ms, jst);
        return
        [
            new(T(30, 0, 120), "Information", "Contoso.Orders.Program", 1, "OrderService {Version} starting on {Machine}", ["3.14.2", "TKY-WEB03"]),
            new(T(30, 1, 5), "Debug", "Contoso.Orders.Cache", 4, "Tariff cache refreshed {@Stats}", [new CacheStats(412, 0.93)]),
            new(T(31, 12, 480), "Information", "Contoso.Orders.Intake", 7, "注文 {OrderNo} を受け付けました（顧客 {CustomerId}、合計 {Total} 円）", [88412, "C-20931", 12800]),
            new(T(35, 40, 2), "Warning", "Contoso.Orders.Payments", 7, "Slow response from {Endpoint}: {ElapsedMs} ms", ["/v2/captures", 4870]),
            new(T(41, 24, 50), "Error", "Contoso.Orders.Payments", 7, "Payment capture failed for order {OrderNo}", [88412], Failures.Capture()),
            new(T(41, 24, 61), "Warning", "Contoso.Orders.Payments", 7, "決済を再試行します（{Attempt}/{MaxAttempts}）", [2, 3]),
            new(T(42, 0, 999), "Fatal", "Contoso.Orders.Program", 1, "Unhandled exception, the service stops", [], Failures.Shutdown()),
        ];
    }
}

record CacheStats(int Entries, double HitRatio);

/// <summary>Real exceptions with real stack traces and inner exceptions (thrown, then caught).</summary>
static class Failures
{
    public static Exception Capture()
    {
        try
        {
            PaymentGateway.Capture(88412);
        }
        catch (Exception e)
        {
            return e;
        }
        throw new UnreachableException();
    }

    public static Exception Shutdown()
    {
        try
        {
            try
            {
                PaymentGateway.Capture(88413);
            }
            catch (Exception e)
            {
                throw new AggregateException("One or more background tasks failed.", e, new IOException("ディスクに空き容量がありません"));
            }
        }
        catch (Exception e)
        {
            return e;
        }
        throw new UnreachableException();
    }
}

static class PaymentGateway
{
    public static void Capture(int orderNo)
    {
        try
        {
            Send(orderNo);
        }
        catch (TimeoutException e)
        {
            throw new InvalidOperationException($"Payment capture failed for order {orderNo}", e);
        }
    }

    static void Send(int orderNo) => throw new TimeoutException($"決済ゲートウェイがタイムアウトしました (order {orderNo}, 30000 ms)");
}

class UnreachableException : Exception;

static class SerilogOutput
{
    public static void Write(string dir, List<Ev> events)
    {
        using var logger = new LoggerConfiguration()
            .MinimumLevel.Verbose()
            .Enrich.WithProperty("MachineName", "TKY-WEB03")
            .Enrich.WithProperty("ProcessId", 4120)
            .WriteTo.File(new CompactJsonFormatter(), Path.Combine(dir, "serilog-compact.clef"))
            .WriteTo.File(new JsonFormatter(renderMessage: true), Path.Combine(dir, "serilog-json.json"))
            .WriteTo.File(Path.Combine(dir, "serilog-text.txt"), encoding: new UTF8Encoding(false))
            .CreateLogger();
        foreach (var e in events)
        {
            var log = logger.ForContext("SourceContext", e.Logger);
            if (!log.BindMessageTemplate(e.Template, e.Values, out var template, out var props))
            {
                throw new InvalidOperationException($"template {e.Template}");
            }
            var level = Enum.Parse<LogEventLevel>(e.Level);
            log.Write(new LogEvent(e.At, level, e.Error, template, props));
        }
    }
}

static class Log4NetOutput
{
    public static void Write(string dir, List<Ev> events)
    {
        var repo = (Hierarchy)log4net.LogManager.CreateRepository("imt-samples");
        Add(repo, Path.Combine(dir, "log4net-pattern.log"), new PatternLayout("%date [%thread] %-5level %logger - %message%newline%exception"));
        // A CSV-like layout without a header: date, time with offset, level, event code, process, pid, message.
        Add(repo, Path.Combine(dir, "log4net-csv.log"),
            new PatternLayout("%date{yyyy/MM/dd},%date{HH:mm:ss.fffzzz},%level,%property{EventCode},%property{ProcessName},%property{Pid},%message%newline"));
        Add(repo, Path.Combine(dir, "log4net-log4j.xml"), new XmlLayoutSchemaLog4J { LocationInfo = false });
        repo.Root.Level = Level.All;
        repo.Configured = true;

        var code = 1000;
        foreach (var e in events)
        {
            var props = new log4net.Util.PropertiesDictionary();
            props["EventCode"] = (code++).ToString();
            props["ProcessName"] = "OrderService";
            props["Pid"] = "4120";
            var data = new LoggingEventData
            {
                LoggerName = e.Logger,
                Level = e.Level switch
                {
                    "Debug" => Level.Debug,
                    "Information" => Level.Info,
                    "Warning" => Level.Warn,
                    "Error" => Level.Error,
                    _ => Level.Fatal,
                },
                Message = Render(e),
                ThreadName = e.Thread.ToString(),
                TimeStampUtc = e.At.UtcDateTime,
                ExceptionString = e.Error?.ToString(),
                Identity = "",
                UserName = "CONTOSO\\svc-orders",
                Domain = "OrderService.exe",
                Properties = props,
            };
            repo.Log(new LoggingEvent(data));
        }
        repo.Shutdown();
    }

    static void Add(Hierarchy repo, string file, LayoutSkeleton layout)
    {
        layout.ActivateOptions();
        var appender = new FileAppender { File = file, AppendToFile = false, Layout = layout, Encoding = new UTF8Encoding(false) };
        appender.ActivateOptions();
        repo.Root.AddAppender(appender);
    }

    /// <summary>log4net has no message templates: the message is the template with its values filled in.</summary>
    public static string Render(Ev e)
    {
        var text = new StringBuilder();
        var i = 0;
        foreach (var token in new Serilog.Parsing.MessageTemplateParser().Parse(e.Template).Tokens)
        {
            text.Append(token is Serilog.Parsing.PropertyToken ? e.Values[i++]?.ToString() : token.ToString());
        }
        return text.ToString();
    }
}
static class NLogOutput
{
    public static void Write(string dir, List<Ev> events)
    {
        var config = new NLog.Config.LoggingConfiguration();
        // The FileTarget's default layout: ${longdate}|${level:uppercase=true}|${logger}|${message:withexception=true}.
        var file = new FileTarget("file") { FileName = Path.Combine(dir, "nlog-default.log"), Encoding = new UTF8Encoding(false), KeepFileOpen = false };
        config.AddRuleForAllLevels(file);
        var factory = new NLog.LogFactory { Configuration = config };
        foreach (var e in events)
        {
            var info = new NLog.LogEventInfo(
                e.Level switch
                {
                    "Debug" => NLog.LogLevel.Debug,
                    "Information" => NLog.LogLevel.Info,
                    "Warning" => NLog.LogLevel.Warn,
                    "Error" => NLog.LogLevel.Error,
                    _ => NLog.LogLevel.Fatal,
                },
                e.Logger,
                null,
                e.Template,
                e.Values)
            {
                // NLog's longdate is the event's wall-clock time without an offset: Tokyo time here.
                TimeStamp = e.At.DateTime,
                Exception = e.Error,
            };
            factory.GetLogger(e.Logger).Log(info);
        }
        factory.Shutdown();
    }
}

static class MelOutput
{
    public static void Write(string dir, List<Ev> events)
    {
        var path = Path.Combine(dir, "mel-console.log");
        var original = Console.Out;
        using (var writer = new StreamWriter(path, false, new UTF8Encoding(false)) { AutoFlush = true, NewLine = "\n" })
        {
            Console.SetOut(writer);
            using (var factory = LoggerFactory.Create(b => b
                       .SetMinimumLevel(LogLevel.Trace)
                       .AddSimpleConsole(o =>
                       {
                           o.TimestampFormat = "yyyy-MM-dd HH:mm:ss.fffzzz ";
                           o.ColorBehavior = LoggerColorBehavior.Disabled;
                           o.IncludeScopes = true;
                       })))
            {
                var id = 1000;
                foreach (var e in events)
                {
                    var logger = factory.CreateLogger(e.Logger);
                    var level = e.Level switch
                    {
                        "Debug" => LogLevel.Debug,
                        "Information" => LogLevel.Information,
                        "Warning" => LogLevel.Warning,
                        "Error" => LogLevel.Error,
                        _ => LogLevel.Critical,
                    };
                    using (logger.BeginScope("OrderNo:{OrderNo}", 88412))
                    {
#pragma warning disable CA2254 // the templates are the scenario's
                        logger.Log(level, new EventId(id++), e.Error, e.Template, e.Values);
#pragma warning restore CA2254
                    }
                }
            }
            Console.SetOut(original);
        }
    }
}
