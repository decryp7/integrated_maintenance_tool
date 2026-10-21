#![no_main]
//! What a model writes about a screenshot (strict JSON, bounded) and the EXIF capture-time reader: any input ends
//! in a value within the bounds or a typed error, never a panic.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = imt_ai::vision::exif::capture_time(data);
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(f) = imt_ai::vision::ScreenFacts::parse(text) {
            assert!(f.ids.len() <= 16 && f.error_texts.len() <= 8 && f.timestamps.len() <= 8);
            let _ = f.clues();
        }
    }
});
