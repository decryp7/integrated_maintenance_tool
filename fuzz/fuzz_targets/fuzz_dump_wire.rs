#![no_main]
//! What Debrief accepts from the dumps plugin's process: any bytes decode to a checked document or an error.
use debrief_dump::Limits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(doc) = debrief_dump::wire::decode_result(data, &Limits::default()) {
        if let debrief_dump::model::Document::Dump(d) = &doc {
            let _ = debrief_dump::findings::findings(1, d);
        }
    }
});
