//! Reads the NetTrace V4/V5 container: the FastSerialization stream of
//! objects (Trace, MetadataBlock, StackBlock, EventBlock, SPBlock), event
//! rows with compressed or uncompressed headers, and the stack cache.
//!
//! The result is an intermediate [`Parsed`] trace holding only what the
//! importer uses: samples, method and module records and runtime events,
//! with stacks interned as address lists. Every table is bounded by the
//! [`Limits`]; every loop consumes input or is bounded by a checked count.

use super::cursor::Cursor;
use super::runtime::{self, Known, MethodInfo};
use super::FORMAT;
use imt_profile::{Limits, ProfileError, Result};
use std::collections::HashMap;

/// `Nettrace` magic.
pub const MAGIC: &[u8; 8] = b"Nettrace";
/// The FastSerialization signature after the magic.
pub const FAST_SERIALIZATION: &[u8; 20] = b"!FastSerialization.1";

/// FastSerialization tags.
pub const TAG_NULL: u8 = 1;
pub const TAG_BEGIN_PRIVATE_OBJECT: u8 = 5;
pub const TAG_END_OBJECT: u8 = 6;

/// Trace object versions this reader understands (format V4 and V5).
pub const TRACE_VERSIONS: std::ops::RangeInclusive<i32> = 4..=5;
/// Highest block object (minimum reader) version understood.
const BLOCK_VERSION: i32 = 2;
/// Size of the Trace object's fields.
const TRACE_FIELDS: usize = 48;
/// Longest object type name accepted.
const MAX_TYPE_NAME: usize = 64;

/// The Trace object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TraceInfo {
    pub version: i32,
    /// SYSTEMTIME: year, month, day of week, day, hour, minute, second, millisecond (UTC).
    pub sync_time: [u16; 8],
    pub sync_qpc: i64,
    pub qpc_freq: i64,
    pub pointer_size: u32,
    pub process_id: u32,
    pub processors: u32,
    /// The sample profiler's interval in nanoseconds (the runtime writes its sampling rate here).
    pub sampling_ns: u32,
}

/// A thread sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SampleRec {
    pub thread: u64,
    pub ts: i64,
    /// Index into [`Parsed::stacks`], or `None` without a stack.
    pub stack: Option<u32>,
}

/// A runtime event, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Rt {
    GcStart { count: u32, generation: u32, reason: u32 },
    GcEnd { count: u32 },
    Exception { type_name: String },
    ContentionStart { native: bool },
    ContentionStop,
    JitStart { method_id: u64, name: String },
    JitDone { method_id: u64, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RtRec {
    pub rt: Rt,
    pub thread: u64,
    pub ts: i64,
    pub stack: Option<u32>,
}

/// What the importer needs from a trace.
#[derive(Debug, Default)]
pub(super) struct Parsed {
    pub info: Option<TraceInfo>,
    pub samples: Vec<SampleRec>,
    pub events: Vec<RtRec>,
    pub methods: Vec<MethodInfo>,
    pub modules: HashMap<u64, String>,
    /// Distinct stacks (addresses, leaf first).
    pub stacks: Vec<Vec<u64>>,
    /// Earliest and latest event timestamps (QPC).
    pub min_ts: Option<i64>,
    pub max_ts: Option<i64>,
    /// Counts of things skipped, for notes.
    pub unknown_metadata: u64,
    pub missing_stacks: u64,
    pub damaged_payloads: u64,
    pub dropped_events: u64,
    pub dropped_methods: u64,
    pub unknown_objects: u64,
}

/// An event row's header (fields persist between rows of one compressed block).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct RowHeader {
    pub metadata_id: u32,
    pub sequence: u32,
    pub thread: u64,
    pub capture_thread: u64,
    pub processor: u32,
    pub stack_id: u32,
    pub ts: i64,
    pub payload_len: u32,
}

/// Compressed header flags.
pub mod flags {
    pub const METADATA_ID: u8 = 1 << 0;
    pub const CAPTURE_THREAD_AND_SEQUENCE: u8 = 1 << 1;
    pub const THREAD_ID: u8 = 1 << 2;
    pub const STACK_ID: u8 = 1 << 3;
    pub const ACTIVITY_ID: u8 = 1 << 4;
    pub const RELATED_ACTIVITY_ID: u8 = 1 << 5;
    pub const SORTED: u8 = 1 << 6;
    pub const DATA_LENGTH: u8 = 1 << 7;
}

/// Event/metadata block header flag: rows use header compression.
pub const BLOCK_COMPRESSED: u16 = 1;

/// The stream reader's state.
struct Reader<'l> {
    limits: &'l Limits,
    out: Parsed,
    /// metadata id → what it means (`None`: an event the importer ignores).
    metadata: HashMap<u32, Option<Known>>,
    /// Stack ids of the current sequence-point region → index into `out.stacks`.
    stack_ids: HashMap<u32, u32>,
    /// Content → index into `out.stacks`.
    stack_index: HashMap<Vec<u64>, u32>,
}

