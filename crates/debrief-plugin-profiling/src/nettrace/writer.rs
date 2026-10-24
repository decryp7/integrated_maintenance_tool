//! A small NetTrace V4/V5 writer for tests, fuzz seeds and the committed
//! sample. It follows the runtime's writer (`ep-file.c`, `ep-block.c`):
//! FastSerialization objects, 4-byte aligned block payloads, delta-encoded
//! compressed event headers, interned stacks and sequence points.
//!
//! Not part of the importer's API: hidden from the docs, used by
//! `tests/it/gen_nettrace.rs` and the property tests.

pub use super::runtime::{ids, RUNDOWN, RUNTIME, SAMPLE_PROFILER};

use super::format::{flags, BLOCK_COMPRESSED, FAST_SERIALIZATION, MAGIC, TAG_BEGIN_PRIVATE_OBJECT, TAG_END_OBJECT, TAG_NULL};

/// The Trace object's fields.
#[derive(Debug, Clone, Copy)]
pub struct TraceHeader {
    pub version: i32,
    pub min_version: i32,
    /// SYSTEMTIME (UTC).
    pub sync_time: [u16; 8],
    pub sync_qpc: i64,
    pub qpc_freq: i64,
    pub pointer_size: u32,
    pub process_id: u32,
    pub processors: u32,
    pub sampling_ns: u32,
}

impl Default for TraceHeader {
    fn default() -> Self {
        TraceHeader {
            version: 4,
            min_version: 4,
            // 2026-10-17 (Saturday) 09:30:00.000 UTC
            sync_time: [2026, 10, 6, 17, 9, 30, 0, 0],
            sync_qpc: 1_000_000,
            qpc_freq: 10_000_000,
            pointer_size: 8,
            process_id: 4242,
            processors: 8,
            sampling_ns: 1_000_000,
        }
    }
}

/// One event type: metadata id → provider / event.
#[derive(Debug, Clone)]
pub struct Meta<'a> {
    pub id: u32,
    pub provider: &'a str,
    pub event_id: u32,
    pub name: &'a str,
}

/// An event row.
#[derive(Debug, Clone, Default)]
pub struct Row {
    pub metadata_id: u32,
    pub sequence: u32,
    pub thread: u64,
    pub capture_thread: u64,
    pub processor: u32,
    pub stack_id: u32,
    pub ts: i64,
    pub sorted: bool,
    pub payload: Vec<u8>,
}

/// Writes a nettrace stream.
#[derive(Debug, Clone)]
pub struct Writer {
    out: Vec<u8>,
    pointer_size: u32,
}

