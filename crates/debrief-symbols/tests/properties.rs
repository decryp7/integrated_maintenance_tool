//! "Never panics" properties: the PE parser, the search-path parser, PDB identities and the PDB (MSF) identity
//! reader over arbitrary input.

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, ..ProptestConfig::default() })]

    #[test]
    fn pe_parsing_never_panics(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = debrief_symbols::pe::parse(&mut data.as_slice());
    }

    #[test]
    fn mutated_images_never_panic(at in 0usize..0x400, byte in any::<u8>(), plus in any::<bool>()) {
        let mut img = debrief_symbols::pe::synthetic(plus, Some((&[7u8; 16], 1, "busy.pdb")));
        img[at] = byte;
        if let Ok(info) = debrief_symbols::pe::parse(&mut img.as_slice()) {
            if let Some(pdb) = info.pdb {
                let _ = debrief_symbols::config::cache_file(std::path::Path::new("c"), &pdb);
                let _ = pdb.key();
            }
        }
    }

    #[test]
    fn search_paths_never_panic(text in ".{0,300}") {
        let _ = debrief_symbols::parse_search_path(&text);
    }

    #[test]
    fn pdb_identities_never_panic(file in ".{0,300}", id in "[0-9a-fA-F{}-]{0,48}|.{0,60}") {
        if let Ok(k) = debrief_symbols::id::PdbKey::parse(&file, &id) {
            // A parsed key is always one safe component.
            let path = k.store_path();
            prop_assert_eq!(path.split('/').count(), 3);
            prop_assert!(path.split('/').all(|c| !c.is_empty() && c != "." && c != ".." && !c.contains(['\\', ':'])));
        }
    }

    #[test]
    fn mutated_pdbs_never_panic(at in 0usize..(7 * 512), byte in any::<u8>()) {
        let mut pdb = debrief_symbols::msf::synthetic(&[3u8; 16], 2);
        pdb[at] = byte;
        let _ = debrief_symbols::msf::identity(&mut pdb.as_slice());
    }
}
