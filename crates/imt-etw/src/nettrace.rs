//! A streaming NetTrace writer (the container .NET's EventPipe writes, as the
//! runtime's `ep-file.c` / `ep-block.c` do for formats V4/V5): the
//! FastSerialization header and Trace object, then MetadataBlock, StackBlock,
//! EventBlock and SPBlock objects with 4-byte aligned payloads and
//! header-compressed event rows.
//!
//! ETW captures and `.etl` conversions go through it so the nettrace importer
//! opens them like EventPipe captures (ADR 0019). Memory stays bounded: event
//! rows are flushed in blocks of about [`BLOCK_BYTES`], and the stack table is
//! reset at a sequence point every [`STACKS_PER_REGION`] distinct stacks.

use std::collections::HashMap;
use std::io::{self, Write};

const MAGIC: &[u8; 8] = b"Nettrace";
const FAST_SERIALIZATION: &[u8; 20] = b"!FastSerialization.1";
const TAG_NULL: u8 = 1;
const TAG_BEGIN_PRIVATE_OBJECT: u8 = 5;
const TAG_END_OBJECT: u8 = 6;
/// Trace object version (what .NET 5–9 write; read by the importer as V4/V5).
const TRACE_VERSION: i32 = 4;
/// Block object version.
const BLOCK_VERSION: i32 = 2;
/// Block header: size, flags, min and max timestamp.
const BLOCK_HEADER: u16 = 20;
const BLOCK_COMPRESSED: u16 = 1;

/// Event rows are flushed once a block holds about this many bytes.
pub const BLOCK_BYTES: usize = 64 * 1024;
/// Distinct stacks between sequence points (the table is reset at each).
pub const STACKS_PER_REGION: usize = 20_000;
/// Deepest stack written (deeper ones keep their leaf-most frames).
pub const MAX_STACK_DEPTH: usize = 512;

mod flags {
    pub const METADATA_ID: u8 = 1 << 0;
    pub const CAPTURE_THREAD_AND_SEQUENCE: u8 = 1 << 1;
    pub const THREAD_ID: u8 = 1 << 2;
    pub const STACK_ID: u8 = 1 << 3;
    pub const DATA_LENGTH: u8 = 1 << 7;
}

/// The Trace object's fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceHeader {
    /// SYSTEMTIME (UTC) of `sync_ticks`: year, month, day of week, day, hour, minute, second, millisecond.
    pub sync_time: [u16; 8],
    /// Timestamp (in `ticks_per_second` units) at `sync_time`.
    pub sync_ticks: i64,
    pub ticks_per_second: i64,
    /// The target's pointer size (4 or 8): pointer-sized payload fields and stack addresses use it.
    pub pointer_size: u32,
    pub process_id: u32,
    pub processors: u32,
    /// CPU sampling interval in nanoseconds.
    pub sampling_ns: u32,
}

/// Identity of an event type (one metadata record each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventType<'a> {
    pub provider: &'a str,
    pub id: u32,
    pub version: u32,
    pub name: &'a str,
}

#[derive(Debug, Clone, Copy, Default)]
struct RowState {
    metadata_id: u32,
    sequence: u32,
    thread: u64,
    capture_thread: u64,
    stack_id: u32,
    ts: i64,
    len: usize,
}

/// A block's rows being built.
#[derive(Default)]
struct Block {
    rows: Vec<u8>,
    last: RowState,
    min_ts: i64,
    max_ts: i64,
    count: usize,
}

