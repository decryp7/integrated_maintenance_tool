//! "Never panics" properties of the readers: arbitrary bytes, damaged sample dumps and arbitrary UMDH-like text
//! give a document within the limits or an error, and the engine's analyses of any document read stay quiet.

use imt_dump::model::Document;
use imt_dump::{ImportOptions, Limits};
use imt_plugin_dumps::{dump_reader, synth, umdh_reader};
use proptest::prelude::*;

fn small() -> Limits {
    Limits {
        max_threads: 8,
        max_frames: 16,
        max_modules: 16,
        max_stacks: 64,
        max_umdh_frames: 8,
        ..Limits::default()
    }
}

fn analyse(doc: &Document, limits: &Limits) {
    imt_dump::wire::check(doc, limits).unwrap();
    let bytes = imt_dump::wire::encode_result(&Ok(doc.clone()));
    assert_eq!(&imt_dump::wire::decode_result(&bytes, limits).unwrap(), doc);
    match doc {
        Document::Dump(d) => {
            let _ = imt_dump::findings::findings(1, d);
            let _ = imt_dump::explain::dump_prompt("x.dmp", d, &[]);
        }
        Document::Umdh(u) => {
            let _ = imt_dump::umdh::findings(&[(1, u)]);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_bytes_after_the_signature_never_panic(tail in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let mut bytes = b"MDMP".to_vec();
        bytes.extend(tail);
        let l = small();
        if let Ok(d) = dump_reader::read_dump(&bytes, &l) {
            analyse(&Document::Dump(d), &l);
        }
    }

    #[test]
    fn damaged_sample_dumps_never_panic(
        which in 0usize..3,
        edits in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..24),
        cut in proptest::option::of(any::<usize>()),
    ) {
        let mut bytes = match which {
            0 => synth::sample_crash(),
            1 => synth::sample_full(512),
            _ => synth::sample_hang_x86(),
        };
        for (at, v) in edits {
            let i = at % bytes.len();
            bytes[i] = v;
        }
        if let Some(c) = cut {
            bytes.truncate(c % (bytes.len() + 1));
        }
        let l = small();
        if let Ok(d) = dump_reader::read_dump(&bytes, &l) {
            prop_assert!(d.threads.len() <= l.max_threads && d.modules.len() <= l.max_modules);
            analyse(&Document::Dump(d), &l);
        }
    }

    #[test]
    fn arbitrary_umdh_like_text_never_panics(lines in proptest::collection::vec(
        prop_oneof![
            "[0-9A-F]{1,9} bytes \\+ [0-9A-F]{1,9} at [0-9A-F]{1,17} by BackTrace[0-9A-F]{0,9}",
            "[+-] +[0-9A-F]{1,9} \\( +[0-9A-F]{0,9} - +[0-9A-F]{0,9}\\) +[0-9A-F]{1,5} allocs\tBackTrace[0-9A-F]{1,5}",
            "[+-] +[0-9A-F]{1,9} \\( +[0-9A-F]{0,9} - +[0-9A-F]{0,9}\\)\tBackTrace[0-9A-F]{1,5}\tallocations",
            "\t[A-Za-z.]{0,8}![A-Za-z:]{0,12}\\+[0-9A-Fx]{0,10}( \\([a-z:\\\\.]{0,10}, ?[0-9]{0,6}\\))?",
            "\t[0-9A-F]{1,17}",
            "// Loaded modules:",
            "//  +[0-9A-F]{1,17} +[0-9A-F]{1,9} [A-Za-z:\\\\.]{0,20}",
            "Total increase == [0-9A-F]{1,8} requested \\+ +[0-9A-F]{1,8} overhead = +[0-9A-F]{1,8}",
            ".{0,40}",
        ],
        0..60,
    ), crlf in any::<bool>()) {
        let text = lines.join(if crlf { "\r\n" } else { "\n" });
        let l = small();
        let _ = umdh_reader::sniff(text.as_bytes());
        if let Ok(u) = umdh_reader::parse(&text, &l, None) {
            prop_assert!(u.stacks.len() <= l.max_stacks);
            analyse(&Document::Umdh(u), &l);
        }
    }

    #[test]
    fn arbitrary_bytes_as_umdh_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = umdh_reader::sniff(&bytes);
        let opts = ImportOptions { limits: small(), file_time_ms: None };
        let _ = imt_plugin_dumps::import::import_umdh(&bytes, &opts, &[]);
    }

    #[test]
    fn arbitrary_wire_bytes_never_panic_the_engine(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        if let Ok(doc) = imt_dump::wire::decode_result(&bytes, &small()) {
            analyse(&doc, &small());
        }
    }
}
