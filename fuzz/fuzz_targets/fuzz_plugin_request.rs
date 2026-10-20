#![no_main]
//! What a plugin process decodes from the host: requests, the replies to its own host calls, and the
//! descriptive JSON. Never panics.
use imt_plugin_api::protocol::{HostRequest, PluginRequest};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = imt_ipc::wire::decode::<PluginRequest>(data);
    let _ = imt_ipc::wire::decode::<Vec<u8>>(data);
    let _ = imt_ipc::wire::decode::<Vec<imt_plugin_api::protocol::WireViewRecord>>(data);
    let _ = imt_ipc::wire::decode::<HostRequest>(data);
});
