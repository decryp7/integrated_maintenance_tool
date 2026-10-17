using Imt.Plugin;

namespace OrderAudit;

/// <summary>The plugin's settings, shared by its contributions (IMT applies them at start and on change).</summary>
public sealed class OrderAuditSettings : ISettingsSection
{
    private long slowOrderMs = 2000;
    private volatile bool dropHeartbeats = true;
    private volatile string defaultCurrency = "EUR";

    /// <summary>An order that takes longer is marked <c>slow-order</c> and its span is a warning.</summary>
    public long SlowOrderMs => Interlocked.Read(ref slowOrderMs);

    /// <summary>Heartbeat records are dropped from the workspace.</summary>
    public bool DropHeartbeats => dropHeartbeats;

    /// <summary>The currency of amounts that do not name one.</summary>
    public string DefaultCurrency => defaultCurrency;

    /// <inheritdoc/>
    public SettingsSection Section { get; } = new(
        "Order audit",
        [
            new SettingField("slowOrderMs", "Slow order threshold", new DurationSetting(100, 3_600_000), 2000)
            {
                Description = "Orders that take longer from start to completion are marked as slow.",
            },
            new SettingField("dropHeartbeats", "Hide heartbeats", new BoolSetting(), true)
            {
                Description = "Drop the service's Heartbeat records while importing.",
            },
            new SettingField("defaultCurrency", "Default currency", new TextSetting(3, "[A-Z]{3}"), "EUR")
            {
                Description = "ISO 4217 code used for amounts without a currency.",
                Advanced = true,
            },
        ])
    {
        Description = "How the order audit plugin enriches and marks orders.",
    };

    /// <inheritdoc/>
    public ValueTask ApplyAsync(SettingsValues values, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(values);
        Interlocked.Exchange(ref slowOrderMs, values.GetInt64("slowOrderMs"));
        dropHeartbeats = values.GetBool("dropHeartbeats");
        defaultCurrency = values.GetString("defaultCurrency");
        return ValueTask.CompletedTask;
    }
}
