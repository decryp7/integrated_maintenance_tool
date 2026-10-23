//! "Never panics" properties for the decoders that read untrusted input: ETW event payloads
//! (live sessions and `.etl` files), diagnostics IPC replies, and the NetTrace they produce.

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn etw_events_never_panic(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        debrief_plugin_dotnet::fuzz::etw_events(&data);
    }

    #[test]
    fn ipc_replies_never_panic(data in proptest::collection::vec(any::<u8>(), 0..512)) {
        debrief_plugin_dotnet::fuzz::ipc_reply(&data);
    }

    #[test]
    fn ipc_replies_with_a_valid_header_never_panic(set in any::<u8>(), id in any::<u8>(), payload in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut m = b"DOTNET_IPC_V1\0".to_vec();
        m.extend_from_slice(&((20 + payload.len()) as u16).to_le_bytes());
        m.extend_from_slice(&[set, id, 0, 0]);
        m.extend_from_slice(&payload);
        debrief_plugin_dotnet::fuzz::ipc_reply(&m);
    }
}