impl Block {
    fn row(&mut self, r: RowState, payload: &[u8]) {
        use flags::*;
        if self.count == 0 {
            self.min_ts = r.ts;
            self.max_ts = r.ts;
        }
        self.min_ts = self.min_ts.min(r.ts);
        self.max_ts = self.max_ts.max(r.ts);
        let last = self.last;
        let mut fl = 0u8;
        let mut h = Vec::with_capacity(24);
        if r.metadata_id != last.metadata_id {
            var(&mut h, u64::from(r.metadata_id));
            fl |= METADATA_ID;
        }
        let expected = last.sequence.wrapping_add(u32::from(r.metadata_id != 0));
        if expected != r.sequence || last.capture_thread != r.capture_thread {
            var(&mut h, u64::from(r.sequence.wrapping_sub(last.sequence).wrapping_sub(1)));
            var(&mut h, r.capture_thread);
            var(&mut h, 0);
            fl |= CAPTURE_THREAD_AND_SEQUENCE;
        }
        if r.thread != last.thread {
            var(&mut h, r.thread);
            fl |= THREAD_ID;
        }
        if r.stack_id != last.stack_id {
            var(&mut h, u64::from(r.stack_id));
            fl |= STACK_ID;
        }
        var(&mut h, r.ts.wrapping_sub(last.ts) as u64);
        if payload.len() != last.len {
            var(&mut h, payload.len() as u64);
            fl |= DATA_LENGTH;
        }
        self.rows.push(fl);
        self.rows.extend_from_slice(&h);
        self.rows.extend_from_slice(payload);
        self.last = RowState { len: payload.len(), ..r };
        self.count += 1;
    }

    fn payload(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(BLOCK_HEADER as usize + self.rows.len());
        p.extend_from_slice(&BLOCK_HEADER.to_le_bytes());
        p.extend_from_slice(&BLOCK_COMPRESSED.to_le_bytes());
        p.extend_from_slice(&self.min_ts.to_le_bytes());
        p.extend_from_slice(&self.max_ts.to_le_bytes());
        p.extend_from_slice(&self.rows);
        p
    }
}

/// Writes a NetTrace stream to `W` as events arrive.
///
/// Call [`NetTraceWriter::event`] for each event and [`NetTraceWriter::finish`]
/// at the end; a writer dropped without `finish` leaves a file the importer
/// rejects as truncated (it has no end tag).
pub struct NetTraceWriter<W: Write> {
    out: W,
    written: u64,
    pointer_size: usize,
    types: HashMap<(String, u32, u32), u32>,
    next_type: u32,
    metadata: Block,
    events: Block,
    stack_ids: HashMap<Vec<u64>, u32>,
    pending_stacks: Vec<Vec<u64>>,
    first_pending_stack: u32,
    next_stack: u32,
    sequences: HashMap<u64, u32>,
    last_ts: i64,
    events_written: u64,
}

