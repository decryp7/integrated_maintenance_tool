#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

// log4j / log4net / NLog XML events: sniff and import never panic.
fuzz_target!(|data: &[u8]| common::plugin_text(&imt_plugin_formats::Log4jXmlImporter, data));
