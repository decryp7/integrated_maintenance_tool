#![no_main]
//! The frame decoder every IMT connection runs on untrusted bytes: the length prefix with its limit, the
//! postcard envelope and the generic JSON newtype. Never panics; what decodes re-encodes to the same frame.
use imt_ipc::frame::{self, Frame};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let max = 1 << 20;
    let mut rest = data;
    while let Ok(Some((f, used))) = frame::decode(rest, max) {
        assert!(used > 0 && used <= rest.len());
        let again = frame::encode(&f, max).expect("a decoded frame re-encodes");
        assert_eq!(frame::decode(&again, max).ok().flatten().map(|(g, _)| g), Some(f));
        rest = &rest[used..];
    }
    let _: Result<Frame, _> = frame::decode_body(data);
    let _ = imt_ipc::wire::decode::<imt_ipc::Json>(data);
    let _ = imt_ipc::wire::decode::<imt_ipc::RemoteError>(data);
});
