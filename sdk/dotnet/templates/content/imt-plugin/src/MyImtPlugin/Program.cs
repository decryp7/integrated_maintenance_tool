using Imt.Plugin;
using MyImtPlugin;

// IMT starts this executable, tells it where to listen on standard input, and calls it over gRPC.
// It exits when IMT closes standard input. Logs go to standard error.
return await ImtPlugin.RunAsync(args, Plugin.Build());