/// Parses a whole nettrace file.
pub(super) fn parse(bytes: &[u8], limits: &Limits) -> Result<Parsed> {
    let mut c = Cursor::new(bytes, 0);
    if c.take(MAGIC.len()).ok() != Some(&MAGIC[..]) {
        return Err(ProfileError::malformed(
            FORMAT,
            "not a .nettrace file (the Nettrace signature is missing)",
        ));
    }
    if bytes.get(8..12) == Some(&[0, 0, 0, 0]) {
        // NetTrace V6+ replaced the FastSerialization header with Reserved=0, Major, Minor.
        let major = bytes.get(12..16).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0);
        return Err(ProfileError::unsupported(
            FORMAT,
            format!("NetTrace format version {major} (written by .NET 10 or later) is not supported yet; versions 4 and 5 are"),
        ));
    }
    let len = c.u32()?;
    if len as usize != FAST_SERIALIZATION.len() || c.take(FAST_SERIALIZATION.len())? != &FAST_SERIALIZATION[..] {
        return Err(c.error("the FastSerialization header is missing"));
    }
    let mut r = Reader {
        limits,
        out: Parsed::default(),
        metadata: HashMap::new(),
        stack_ids: HashMap::new(),
        stack_index: HashMap::new(),
    };
    loop {
        let tag = c.u8()?;
        match tag {
            TAG_NULL => break,
            TAG_BEGIN_PRIVATE_OBJECT => r.object(&mut c)?,
            t => return Err(c.error(format_args!("unexpected tag {t} (expected the start of an object)"))),
        }
    }
    if r.out.info.is_none() {
        return Err(ProfileError::malformed(FORMAT, "the trace has no Trace object"));
    }
    Ok(r.out)
}

fn expect_tag(c: &mut Cursor<'_>, want: u8) -> Result<()> {
    let got = c.u8()?;
    if got != want {
        return Err(c.error(format_args!("expected tag {want}, found {got}")));
    }
    Ok(())
}

