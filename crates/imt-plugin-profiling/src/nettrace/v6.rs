//! NetTrace V6 (.NET 10): after `Nettrace`, a header (reserved 0, major 6,
//! minor) and blocks with a 4-byte header (size: low 24 bits, kind: high 8
//! bits), unaligned. Kinds: 0 EndOfStream, 1 Trace, 2 Event, 3 Metadata,
//! 4 SequencePoint, 5 Stack, 6 Thread, 7 RemoveThread, 8 LabelList; unknown
//! kinds are skipped. Strings are a varuint byte length and UTF-8.
//!
//! Events refer to threads by index; Thread blocks give each index its OS
//! thread id and name. Metadata rows are not events any more (id, provider,
//! event id, name, then field descriptions and tagged optional data). The rest
//! (event payloads, stacks, the event reader) is shared with V4/V5.
//! Source: PerfView `src/TraceEvent/EventPipe/NetTraceFormat.md` (V6).

use super::cursor::Cursor;
use super::format::{flags, Parsed, Reader, RowHeader, TraceInfo, BLOCK_COMPRESSED, MAGIC};
use super::runtime;
use super::FORMAT;
use imt_profile::{Limits, ProfileError, Result};
use std::collections::HashMap;

/// Highest major version read.
pub const MAJOR: u32 = 6;

/// Block kinds.
pub mod kinds {
    pub const END_OF_STREAM: u8 = 0;
    pub const TRACE: u8 = 1;
    pub const EVENT: u8 = 2;
    pub const METADATA: u8 = 3;
    pub const SEQUENCE_POINT: u8 = 4;
    pub const STACK: u8 = 5;
    pub const THREAD: u8 = 6;
    pub const REMOVE_THREAD: u8 = 7;
    pub const LABEL_LIST: u8 = 8;
}

/// The V6 compressed header's label-list flag (V5: activity id).
const LABEL_LIST_ID: u8 = 1 << 4;
/// Longest key or value string kept from the Trace block.
const MAX_KV: usize = 4096;
/// Most key/value pairs read from the Trace block.
const MAX_PAIRS: usize = 1024;

/// A varuint-length UTF-8 string (at most `max` bytes kept; the rest is consumed).
fn string(c: &mut Cursor<'_>, max: usize) -> Result<String> {
    let n = c.var_u32()? as usize;
    let raw = c.take(n)?;
    Ok(imt_profile::limits::clip(&String::from_utf8_lossy(raw), max))
}

struct V6<'l> {
    r: Reader<'l>,
    /// Thread index → OS thread id.
    threads: HashMap<u64, u64>,
}

/// Parses a NetTrace V6 stream (`bytes` starts with the magic).
pub(super) fn parse(bytes: &[u8], limits: &Limits) -> Result<Parsed> {
    let mut c = Cursor::new(bytes, 0);
    if c.take(MAGIC.len()).ok() != Some(&MAGIC[..]) {
        return Err(ProfileError::malformed(FORMAT, "not a .nettrace file"));
    }
    let _reserved = c.u32()?;
    let major = c.u32()?;
    let _minor = c.u32()?;
    if major > MAJOR {
        return Err(ProfileError::unsupported(
            FORMAT,
            format!("NetTrace format version {major} is not supported yet; versions 4 to 6 are"),
        ));
    }
    if major < MAJOR {
        return Err(c.error(format_args!("a V6-style header with version {major}")));
    }
    let mut v = V6 {
        r: Reader {
            limits,
            out: Parsed::default(),
            metadata: HashMap::new(),
            stack_ids: HashMap::new(),
            stack_index: HashMap::new(),
        },
        threads: HashMap::new(),
    };
    loop {
        if c.is_empty() {
            // A stream cut before EndOfStream: keep what was read (a live capture stopped abruptly).
            if v.r.out.info.is_none() {
                return Err(c.error("the trace ends before its Trace block"));
            }
            v.r.out.unknown_objects += 1;
            break;
        }
        let header = c.u32()?;
        let size = (header & 0x00ff_ffff) as usize;
        let kind = (header >> 24) as u8;
        let block = c.sub(size)?;
        if kind == kinds::END_OF_STREAM {
            break;
        }
        if kind != kinds::TRACE && v.r.out.info.is_none() {
            return Err(block.error("the Trace block must come first"));
        }
        match kind {
            kinds::TRACE => v.trace(block)?,
            kinds::EVENT => v.events(block)?,
            kinds::METADATA => v.metadata(block)?,
            kinds::SEQUENCE_POINT => v.sequence_point(block)?,
            kinds::STACK => v.r.stack_block(block)?,
            kinds::THREAD => v.thread_block(block)?,
            kinds::REMOVE_THREAD | kinds::LABEL_LIST => {}
            _ => v.r.out.unknown_objects += 1,
        }
    }
    if v.r.out.info.is_none() {
        return Err(ProfileError::malformed(FORMAT, "the trace has no Trace block"));
    }
    Ok(v.r.out)
}

