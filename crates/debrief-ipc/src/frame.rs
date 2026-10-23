//! The one frame codec of Debrief's processes (ADR 0017 §1): a 4-byte little-endian length, then a postcard-encoded
//! [`Frame`]. Every connection (plugins, the AI worker, the store worker) carries these frames; a frame's `body`
//! is the protocol's own postcard-encoded message.
//!
//! Decoding is bounded and never panics: the length is checked against the connection's limit before anything
//! is allocated, and a body that does not decode is a [`FrameError`], not a crash.

use crate::RemoteError;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Version of the frame protocol, exchanged in [`Frame::Hello`] / [`Frame::Welcome`]. Version 2 added the
/// connection's [`ProtocolVersion`] to both handshake frames.
pub const VERSION: u16 = 2;
/// Largest frame a connection accepts unless configured otherwise (16 MiB).
pub const DEFAULT_MAX_FRAME: usize = 16 << 20;
/// No connection accepts frames larger than this (64 MiB), whatever it is configured to.
pub const HARD_MAX_FRAME: usize = 64 << 20;
/// Bytes of the length prefix.
pub const LEN_BYTES: usize = 4;
/// Size of the per-launch token in [`Frame::Hello`].
pub const TOKEN_BYTES: usize = 32;

/// The version of the protocol a connection's bodies speak (the plugin protocol, the AI worker's, ...), as
/// opposed to the frame [`VERSION`]. Both sides send theirs in the handshake; the host accepts a child that speaks
/// the same major version and a minor version it knows ([`ProtocolVersion::accepts`]), and otherwise refuses it
/// with a message that says which side must be updated ([`ProtocolVersion::mismatch`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProtocolVersion {
    /// Incompatible changes (a request or reply changed or was removed).
    pub major: u16,
    /// Compatible additions (a request or reply variant added at the end of its enum).
    pub minor: u16,
}

impl ProtocolVersion {
    /// Connections whose protocol carries no version of its own (both sides are built together).
    pub const UNVERSIONED: ProtocolVersion = ProtocolVersion::new(0, 0);

    /// `major.minor`.
    pub const fn new(major: u16, minor: u16) -> ProtocolVersion {
        ProtocolVersion { major, minor }
    }

    /// Whether a host speaking `self` can serve a child built for `child`: same major version, and a minor version
    /// no newer than the host's (a newer minor may send requests this host does not know).
    pub fn accepts(self, child: ProtocolVersion) -> bool {
        self.major == child.major && child.minor <= self.minor
    }

    /// Why a host speaking `self` refuses `label` built for `child`, telling the user what to update; `None` when
    /// [`ProtocolVersion::accepts`] holds.
    pub fn mismatch(self, child: ProtocolVersion, label: &str) -> Option<String> {
        if self.accepts(child) {
            None
        } else if child.major < self.major {
            Some(format!(
                "{label} is too old: it was built for protocol {child} and this Debrief speaks {self}; it needs an update built for                  protocol {}.x",
                self.major
            ))
        } else {
            Some(format!(
                "{label} needs a newer Debrief: it was built for protocol {child} and this Debrief speaks {self}; update Debrief to a version                  that speaks protocol {child} or later"
            ))
        }
    }
}

impl std::fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl std::str::FromStr for ProtocolVersion {
    type Err = String;

    /// `major.minor` (e.g. `1.0`).
    fn from_str(s: &str) -> Result<ProtocolVersion, String> {
        let bad = || format!("\"{s}\" is not a protocol version (major.minor, e.g. 1.0)");
        let (major, minor) = s.trim().split_once('.').ok_or_else(bad)?;
        Ok(ProtocolVersion::new(
            major.parse().map_err(|_| bad())?,
            minor.parse().map_err(|_| bad())?,
        ))
    }
}

/// One message on a connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Frame {
    /// First frame of the connecting side (the child): the frame version, the protocol it speaks, the per-launch
    /// token and its pid. The two versions come first, so a host can read them from a child of any later or earlier
    /// version (see [`handshake_versions`]).
    Hello {
        version: u16,
        protocol: ProtocolVersion,
        token: [u8; TOKEN_BYTES],
        pid: u32,
    },
    /// The listening side (the host) accepted [`Frame::Hello`]; the protocol it speaks.
    Welcome { version: u16, protocol: ProtocolVersion },
    /// A request; `credit` is how many [`Frame::Item`]s the callee may send before the caller's first [`Frame::Credit`].
    Call { id: u64, credit: u32, body: Vec<u8> },
    /// The single answer to a call (after its items, for a streamed call).
    Reply { id: u64, body: Vec<u8> },
    /// One item of a streamed answer.
    Item { id: u64, body: Vec<u8> },
    /// The call failed with a typed error.
    Fail { id: u64, error: RemoteError },
    /// The caller gave up on the call; the callee stops at its next check.
    Cancel { id: u64 },
    /// The caller can take `n` more items of the call.
    Credit { id: u64, n: u32 },
}

