#![no_main]
// ETW events (live sessions and .etl files) → the NetTrace converter, the .etl process scanner and the importer.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| imt_plugin_dotnet::fuzz::etw_events(data));
