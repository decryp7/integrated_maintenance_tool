//! "Never panics" properties: the PE parser and the search-path parser over arbitrary input.

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

    #[test]
    fn pe_parsing_never_panics(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = imt_symbols::pe::parse(&mut data.as_slice());
    }

    #[test]
    fn mutated_images_never_panic(at in 0usize..0x400, byte in any::<u8>(), plus in any::<bool>()) {
        let mut img = imt_symbols::pe::synthetic(plus, Some((&[7u8; 16], 1, "busy.pdb")));
        img[at] = byte;
        if let Ok(info) = imt_symbols::pe::parse(&mut img.as_slice()) {
            if let Some(pdb) = info.pdb {
                let _ = imt_symbols::config::cache_file(std::path::Path::new("c"), &pdb);
                let _ = pdb.key();
            }
        }
    }

    #[test]
    fn search_paths_never_panic(text in ".{0,300}") {
        let _ = imt_symbols::parse_search_path(&text);
    }
}