impl Frame {
    /// The call this frame belongs to (none for the handshake frames).
    pub fn call_id(&self) -> Option<u64> {
        match self {
            Frame::Hello { .. } | Frame::Welcome { .. } => None,
            Frame::Call { id, .. }
            | Frame::Reply { id, .. }
            | Frame::Item { id, .. }
            | Frame::Fail { id, .. }
            | Frame::Cancel { id }
            | Frame::Credit { id, .. } => Some(*id),
        }
    }
}

/// Why a frame could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The stream ended (cleanly between frames).
    Closed,
    /// The stream ended in the middle of a frame, or the transport failed.
    Io(String),
    /// The announced length is larger than the connection accepts.
    TooLarge { len: usize, max: usize },
    /// The body is not a valid frame.
    Invalid(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Closed => write!(f, "the connection was closed"),
            FrameError::Io(e) => write!(f, "the connection failed ({e})"),
            FrameError::TooLarge { len, max } => write!(f, "a frame of {len} bytes exceeds the limit of {max} bytes"),
            FrameError::Invalid(e) => write!(f, "a frame is not valid ({e})"),
        }
    }
}

impl std::error::Error for FrameError {}

/// The effective limit for a configured `max` (never above [`HARD_MAX_FRAME`], never below 1 KiB).
pub fn clamp_max(max: usize) -> usize {
    max.clamp(1024, HARD_MAX_FRAME)
}

/// `frame` as bytes on the wire (length prefix included). Fails when the encoded frame exceeds `max`.
pub fn encode(frame: &Frame, max: usize) -> Result<Vec<u8>, FrameError> {
    let mut out = postcard::to_extend(frame, vec![0u8; LEN_BYTES]).map_err(|e| FrameError::Invalid(e.to_string()))?;
    let len = out.len() - LEN_BYTES;
    if len > clamp_max(max) {
        return Err(FrameError::TooLarge { len, max: clamp_max(max) });
    }
    out[..LEN_BYTES].copy_from_slice(&(len as u32).to_le_bytes());
    Ok(out)
}

/// The body length a 4-byte prefix announces, checked against `max`.
pub fn body_len(prefix: [u8; LEN_BYTES], max: usize) -> Result<usize, FrameError> {
    let len = u32::from_le_bytes(prefix) as usize;
    if len > clamp_max(max) {
        return Err(FrameError::TooLarge { len, max: clamp_max(max) });
    }
    Ok(len)
}

/// Decodes one frame body (without its length prefix). The whole body must be one frame.
pub fn decode_body(body: &[u8]) -> Result<Frame, FrameError> {
    match postcard::take_from_bytes::<Frame>(body) {
        Ok((frame, [])) => Ok(frame),
        Ok((_, rest)) => Err(FrameError::Invalid(format!("{} trailing bytes", rest.len()))),
        Err(e) => Err(FrameError::Invalid(e.to_string())),
    }
}

/// Splits `buf` into the first complete frame and the bytes after it: `Ok(None)` when more bytes are needed.
/// For callers that buffer themselves (and the fuzz target).
pub fn decode(buf: &[u8], max: usize) -> Result<Option<(Frame, usize)>, FrameError> {
    let Some(prefix) = buf.first_chunk::<LEN_BYTES>() else {
        return Ok(None);
    };
    let len = body_len(*prefix, max)?;
    let Some(body) = buf.get(LEN_BYTES..LEN_BYTES + len) else {
        return Ok(None);
    };
    decode_body(body).map(|f| Some((f, LEN_BYTES + len)))
}

/// The frame version and, from frame version 2 on, the protocol version at the start of a [`Frame::Hello`] body,
/// read without decoding the rest: a peer of another version may lay the rest out differently. `None` when the
/// body does not start like a `Hello`.
pub fn handshake_versions(body: &[u8]) -> Option<(u16, Option<ProtocolVersion>)> {
    let (variant, rest) = postcard::take_from_bytes::<u32>(body).ok()?;
    if variant != 0 {
        return None;
    }
    let (version, rest) = postcard::take_from_bytes::<u16>(rest).ok()?;
    let protocol = if version >= 2 {
        postcard::take_from_bytes::<ProtocolVersion>(rest).ok().map(|(p, _)| p)
    } else {
        None
    };
    Some((version, protocol))
}

