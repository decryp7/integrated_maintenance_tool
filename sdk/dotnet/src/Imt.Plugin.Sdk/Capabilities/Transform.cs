using System.Collections.Concurrent;

namespace Imt.Plugin;

/// <summary>Whether a transformed record stays in the workspace.</summary>
public enum Verdict
{
    /// <summary>Keep the record.</summary>
    Keep = 0,

    /// <summary>Drop the record.</summary>
    Drop = 1,
}

/// <summary>
/// One transform call: the file the records come from and a property bag that lives for the whole call
/// (every batch of one file), e.g. to pair start and end records across batches.
/// </summary>
public sealed class TransformContext
{
    /// <summary>Creates a context.</summary>
    /// <param name="sourceName">The display name of the file the records came from.</param>
    public TransformContext(string sourceName) => SourceName = sourceName;

    /// <summary>The display name of the file the records came from.</summary>
    public string SourceName { get; }

    /// <summary>State kept across the batches of this call.</summary>
    public ConcurrentDictionary<string, object> Items { get; } = new(StringComparer.Ordinal);
}

/// <summary>
/// A batch of records to transform in place: change fields, messages or levels, drop records, or mark them
/// with the plugin's own marker kinds. The row count and order stay the same.
/// </summary>
public sealed class TransformBatch
{
    private readonly Verdict[] verdicts;
    private readonly List<string>?[] marks;

    /// <summary>Creates a batch.</summary>
    /// <param name="records">The records.</param>
    public TransformBatch(IReadOnlyList<Record> records)
    {
        ArgumentNullException.ThrowIfNull(records);
        Records = records;
        verdicts = new Verdict[records.Count];
        marks = new List<string>?[records.Count];
    }

    /// <summary>The records, changed in place.</summary>
    public IReadOnlyList<Record> Records { get; }

    /// <summary>The verdict of each row (default <see cref="Verdict.Keep"/>).</summary>
    public IReadOnlyList<Verdict> Verdicts => verdicts;

    /// <summary>Drops row <paramref name="row"/>.</summary>
    /// <param name="row">The row index.</param>
    public void Drop(int row) => verdicts[row] = Verdict.Drop;

    /// <summary>Sets row <paramref name="row"/>'s verdict.</summary>
    /// <param name="row">The row index.</param>
    /// <param name="verdict">The verdict.</param>
    public void SetVerdict(int row, Verdict verdict) => verdicts[row] = verdict;

    /// <summary>
    /// Marks row <paramref name="row"/> with one of this plugin's marker kinds (short name, e.g.
    /// <c>slow-order</c>). Kinds of other plugins are refused by IMT.
    /// </summary>
    /// <param name="row">The row index.</param>
    /// <param name="kind">The marker kind.</param>
    public void Mark(int row, string kind)
    {
        var list = marks[row] ??= [];
        if (!list.Contains(kind, StringComparer.Ordinal))
        {
            list.Add(kind);
        }
    }

    /// <summary>The kinds row <paramref name="row"/> was marked with.</summary>
    /// <param name="row">The row index.</param>
    /// <returns>The kinds.</returns>
    public IReadOnlyList<string> MarksOf(int row) => marks[row] ?? (IReadOnlyList<string>)[];
}

/// <summary>A record transform, used as a <c>plugin</c> step of a profile's pipeline.</summary>
public interface ITransform
{
    /// <summary>Stable id (users reference it from their profile pipelines).</summary>
    string Id { get; }

    /// <summary>Transforms one batch in place.</summary>
    /// <param name="batch">The records, verdicts and marks.</param>
    /// <param name="context">The call (file name, state across batches).</param>
    /// <param name="cancellationToken">Cancelled when IMT's deadline passes.</param>
    /// <returns>A task.</returns>
    ValueTask ApplyAsync(TransformBatch batch, TransformContext context, CancellationToken cancellationToken);
}

/// <summary>A transform that handles one record at a time; implement <see cref="Apply"/>.</summary>
public abstract class RecordTransform : ITransform
{
    /// <inheritdoc/>
    public abstract string Id { get; }

    /// <summary>Transforms one record in place.</summary>
    /// <param name="record">The record.</param>
    /// <param name="mark">Marks the record with one of the plugin's marker kinds.</param>
    /// <param name="context">The call.</param>
    /// <returns>Whether to keep the record.</returns>
    public abstract Verdict Apply(Record record, Action<string> mark, TransformContext context);

    /// <inheritdoc/>
    public ValueTask ApplyAsync(TransformBatch batch, TransformContext context, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(batch);
        for (int i = 0; i < batch.Records.Count; i++)
        {
            if ((i & 1023) == 0)
            {
                cancellationToken.ThrowIfCancellationRequested();
            }

            int row = i;
            batch.SetVerdict(i, Apply(batch.Records[i], kind => batch.Mark(row, kind), context));
        }

        return ValueTask.CompletedTask;
    }
}