impl Reader<'_> {
    /// One object after its BeginPrivateObject tag.
    fn object(&mut self, c: &mut Cursor<'_>) -> Result<()> {
        expect_tag(c, TAG_BEGIN_PRIVATE_OBJECT)?;
        expect_tag(c, TAG_NULL)?;
        let version = c.i32()?;
        let min_version = c.i32()?;
        let name_len = c.len32("type name length")?;
        if name_len > MAX_TYPE_NAME {
            return Err(c.error(format_args!("object type name of {name_len} bytes is too long")));
        }
        let name = c.take(name_len)?;
        expect_tag(c, TAG_END_OBJECT)?;
        if name == b"Trace" {
            self.trace(c, version, min_version)?;
        } else {
            if self.out.info.is_none() {
                return Err(c.error("the Trace object must come first"));
            }
            let size = c.len32("block size")?;
            let pad = (4 - c.offset() % 4) % 4;
            c.take(pad)?;
            let block = c.sub(size)?;
            if min_version > BLOCK_VERSION {
                let name = String::from_utf8_lossy(name);
                return Err(ProfileError::unsupported(
                    FORMAT,
                    format!("{name} version {version} (minimum reader version {min_version}) is not supported"),
                ));
            }
            match name {
                b"MetadataBlock" => self.metadata_block(block)?,
                b"EventBlock" => self.event_block(block)?,
                b"StackBlock" => self.stack_block(block)?,
                b"SPBlock" => self.sequence_point(block)?,
                _ => self.out.unknown_objects += 1,
            }
        }
        expect_tag(c, TAG_END_OBJECT)
    }

    fn trace(&mut self, c: &mut Cursor<'_>, version: i32, min_version: i32) -> Result<()> {
        if self.out.info.is_some() {
            return Err(c.error("a second Trace object"));
        }
        if !TRACE_VERSIONS.contains(&version) || min_version > *TRACE_VERSIONS.end() {
            return Err(ProfileError::unsupported(
                FORMAT,
                format!("NetTrace format version {version} is not supported; versions 4 and 5 (.NET Core 3.0 to .NET 9) are"),
            ));
        }
        let mut f = c.sub(TRACE_FIELDS)?;
        let mut sync_time = [0u16; 8];
        for v in &mut sync_time {
            *v = f.u16()?;
        }
        let info = TraceInfo {
            version,
            sync_time,
            sync_qpc: f.i64()?,
            qpc_freq: f.i64()?,
            pointer_size: f.u32()?,
            process_id: f.u32()?,
            processors: f.u32()?,
            sampling_ns: f.u32()?,
        };
        if info.qpc_freq <= 0 {
            return Err(f.error(format_args!("invalid timestamp frequency {}", info.qpc_freq)));
        }
        if info.pointer_size != 4 && info.pointer_size != 8 {
            return Err(f.error(format_args!("invalid pointer size {}", info.pointer_size)));
        }
        self.out.info = Some(info);
        Ok(())
    }

    fn metadata_block(&mut self, block: Cursor<'_>) -> Result<()> {
        let max_name = self.limits.max_name_len;
        rows(block, |_, mut payload| {
            let id = payload.u32()?;
            let provider = payload.utf16z(max_name)?;
            let event_id = payload.u32()?;
            let event_name = payload.utf16z(max_name)?;
            // Keywords, version, level and the field descriptions follow; not needed.
            self.metadata.insert(id, runtime::classify(&provider, event_id, &event_name));
            Ok(())
        })
    }

    fn event_block(&mut self, block: Cursor<'_>) -> Result<()> {
        rows(block, |h, mut payload| self.event(&h, payload.rest()))
    }

    fn event(&mut self, h: &RowHeader, bytes: &[u8]) -> Result<()> {
        self.out.min_ts = Some(self.out.min_ts.map_or(h.ts, |m| m.min(h.ts)));
        self.out.max_ts = Some(self.out.max_ts.map_or(h.ts, |m| m.max(h.ts)));
        let kind = match self.metadata.get(&h.metadata_id) {
            None => {
                self.out.unknown_metadata += 1;
                return Ok(());
            }
            Some(None) => return Ok(()),
            Some(Some(k)) => *k,
        };
        let stack = if h.stack_id == 0 {
            None
        } else {
            let s = self.stack_ids.get(&h.stack_id).copied();
            if s.is_none() {
                self.out.missing_stacks += 1;
            }
            s
        };
        let max_name = self.limits.max_name_len;
        let rt = match kind {
            Known::Sample => {
                if self.out.samples.len() as u64 >= self.limits.max_samples {
                    return Err(ProfileError::LimitExceeded {
                        what: "samples",
                        limit: self.limits.max_samples,
                        hint: "open a shorter recording or a lower sampling rate",
                    });
                }
                self.out.samples.push(SampleRec {
                    thread: h.thread,
                    ts: h.ts,
                    stack,
                });
                return Ok(());
            }
            Known::Module => {
                match runtime::module(bytes, max_name) {
                    Some((id, name)) if self.out.modules.len() < self.limits.max_frames as usize => {
                        self.out.modules.insert(id, name);
                    }
                    Some(_) => self.out.dropped_methods += 1,
                    None => self.out.damaged_payloads += 1,
                }
                return Ok(());
            }
            Known::Method { jitted } => {
                let Some(m) = runtime::method(bytes, max_name) else {
                    self.out.damaged_payloads += 1;
                    return Ok(());
                };
                let done = Rt::JitDone {
                    method_id: m.method_id,
                    name: m.name.clone(),
                };
                if self.out.methods.len() < self.limits.max_frames as usize {
                    self.out.methods.push(m);
                } else {
                    self.out.dropped_methods += 1;
                }
                if !jitted {
                    return Ok(());
                }
                Some(done)
            }
            Known::JitStart => runtime::jitting_started(bytes, max_name).map(|(method_id, name)| Rt::JitStart { method_id, name }),
            Known::GcStart => runtime::gc_start(bytes).map(|(count, generation, reason)| Rt::GcStart { count, generation, reason }),
            Known::GcEnd => runtime::gc_end(bytes).map(|count| Rt::GcEnd { count }),
            Known::Exception => runtime::exception(bytes, max_name).map(|type_name| Rt::Exception { type_name }),
            Known::ContentionStart => runtime::contention_native(bytes).map(|native| Rt::ContentionStart { native }),
            Known::ContentionStop => Some(Rt::ContentionStop),
        };
        let Some(rt) = rt else {
            self.out.damaged_payloads += 1;
            return Ok(());
        };
        // Pairs of events (start/stop) become one timeline event; keep room for both.
        if self.out.events.len() as u64 >= u64::from(self.limits.max_events).saturating_mul(2) {
            self.out.dropped_events += 1;
            return Ok(());
        }
        self.out.events.push(RtRec {
            rt,
            thread: h.thread,
            ts: h.ts,
            stack,
        });
        Ok(())
    }

    fn stack_block(&mut self, mut block: Cursor<'_>) -> Result<()> {
        let first_id = block.u32()?;
        let count = block.u32()?;
        let ptr = self.out.info.map_or(8, |i| i.pointer_size as usize);
        let mut id = first_id;
        // Each stack takes at least its 4-byte size, so the count is bounded by the bytes.
        for _ in 0..count {
            if block.is_empty() {
                return Err(block.error(format_args!("the stack block declares {count} stacks but ends early")));
            }
            let size = block.len32("stack size")?;
            let raw = block.take(size)?;
            let addrs: Vec<u64> = raw
                .chunks_exact(ptr)
                .map(|w| {
                    let mut b = [0u8; 8];
                    b[..ptr].copy_from_slice(w);
                    u64::from_le_bytes(b)
                })
                .collect();
            let index = match self.stack_index.get(&addrs) {
                Some(&i) => i,
                None => {
                    if self.out.stacks.len() as u64 >= u64::from(self.limits.max_stack_nodes) {
                        return Err(ProfileError::LimitExceeded {
                            what: "distinct stacks",
                            limit: u64::from(self.limits.max_stack_nodes),
                            hint: "split the capture into shorter recordings",
                        });
                    }
                    let i = self.out.stacks.len() as u32;
                    self.out.stacks.push(addrs.clone());
                    self.stack_index.insert(addrs, i);
                    i
                }
            };
            self.stack_ids.insert(id, index);
            id = id.wrapping_add(1);
        }
        Ok(())
    }

    fn sequence_point(&mut self, mut block: Cursor<'_>) -> Result<()> {
        let _ts = block.i64()?;
        let threads = block.u32()?;
        // (thread id u64, sequence number u32) per thread.
        if u64::from(threads) * 12 > block.remaining() as u64 {
            return Err(block.error(format_args!("the sequence point lists {threads} threads but is too short")));
        }
        // Stack ids are only valid up to the next sequence point.
        self.stack_ids.clear();
        Ok(())
    }
}

