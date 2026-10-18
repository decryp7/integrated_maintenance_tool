//! Capture files: a magic, a version, then records `[tag u8][len u32 LE][payload]`.
//!
//! Records are written as they happen, so a file cut short (the target process
//! exited, the disk filled) is still readable up to its last whole record.
//! Unknown tags are skipped, so newer agents stay readable by older importers.

use crate::wire::{Dec, Enc};
use crate::{FormatError, MAX_RECORD};

/// First bytes of a CPU/exception profile capture (`.imtclr`).
pub const PROFILE_MAGIC: &[u8; 8] = b"IMTCLRP\x01";
/// First bytes of a heap snapshot capture (`.imtclrheap`).
pub const HEAP_MAGIC: &[u8; 8] = b"IMTCLRH\x01";
/// Version written after the magic.
pub const VERSION: u16 = 1;

/// The file header for a capture kind.
pub fn header(magic: &[u8; 8]) -> Vec<u8> {
    let mut v = magic.to_vec();
    v.extend_from_slice(&VERSION.to_le_bytes());
    v
}

/// Frames one record: `[tag][len][payload]`.
pub fn frame(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 5);
    v.push(tag);
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

/// Iterates the raw records of a capture after checking its header.
#[derive(Debug, Clone)]
pub struct Records<'a> {
    dec: Dec<'a>,
    /// The file ended inside a record.
    pub truncated: bool,
}

impl<'a> Records<'a> {
    /// Checks the magic and version.
    pub fn open(bytes: &'a [u8], magic: &[u8; 8]) -> Result<Records<'a>, FormatError> {
        let mut dec = Dec::new(bytes);
        if dec.take(8).map_err(|_| FormatError::NotThisFormat)? != magic {
            return Err(FormatError::NotThisFormat);
        }
        let v = dec.take(2)?;
        let version = u16::from_le_bytes([v[0], v[1]]);
        if version != VERSION {
            return Err(FormatError::UnsupportedVersion(version));
        }
        Ok(Records { dec, truncated: false })
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = Result<(u8, &'a [u8]), FormatError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.dec.is_empty() || self.truncated {
            return None;
        }
        let start = self.dec.clone();
        let item = (|| {
            let tag = self.dec.u8()?;
            let len = self.dec.u32_le()? as usize;
            if len > MAX_RECORD {
                return Err(FormatError::Invalid("record is larger than the format allows"));
            }
            Ok((tag, self.dec.take(len)?))
        })();
        match item {
            Err(FormatError::Truncated) => {
                self.dec = start;
                self.truncated = true;
                None
            }
            other => Some(other),
        }
    }
}

/// Where a profile frame's code runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Managed = 0,
    Native = 1,
}

/// A record of a profile capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileRecord {
    /// First record: the process and the capture settings.
    Info {
        pid: u32,
        process: String,
        runtime: String,
        pointer_bits: u8,
        started_unix_ms: i64,
        interval_us: u32,
    },
    /// Declares frame `id` before any sample uses it.
    Frame {
        id: u32,
        function: String,
        module: String,
        kind: FrameKind,
        framework: bool,
    },
    /// Declares (or renames) thread `id`.
    Thread { id: u32, os_id: u32, name: String },
    /// One stack observation; `weight_ns` is the time it stands for.
    Sample {
        thread: u32,
        time_ns: u64,
        weight_ns: u64,
        on_cpu: bool,
        frames_leaf_first: Vec<u32>,
    },
    /// A thrown exception with the throwing stack.
    Exception {
        thread: u32,
        time_ns: u64,
        type_name: String,
        frames_leaf_first: Vec<u32>,
    },
    /// The runtime was suspended for a garbage collection.
    Gc { start_ns: u64, dur_ns: u64 },
    /// Counters so far (the last one wins).
    Stats {
        time_ns: u64,
        samples: u64,
        dropped: u64,
        failed_walks: u64,
    },
    /// Last record of a capture that ended normally.
    End { time_ns: u64 },
}