impl<W: Write> NetTraceWriter<W> {
    /// Writes the header and the Trace object.
    pub fn new(mut out: W, h: &TraceHeader) -> io::Result<NetTraceWriter<W>> {
        if h.pointer_size != 4 && h.pointer_size != 8 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "pointer size must be 4 or 8"));
        }
        if h.ticks_per_second <= 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the timestamp frequency must be positive",
            ));
        }
        let mut head = Vec::with_capacity(128);
        head.extend_from_slice(MAGIC);
        head.extend_from_slice(&(FAST_SERIALIZATION.len() as u32).to_le_bytes());
        head.extend_from_slice(FAST_SERIALIZATION);
        type_header(&mut head, "Trace", TRACE_VERSION, TRACE_VERSION);
        for v in h.sync_time {
            head.extend_from_slice(&v.to_le_bytes());
        }
        head.extend_from_slice(&h.sync_ticks.to_le_bytes());
        head.extend_from_slice(&h.ticks_per_second.to_le_bytes());
        for v in [h.pointer_size, h.process_id, h.processors, h.sampling_ns] {
            head.extend_from_slice(&v.to_le_bytes());
        }
        head.push(TAG_END_OBJECT);
        out.write_all(&head)?;
        Ok(NetTraceWriter {
            out,
            written: head.len() as u64,
            pointer_size: h.pointer_size as usize,
            types: HashMap::new(),
            next_type: 1,
            metadata: Block::default(),
            events: Block::default(),
            stack_ids: HashMap::new(),
            pending_stacks: Vec::new(),
            first_pending_stack: 1,
            next_stack: 1,
            sequences: HashMap::new(),
            last_ts: h.sync_ticks,
            events_written: 0,
        })
    }

    /// Bytes written so far plus what is buffered (what the file will hold if finished now, about).
    pub fn bytes(&self) -> u64 {
        self.written + (self.metadata.rows.len() + self.events.rows.len()) as u64 + self.pending_stacks.len() as u64 * 64
    }

    /// Events written so far.
    pub fn events(&self) -> u64 {
        self.events_written
    }

    /// Adds one event on `thread` at `ts` (ticks) with its stack (addresses, leaf first; empty: none).
    pub fn event(&mut self, ty: EventType<'_>, thread: u64, ts: i64, stack: &[u64], payload: &[u8]) -> io::Result<()> {
        let metadata_id = self.type_id(ty);
        let stack_id = self.stack_id(stack)?;
        let sequence = {
            let s = self.sequences.entry(thread).or_insert(0);
            *s = s.wrapping_add(1);
            *s
        };
        self.events.row(
            RowState {
                metadata_id,
                sequence,
                thread,
                capture_thread: thread,
                stack_id,
                ts,
                len: 0,
            },
            payload,
        );
        self.last_ts = self.last_ts.max(ts);
        self.events_written += 1;
        if self.events.rows.len() >= BLOCK_BYTES {
            self.flush()?;
        }
        Ok(())
    }

    fn type_id(&mut self, ty: EventType<'_>) -> u32 {
        let key = (ty.provider.to_string(), ty.id, ty.version);
        if let Some(&id) = self.types.get(&key) {
            return id;
        }
        let id = self.next_type;
        self.next_type += 1;
        self.types.insert(key, id);
        let mut p = Vec::with_capacity(64);
        p.extend_from_slice(&id.to_le_bytes());
        utf16z(&mut p, ty.provider);
        p.extend_from_slice(&ty.id.to_le_bytes());
        utf16z(&mut p, ty.name);
        p.extend_from_slice(&0u64.to_le_bytes()); // keywords
        p.extend_from_slice(&ty.version.to_le_bytes());
        p.extend_from_slice(&5u32.to_le_bytes()); // level: verbose
        p.extend_from_slice(&0u32.to_le_bytes()); // no field descriptions
        self.metadata.row(RowState::default(), &p);
        id
    }

    fn stack_id(&mut self, stack: &[u64]) -> io::Result<u32> {
        if stack.is_empty() {
            return Ok(0);
        }
        let stack = &stack[..stack.len().min(MAX_STACK_DEPTH)];
        if let Some(&id) = self.stack_ids.get(stack) {
            return Ok(id);
        }
        if self.stack_ids.len() >= STACKS_PER_REGION {
            // Ids are valid only up to the next sequence point: flush, mark one, start over.
            self.flush()?;
            self.sequence_point()?;
        }
        let id = self.next_stack;
        self.next_stack += 1;
        self.stack_ids.insert(stack.to_vec(), id);
        self.pending_stacks.push(stack.to_vec());
        Ok(id)
    }

    /// Writes buffered metadata, stacks and events (in that order: rows refer to both).
    pub fn flush(&mut self) -> io::Result<()> {
        if self.metadata.count > 0 {
            let p = self.metadata.payload();
            self.block("MetadataBlock", &p)?;
            self.metadata = Block::default();
        }
        if !self.pending_stacks.is_empty() {
            let mut p = Vec::new();
            p.extend_from_slice(&self.first_pending_stack.to_le_bytes());
            p.extend_from_slice(&(self.pending_stacks.len() as u32).to_le_bytes());
            for s in &self.pending_stacks {
                p.extend_from_slice(&((s.len() * self.pointer_size) as u32).to_le_bytes());
                for a in s {
                    p.extend_from_slice(&a.to_le_bytes()[..self.pointer_size]);
                }
            }
            self.block("StackBlock", &p)?;
            self.first_pending_stack = self.next_stack;
            self.pending_stacks.clear();
        }
        if self.events.count > 0 {
            let p = self.events.payload();
            self.block("EventBlock", &p)?;
            self.events = Block::default();
        }
        self.out.flush()
    }

    fn sequence_point(&mut self) -> io::Result<()> {
        let mut p = Vec::with_capacity(12 + self.sequences.len() * 12);
        p.extend_from_slice(&self.last_ts.to_le_bytes());
        p.extend_from_slice(&(self.sequences.len() as u32).to_le_bytes());
        let mut threads: Vec<(&u64, &u32)> = self.sequences.iter().collect();
        threads.sort();
        for (t, s) in threads {
            p.extend_from_slice(&t.to_le_bytes());
            p.extend_from_slice(&s.to_le_bytes());
        }
        self.block("SPBlock", &p)?;
        self.stack_ids.clear();
        self.next_stack = 1;
        self.first_pending_stack = 1;
        Ok(())
    }

    fn block(&mut self, name: &str, payload: &[u8]) -> io::Result<()> {
        let mut b = Vec::with_capacity(payload.len() + 48);
        type_header(&mut b, name, BLOCK_VERSION, BLOCK_VERSION);
        b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        // The payload starts at a 4-byte aligned file offset.
        while !(self.written + b.len() as u64).is_multiple_of(4) {
            b.push(0);
        }
        b.extend_from_slice(payload);
        b.push(TAG_END_OBJECT);
        self.out.write_all(&b)?;
        self.written += b.len() as u64;
        Ok(())
    }

    /// Flushes everything, writes the end tag and returns the output.
    pub fn finish(mut self) -> io::Result<W> {
        self.flush()?;
        self.out.write_all(&[TAG_NULL])?;
        self.written += 1;
        self.out.flush()?;
        Ok(self.out)
    }
}

