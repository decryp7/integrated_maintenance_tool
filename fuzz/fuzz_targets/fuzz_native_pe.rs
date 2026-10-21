#![no_main]
// PE images as the symbolisation step reads them (headers, sections, the CodeView PDB identity), and the cache
// path a PDB name from the image becomes.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(info) = imt_symbols::pe::parse(&mut &data[..]) {
        if let Some(pdb) = info.pdb {
            let _ = pdb.key();
            let _ = imt_symbols::config::cache_file(std::path::Path::new("cache"), &pdb);
        }
    }
});
