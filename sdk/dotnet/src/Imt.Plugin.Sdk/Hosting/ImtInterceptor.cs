using System.Security.Cryptography;
using System.Text;
using Grpc.Core;
using Grpc.Core.Interceptors;
using Microsoft.Extensions.Logging;

namespace Imt.Plugin.Hosting;

/// <summary>
/// Every call, every service (including <c>imt.ipc.v1.Control</c>): refuses calls without the per-launch
/// token (<c>UNAUTHENTICATED</c>, compared in constant time like <c>imt_ipc::auth::RequireToken</c>), and turns
/// exceptions of plugin code into typed errors: <see cref="PluginException"/> keeps its code; a cancelled call
/// is <c>DEADLINE_EXCEEDED</c> or <c>CANCELLED</c>; anything else is <see cref="ErrorCode.Crashed"/>.
/// </summary>
public sealed class ImtInterceptor : Interceptor
{
    private readonly byte[] token;
    private readonly ILogger logger;

    /// <summary>Creates the interceptor.</summary>
    /// <param name="token">The expected token.</param>
    /// <param name="logger">Where failures are logged (standard error).</param>
    public ImtInterceptor(string token, ILogger<ImtInterceptor> logger)
    {
        ArgumentNullException.ThrowIfNull(token);
        this.token = Encoding.UTF8.GetBytes(token);
        this.logger = logger;
    }

    /// <summary>Whether <paramref name="presented"/> is the expected token (constant time for equal lengths).</summary>
    /// <param name="presented">The token a call presented, or null.</param>
    /// <returns>Whether it matches.</returns>
    public bool Accepts(string? presented)
    {
        if (presented is null)
        {
            return false;
        }

        var p = Encoding.UTF8.GetBytes(presented);
        return p.Length == token.Length && CryptographicOperations.FixedTimeEquals(p, token);
    }

    /// <inheritdoc/>
    public override async Task<TResponse> UnaryServerHandler<TRequest, TResponse>(
        TRequest request, ServerCallContext context, UnaryServerMethod<TRequest, TResponse> continuation)
    {
        ArgumentNullException.ThrowIfNull(continuation);
        Authorize(context);
        try
        {
            return await continuation(request, context).ConfigureAwait(false);
        }
        catch (Exception e) when (e is not RpcException)
        {
            throw Map(e, context);
        }
    }

    /// <inheritdoc/>
    public override async Task<TResponse> ClientStreamingServerHandler<TRequest, TResponse>(
        IAsyncStreamReader<TRequest> requestStream, ServerCallContext context, ClientStreamingServerMethod<TRequest, TResponse> continuation)
    {
        ArgumentNullException.ThrowIfNull(continuation);
        Authorize(context);
        try
        {
            return await continuation(requestStream, context).ConfigureAwait(false);
        }
        catch (Exception e) when (e is not RpcException)
        {
            throw Map(e, context);
        }
    }

    /// <inheritdoc/>
    public override async Task ServerStreamingServerHandler<TRequest, TResponse>(
        TRequest request, IServerStreamWriter<TResponse> responseStream, ServerCallContext context, ServerStreamingServerMethod<TRequest, TResponse> continuation)
    {
        ArgumentNullException.ThrowIfNull(continuation);
        Authorize(context);
        try
        {
            await continuation(request, responseStream, context).ConfigureAwait(false);
        }
        catch (Exception e) when (e is not RpcException)
        {
            throw Map(e, context);
        }
    }

    /// <inheritdoc/>
    public override async Task DuplexStreamingServerHandler<TRequest, TResponse>(
        IAsyncStreamReader<TRequest> requestStream, IServerStreamWriter<TResponse> responseStream, ServerCallContext context, DuplexStreamingServerMethod<TRequest, TResponse> continuation)
    {
        ArgumentNullException.ThrowIfNull(continuation);
        Authorize(context);
        try
        {
            await continuation(requestStream, responseStream, context).ConfigureAwait(false);
        }
        catch (Exception e) when (e is not RpcException)
        {
            throw Map(e, context);
        }
    }

    private void Authorize(ServerCallContext context)
    {
        if (!Accepts(context.RequestHeaders.GetValue(Contract.TokenHeader)))
        {
            throw new RpcException(new Status(StatusCode.Unauthenticated, "missing or wrong session token"));
        }
    }

    /// <summary>The typed error for an exception of plugin code.</summary>
    /// <param name="e">The exception.</param>
    /// <param name="context">The call.</param>
    /// <returns>The status to fail the call with.</returns>
    internal RpcException Map(Exception e, ServerCallContext context)
    {
        switch (e)
        {
            case PluginException p:
                return RemoteError.ToRpcException(p.Code, p.Message, p.Details);
            case OperationCanceledException when context.CancellationToken.IsCancellationRequested:
                return context.Deadline <= DateTime.UtcNow
                    ? new RpcException(new Status(StatusCode.DeadlineExceeded, "the call's deadline passed"))
                    : new RpcException(new Status(StatusCode.Cancelled, "the call was cancelled"));
            default:
                LogCrash(logger, context.Method, e);
                return RemoteError.ToRpcException(ErrorCode.Crashed, $"{e.GetType().Name}: {e.Message}");
        }
    }

    private static readonly Action<ILogger, string, Exception?> LogCrash =
        LoggerMessage.Define<string>(LogLevel.Error, new EventId(1, "PluginCrashed"), "{Method} failed with an unhandled exception");
}
