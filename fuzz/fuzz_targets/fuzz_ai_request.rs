#![no_main]
//! What the AI worker decodes from IMT (and IMT from the worker). Never panics.
use imt_ai::worker::frames::WorkerRequest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = imt_ipc::wire::decode::<WorkerRequest>(data);
    let _ = imt_ipc::wire::decode::<imt_ipc::Json>(data);
});
