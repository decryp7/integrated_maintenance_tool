using System.Text;
using System.Text.Json;

namespace Imt.Plugin;

/// <summary>What an importer learns about the file it reads.</summary>
public sealed class ImportContext
{
    private readonly Lazy<JsonDocument?> profile;
    private long skipped;

    /// <summary>Creates a context (the SDK does this for each call; tests may too).</summary>
    /// <param name="fileName">The file's display name (no directory).</param>
    /// <param name="path">The full path, only when the user granted <see cref="PermissionKind.ReadOpenedFiles"/>.</param>
    /// <param name="fileDate">The file's last modification date, for logs that only carry a time of day.</param>
    /// <param name="profileJson">The active parsing profile as JSON (without its pipeline), or empty.</param>
    public ImportContext(string fileName, string? path = null, DateOnly? fileDate = null, string? profileJson = null)
    {
        FileName = fileName;
        Path = string.IsNullOrEmpty(path) ? null : path;
        FileDate = fileDate;
        ProfileJson = profileJson ?? string.Empty;
        profile = new Lazy<JsonDocument?>(ParseProfile);
    }

    /// <summary>The file's display name (no directory).</summary>
    public string FileName { get; }

    /// <summary>The full path; null unless the user granted <see cref="PermissionKind.ReadOpenedFiles"/>.</summary>
    public string? Path { get; }

    /// <summary>The file's last modification date, or null when unknown.</summary>
    public DateOnly? FileDate { get; }

    /// <summary>The active parsing profile as JSON (schema of IMT's <c>default_profile.toml</c>), or empty for the default.</summary>
    public string ProfileJson { get; }

    /// <summary>The parsed profile, or null when it is empty or not valid JSON.</summary>
    public JsonDocument? Profile => profile.Value;

    /// <summary>Lines or entries the importer skipped so far (reported to IMT in <c>ImportDone</c>).</summary>
    public long Skipped => Interlocked.Read(ref skipped);

    /// <summary>Counts lines or entries the importer could not read.</summary>
    /// <param name="count">How many.</param>
    public void Skip(long count = 1) => Interlocked.Add(ref skipped, count);

    private JsonDocument? ParseProfile()
    {
        if (ProfileJson.Length == 0)
        {
            return null;
        }

        try
        {
            return JsonDocument.Parse(ProfileJson);
        }
        catch (JsonException)
        {
            return null;
        }
    }
}

/// <summary>The content of a file to import.</summary>
public sealed class ImportInput
{
    private string? text;

    /// <summary>Creates an input.</summary>
    /// <param name="bytes">The content (for text importers: UTF-8 text IMT decoded).</param>
    public ImportInput(ReadOnlyMemory<byte> bytes) => Bytes = bytes;

    /// <summary>Creates an input from text.</summary>
    /// <param name="text">The content.</param>
    public ImportInput(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        this.text = text;
        Bytes = Encoding.UTF8.GetBytes(text);
    }

    /// <summary>The raw bytes.</summary>
    public ReadOnlyMemory<byte> Bytes { get; }

    /// <summary>The content as text (invalid UTF-8 replaced by U+FFFD), decoded once.</summary>
    public string Text => text ??= Encoding.UTF8.GetString(Bytes.Span);
}

/// <summary>
/// An importer: recognises a file format and turns a file into records. Text importers receive text IMT
/// decoded; set <see cref="Binary"/> for raw bytes.
/// </summary>
public interface IImporter
{
    /// <summary>Stable id, unique within the plugin (users select it in profiles: <c>parse.format</c>).</summary>
    string Id { get; }

    /// <summary>Receives raw bytes instead of decoded text (binary importers cannot live-tail).</summary>
    bool Binary => false;

    /// <summary>A record may span several lines (live tail then re-reads from the start of the last record).</summary>
    bool Multiline => false;

    /// <summary>How confident the importer is that it can read the file: 0 (not at all) to 100 (certain).</summary>
    /// <param name="context">The file.</param>
    /// <param name="head">The first bytes of the file (up to 64 KiB).</param>
    /// <param name="cancellationToken">Cancelled when IMT's deadline passes.</param>
    /// <returns>The score.</returns>
    ValueTask<int> SniffAsync(ImportContext context, ReadOnlyMemory<byte> head, CancellationToken cancellationToken);

    /// <summary>
    /// Reads the file. Yield records as they are parsed: the SDK batches them into Arrow record batches
    /// and streams them to IMT while parsing continues. Count unreadable lines with <see cref="ImportContext.Skip"/>.
    /// Throw <see cref="PluginException"/> when the file cannot be read at all.
    /// </summary>
    /// <param name="context">The file.</param>
    /// <param name="input">The content.</param>
    /// <param name="cancellationToken">Cancelled when IMT's deadline passes or IMT gives up.</param>
    /// <returns>The records, in file order.</returns>
    IAsyncEnumerable<Record> ImportAsync(ImportContext context, ImportInput input, CancellationToken cancellationToken);
}

/// <summary>An importer that can parse text appended to a file it already read (live tail).</summary>
public interface ITailImporter : IImporter
{
    /// <summary>
    /// Parses <paramref name="tail"/>, text appended to a file whose start is <paramref name="head"/> (for
    /// headers and detected settings). For a <see cref="IImporter.Multiline"/> importer the tail starts at the
    /// last record. Line numbers are relative to the tail. Return null when the tail cannot be appended
    /// (IMT then re-imports the whole file).
    /// </summary>
    /// <param name="context">The file.</param>
    /// <param name="head">The start of the file.</param>
    /// <param name="tail">The appended text.</param>
    /// <param name="cancellationToken">Cancelled when IMT's deadline passes.</param>
    /// <returns>The new records, or null.</returns>
    IAsyncEnumerable<Record>? ImportTail(ImportContext context, string head, string tail, CancellationToken cancellationToken);
}
