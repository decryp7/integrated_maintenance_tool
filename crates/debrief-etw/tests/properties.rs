//! "Never panics" properties over arbitrary input for every decoder and the converter.

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn the_converter_and_importer_never_panic(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
        debrief_etw::fuzz::etw_events(&data);
    }

    #[test]
    fn kernel_and_heap_decoders_never_panic(data in proptest::collection::vec(any::<u8>(), 0..256), opcode in any::<u8>()) {
        for ptr in [4usize, 8] {
            let _ = debrief_etw::kernel::sampled_profile(&data, ptr);
            let _ = debrief_etw::kernel::image_load(&data, ptr, 64);
            let _ = debrief_etw::kernel::process(&data, ptr, 4, 64);
            let _ = debrief_etw::kernel::stack_walk(&data, ptr);
            let _ = debrief_etw::heap::decode(opcode, &data, ptr);
        }
        let _ = debrief_etw::kernel::cswitch(&data);
        let _ = debrief_etw::kernel::thread_set_name(&data, 64);
    }
}