impl V6<'_> {
    fn trace(&mut self, mut c: Cursor<'_>) -> Result<()> {
        if self.r.out.info.is_some() {
            return Err(c.error("a second Trace block"));
        }
        let mut sync_time = [0u16; 8];
        for v in &mut sync_time {
            *v = c.u16()?;
        }
        let sync_qpc = c.i64()?;
        let qpc_freq = c.i64()?;
        let pointer_size = c.u32()?;
        let pairs = c.u32()? as usize;
        let mut info = TraceInfo {
            version: 6,
            sync_time,
            sync_qpc,
            qpc_freq,
            pointer_size,
            process_id: 0,
            processors: 0,
            sampling_ns: 0,
        };
        for _ in 0..pairs.min(MAX_PAIRS) {
            let key = string(&mut c, MAX_KV)?;
            let value = string(&mut c, MAX_KV)?;
            let n = value.trim().parse::<u64>().ok();
            match key.as_str() {
                "ProcessId" => info.process_id = n.and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
                "HardwareThreadCount" => info.processors = n.and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
                "ExpectedCPUSamplingRate" => info.sampling_ns = n.and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
                _ => {}
            }
        }
        if info.qpc_freq <= 0 {
            return Err(c.error(format_args!("invalid timestamp frequency {}", info.qpc_freq)));
        }
        if info.pointer_size != 4 && info.pointer_size != 8 {
            return Err(c.error(format_args!("invalid pointer size {}", info.pointer_size)));
        }
        self.r.out.info = Some(info);
        Ok(())
    }

    fn metadata(&mut self, mut c: Cursor<'_>) -> Result<()> {
        let header = usize::from(c.u16()?);
        c.take(header)?;
        let max = self.r.limits.max_name_len;
        while !c.is_empty() {
            let size = usize::from(c.u16()?);
            let mut row = c.sub(size)?;
            let id = row.var_u32()?;
            let provider = string(&mut row, max)?;
            let event_id = row.var_u32()?;
            let name = string(&mut row, max)?;
            // Field descriptions and optional data follow; the classification needs neither.
            self.r.metadata.insert(id, runtime::classify(&provider, event_id, &name));
        }
        Ok(())
    }

    fn thread_block(&mut self, mut c: Cursor<'_>) -> Result<()> {
        let max = self.r.limits.max_name_len;
        while !c.is_empty() {
            let size = usize::from(c.u16()?);
            let mut row = c.sub(size)?;
            let index = row.var_u64()?;
            let mut name = None;
            while !row.is_empty() {
                match row.u8()? {
                    1 => name = Some(string(&mut row, max)?),
                    2 => {
                        let _pid = row.var_u64()?;
                    }
                    3 => {
                        let tid = row.var_u64()?;
                        if self.threads.len() < self.r.limits.max_threads as usize * 4 || self.threads.contains_key(&index) {
                            self.threads.insert(index, tid);
                        }
                    }
                    4 => {
                        string(&mut row, max)?;
                        string(&mut row, max)?;
                    }
                    // An unknown entry: its length is unknown, the rest of the row is skipped.
                    _ => break,
                }
            }
            if let Some(name) = name.filter(|n| !n.is_empty()) {
                let tid = self.threads.get(&index).copied().unwrap_or(index);
                self.r.name_thread(tid, &name);
            }
        }
        Ok(())
    }

    fn sequence_point(&mut self, mut c: Cursor<'_>) -> Result<()> {
        let _ts = c.u64()?;
        let _flags = c.u32()?;
        // Stack ids are valid only up to the next sequence point (as in V5).
        self.r.stack_ids.clear();
        Ok(())
    }

    fn events(&mut self, mut block: Cursor<'_>) -> Result<()> {
        let at = block.clone();
        let header = usize::from(block.u16()?);
        if header < 4 {
            return Err(at.error(format_args!("invalid block header size {header}")));
        }
        let block_flags = block.u16()?;
        block.take(header - 4)?;
        let compressed = block_flags & BLOCK_COMPRESSED != 0;
        let mut h = RowHeader::default();
        while !block.is_empty() {
            let mut payload = if compressed {
                compressed_row(&mut block, &mut h)?
            } else {
                uncompressed_row(&mut block, &mut h)?
            };
            let mut row = h;
            row.thread = self.threads.get(&h.thread).copied().unwrap_or(h.thread);
            self.r.event(&row, payload.rest())?;
        }
        Ok(())
    }
}

fn compressed_row<'a>(c: &mut Cursor<'a>, h: &mut RowHeader) -> Result<Cursor<'a>> {
    use flags::*;
    let fl = c.u8()?;
    if fl & METADATA_ID != 0 {
        h.metadata_id = c.var_u32()?;
    }
    if fl & CAPTURE_THREAD_AND_SEQUENCE != 0 {
        h.sequence = h.sequence.wrapping_add(c.var_u32()?);
        h.capture_thread = c.var_u64()?;
        h.processor = c.var_u32()?;
    } else {
        h.sequence = h.sequence.wrapping_add(1);
    }
    if fl & THREAD_ID != 0 {
        h.thread = c.var_u64()?;
    }
    if fl & STACK_ID != 0 {
        h.stack_id = c.var_u32()?;
    }
    h.ts = h.ts.wrapping_add(c.var_u64()? as i64);
    if fl & LABEL_LIST_ID != 0 {
        let _labels = c.var_u32()?;
    }
    if fl & DATA_LENGTH != 0 {
        h.payload_len = c.var_u32()?;
    }
    c.sub(h.payload_len as usize)
}

/// Fixed part of an uncompressed V6 row after its size field.
const UNCOMPRESSED_HEADER: usize = 4 + 4 + 8 + 8 + 4 + 4 + 8 + 4 + 4;

fn uncompressed_row<'a>(c: &mut Cursor<'a>, h: &mut RowHeader) -> Result<Cursor<'a>> {
    let size = c.len32("event size")?;
    let mut row = c.sub(size)?;
    if size < UNCOMPRESSED_HEADER {
        return Err(row.error(format_args!("event of {size} bytes is shorter than its header")));
    }
    h.metadata_id = row.u32()? & 0x7fff_ffff;
    h.sequence = row.u32()?;
    h.thread = row.u64()?;
    h.capture_thread = row.u64()?;
    h.processor = row.u32()?;
    h.stack_id = row.u32()?;
    h.ts = row.i64()?;
    let _labels = row.u32()?;
    h.payload_len = row.u32()?;
    row.sub(h.payload_len as usize)
}
