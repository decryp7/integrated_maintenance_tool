using Imt.Plugin;

namespace ToyFormat;

/// <summary>The toy plugin's settings: how long a message may be.</summary>
public sealed class ToySettings : ISettingsSection
{
    private volatile int maxMessageLength = 10_000;

    /// <summary>Longest message kept (longer ones are cut).</summary>
    public int MaxMessageLength => maxMessageLength;

    /// <inheritdoc/>
    public SettingsSection Section { get; } = new(
        "Toy format",
        [
            new SettingField("maxMessageLength", "Longest message", new IntSetting(100, 1_000_000, 1, "characters"), 10_000)
            {
                Description = "Longer messages are cut to this many characters.",
            },
        ])
    {
        Description = "How the toy format importer reads files.",
    };

    /// <inheritdoc/>
    public ValueTask ApplyAsync(SettingsValues values, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(values);
        maxMessageLength = (int)values.GetInt64("maxMessageLength");
        return ValueTask.CompletedTask;
    }
}
