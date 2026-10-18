//! Decoding arbitrary bytes never panics.

use imt_clr_format::{protocol, AttachConfig, Command, Event, HeapRecord, ProfileRecord, Records};
use proptest::prelude::*;

proptest! {
    #[test]
    fn records_and_messages(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        for magic in [imt_clr_format::record::PROFILE_MAGIC, imt_clr_format::record::HEAP_MAGIC] {
            let mut file = imt_clr_format::record::header(magic);
            file.extend_from_slice(&bytes);
            if let Ok(records) = Records::open(&file, magic) {
                for (tag, payload) in records.flatten() {
                    let _ = ProfileRecord::decode(tag, payload);
                    let _ = HeapRecord::decode(tag, payload);
                }
            }
        }
        let _ = AttachConfig::decode(&bytes);
        let _ = Event::decode(&bytes);
        let _ = Command::decode(&bytes);
        if bytes.len() >= 5 {
            let _ = protocol::data_frame_header(&[bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]]);
        }
    }

    #[test]
    fn heap_objects_round_trip(addr in any::<u64>(), refs in proptest::collection::vec(any::<u64>(), 0..32)) {
        let r = HeapRecord::Object { addr, type_id: 3, size: 24, refs };
        let bytes = r.encode();
        let len = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
        prop_assert_eq!(HeapRecord::decode(bytes[0], &bytes[5..5 + len]).unwrap(), Some(r));
    }
}