impl Writer {
    /// The magic, the FastSerialization header and the Trace object.
    pub fn new(h: &TraceHeader) -> Writer {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(FAST_SERIALIZATION.len() as u32).to_le_bytes());
        out.extend_from_slice(FAST_SERIALIZATION);
        let mut w = Writer {
            out,
            pointer_size: h.pointer_size,
        };
        w.type_header("Trace", h.version, h.min_version);
        for v in h.sync_time {
            w.out.extend_from_slice(&v.to_le_bytes());
        }
        w.out.extend_from_slice(&h.sync_qpc.to_le_bytes());
        w.out.extend_from_slice(&h.qpc_freq.to_le_bytes());
        for v in [h.pointer_size, h.process_id, h.processors, h.sampling_ns] {
            w.out.extend_from_slice(&v.to_le_bytes());
        }
        w.out.push(TAG_END_OBJECT);
        w
    }

    fn type_header(&mut self, name: &str, version: i32, min_version: i32) {
        self.out
            .extend_from_slice(&[TAG_BEGIN_PRIVATE_OBJECT, TAG_BEGIN_PRIVATE_OBJECT, TAG_NULL]);
        self.out.extend_from_slice(&version.to_le_bytes());
        self.out.extend_from_slice(&min_version.to_le_bytes());
        self.out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        self.out.extend_from_slice(name.as_bytes());
        self.out.push(TAG_END_OBJECT);
    }

    /// A block object: type, size, padding to a 4-byte file offset, payload, end tag.
    pub fn block(&mut self, name: &str, version: i32, payload: &[u8]) -> &mut Self {
        self.type_header(name, version, version);
        self.out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        while !self.out.len().is_multiple_of(4) {
            self.out.push(0);
        }
        self.out.extend_from_slice(payload);
        self.out.push(TAG_END_OBJECT);
        self
    }

    /// A MetadataBlock defining `defs`.
    pub fn metadata(&mut self, compressed: bool, defs: &[Meta<'_>]) -> &mut Self {
        let rows: Vec<Row> = defs
            .iter()
            .map(|d| Row {
                payload: payload::metadata(d.id, d.provider, d.event_id, d.name),
                ..Row::default()
            })
            .collect();
        let block = rows_block(compressed, &rows);
        self.block("MetadataBlock", 2, &block)
    }

    /// An EventBlock with `rows`.
    pub fn events(&mut self, compressed: bool, rows: &[Row]) -> &mut Self {
        let block = rows_block(compressed, rows);
        self.block("EventBlock", 2, &block)
    }

    /// A StackBlock with ids `first_id..` (addresses leaf first).
    pub fn stacks(&mut self, first_id: u32, stacks: &[Vec<u64>]) -> &mut Self {
        let mut p = Vec::new();
        p.extend_from_slice(&first_id.to_le_bytes());
        p.extend_from_slice(&(stacks.len() as u32).to_le_bytes());
        for s in stacks {
            p.extend_from_slice(&(s.len() as u32 * self.pointer_size).to_le_bytes());
            for a in s {
                p.extend_from_slice(&a.to_le_bytes()[..self.pointer_size as usize]);
            }
        }
        self.block("StackBlock", 2, &p)
    }

    /// An SPBlock; stack ids written before it are no longer valid after it.
    pub fn sequence_point(&mut self, ts: i64, threads: &[(u64, u32)]) -> &mut Self {
        let mut p = Vec::new();
        p.extend_from_slice(&ts.to_le_bytes());
        p.extend_from_slice(&(threads.len() as u32).to_le_bytes());
        for (t, s) in threads {
            p.extend_from_slice(&t.to_le_bytes());
            p.extend_from_slice(&s.to_le_bytes());
        }
        self.block("SPBlock", 2, &p)
    }

    /// Raw bytes (for malformed-input tests).
    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.out.extend_from_slice(bytes);
        self
    }

    /// The stream with its end tag.
    pub fn finish(mut self) -> Vec<u8> {
        self.out.push(TAG_NULL);
        self.out
    }
}

/// An event or metadata block payload: header (size 20, flags, min/max timestamp) and rows.
pub fn rows_block(compressed: bool, rows: &[Row]) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&20u16.to_le_bytes());
    p.extend_from_slice(&(if compressed { BLOCK_COMPRESSED } else { 0 }).to_le_bytes());
    let min = rows.iter().map(|r| r.ts).min().unwrap_or(0);
    let max = rows.iter().map(|r| r.ts).max().unwrap_or(0);
    p.extend_from_slice(&min.to_le_bytes());
    p.extend_from_slice(&max.to_le_bytes());
    let mut last = Row::default();
    for r in rows {
        if compressed {
            compressed_row(&mut p, r, &last);
            last = r.clone();
        } else {
            uncompressed_row(&mut p, r);
        }
    }
    p
}

