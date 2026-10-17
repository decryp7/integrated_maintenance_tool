using System.Text.Json.Nodes;
using Imt.Plugin;

namespace OrderAudit;

/// <summary>Running statistics of the orders the transform has seen (reset when IMT clears the workspace).</summary>
public sealed class OrderStats : IEventSubscriber
{
    private long started;
    private long completed;
    private long failed;
    private long slow;
    private long appended;

    /// <summary>Orders started.</summary>
    public long Started => Interlocked.Read(ref started);

    /// <summary>Orders completed.</summary>
    public long Completed => Interlocked.Read(ref completed);

    /// <summary>Orders failed.</summary>
    public long Failed => Interlocked.Read(ref failed);

    /// <summary>Orders slower than the threshold.</summary>
    public long Slow => Interlocked.Read(ref slow);

    /// <summary>Records IMT reported as appended (from <c>recordsAppended</c> events).</summary>
    public long Appended => Interlocked.Read(ref appended);

    /// <inheritdoc/>
    public IReadOnlyList<string> EventKinds { get; } = [Imt.Plugin.EventKinds.RecordsAppended, Imt.Plugin.EventKinds.WorkspaceCleared];

    /// <summary>Counts one event of an order.</summary>
    /// <param name="evt">The event name.</param>
    public void Count(string evt)
    {
        switch (evt)
        {
            case "OrderStarted":
                Interlocked.Increment(ref started);
                break;
            case "OrderCompleted":
                Interlocked.Increment(ref completed);
                break;
            case "OrderFailed":
                Interlocked.Increment(ref failed);
                break;
        }
    }

    /// <summary>Counts a slow order.</summary>
    public void CountSlow() => Interlocked.Increment(ref slow);

    /// <summary>The statistics as JSON.</summary>
    /// <returns>The object.</returns>
    public JsonObject ToJson() => new()
    {
        ["started"] = Started,
        ["completed"] = Completed,
        ["failed"] = Failed,
        ["slow"] = Slow,
        ["appended"] = Appended,
    };

    /// <inheritdoc/>
    public ValueTask OnEventAsync(PluginEvent e, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(e);
        if (e.Kind == Imt.Plugin.EventKinds.WorkspaceCleared)
        {
            Interlocked.Exchange(ref started, 0);
            Interlocked.Exchange(ref completed, 0);
            Interlocked.Exchange(ref failed, 0);
            Interlocked.Exchange(ref slow, 0);
            Interlocked.Exchange(ref appended, 0);
        }
        else if (e.Kind == Imt.Plugin.EventKinds.RecordsAppended && e.Payload["count"] is JsonValue c && c.TryGetValue(out long n))
        {
            Interlocked.Add(ref appended, n);
        }

        return ValueTask.CompletedTask;
    }
}

/// <summary>
/// The <c>com.example.orderaudit.stats</c> service other plugins can call: method <c>summary</c> returns the
/// statistics as UTF-8 JSON.
/// </summary>
public sealed class OrderStatsService : IPluginService
{
    private readonly OrderStats stats;

    /// <summary>Creates the service.</summary>
    /// <param name="stats">The statistics.</param>
    public OrderStatsService(OrderStats stats) => this.stats = stats;

    /// <inheritdoc/>
    public string ServiceId => "com.example.orderaudit.stats";

    /// <inheritdoc/>
    public string ServiceVersion => "1.0.0";

    /// <inheritdoc/>
    public ValueTask<ReadOnlyMemory<byte>> CallAsync(string method, ReadOnlyMemory<byte> payload, CancellationToken cancellationToken) => method switch
    {
        "summary" => ValueTask.FromResult<ReadOnlyMemory<byte>>(System.Text.Encoding.UTF8.GetBytes(stats.ToJson().ToJsonString())),
        _ => throw PluginException.Unimplemented($"the stats service has no method \"{method}\" (use \"summary\")"),
    };
}
