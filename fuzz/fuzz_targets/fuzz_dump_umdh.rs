#![no_main]
//! The dumps plugin's UMDH reader (snapshots and diffs, any encoding), then the engine's growth, tree and leak
//! findings over what it read.
use imt_dump::api::{UmdhDiffRequest, UmdhGroup};
use imt_dump::Limits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = imt_plugin_dumps::umdh_reader::sniff(data);
    let text = imt_plugin_dumps::umdh_reader::decode(data);
    if let Ok(u) = imt_plugin_dumps::umdh_reader::parse(&text, &Limits::default(), None) {
        let _ = imt_dump::umdh::findings(&[(1, &u)]);
        if let Ok(rows) = imt_dump::umdh::growth(None, &u) {
            for group in [UmdhGroup::Stack, UmdhGroup::Module, UmdhGroup::Function] {
                let req = UmdhDiffRequest {
                    base: None,
                    target: 1,
                    group,
                    offset: 0,
                    limit: 50,
                    filter: None,
                };
                let _ = imt_dump::umdh::diff_page(&rows, &req, 1000);
            }
            let _ = imt_dump::umdh::tree(&rows, &[], 50);
        }
    }
});
