#![no_main]
// .NET diagnostics IPC replies (ProcessInfo, CollectTracing, StopTracing).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| imt_plugin_dotnet::fuzz::ipc_reply(data));
