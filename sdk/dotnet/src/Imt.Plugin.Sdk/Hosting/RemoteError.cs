using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using Grpc.Core;

namespace Imt.Plugin.Hosting;

/// <summary>
/// The typed error of a failed call, carried as JSON <c>{code, message, data?}</c> in the binary trailer
/// <c>x-imt-error-bin</c> (the Rust <c>imt_ipc::RemoteError</c>).
/// </summary>
/// <param name="Code">The error code (see <see cref="ErrorCode"/>).</param>
/// <param name="Message">The message.</param>
/// <param name="Data">Optional details.</param>
public sealed record RemoteError(int Code, string Message, JsonNode? Data = null)
{
    /// <summary>The gRPC status code a typed error is sent with (the Rust SDK's mapping).</summary>
    /// <param name="code">The typed error code.</param>
    /// <returns>The gRPC status code.</returns>
    public static StatusCode StatusFor(ErrorCode code) => code switch
    {
        ErrorCode.Invalid or ErrorCode.NoHandshake => StatusCode.InvalidArgument,
        ErrorCode.Unimplemented => StatusCode.Unimplemented,
        ErrorCode.Version => StatusCode.FailedPrecondition,
        ErrorCode.TooLarge => StatusCode.ResourceExhausted,
        ErrorCode.Crashed => StatusCode.Internal,
        _ => StatusCode.Aborted,
    };

    /// <summary>The error as an <see cref="RpcException"/> with the typed trailer.</summary>
    /// <param name="code">The typed error code.</param>
    /// <param name="message">The message.</param>
    /// <param name="data">Optional details.</param>
    /// <returns>The exception to throw from a gRPC handler.</returns>
    public static RpcException ToRpcException(ErrorCode code, string message, JsonNode? data = null)
    {
        var json = new JsonObject { ["code"] = (int)code, ["message"] = message };
        if (data is not null)
        {
            json["data"] = data.DeepClone();
        }

        var trailers = new Metadata { { Contract.ErrorTrailer, Encoding.UTF8.GetBytes(json.ToJsonString()) } };
        return new RpcException(new Status(StatusFor(code), message), trailers);
    }

    /// <summary>Reads the typed error of a failed call, if it has one.</summary>
    /// <param name="trailers">The call's trailers.</param>
    /// <returns>The error, or null.</returns>
    public static RemoteError? TryRead(Metadata? trailers)
    {
        var entry = trailers?.Get(Contract.ErrorTrailer);
        if (entry is null || !entry.IsBinary)
        {
            return null;
        }

        try
        {
            if (JsonNode.Parse(entry.ValueBytes) is not JsonObject o
                || o["code"] is not JsonValue c || !c.TryGetValue(out int code)
                || o["message"] is not JsonValue m || !m.TryGetValue(out string? message))
            {
                return null;
            }

            return new RemoteError(code, message, o["data"]?.DeepClone());
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>Reads the typed error of a failed call, if it has one.</summary>
    /// <param name="e">The exception of the failed call.</param>
    /// <returns>The error, or null.</returns>
    public static RemoteError? TryRead(RpcException e) => TryRead(e?.Trailers);
}