mod ptag {
    pub const INFO: u8 = 1;
    pub const FRAME: u8 = 2;
    pub const THREAD: u8 = 3;
    pub const SAMPLE: u8 = 4;
    pub const EXCEPTION: u8 = 5;
    pub const GC: u8 = 6;
    pub const STATS: u8 = 7;
    pub const END: u8 = 8;
}

fn frames(e: &mut Enc, frames: &[u32]) {
    let n = frames.len().min(crate::MAX_STACK);
    e.u(n as u64);
    for &f in &frames[..n] {
        e.u(u64::from(f));
    }
}

fn read_frames(d: &mut Dec<'_>) -> Result<Vec<u32>, FormatError> {
    let n = d.count(1)?;
    if n > crate::MAX_STACK {
        return Err(FormatError::Invalid("stack is deeper than the format allows"));
    }
    (0..n).map(|_| d.u32()).collect()
}

impl ProfileRecord {
    /// The framed bytes of this record.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::with_capacity(32);
        let tag = match self {
            ProfileRecord::Info {
                pid,
                process,
                runtime,
                pointer_bits,
                started_unix_ms,
                interval_us,
            } => {
                e.u(u64::from(*pid))
                    .str(process)
                    .str(runtime)
                    .u8(*pointer_bits)
                    .i(*started_unix_ms)
                    .u(u64::from(*interval_us));
                ptag::INFO
            }
            ProfileRecord::Frame {
                id,
                function,
                module,
                kind,
                framework,
            } => {
                e.u(u64::from(*id)).str(function).str(module).u8(*kind as u8).bool(*framework);
                ptag::FRAME
            }
            ProfileRecord::Thread { id, os_id, name } => {
                e.u(u64::from(*id)).u(u64::from(*os_id)).str(name);
                ptag::THREAD
            }
            ProfileRecord::Sample {
                thread,
                time_ns,
                weight_ns,
                on_cpu,
                frames_leaf_first,
            } => {
                e.u(u64::from(*thread)).u(*time_ns).u(*weight_ns).bool(*on_cpu);
                frames(&mut e, frames_leaf_first);
                ptag::SAMPLE
            }
            ProfileRecord::Exception {
                thread,
                time_ns,
                type_name,
                frames_leaf_first,
            } => {
                e.u(u64::from(*thread)).u(*time_ns).str(type_name);
                frames(&mut e, frames_leaf_first);
                ptag::EXCEPTION
            }
            ProfileRecord::Gc { start_ns, dur_ns } => {
                e.u(*start_ns).u(*dur_ns);
                ptag::GC
            }
            ProfileRecord::Stats {
                time_ns,
                samples,
                dropped,
                failed_walks,
            } => {
                e.u(*time_ns).u(*samples).u(*dropped).u(*failed_walks);
                ptag::STATS
            }
            ProfileRecord::End { time_ns } => {
                e.u(*time_ns);
                ptag::END
            }
        };
        frame(tag, &e.buf)
    }

    /// Decodes a record; `Ok(None)` for tags this version does not know.
    pub fn decode(tag: u8, payload: &[u8]) -> Result<Option<ProfileRecord>, FormatError> {
        let d = &mut Dec::new(payload);
        Ok(Some(match tag {
            ptag::INFO => ProfileRecord::Info {
                pid: d.u32()?,
                process: d.str()?,
                runtime: d.str()?,
                pointer_bits: d.u8()?,
                started_unix_ms: d.i()?,
                interval_us: d.u32()?,
            },
            ptag::FRAME => ProfileRecord::Frame {
                id: d.u32()?,
                function: d.str()?,
                module: d.str()?,
                kind: match d.u8()? {
                    0 => FrameKind::Managed,
                    1 => FrameKind::Native,
                    _ => return Err(FormatError::Invalid("unknown frame kind")),
                },
                framework: d.bool()?,
            },
            ptag::THREAD => ProfileRecord::Thread {
                id: d.u32()?,
                os_id: d.u32()?,
                name: d.str()?,
            },
            ptag::SAMPLE => ProfileRecord::Sample {
                thread: d.u32()?,
                time_ns: d.u()?,
                weight_ns: d.u()?,
                on_cpu: d.bool()?,
                frames_leaf_first: read_frames(d)?,
            },
            ptag::EXCEPTION => ProfileRecord::Exception {
                thread: d.u32()?,
                time_ns: d.u()?,
                type_name: d.str()?,
                frames_leaf_first: read_frames(d)?,
            },
            ptag::GC => ProfileRecord::Gc {
                start_ns: d.u()?,
                dur_ns: d.u()?,
            },
            ptag::STATS => ProfileRecord::Stats {
                time_ns: d.u()?,
                samples: d.u()?,
                dropped: d.u()?,
                failed_walks: d.u()?,
            },
            ptag::END => ProfileRecord::End { time_ns: d.u()? },
            _ => return Ok(None),
        }))
    }
}

