namespace Imt.Plugin.Hosting;

/// <summary>
/// The lifeline: IMT keeps the plugin's standard input open while it wants the plugin; when IMT exits or
/// closes it (even after a crash of IMT), reading returns end-of-file and the plugin stops. Reading blocks,
/// so it runs on a dedicated background thread, never on the thread pool.
/// </summary>
public static class Lifeline
{
    /// <summary>Calls <paramref name="onClosed"/> once <paramref name="input"/> reaches end-of-file or fails.</summary>
    /// <param name="input">Standard input (after the handshake line).</param>
    /// <param name="onClosed">Called once, on the lifeline thread.</param>
    /// <returns>The thread (already started).</returns>
    public static Thread Watch(Stream input, Action onClosed)
    {
        ArgumentNullException.ThrowIfNull(input);
        ArgumentNullException.ThrowIfNull(onClosed);
        var thread = new Thread(() =>
        {
            var sink = new byte[256];
            try
            {
                while (input.Read(sink, 0, sink.Length) > 0)
                {
                }
            }
            catch (IOException)
            {
            }
            catch (ObjectDisposedException)
            {
            }

            onClosed();
        })
        {
            IsBackground = true,
            Name = "imt-plugin-lifeline",
        };
        thread.Start();
        return thread;
    }
}
