using System.Text.Json.Nodes;

namespace Imt.Plugin;

/// <summary>
/// An error a plugin reports on purpose: the input cannot be read, a value is out of range, a request is
/// wrong. It travels to IMT as a typed error and does <b>not</b> count towards the plugin's quarantine.
/// Any other exception escaping plugin code is treated as a bug (<see cref="ErrorCode.Crashed"/>).
/// </summary>
public class PluginException : Exception
{
    /// <summary>Creates an error with <see cref="ErrorCode.Failed"/>.</summary>
    public PluginException()
        : this(ErrorCode.Failed, "the plugin failed")
    {
    }

    /// <summary>Creates an error with <see cref="ErrorCode.Failed"/>.</summary>
    /// <param name="message">What went wrong, shown to the user.</param>
    public PluginException(string message)
        : this(ErrorCode.Failed, message)
    {
    }

    /// <summary>Creates an error with <see cref="ErrorCode.Failed"/> and its cause.</summary>
    /// <param name="message">What went wrong, shown to the user.</param>
    /// <param name="innerException">The cause.</param>
    public PluginException(string message, Exception innerException)
        : base(message, innerException)
    {
        Code = ErrorCode.Failed;
    }

    /// <summary>Creates an error with a specific code.</summary>
    /// <param name="code">The typed error code.</param>
    /// <param name="message">What went wrong, shown to the user.</param>
    /// <param name="data">Optional structured details (sent as the error's <c>data</c>).</param>
    public PluginException(ErrorCode code, string message, JsonNode? data = null)
        : base(message)
    {
        Code = code;
        Details = data;
    }

    /// <summary>The typed error code.</summary>
    public ErrorCode Code { get; }

    /// <summary>Optional structured details, sent as the typed error's <c>data</c>.</summary>
    public JsonNode? Details { get; }

    /// <summary>An <see cref="ErrorCode.Invalid"/> error.</summary>
    /// <param name="message">What is wrong with the request.</param>
    /// <returns>The exception to throw.</returns>
    public static PluginException Invalid(string message) => new(ErrorCode.Invalid, message);

    /// <summary>An <see cref="ErrorCode.TooLarge"/> error.</summary>
    /// <param name="message">Which limit was exceeded.</param>
    /// <returns>The exception to throw.</returns>
    public static PluginException TooLarge(string message) => new(ErrorCode.TooLarge, message);

    /// <summary>An <see cref="ErrorCode.Unimplemented"/> error.</summary>
    /// <param name="message">What is not implemented.</param>
    /// <returns>The exception to throw.</returns>
    public static PluginException Unimplemented(string message) => new(ErrorCode.Unimplemented, message);
}