/// What kind of GC root holds an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RootKind {
    Other = 0,
    Stack = 1,
    Finalizer = 2,
    Handle = 3,
}

impl RootKind {
    pub fn from_u8(v: u8) -> Option<RootKind> {
        Some(match v {
            0 => RootKind::Other,
            1 => RootKind::Stack,
            2 => RootKind::Finalizer,
            3 => RootKind::Handle,
            _ => return None,
        })
    }
}

/// Root flags (as `COR_PRF_GC_ROOT_FLAGS`).
pub mod root_flags {
    pub const PINNING: u8 = 0x1;
    pub const WEAK: u8 = 0x2;
    pub const INTERIOR: u8 = 0x4;
    pub const REFCOUNTED: u8 = 0x8;
}

/// A record of a heap snapshot capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeapRecord {
    /// First record.
    Info {
        pid: u32,
        process: String,
        runtime: String,
        pointer_bits: u8,
        taken_unix_ms: i64,
    },
    /// Declares type `id` before any object uses it.
    Type { id: u32, name: String },
    /// A live object, its shallow size and the objects it references.
    Object {
        addr: u64,
        type_id: u32,
        size: u64,
        refs: Vec<u64>,
    },
    /// An object held by a GC root.
    Root { addr: u64, kind: RootKind, flags: u8 },
    /// Last record; `truncated` when the agent hit a limit or dropped data.
    End { objects: u64, truncated: bool },
}

mod htag {
    pub const INFO: u8 = 1;
    pub const TYPE: u8 = 2;
    pub const OBJECT: u8 = 3;
    pub const ROOT: u8 = 4;
    pub const END: u8 = 5;
}

impl HeapRecord {
    /// The framed bytes of this record. References are zigzag deltas from the
    /// object's address, which keeps neighbouring objects short.
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::with_capacity(16);
        let tag = match self {
            HeapRecord::Info {
                pid,
                process,
                runtime,
                pointer_bits,
                taken_unix_ms,
            } => {
                e.u(u64::from(*pid)).str(process).str(runtime).u8(*pointer_bits).i(*taken_unix_ms);
                htag::INFO
            }
            HeapRecord::Type { id, name } => {
                e.u(u64::from(*id)).str(name);
                htag::TYPE
            }
            HeapRecord::Object { addr, type_id, size, refs } => {
                encode_object(&mut e, *addr, *type_id, *size, refs);
                htag::OBJECT
            }
            HeapRecord::Root { addr, kind, flags } => {
                e.u(*addr).u8(*kind as u8).u8(*flags);
                htag::ROOT
            }
            HeapRecord::End { objects, truncated } => {
                e.u(*objects).bool(*truncated);
                htag::END
            }
        };
        frame(tag, &e.buf)
    }

    pub fn decode(tag: u8, payload: &[u8]) -> Result<Option<HeapRecord>, FormatError> {
        let d = &mut Dec::new(payload);
        Ok(Some(match tag {
            htag::INFO => HeapRecord::Info {
                pid: d.u32()?,
                process: d.str()?,
                runtime: d.str()?,
                pointer_bits: d.u8()?,
                taken_unix_ms: d.i()?,
            },
            htag::TYPE => HeapRecord::Type {
                id: d.u32()?,
                name: d.str()?,
            },
            htag::OBJECT => {
                let addr = d.u()?;
                let type_id = d.u32()?;
                let size = d.u()?;
                let n = d.count(1)?;
                let mut refs = Vec::with_capacity(n);
                for _ in 0..n {
                    refs.push(addr.wrapping_add(d.i()? as u64));
                }
                HeapRecord::Object { addr, type_id, size, refs }
            }
            htag::ROOT => HeapRecord::Root {
                addr: d.u()?,
                kind: RootKind::from_u8(d.u8()?).ok_or(FormatError::Invalid("unknown root kind"))?,
                flags: d.u8()?,
            },
            htag::END => HeapRecord::End {
                objects: d.u()?,
                truncated: d.bool()?,
            },
            _ => return Ok(None),
        }))
    }
}