/// Reads one frame's body without decoding it (the handshake, which first looks at the versions).
pub async fn read_body<R: AsyncRead + Unpin>(r: &mut R, max: usize) -> Result<Vec<u8>, FrameError> {
    let mut prefix = [0u8; LEN_BYTES];
    let mut got = 0;
    while got < LEN_BYTES {
        match r.read(&mut prefix[got..]).await {
            Ok(0) if got == 0 => return Err(FrameError::Closed),
            Ok(0) => return Err(FrameError::Io("the stream ended inside a frame".into())),
            Ok(n) => got += n,
            Err(e) => return Err(io_error(e)),
        }
    }
    let len = body_len(prefix, max)?;
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::UnexpectedEof => FrameError::Io("the stream ended inside a frame".into()),
        _ => io_error(e),
    })?;
    Ok(body)
}

/// Reads one frame. [`FrameError::Closed`] when the stream ends before its first byte.
pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R, max: usize) -> Result<Frame, FrameError> {
    decode_body(&read_body(r, max).await?)
}

/// Writes one frame (and flushes).
pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, frame: &Frame, max: usize) -> Result<(), FrameError> {
    let bytes = encode(frame, max)?;
    w.write_all(&bytes).await.map_err(io_error)?;
    w.flush().await.map_err(io_error)
}

