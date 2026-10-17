using System.Text.Json.Nodes;

namespace Imt.Plugin;

/// <summary>Marker kinds the plugin contributes to the timeline (style and default rules).</summary>
public interface IMarkerKinds
{
    /// <summary>The kinds.</summary>
    IReadOnlyList<MarkerKind> MarkerKinds { get; }
}

/// <summary>A record an annotator receives: the record plus the file and instance it belongs to.</summary>
/// <param name="Record">The record.</param>
/// <param name="SourceName">The display name of its file.</param>
/// <param name="Instance">Its instance key.</param>
public sealed record AnnotatedRecord(Record Record, string SourceName, string Instance);

/// <summary>The first record that is new since the previous annotator run.</summary>
/// <param name="Id">Its id.</param>
/// <param name="Timestamp">Its time (ms since the epoch), if any.</param>
public sealed record RecordCursor(uint Id, long? Timestamp);

/// <summary>What an annotator run receives.</summary>
public sealed class AnnotationInput
{
    /// <summary>Creates an input.</summary>
    /// <param name="records">The records matching the annotator's interest, oldest first.</param>
    public AnnotationInput(IReadOnlyList<AnnotatedRecord> records) => Records = records;

    /// <summary>The records matching the annotator's <see cref="IAnnotator.Interest"/>, oldest first.</summary>
    public IReadOnlyList<AnnotatedRecord> Records { get; }

    /// <summary>The first record that is new since the previous run, or null for a full run.</summary>
    public RecordCursor? Since { get; init; }

    /// <summary>Oldest and newest record time of the workspace, or null when unknown.</summary>
    public (long From, long To)? TimeRange { get; init; }

    /// <summary>Every instance key of the workspace, in first-seen order.</summary>
    public IReadOnlyList<string> Instances { get; init; } = [];
}

/// <summary>
/// An annotator: finds ranges and points across records (e.g. an order from start to end). It runs after
/// every import and live-tail append; each run returns its complete set, which replaces the previous one.
/// </summary>
public interface IAnnotator
{
    /// <summary>Stable id.</summary>
    string Id { get; }

    /// <summary>Which records IMT sends (all must match; empty: every record).</summary>
    IReadOnlyList<Condition> Interest => [];

    /// <summary>At most this many records, newest last (0: IMT's limit of 100,000).</summary>
    int Limit => 0;

    /// <summary>Finds the annotations.</summary>
    /// <param name="input">The records and workspace facts.</param>
    /// <param name="cancellationToken">Cancelled when IMT's deadline passes.</param>
    /// <returns>The complete set of annotations.</returns>
    ValueTask<IReadOnlyList<Annotation>> AnnotateAsync(AnnotationInput input, CancellationToken cancellationToken);
}

/// <summary>A row to export: the record, its file and its group.</summary>
/// <param name="Record">The record.</param>
/// <param name="SourceName">The display name of its file.</param>
/// <param name="Group">Its group (the view's grouping), or empty.</param>
public sealed record ExportRow(Record Record, string SourceName, string Group);

/// <summary>An exporter: writes the rows of the current view to a file of its format.</summary>
public interface IExporter
{
    /// <summary>Stable id.</summary>
    string Id { get; }

    /// <summary>The name shown in the export menu.</summary>
    string Name { get; }

    /// <summary>The file extension, without the dot (1–16 letters or digits).</summary>
    string Extension { get; }

    /// <summary>
    /// Writes <paramref name="rows"/> to <paramref name="output"/>. Prefer the stream's async methods: output is
    /// streamed to IMT in 1 MiB chunks as it is written.
    /// </summary>
    /// <param name="rows">The rows, streamed from IMT as they arrive.</param>
    /// <param name="output">The output file's content.</param>
    /// <param name="cancellationToken">Cancelled when IMT stops the export.</param>
    /// <returns>The number of rows written.</returns>
    ValueTask<long> ExportAsync(IAsyncEnumerable<ExportRow> rows, Stream output, CancellationToken cancellationToken);
}

/// <summary>A settings section: IMT renders, validates and stores the values and applies them at start and on change.</summary>
public interface ISettingsSection
{
    /// <summary>The section's schema.</summary>
    SettingsSection Section { get; }

    /// <summary>Applies values that passed validation (the SDK validates again before calling this).</summary>
    /// <param name="values">The values, one per field.</param>
    /// <param name="cancellationToken">Cancelled when IMT's deadline passes.</param>
    /// <returns>A task.</returns>
    ValueTask ApplyAsync(SettingsValues values, CancellationToken cancellationToken);
}

/// <summary>
/// A service other plugins call (ADR 0003): opaque payloads typed by the service's API, identified by id and
/// semver version.
/// </summary>
public interface IPluginService
{
    /// <summary>The service id, e.g. <c>com.example.orders.stats</c>.</summary>
    string ServiceId { get; }

    /// <summary>The service's semver version.</summary>
    string ServiceVersion { get; }

    /// <summary>Handles one call.</summary>
    /// <param name="method">The method name.</param>
    /// <param name="payload">The request payload.</param>
    /// <param name="cancellationToken">Cancelled when IMT's deadline passes.</param>
    /// <returns>The reply payload.</returns>
    ValueTask<ReadOnlyMemory<byte>> CallAsync(string method, ReadOnlyMemory<byte> payload, CancellationToken cancellationToken);
}

/// <summary>An event from IMT's event bus.</summary>
/// <param name="Kind">The kind, e.g. <see cref="EventKinds.RecordsAppended"/>.</param>
/// <param name="Payload">The event as JSON (an empty object when it has none).</param>
public sealed record PluginEvent(string Kind, JsonNode Payload);

/// <summary>Event kinds IMT delivers.</summary>
public static class EventKinds
{
    /// <summary>Records were imported or appended: <c>{count, sources}</c>.</summary>
    public const string RecordsAppended = "recordsAppended";

    /// <summary>The active profile changed: <c>{name}</c>.</summary>
    public const string ProfileChanged = "profileChanged";

    /// <summary>The workspace was cleared: <c>{}</c>.</summary>
    public const string WorkspaceCleared = "workspaceCleared";
}

/// <summary>Receives events from IMT's event bus.</summary>
public interface IEventSubscriber
{
    /// <summary>The kinds to receive (see <see cref="EventKinds"/>).</summary>
    IReadOnlyList<string> EventKinds { get; }

    /// <summary>Handles one event. Keep it short: a slow subscriber loses events.</summary>
    /// <param name="e">The event.</param>
    /// <param name="cancellationToken">Cancelled when IMT's deadline passes.</param>
    /// <returns>A task.</returns>
    ValueTask OnEventAsync(PluginEvent e, CancellationToken cancellationToken);
}