fn type_header(out: &mut Vec<u8>, name: &str, version: i32, min_version: i32) {
    out.extend_from_slice(&[TAG_BEGIN_PRIVATE_OBJECT, TAG_BEGIN_PRIVATE_OBJECT, TAG_NULL]);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&min_version.to_le_bytes());
    out.extend_from_slice(&(name.len() as u32).to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out.push(TAG_END_OBJECT);
}

/// Unsigned LEB128.
fn var(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// NUL-terminated UTF-16LE.
pub fn utf16z(out: &mut Vec<u8>, s: &str) {
    for u in s.encode_utf16().chain([0]) {
        out.extend_from_slice(&u.to_le_bytes());
    }
}

/// A SYSTEMTIME for Unix milliseconds (UTC); `None` before 1601 or after 30827.
pub fn system_time(unix_ms: i64) -> Option<[u16; 8]> {
    let days = unix_ms.div_euclid(86_400_000);
    let ms_of_day = unix_ms.rem_euclid(86_400_000);
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    if !(1601..=30827).contains(&year) {
        return None;
    }
    let dow = (days + 4).rem_euclid(7); // 1970-01-01 was a Thursday
    let v = [
        year,
        month,
        dow,
        day,
        ms_of_day / 3_600_000,
        ms_of_day / 60_000 % 60,
        ms_of_day / 1000 % 60,
        ms_of_day % 1000,
    ];
    Some(v.map(|x| x as u16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_plugin_profiling::nettrace::NettraceImporter;
    use imt_profile::{Capture, CaptureImporter, Limits, MetricId};

    pub(crate) const SAMPLE: EventType<'static> = EventType {
        provider: "Microsoft-DotNETCore-SampleProfiler",
        id: 0,
        version: 0,
        name: "ThreadSample",
    };

    pub(crate) fn header(ptr: u32) -> TraceHeader {
        TraceHeader {
            sync_time: system_time(1_792_000_000_000).unwrap(),
            sync_ticks: 1_000,
            ticks_per_second: 10_000_000,
            pointer_size: ptr,
            process_id: 42,
            processors: 4,
            sampling_ns: 1_000_000,
        }
    }

    fn method(id: u64, start: u64, name: &str) -> Vec<u8> {
        let mut p = Vec::new();
        for v in [id, 1, start] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(&0x100u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        utf16z(&mut p, "App");
        utf16z(&mut p, name);
        utf16z(&mut p, "");
        p
    }

    #[test]
    fn writes_what_the_importer_reads_for_both_bitnesses() {
        for ptr in [4u32, 8] {
            let mut w = NetTraceWriter::new(Vec::new(), &header(ptr)).unwrap();
            let rundown = EventType {
                provider: "Microsoft-Windows-DotNETRuntimeRundown",
                id: 144,
                version: 1,
                name: "",
            };
            w.event(rundown, 1, 1_500, &[], &method(1, 0x1000, "Main")).unwrap();
            w.event(rundown, 1, 1_500, &[], &method(2, 0x2000, "Work")).unwrap();
            // Many samples across blocks and stack regions.
            for i in 0..(STACKS_PER_REGION as i64 + 3000) {
                let stack = [0x2000 + (i as u64 % 0x100), 0x1000, 0x9000_0000 + i as u64];
                w.event(SAMPLE, 7 + (i as u64 % 3), 2_000 + i * 10, &stack, &2u32.to_le_bytes())
                    .unwrap();
            }
            assert!(w.bytes() > 0);
            assert_eq!(w.events(), STACKS_PER_REGION as u64 + 3002);
            let bytes = w.finish().unwrap();
            let Capture::Profile(p) = NettraceImporter.import("x.nettrace", &bytes, &Limits::default()).unwrap() else {
                panic!("a profile");
            };
            assert_eq!(p.samples.len(), STACKS_PER_REGION + 3000);
            assert!(p.frames.iter().any(|f| f.function == "App.Main"));
            assert!(p.frames.iter().any(|f| f.function == "App.Work"));
            assert!(p.metrics.iter().any(|m| m.id == MetricId::TimeNs));
            assert!(p.notes.iter().all(|n| !n.contains("missing")), "{:?}", p.notes);
        }
    }

    #[test]
    fn rejects_bad_headers_and_reports_io_errors() {
        assert!(NetTraceWriter::new(
            Vec::new(),
            &TraceHeader {
                pointer_size: 2,
                ..header(8)
            }
        )
        .is_err());
        assert!(NetTraceWriter::new(
            Vec::new(),
            &TraceHeader {
                ticks_per_second: 0,
                ..header(8)
            }
        )
        .is_err());
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(NetTraceWriter::new(Full, &header(8)).is_err());
        // A truncated (unfinished) file is refused by the importer, not misread.
        let mut w = NetTraceWriter::new(Vec::new(), &header(8)).unwrap();
        w.event(SAMPLE, 1, 5, &[1], &[]).unwrap();
        w.flush().unwrap();
        let unfinished = std::mem::take(&mut w.out);
        assert!(NettraceImporter.import("x", &unfinished, &Limits::default()).is_err());
    }

    #[test]
    fn system_times() {
        assert_eq!(system_time(0), Some([1970, 1, 4, 1, 0, 0, 0, 0]));
        assert_eq!(system_time(951_782_400_000 + 3_723_004), Some([2000, 2, 2, 29, 1, 2, 3, 4]));
        assert_eq!(system_time(-12_000_000_000_000), None);
        let mut v = Vec::new();
        var(&mut v, 300);
        assert_eq!(v, [0xac, 0x02]);
    }

    #[test]
    fn deep_stacks_are_cut() {
        let mut w = NetTraceWriter::new(Vec::new(), &header(8)).unwrap();
        let deep: Vec<u64> = (1..=2000).collect();
        w.event(SAMPLE, 1, 5, &deep, &[]).unwrap();
        assert_eq!(w.pending_stacks[0].len(), MAX_STACK_DEPTH);
    }
}
