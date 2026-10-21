#![no_main]
//! What the host decodes from a plugin process: manifests, event items, the end of an import, tool failures,
//! transform output and profiles (validated on decode). Never panics.
use imt_plugin_api::protocol::{ImportDone, ProfileDone, TailDone, ToolFail, TransformOut, WireEvent};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(m) = serde_json::from_slice::<imt_plugin_api::Manifest>(data) {
        let _ = m.check();
    }
    if let Ok(rows) = imt_ipc::wire::decode::<Vec<WireEvent>>(data) {
        for r in rows {
            let _ = r.into_record();
        }
    }
    let _ = imt_ipc::wire::decode::<ImportDone>(data);
    let _ = imt_ipc::wire::decode::<TailDone>(data);
    let _ = imt_ipc::wire::decode::<ProfileDone>(data);
    let _ = imt_ipc::wire::decode::<ToolFail>(data);
    let _ = imt_ipc::wire::decode::<TransformOut>(data);
    let _ = imt_profile::wire::decode_result(data, &imt_profile::Limits::default());
});
