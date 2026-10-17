namespace Imt.Plugin;

/// <summary>
/// Codes of the typed errors a plugin returns in the <c>x-imt-error-bin</c> trailer (the Rust SDK's
/// <c>imt_plugin_sdk::code</c>).
/// </summary>
public enum ErrorCode
{
    /// <summary>The plugin's code reported an error; the plugin itself is fine (gRPC <c>ABORTED</c>).</summary>
    Failed = -32000,

    /// <summary>The request is malformed: unknown id, bad JSON, wrong message order (gRPC <c>INVALID_ARGUMENT</c>).</summary>
    Invalid = -32602,

    /// <summary>The plugin does not implement this call (gRPC <c>UNIMPLEMENTED</c>).</summary>
    Unimplemented = -32601,

    /// <summary>The plugin's code failed unexpectedly (an unhandled exception); IMT counts it like a crash (gRPC <c>INTERNAL</c>).</summary>
    Crashed = -32010,

    /// <summary>The contract versions are incompatible (gRPC <c>FAILED_PRECONDITION</c>).</summary>
    Version = -32030,

    /// <summary>A call arrived before <c>Handshake</c> (gRPC <c>INVALID_ARGUMENT</c>).</summary>
    NoHandshake = -32031,

    /// <summary>A limit was exceeded: content too large, too many rows (gRPC <c>RESOURCE_EXHAUSTED</c>).</summary>
    TooLarge = -32040,
}
