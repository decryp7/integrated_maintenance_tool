#![no_main]
// ETW events of a native capture (kernel samples, stack walks, context switches, image loads, heap allocations,
// CLR events) with fuzzed payloads through the converter and the .etl scanner; the NetTrace written (with
// symbols for the collected addresses) goes through the importer.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| imt_etw::fuzz::etw_events(data));
