#![no_main]
//! The dumps plugin's minidump reader (ranges, full-dump compaction, stack walks), then the engine's findings,
//! views and wire round trip over what it read.
use debrief_dump::model::Document;
use debrief_dump::Limits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_threads: 64,
        max_frames: 64,
        ..Limits::default()
    };
    let bytes = data.to_vec();
    if let Ok(d) = debrief_plugin_dumps::dump_reader::read_dump(&bytes, &limits) {
        let _ = debrief_dump::findings::findings(1, &d);
        let _ = debrief_dump::views::overview(&d, debrief_dump::testing::summary());
        for t in &d.threads {
            let _ = debrief_dump::views::stack(&d, t);
        }
        let doc = Document::Dump(d);
        let wire = debrief_dump::wire::encode_result(&Ok(doc));
        assert!(debrief_dump::wire::decode_result(&wire, &limits).is_ok());
    }
});
