#![no_main]
//! What the AI worker decodes from Debrief (and Debrief from the worker). Never panics.
use debrief_ai::worker::frames::WorkerRequest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = debrief_ipc::wire::decode::<WorkerRequest>(data);
    let _ = debrief_ipc::wire::decode::<debrief_ipc::Json>(data);
});
