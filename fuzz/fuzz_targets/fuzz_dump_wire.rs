#![no_main]
//! What IMT accepts from the dumps plugin's process: any bytes decode to a checked document or an error.
use imt_dump::Limits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(doc) = imt_dump::wire::decode_result(data, &Limits::default()) {
        if let imt_dump::model::Document::Dump(d) = &doc {
            let _ = imt_dump::findings::findings(1, d);
        }
    }
});
