#![no_main]
//! The dumps plugin's UMDH reader (snapshots and diffs, any encoding), then the engine's growth, tree and leak
//! findings over what it read.
use debrief_dump::api::{UmdhDiffRequest, UmdhGroup};
use debrief_dump::Limits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = debrief_plugin_dumps::umdh_reader::sniff(data);
    let text = debrief_plugin_dumps::umdh_reader::decode(data);
    if let Ok(u) = debrief_plugin_dumps::umdh_reader::parse(&text, &Limits::default(), None) {
        let _ = debrief_dump::umdh::findings(&[(1, &u)]);
        if let Ok(rows) = debrief_dump::umdh::growth(None, &u) {
            for group in [UmdhGroup::Stack, UmdhGroup::Module, UmdhGroup::Function] {
                let req = UmdhDiffRequest {
                    base: None,
                    target: 1,
                    group,
                    offset: 0,
                    limit: 50,
                    filter: None,
                };
                let _ = debrief_dump::umdh::diff_page(&rows, &req, 1000);
            }
            let _ = debrief_dump::umdh::tree(&rows, &[], 50);
        }
    }
});