fn io_error(e: std::io::Error) -> FrameError {
    match e.kind() {
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted => FrameError::Closed,
        _ => FrameError::Io(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Frame> {
        vec![
            Frame::Hello {
                version: VERSION,
                protocol: ProtocolVersion::new(1, 3),
                token: [7; TOKEN_BYTES],
                pid: 42,
            },
            Frame::Welcome {
                version: VERSION,
                protocol: ProtocolVersion::new(1, 4),
            },
            Frame::Call {
                id: 1,
                credit: 4,
                body: b"req".to_vec(),
            },
            Frame::Reply { id: 1, body: Vec::new() },
            Frame::Item {
                id: u64::MAX,
                body: vec![0; 300],
            },
            Frame::Fail {
                id: 2,
                error: RemoteError::new(-1, "no — café"),
            },
            Frame::Cancel { id: 3 },
            Frame::Credit { id: 3, n: 8 },
        ]
    }

    #[test]
    fn every_frame_roundtrips_through_bytes() {
        for f in all() {
            let bytes = encode(&f, DEFAULT_MAX_FRAME).unwrap();
            let (back, used) = decode(&bytes, DEFAULT_MAX_FRAME).unwrap().unwrap();
            assert_eq!((back, used), (f.clone(), bytes.len()));
            // Every strict prefix asks for more bytes.
            for cut in 0..bytes.len() {
                assert_eq!(decode(&bytes[..cut], DEFAULT_MAX_FRAME).unwrap(), None, "{f:?} cut at {cut}");
            }
        }
        assert_eq!(all()[2].call_id(), Some(1));
        assert_eq!(all()[0].call_id(), None);
    }

    #[test]
    fn protocol_versions_say_which_side_to_update() {
        let host = ProtocolVersion::new(2, 3);
        assert!(host.accepts(ProtocolVersion::new(2, 0)) && host.accepts(host));
        assert!(!host.accepts(ProtocolVersion::new(2, 4)) && !host.accepts(ProtocolVersion::new(1, 9)));
        assert_eq!(host.mismatch(ProtocolVersion::new(2, 1), "p"), None);
        let old = host.mismatch(ProtocolVersion::new(1, 7), "plugin \"x\"").unwrap();
        assert!(old.contains("too old") && old.contains("1.7") && old.contains("2.x"), "{old}");
        let new = host.mismatch(ProtocolVersion::new(2, 4), "plugin \"x\"").unwrap();
        assert!(new.contains("needs a newer Debrief") && new.contains("2.4"), "{new}");
        assert!(host.mismatch(ProtocolVersion::new(3, 0), "p").unwrap().contains("newer Debrief"));
        assert_eq!("1.12".parse::<ProtocolVersion>(), Ok(ProtocolVersion::new(1, 12)));
        for bad in ["1", "a.b", "1.2.3", "", "70000.1"] {
            assert!(bad.parse::<ProtocolVersion>().is_err(), "{bad}");
        }
        assert_eq!(ProtocolVersion::UNVERSIONED.to_string(), "0.0");
    }

    #[test]
    fn handshake_versions_are_read_from_any_hello() {
        let hello = Frame::Hello {
            version: VERSION,
            protocol: ProtocolVersion::new(1, 2),
            token: [1; TOKEN_BYTES],
            pid: 7,
        };
        let body = encode(&hello, DEFAULT_MAX_FRAME).unwrap()[LEN_BYTES..].to_vec();
        assert_eq!(handshake_versions(&body), Some((VERSION, Some(ProtocolVersion::new(1, 2)))));
        // A version-1 Hello (no protocol) still yields its frame version.
        let v1 = postcard::to_stdvec(&(0u32, 1u16, [1u8; TOKEN_BYTES], 7u32)).unwrap();
        assert_eq!(handshake_versions(&v1), Some((1, None)));
        assert_eq!(handshake_versions(&[]), None);
        let cancel = encode(&Frame::Cancel { id: 1 }, DEFAULT_MAX_FRAME).unwrap();
        assert_eq!(handshake_versions(&cancel[LEN_BYTES..]), None);
    }

    #[test]
    fn oversized_frames_are_refused_before_allocation() {
        let big = Frame::Item {
            id: 1,
            body: vec![1; 4096],
        };
        assert!(matches!(encode(&big, 1024), Err(FrameError::TooLarge { .. })));
        // A prefix announcing 4 GiB is refused from the prefix alone.
        let prefix = u32::MAX.to_le_bytes();
        assert_eq!(
            decode(&prefix, DEFAULT_MAX_FRAME),
            Err(FrameError::TooLarge {
                len: u32::MAX as usize,
                max: DEFAULT_MAX_FRAME
            })
        );
        assert_eq!(clamp_max(usize::MAX), HARD_MAX_FRAME);
        assert_eq!(clamp_max(0), 1024);
    }

    #[test]
    fn garbage_and_trailing_bytes_are_invalid() {
        let mut bytes = vec![3, 0, 0, 0, 0xff, 0xff, 0xff];
        assert!(matches!(decode(&bytes, DEFAULT_MAX_FRAME), Err(FrameError::Invalid(_))));
        let ok = encode(&Frame::Cancel { id: 1 }, DEFAULT_MAX_FRAME).unwrap();
        bytes = ok.clone();
        bytes[0] += 1;
        bytes.push(0);
        assert!(matches!(decode(&bytes, DEFAULT_MAX_FRAME), Err(FrameError::Invalid(m)) if m.contains("trailing")));
        assert_eq!(decode_body(&ok[LEN_BYTES..]).unwrap(), Frame::Cancel { id: 1 });
        for e in [
            FrameError::Closed,
            FrameError::Io("x".into()),
            FrameError::TooLarge { len: 2, max: 1 },
            FrameError::Invalid("y".into()),
        ] {
            assert!(!e.to_string().is_empty());
        }
    }

    #[test]
    fn frames_stream_over_async_io() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let (mut a, mut b) = tokio::io::duplex(64);
            let frames = all();
            let sent = frames.clone();
            let writer = tokio::spawn(async move {
                for f in &sent {
                    write_frame(&mut a, f, DEFAULT_MAX_FRAME).await.unwrap();
                }
            });
            for f in frames {
                assert_eq!(read_frame(&mut b, DEFAULT_MAX_FRAME).await.unwrap(), f);
            }
            writer.await.unwrap();
            assert_eq!(read_frame(&mut b, DEFAULT_MAX_FRAME).await, Err(FrameError::Closed));
            // A stream that ends inside a frame is an I/O error, not a clean close.
            let (mut a, mut b) = tokio::io::duplex(64);
            tokio::io::AsyncWriteExt::write_all(&mut a, &[9, 0, 0, 0, 1]).await.unwrap();
            drop(a);
            assert!(matches!(read_frame(&mut b, DEFAULT_MAX_FRAME).await, Err(FrameError::Io(_))));
            let (mut a, mut b) = tokio::io::duplex(64);
            tokio::io::AsyncWriteExt::write_all(&mut a, &[9, 0]).await.unwrap();
            drop(a);
            assert!(matches!(read_frame(&mut b, DEFAULT_MAX_FRAME).await, Err(FrameError::Io(_))));
            // Too large, from the prefix.
            let (mut a, mut b) = tokio::io::duplex(64);
            tokio::io::AsyncWriteExt::write_all(&mut a, &(2048u32).to_le_bytes()).await.unwrap();
            assert!(matches!(read_frame(&mut b, 1024).await, Err(FrameError::TooLarge { .. })));
        });
    }
}