/// Calls `f` with each event row's header and payload in a metadata or event block.
fn rows<'a>(mut block: Cursor<'a>, mut f: impl FnMut(RowHeader, Cursor<'a>) -> Result<()>) -> Result<()> {
    let at = block.clone();
    let header_size = usize::from(block.u16()?);
    if header_size < 4 {
        return Err(at.error(format_args!("invalid block header size {header_size}")));
    }
    let block_flags = block.u16()?;
    block.take(header_size - 4)?;
    let compressed = block_flags & BLOCK_COMPRESSED != 0;
    let mut h = RowHeader::default();
    while !block.is_empty() {
        let payload = if compressed {
            compressed_row(&mut block, &mut h)?
        } else {
            uncompressed_row(&mut block, &mut h)?
        };
        f(h, payload)?;
    }
    Ok(())
}

fn compressed_row<'a>(c: &mut Cursor<'a>, h: &mut RowHeader) -> Result<Cursor<'a>> {
    use flags::*;
    let fl = c.u8()?;
    if fl & METADATA_ID != 0 {
        h.metadata_id = c.var_u32()?;
    }
    if fl & CAPTURE_THREAD_AND_SEQUENCE != 0 {
        h.sequence = h.sequence.wrapping_add(c.var_u32()?).wrapping_add(1);
        h.capture_thread = c.var_u64()?;
        h.processor = c.var_u32()?;
    } else if h.metadata_id != 0 {
        h.sequence = h.sequence.wrapping_add(1);
    }
    if fl & THREAD_ID != 0 {
        h.thread = c.var_u64()?;
    }
    if fl & STACK_ID != 0 {
        h.stack_id = c.var_u32()?;
    }
    h.ts = h.ts.wrapping_add(c.var_u64()? as i64);
    if fl & ACTIVITY_ID != 0 {
        c.take(16)?;
    }
    if fl & RELATED_ACTIVITY_ID != 0 {
        c.take(16)?;
    }
    if fl & DATA_LENGTH != 0 {
        h.payload_len = c.var_u32()?;
    }
    c.sub(h.payload_len as usize)
}

/// Fixed part of an uncompressed row after its size field.
const UNCOMPRESSED_HEADER: usize = 4 + 4 + 8 + 8 + 4 + 4 + 8 + 16 + 16 + 4;

fn uncompressed_row<'a>(c: &mut Cursor<'a>, h: &mut RowHeader) -> Result<Cursor<'a>> {
    let size = c.len32("event size")?;
    let mut row = c.sub(size)?;
    if size < UNCOMPRESSED_HEADER {
        return Err(row.error(format_args!("event of {size} bytes is shorter than its header")));
    }
    // The high bit of the metadata id is the (inverted) IsSorted flag.
    h.metadata_id = row.u32()? & 0x7fff_ffff;
    h.sequence = row.u32()?;
    h.thread = row.u64()?;
    h.capture_thread = row.u64()?;
    h.processor = row.u32()?;
    h.stack_id = row.u32()?;
    h.ts = row.i64()?;
    row.take(32)?;
    h.payload_len = row.u32()?;
    // Alignment padding after the payload is part of the event size.
    row.sub(h.payload_len as usize)
}
