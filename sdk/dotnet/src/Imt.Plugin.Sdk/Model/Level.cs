namespace Imt.Plugin;

/// <summary>A record's severity. The numeric values are the <c>level</c> column of the Arrow record schema.</summary>
public enum Level : byte
{
    /// <summary>Most detailed tracing.</summary>
    Verbose = 0,

    /// <summary>Debugging detail.</summary>
    Debug = 1,

    /// <summary>Normal operation (the default).</summary>
    Information = 2,

    /// <summary>Something unexpected that the application handled.</summary>
    Warning = 3,

    /// <summary>An operation failed.</summary>
    Error = 4,

    /// <summary>The application cannot continue.</summary>
    Fatal = 5,
}

/// <summary>Parsing level names as they appear in logs.</summary>
public static class Levels
{
    /// <summary>
    /// Parses common level spellings (<c>VRB</c>, <c>TRACE</c>, <c>DBG</c>, <c>INF</c>, <c>INFO</c>, <c>WRN</c>,
    /// <c>WARN</c>, <c>ERR</c>, <c>ERROR</c>, <c>FTL</c>, <c>FATAL</c>, <c>CRITICAL</c>, any case).
    /// </summary>
    /// <param name="text">The level text.</param>
    /// <param name="level">The parsed level.</param>
    /// <returns>Whether the text names a level.</returns>
    public static bool TryParse(ReadOnlySpan<char> text, out Level level)
    {
        Span<char> upper = stackalloc char[12];
        text = text.Trim();
        if (text.Length is 0 or > 12)
        {
            level = Level.Information;
            return false;
        }

        int n = text.ToUpperInvariant(upper);
        level = upper[..n] switch
        {
            "V" or "VRB" or "VERBOSE" or "TRACE" or "TRC" => Level.Verbose,
            "D" or "DBG" or "DEBUG" => Level.Debug,
            "I" or "INF" or "INFO" or "INFORMATION" => Level.Information,
            "W" or "WRN" or "WARN" or "WARNING" => Level.Warning,
            "E" or "ERR" or "ERROR" or "FAIL" => Level.Error,
            "F" or "FTL" or "FATAL" or "CRIT" or "CRITICAL" => Level.Fatal,
            _ => (Level)255,
        };
        if ((byte)level > (byte)Level.Fatal)
        {
            level = Level.Information;
            return false;
        }

        return true;
    }
}
