//! Property-based tests of the frame codec every Debrief connection runs on untrusted bytes: decoding never panics,
//! respects the length limit, and every frame round-trips through the wire format. `fuzz/fuzz_ipc_frame` runs
//! the same decoder under libFuzzer.
//!
//! Run more cases with `PROPTEST_CASES=10000 cargo test -p debrief-ipc --test it properties::`.

use debrief_ipc::frame::{self, Frame, FrameError, ProtocolVersion, LEN_BYTES, TOKEN_BYTES};
use debrief_ipc::{wire, Json, RemoteError};
use proptest::prelude::*;
use serde_json::json;

fn protocol() -> impl Strategy<Value = ProtocolVersion> {
    (any::<u16>(), any::<u16>()).prop_map(|(a, b)| ProtocolVersion::new(a, b))
}

fn body() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..512)
}

fn remote_error() -> impl Strategy<Value = RemoteError> {
    (
        any::<i32>(),
        ".{0,40}",
        prop::option::of(prop_oneof![
            Just(json!(null)),
            Just(json!({"reason": "no_runtime"})),
            ".{0,20}".prop_map(|s| json!(s)),
            any::<i64>().prop_map(|n| json!(n))
        ]),
    )
        .prop_map(|(code, message, data)| RemoteError {
            code,
            message,
            data: data.map(Json),
        })
}

fn any_frame() -> impl Strategy<Value = Frame> {
    prop_oneof![
        (any::<u16>(), protocol(), any::<[u8; TOKEN_BYTES]>(), any::<u32>()).prop_map(|(version, protocol, token, pid)| Frame::Hello {
            version,
            protocol,
            token,
            pid
        }),
        (any::<u16>(), protocol()).prop_map(|(version, protocol)| Frame::Welcome { version, protocol }),
        (any::<u64>(), any::<u32>(), body()).prop_map(|(id, credit, body)| Frame::Call { id, credit, body }),
        (any::<u64>(), body()).prop_map(|(id, body)| Frame::Reply { id, body }),
        (any::<u64>(), body()).prop_map(|(id, body)| Frame::Item { id, body }),
        (any::<u64>(), remote_error()).prop_map(|(id, error)| Frame::Fail { id, error }),
        any::<u64>().prop_map(|id| Frame::Cancel { id }),
        (any::<u64>(), any::<u32>()).prop_map(|(id, n)| Frame::Credit { id, n }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Any byte stream splits into whole frames or stops with an error; what decodes re-encodes to the same bytes.
    #[test]
    fn decode_never_panics(data in prop::collection::vec(any::<u8>(), 0..2048), max in 0usize..(1 << 22)) {
        let mut rest = &data[..];
        while let Ok(Some((f, used))) = frame::decode(rest, max) {
            prop_assert!(used > LEN_BYTES - 1 && used <= rest.len());
            prop_assert!(used - LEN_BYTES <= frame::clamp_max(max));
            let again = frame::encode(&f, max).expect("a decoded frame re-encodes");
            // Equal as frames (not bytes: JSON text inside a `Fail` may be spelled differently).
            prop_assert_eq!(frame::decode(&again, max).ok().flatten().map(|(g, _)| g), Some(f));
            rest = &rest[used..];
        }
        let _ = frame::decode_body(&data);
        let _ = frame::handshake_versions(&data);
        let _ = wire::decode::<Json>(&data);
        let _ = wire::decode::<RemoteError>(&data);
    }

    #[test]
    fn frames_round_trip(f in any_frame(), max in (1usize << 10)..(1 << 20)) {
        let bytes = frame::encode(&f, max).unwrap();
        prop_assert_eq!(u32::from_le_bytes(bytes[..LEN_BYTES].try_into().unwrap()) as usize, bytes.len() - LEN_BYTES);
        let (back, used) = frame::decode(&bytes, max).unwrap().unwrap();
        prop_assert_eq!(used, bytes.len());
        prop_assert_eq!(&back, &f);
        prop_assert_eq!(back.call_id().is_none(), matches!(f, Frame::Hello { .. } | Frame::Welcome { .. }));
    }

    /// A stream cut anywhere before the end of a frame asks for more bytes instead of failing or guessing.
    #[test]
    fn truncated_frames_need_more(f in any_frame(), cut in any::<prop::sample::Index>()) {
        let bytes = frame::encode(&f, 1 << 20).unwrap();
        let at = cut.index(bytes.len());
        prop_assert_eq!(frame::decode(&bytes[..at], 1 << 20), Ok(None));
    }

    /// Trailing bytes inside a frame's announced body are an error: one body is exactly one frame.
    #[test]
    fn trailing_bytes_are_refused(f in any_frame(), extra in prop::collection::vec(any::<u8>(), 1..16)) {
        let bytes = frame::encode(&f, 1 << 20).unwrap();
        let mut body = bytes[LEN_BYTES..].to_vec();
        body.extend_from_slice(&extra);
        prop_assert!(matches!(frame::decode_body(&body), Err(FrameError::Invalid(_))));
    }

    /// The announced length is checked before anything is read: a frame above the limit is refused by its prefix.
    #[test]
    fn oversized_prefix_is_refused(len in (1u32 << 20)..u32::MAX) {
        let max = 1 << 16;
        let mut buf = len.to_le_bytes().to_vec();
        buf.extend_from_slice(&[0; 8]);
        let refused = matches!(frame::decode(&buf, max), Err(FrameError::TooLarge { .. }));
        prop_assert!(refused);
    }

    #[test]
    fn hello_versions_read_without_the_rest(version in 2u16.., protocol in protocol(), token in any::<[u8; TOKEN_BYTES]>(), pid in any::<u32>()) {
        let bytes = frame::encode(&Frame::Hello { version, protocol, token, pid }, 1 << 20).unwrap();
        prop_assert_eq!(frame::handshake_versions(&bytes[LEN_BYTES..]), Some((version, Some(protocol))));
    }

    #[test]
    fn protocol_version_parse_never_panics(s in ".{0,20}") {
        if let Ok(v) = s.parse::<ProtocolVersion>() {
            prop_assert_eq!(v.to_string().parse::<ProtocolVersion>(), Ok(v));
        }
    }

    #[test]
    fn protocol_acceptance_is_major_equal_minor_not_newer(host in protocol(), child in protocol()) {
        prop_assert_eq!(host.accepts(child), host.major == child.major && child.minor <= host.minor);
        prop_assert_eq!(host.mismatch(child, "x").is_none(), host.accepts(child));
    }

    /// JSON travels as text on postcard and arrives as the same value.
    #[test]
    fn json_round_trips(s in ".{0,40}", n in any::<i64>(), b in any::<bool>()) {
        let v = Json(json!({"s": s, "n": n, "b": b, "a": [1, null, {"x": s}]}));
        let bytes = wire::encode(&v).unwrap();
        prop_assert_eq!(wire::decode::<Json>(&bytes).unwrap(), v);
    }
}

#[test]
fn clamp_max_bounds() {
    assert_eq!(frame::clamp_max(0), 1024);
    assert_eq!(frame::clamp_max(usize::MAX), frame::HARD_MAX_FRAME);
    assert_eq!(frame::clamp_max(4096), 4096);
}

#[test]
fn encode_refuses_frames_over_the_limit() {
    let f = Frame::Reply {
        id: 1,
        body: vec![0; 4096],
    };
    assert!(matches!(frame::encode(&f, 1024), Err(FrameError::TooLarge { max: 1024, .. })));
}
