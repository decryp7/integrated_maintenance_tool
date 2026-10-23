#![no_main]
//! The AI's image sanitiser (ADR 0025): any bytes, with or without a valid signature, decode into a bounded clean
//! copy or a typed error. Never panics, never allocates past its limits, never keeps more than the budget.
use debrief_ai::vision::image::{sanitize, ImageLimits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Small limits so the fuzzer explores the limit paths too.
    let limits = ImageLimits {
        max_file_bytes: 1 << 20,
        max_pixels: 1 << 22,
        max_side: 4096,
        max_alloc: 64 << 20,
    };
    if let Ok(img) = sanitize(data, 1 << 16, &limits) {
        let i = &img.image;
        assert!(u64::from(i.width()) * u64::from(i.height()) <= 1 << 16);
        assert!(!i.bytes().is_empty());
    }
});