/// Encodes an object record payload straight from the runtime's arrays (the
/// agent calls this inside GC callbacks, without building a `HeapRecord`).
pub fn encode_object(e: &mut Enc, addr: u64, type_id: u32, size: u64, refs: &[u64]) {
    let n = refs.len().min(crate::MAX_REFS);
    e.u(addr).u(u64::from(type_id)).u(size).u(n as u64);
    for &r in &refs[..n] {
        e.i(r.wrapping_sub(addr) as i64);
    }
}

/// The framed object record (see [`encode_object`]).
pub fn object_record(addr: u64, type_id: u32, size: u64, refs: &[u64]) -> Vec<u8> {
    let mut e = Enc::with_capacity(16 + refs.len() * 3);
    encode_object(&mut e, addr, type_id, size, refs);
    frame(htag::OBJECT, &e.buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(magic: &[u8; 8], records: &[Vec<u8>]) -> Vec<u8> {
        let mut v = header(magic);
        for r in records {
            v.extend_from_slice(r);
        }
        v
    }

    fn profile_records() -> Vec<ProfileRecord> {
        vec![
            ProfileRecord::Info {
                pid: 42,
                process: "app.exe".into(),
                runtime: "4.8.9000".into(),
                pointer_bits: 64,
                started_unix_ms: 1_700_000_000_000,
                interval_us: 10_000,
            },
            ProfileRecord::Frame {
                id: 0,
                function: "App.Program.Main".into(),
                module: "app.exe".into(),
                kind: FrameKind::Managed,
                framework: false,
            },
            ProfileRecord::Frame {
                id: 1,
                function: "[Native code]".into(),
                module: "clr.dll".into(),
                kind: FrameKind::Native,
                framework: true,
            },
            ProfileRecord::Thread {
                id: 0,
                os_id: 1234,
                name: "Main".into(),
            },
            ProfileRecord::Sample {
                thread: 0,
                time_ns: 5,
                weight_ns: 10_000_000,
                on_cpu: true,
                frames_leaf_first: vec![1, 0],
            },
            ProfileRecord::Exception {
                thread: 0,
                time_ns: 6,
                type_name: "System.InvalidOperationException".into(),
                frames_leaf_first: vec![0],
            },
            ProfileRecord::Gc { start_ns: 7, dur_ns: 3 },
            ProfileRecord::Stats {
                time_ns: 9,
                samples: 1,
                dropped: 0,
                failed_walks: 2,
            },
            ProfileRecord::End { time_ns: 10 },
        ]
    }

    #[test]
    fn profile_records_round_trip() {
        let recs = profile_records();
        let bytes = file(PROFILE_MAGIC, &recs.iter().map(ProfileRecord::encode).collect::<Vec<_>>());
        let mut it = Records::open(&bytes, PROFILE_MAGIC).unwrap();
        let back: Vec<ProfileRecord> = it
            .by_ref()
            .map(|r| {
                let (t, p) = r.unwrap();
                ProfileRecord::decode(t, p).unwrap().unwrap()
            })
            .collect();
        assert_eq!(back, recs);
        assert!(!it.truncated);
    }

    #[test]
    fn heap_records_round_trip_with_delta_references() {
        let recs = vec![
            HeapRecord::Info {
                pid: 1,
                process: "p".into(),
                runtime: "4.8".into(),
                pointer_bits: 32,
                taken_unix_ms: -5,
            },
            HeapRecord::Type {
                id: 0,
                name: "System.Object[]".into(),
            },
            HeapRecord::Object {
                addr: 0x1000,
                type_id: 0,
                size: 48,
                refs: vec![0x0800, 0x2000, 0x1000],
            },
            HeapRecord::Root {
                addr: 0x1000,
                kind: RootKind::Handle,
                flags: root_flags::PINNING,
            },
            HeapRecord::End {
                objects: 1,
                truncated: false,
            },
        ];
        let bytes = file(HEAP_MAGIC, &recs.iter().map(HeapRecord::encode).collect::<Vec<_>>());
        let back: Vec<HeapRecord> = Records::open(&bytes, HEAP_MAGIC)
            .unwrap()
            .map(|r| {
                let (t, p) = r.unwrap();
                HeapRecord::decode(t, p).unwrap().unwrap()
            })
            .collect();
        assert_eq!(back, recs);
        assert_eq!(object_record(0x1000, 0, 48, &[0x0800, 0x2000, 0x1000]), recs[2].encode());
    }

    #[test]
    fn truncated_files_stop_at_the_last_whole_record_and_unknown_tags_are_skipped() {
        let recs = profile_records();
        let mut bytes = file(PROFILE_MAGIC, &[recs[0].encode(), frame(200, b"future"), recs[1].encode()]);
        bytes.truncate(bytes.len() - 3);
        let mut it = Records::open(&bytes, PROFILE_MAGIC).unwrap();
        let (t, p) = it.next().unwrap().unwrap();
        assert!(ProfileRecord::decode(t, p).unwrap().is_some());
        let (t, p) = it.next().unwrap().unwrap();
        assert_eq!(ProfileRecord::decode(t, p).unwrap(), None);
        assert!(it.next().is_none());
        assert!(it.truncated);
        assert!(it.next().is_none());
    }

    #[test]
    fn headers_and_corrupt_records_are_rejected() {
        assert_eq!(Records::open(b"nope", PROFILE_MAGIC).unwrap_err(), FormatError::NotThisFormat);
        assert_eq!(
            Records::open(&header(HEAP_MAGIC), PROFILE_MAGIC).unwrap_err(),
            FormatError::NotThisFormat
        );
        let mut v = PROFILE_MAGIC.to_vec();
        v.extend_from_slice(&9u16.to_le_bytes());
        assert_eq!(Records::open(&v, PROFILE_MAGIC).unwrap_err(), FormatError::UnsupportedVersion(9));
        assert_eq!(Records::open(PROFILE_MAGIC, PROFILE_MAGIC).unwrap_err(), FormatError::Truncated);
        let mut big = header(PROFILE_MAGIC);
        big.push(1);
        big.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(Records::open(&big, PROFILE_MAGIC).unwrap().next().unwrap().is_err());
        assert!(ProfileRecord::decode(ptag::FRAME, &[0, 0, 0, 9, 0]).is_err());
        assert!(HeapRecord::decode(htag::ROOT, &[0, 9, 0]).is_err());
        assert!(ProfileRecord::decode(ptag::SAMPLE, &[0, 0, 0, 0, 0x80]).is_err());
        assert_eq!(HeapRecord::decode(99, b"").unwrap(), None);
        assert_eq!(RootKind::from_u8(2), Some(RootKind::Finalizer));
    }
}