/// A row with header compression, delta-encoded against `last` like the runtime does.
fn compressed_row(p: &mut Vec<u8>, r: &Row, last: &Row) {
    use flags::*;
    let mut fl = 0u8;
    let mut h = Vec::new();
    if r.metadata_id != last.metadata_id {
        var(&mut h, u64::from(r.metadata_id));
        fl |= METADATA_ID;
    }
    if r.sorted {
        fl |= SORTED;
    }
    let expected = last.sequence.wrapping_add(u32::from(r.metadata_id != 0));
    if expected != r.sequence || last.capture_thread != r.capture_thread || last.processor != r.processor {
        var(&mut h, u64::from(r.sequence.wrapping_sub(last.sequence).wrapping_sub(1)));
        var(&mut h, r.capture_thread);
        var(&mut h, u64::from(r.processor));
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
    if r.payload.len() != last.payload.len() {
        var(&mut h, r.payload.len() as u64);
        fl |= DATA_LENGTH;
    }
    p.push(fl);
    p.extend_from_slice(&h);
    p.extend_from_slice(&r.payload);
}

/// A row without compression: fixed header, payload, padding to 4 bytes.
fn uncompressed_row(p: &mut Vec<u8>, r: &Row) {
    let unpadded = 76 + r.payload.len();
    let size = unpadded.div_ceil(4) * 4;
    p.extend_from_slice(&(size as u32).to_le_bytes());
    let sorted_bit = if r.sorted { 0 } else { 1u32 << 31 };
    p.extend_from_slice(&(r.metadata_id | sorted_bit).to_le_bytes());
    p.extend_from_slice(&r.sequence.to_le_bytes());
    p.extend_from_slice(&r.thread.to_le_bytes());
    p.extend_from_slice(&r.capture_thread.to_le_bytes());
    p.extend_from_slice(&r.processor.to_le_bytes());
    p.extend_from_slice(&r.stack_id.to_le_bytes());
    p.extend_from_slice(&r.ts.to_le_bytes());
    p.extend_from_slice(&[0u8; 32]);
    p.extend_from_slice(&(r.payload.len() as u32).to_le_bytes());
    p.extend_from_slice(&r.payload);
    p.resize(p.len() + (size - unpadded), 0);
}

/// Unsigned LEB128.
pub fn var(out: &mut Vec<u8>, mut v: u64) {
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

/// Event payloads.
pub mod payload {
    /// NUL-terminated UTF-16LE.
    pub fn utf16z(out: &mut Vec<u8>, s: &str) {
        for u in s.encode_utf16().chain([0]) {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }

    /// A metadata record (V4 layout with an empty field list).
    pub fn metadata(id: u32, provider: &str, event_id: u32, name: &str) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&id.to_le_bytes());
        utf16z(&mut p, provider);
        p.extend_from_slice(&event_id.to_le_bytes());
        utf16z(&mut p, name);
        p.extend_from_slice(&0u64.to_le_bytes()); // keywords
        p.extend_from_slice(&0u32.to_le_bytes()); // version
        p.extend_from_slice(&5u32.to_le_bytes()); // level
        p.extend_from_slice(&0u32.to_le_bytes()); // field count
        p
    }

    /// `ThreadSample`: 1 = external (native) code, 2 = managed code.
    pub fn sample(managed: bool) -> Vec<u8> {
        (if managed { 2u32 } else { 1 }).to_le_bytes().to_vec()
    }

    /// `MethodLoadVerbose` / `MethodDCEndVerbose`.
    pub fn method_verbose(method_id: u64, module_id: u64, start: u64, size: u32, namespace: &str, name: &str) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&method_id.to_le_bytes());
        p.extend_from_slice(&module_id.to_le_bytes());
        p.extend_from_slice(&start.to_le_bytes());
        p.extend_from_slice(&size.to_le_bytes());
        p.extend_from_slice(&0x0600_0001u32.to_le_bytes()); // token
        p.extend_from_slice(&0u32.to_le_bytes()); // flags
        utf16z(&mut p, namespace);
        utf16z(&mut p, name);
        utf16z(&mut p, "void  ()");
        p.extend_from_slice(&0u16.to_le_bytes()); // clr instance
        p
    }

    /// `MethodJittingStarted_V1`.
    pub fn jitting_started(method_id: u64, module_id: u64, namespace: &str, name: &str) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&method_id.to_le_bytes());
        p.extend_from_slice(&module_id.to_le_bytes());
        p.extend_from_slice(&0x0600_0001u32.to_le_bytes());
        p.extend_from_slice(&64u32.to_le_bytes()); // IL size
        utf16z(&mut p, namespace);
        utf16z(&mut p, name);
        utf16z(&mut p, "void  ()");
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    /// `ModuleLoad_V2` / `ModuleDCEnd_V2`.
    pub fn module_load(module_id: u64, il_path: &str) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&module_id.to_le_bytes());
        p.extend_from_slice(&(module_id + 1).to_le_bytes()); // assembly
        p.extend_from_slice(&0u32.to_le_bytes()); // flags
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved
        utf16z(&mut p, il_path);
        utf16z(&mut p, "");
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    /// `GCStart_V2`.
    pub fn gc_start(count: u32, generation: u32, reason: u32) -> Vec<u8> {
        let mut p = Vec::new();
        for v in [count, generation, reason, 0] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend_from_slice(&0u64.to_le_bytes());
        p
    }

    /// `GCEnd_V1`.
    pub fn gc_end(count: u32, generation: u32) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&count.to_le_bytes());
        p.extend_from_slice(&generation.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    /// `ExceptionThrown_V1`.
    pub fn exception(type_name: &str, message: &str) -> Vec<u8> {
        let mut p = Vec::new();
        utf16z(&mut p, type_name);
        utf16z(&mut p, message);
        p.extend_from_slice(&0u64.to_le_bytes()); // EIP
        p.extend_from_slice(&0x8013_1500u32.to_le_bytes()); // HRESULT
        p.extend_from_slice(&0u16.to_le_bytes()); // flags
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    /// `GCAllocationTick_V3` with a pointer-sized type id and address.
    pub fn alloc_tick(bytes: u64, kind: u32, type_name: &str, ptr: usize) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&(bytes.min(u64::from(u32::MAX)) as u32).to_le_bytes());
        p.extend_from_slice(&kind.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend_from_slice(&bytes.to_le_bytes());
        p.extend_from_slice(&0x7f00_1234u64.to_le_bytes()[..ptr]); // type id
        utf16z(&mut p, type_name);
        p.extend_from_slice(&0u32.to_le_bytes()); // heap index
        p.extend_from_slice(&0x0200_0000u64.to_le_bytes()[..ptr]); // address
        p
    }

    /// `ThreadPoolWorkerThreadAdjustmentAdjustment`.
    pub fn pool_adjust(workers: u32, reason: u32) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&2.5f64.to_le_bytes());
        p.extend_from_slice(&workers.to_le_bytes());
        p.extend_from_slice(&reason.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    /// `ContentionStart_V1` / `ContentionStop`.
    pub fn contention(native: bool) -> Vec<u8> {
        let mut p = vec![u8::from(native)];
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip() {
        for v in [0u64, 1, 127, 128, 300, u64::from(u32::MAX), u64::MAX] {
            let mut b = Vec::new();
            var(&mut b, v);
            let mut c = super::super::cursor::Cursor::new(&b, 0);
            assert_eq!(c.var_u64().unwrap(), v);
            assert!(c.is_empty());
        }
    }

    #[test]
    fn blocks_are_aligned_and_rows_padded() {
        let mut w = Writer::new(&TraceHeader::default());
        w.block("X", 2, &[1, 2, 3]);
        let bytes = w.finish();
        let pos = bytes.windows(3).position(|x| x == [1, 2, 3]).unwrap();
        assert!(pos.is_multiple_of(4));
        let mut p = Vec::new();
        uncompressed_row(
            &mut p,
            &Row {
                payload: vec![9],
                ..Row::default()
            },
        );
        assert_eq!(p.len(), 4 + 80);
    }
}

/// A NetTrace V6 writer (tests and fuzz seeds): header, blocks with a 4-byte size/kind header, no
/// alignment, varuint-length UTF-8 strings, threads by index.
pub mod v6 {
    use super::var;

    /// Block kinds.
    pub const TRACE: u8 = 1;
    pub const EVENT: u8 = 2;
    pub const METADATA: u8 = 3;
    pub const SEQUENCE_POINT: u8 = 4;
    pub const STACK: u8 = 5;
    pub const THREAD: u8 = 6;
    pub const LABEL_LIST: u8 = 8;

    pub fn string(out: &mut Vec<u8>, s: &str) {
        var(out, s.len() as u64);
        out.extend_from_slice(s.as_bytes());
    }

    /// An event row: (metadata id, thread index, stack id, timestamp, payload).
    pub type Event<'a> = (u32, u64, u32, i64, &'a [u8]);

    pub struct V6Writer {
        out: Vec<u8>,
        pointer_size: u32,
    }

    impl V6Writer {
        /// Header and Trace block.
        pub fn new(major: u32, pointer_size: u32, process_id: u32, sampling_ns: u32) -> V6Writer {
            let mut w = V6Writer {
                out: b"Nettrace".to_vec(),
                pointer_size,
            };
            for v in [0u32, major, 0] {
                w.out.extend_from_slice(&v.to_le_bytes());
            }
            let mut p = Vec::new();
            for v in [2026u16, 10, 2, 20, 8, 0, 0, 0] {
                p.extend_from_slice(&v.to_le_bytes());
            }
            p.extend_from_slice(&1_000_000i64.to_le_bytes());
            p.extend_from_slice(&10_000_000i64.to_le_bytes());
            p.extend_from_slice(&pointer_size.to_le_bytes());
            p.extend_from_slice(&3u32.to_le_bytes());
            for (k, v) in [
                ("ProcessId", process_id.to_string()),
                ("HardwareThreadCount", "8".to_string()),
                ("ExpectedCPUSamplingRate", sampling_ns.to_string()),
            ] {
                string(&mut p, k);
                string(&mut p, &v);
            }
            w.block(TRACE, &p);
            w
        }

        pub fn block(&mut self, kind: u8, payload: &[u8]) -> &mut Self {
            self.out
                .extend_from_slice(&((payload.len() as u32) | (u32::from(kind) << 24)).to_le_bytes());
            self.out.extend_from_slice(payload);
            self
        }

        /// Metadata rows: (id, provider, event id, name).
        pub fn metadata(&mut self, rows: &[(u32, &str, u32, &str)]) -> &mut Self {
            let mut p = 0u16.to_le_bytes().to_vec();
            for (id, provider, event_id, name) in rows {
                let mut r = Vec::new();
                var(&mut r, u64::from(*id));
                string(&mut r, provider);
                var(&mut r, u64::from(*event_id));
                string(&mut r, name);
                r.extend_from_slice(&0u16.to_le_bytes()); // no field descriptions
                r.extend_from_slice(&2u16.to_le_bytes()); // optional data: Version 1
                r.extend_from_slice(&[9, 1]);
                p.extend_from_slice(&(r.len() as u16).to_le_bytes());
                p.extend_from_slice(&r);
            }
            self.block(METADATA, &p)
        }

        /// Thread rows: (index, OS thread id, name).
        pub fn threads(&mut self, rows: &[(u64, u64, &str)]) -> &mut Self {
            let mut p = Vec::new();
            for (index, tid, name) in rows {
                let mut r = Vec::new();
                var(&mut r, *index);
                r.push(1);
                string(&mut r, name);
                r.push(2);
                var(&mut r, 4242);
                r.push(3);
                var(&mut r, *tid);
                r.push(4);
                string(&mut r, "k");
                string(&mut r, "v");
                p.extend_from_slice(&(r.len() as u16).to_le_bytes());
                p.extend_from_slice(&r);
            }
            self.block(THREAD, &p)
        }

        /// A Stack block with ids `first_id..` (addresses leaf first).
        pub fn stacks(&mut self, first_id: u32, stacks: &[Vec<u64>]) -> &mut Self {
            let mut p = Vec::new();
            p.extend_from_slice(&first_id.to_le_bytes());
            p.extend_from_slice(&(stacks.len() as u32).to_le_bytes());
            for s in stacks {
                p.extend_from_slice(&(s.len() as u32 * self.pointer_size).to_le_bytes());
                for a in s {
                    p.extend_from_slice(&a.to_le_bytes()[..self.pointer_size as usize]);
                }
            }
            self.block(STACK, &p)
        }

        /// An Event block, with V6 header compression or uncompressed rows.
        pub fn events(&mut self, compressed: bool, rows: &[Event<'_>]) -> &mut Self {
            let mut p = Vec::new();
            p.extend_from_slice(&20u16.to_le_bytes());
            p.extend_from_slice(&u16::from(compressed).to_le_bytes());
            p.extend_from_slice(&[0u8; 16]);
            let (mut meta, mut thread, mut stack, mut ts, mut len) = (0u32, 0u64, 0u32, 0i64, 0usize);
            for (m, t, s, at, payload) in rows {
                if compressed {
                    let mut fl = 0u8;
                    let mut h = Vec::new();
                    if *m != meta {
                        var(&mut h, u64::from(*m));
                        fl |= 1;
                    }
                    if *t != thread {
                        // Sequence delta, capture thread, processor.
                        var(&mut h, 0);
                        var(&mut h, *t);
                        var(&mut h, 0);
                        fl |= 2;
                        var(&mut h, *t);
                        fl |= 4;
                    }
                    if *s != stack {
                        var(&mut h, u64::from(*s));
                        fl |= 8;
                    }
                    var(&mut h, at.wrapping_sub(ts) as u64);
                    // A label list id (ignored by the reader).
                    var(&mut h, 0);
                    fl |= 0x10;
                    if payload.len() != len {
                        var(&mut h, payload.len() as u64);
                        fl |= 0x80;
                    }
                    p.push(fl);
                    p.extend_from_slice(&h);
                    p.extend_from_slice(payload);
                    (meta, thread, stack, ts, len) = (*m, *t, *s, *at, payload.len());
                } else {
                    let size = 4 + 4 + 8 + 8 + 4 + 4 + 8 + 4 + 4 + payload.len();
                    p.extend_from_slice(&(size as u32).to_le_bytes());
                    p.extend_from_slice(&m.to_le_bytes());
                    p.extend_from_slice(&1u32.to_le_bytes());
                    p.extend_from_slice(&t.to_le_bytes());
                    p.extend_from_slice(&t.to_le_bytes());
                    p.extend_from_slice(&0u32.to_le_bytes());
                    p.extend_from_slice(&s.to_le_bytes());
                    p.extend_from_slice(&at.to_le_bytes());
                    p.extend_from_slice(&0u32.to_le_bytes());
                    p.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                    p.extend_from_slice(payload);
                }
            }
            self.block(EVENT, &p)
        }

        /// A SequencePoint block (stack ids are reset after it).
        pub fn sequence_point(&mut self, ts: i64) -> &mut Self {
            let mut p = ts.to_le_bytes().to_vec();
            p.extend_from_slice(&1u32.to_le_bytes());
            p.extend_from_slice(&0u32.to_le_bytes());
            self.block(SEQUENCE_POINT, &p)
        }

        /// The stream with its EndOfStream block.
        pub fn finish(mut self) -> Vec<u8> {
            self.block(0, &[]);
            self.out
        }

        /// The stream without an EndOfStream block (a capture cut short).
        pub fn unfinished(self) -> Vec<u8> {
            self.out
        }
    }
}
