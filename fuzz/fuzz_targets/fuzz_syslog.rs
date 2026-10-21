#![no_main]
#[path = "../common/mod.rs"]
mod common;
use libfuzzer_sys::fuzz_target;

// The first byte picks syslog or journald (JSON); the rest is the file.
fuzz_target!(|data: &[u8]| {
    let Some((&sel, rest)) = data.split_first() else { return };
    if sel % 2 == 0 {
        common::plugin_text(&imt_plugin_syslog::SyslogImporter, rest)
    } else {
        common::plugin_text(&imt_plugin_syslog::JournaldImporter, rest)
    }
});
