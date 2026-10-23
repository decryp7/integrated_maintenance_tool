//! Debrief's extension events in NetTrace files (ADR 0019).
//!
//! ETW captures (live or converted from `.etl`) carry OS facts that NetTrace
//! has no event for. Debrief writes them under the provider [`PROVIDER`] so the
//! same importer reads ETW and EventPipe captures:
//!
//! | Event | Id | Payload (little-endian) | Thread / stack of the row |
//! |---|---|---|---|
//! | `ImageLoad` | 1 | base `u64`, size `u64`, file path UTF-16 NUL-terminated | any / none |
//! | `ThreadWait` | 2 | duration in timestamp ticks `u64`, wait reason `u8`, readying thread `u32` (0: unknown) | the waiting thread; row time = when it stopped running; stack = where it blocked |
//! | `ThreadName` | 3 | name UTF-16 NUL-terminated | the named thread |
//! | `NativeSymbol` | 4 | address `u64`, function start `u64`, line `u32` (0: unknown), function name and source file UTF-16 NUL-terminated | any / none |
//! | `HeapAlloc` | 5 | heap `u64`, address `u64`, size `u64` | the allocating thread / stack |
//! | `HeapFree` | 6 | heap `u64`, address `u64` | the freeing thread / none |
//! | `HeapReAlloc` | 7 | heap `u64`, new address `u64`, old address `u64`, new size `u64` | the reallocating thread / stack |
//!
//! `NativeSymbol` names a code address (ADR 0022: the capture's symbolisation step writes one per distinct
//! address); the heap events come from the Windows heap provider (native allocations, ADR 0022).
//!
//! Decoders return `None` for payloads too short for their fields.

use super::cursor::Cursor;

/// Provider name of Debrief's extension events.
pub const PROVIDER: &str = "Debrief-Windows-Kernel";

/// Event ids.
pub mod ids {
    pub const IMAGE_LOAD: u32 = 1;
    pub const THREAD_WAIT: u32 = 2;
    pub const THREAD_NAME: u32 = 3;
    pub const NATIVE_SYMBOL: u32 = 4;
    pub const HEAP_ALLOC: u32 = 5;
    pub const HEAP_FREE: u32 = 6;
    pub const HEAP_REALLOC: u32 = 7;
}

/// What a native code address is: the function containing it and the function's source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeSymbol {
    pub address: u64,
    /// First address of the function.
    pub start: u64,
    /// The function's first source line (0: unknown).
    pub line: u32,
    pub function: String,
    /// Source file ("" when unknown).
    pub file: String,
}

/// A native heap operation (Windows heap provider).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeapOp {
    Alloc {
        heap: u64,
        address: u64,
        size: u64,
    },
    Free {
        heap: u64,
        address: u64,
    },
    ReAlloc {
        heap: u64,
        new_address: u64,
        old_address: u64,
        size: u64,
    },
}

/// A module mapped into the process: addresses in `base..base + size` belong to `path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageLoad {
    pub base: u64,
    pub size: u64,
    pub path: String,
}

/// A thread that was switched out and later resumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadWait {
    /// Ticks of the trace's timestamp clock.
    pub duration: u64,
    /// The kernel's `KWAIT_REASON` of the switch-out.
    pub reason: u8,
    /// The thread that made it ready (0: unknown).
    pub readied_by: u32,
}

fn utf16z(out: &mut Vec<u8>, s: &str) {
    for u in s.encode_utf16().chain([0]) {
        out.extend_from_slice(&u.to_le_bytes());
    }
}

/// Encodes an `ImageLoad` payload.
pub fn image_load(image: &ImageLoad) -> Vec<u8> {
    let mut p = Vec::with_capacity(18 + image.path.len() * 2);
    p.extend_from_slice(&image.base.to_le_bytes());
    p.extend_from_slice(&image.size.to_le_bytes());
    utf16z(&mut p, &image.path);
    p
}

/// Encodes a `ThreadWait` payload.
pub fn thread_wait(wait: &ThreadWait) -> Vec<u8> {
    let mut p = Vec::with_capacity(13);
    p.extend_from_slice(&wait.duration.to_le_bytes());
    p.push(wait.reason);
    p.extend_from_slice(&wait.readied_by.to_le_bytes());
    p
}

/// Encodes a `ThreadName` payload.
pub fn thread_name(name: &str) -> Vec<u8> {
    let mut p = Vec::new();
    utf16z(&mut p, name);
    p
}

/// Encodes a `NativeSymbol` payload.
pub fn native_symbol(sym: &NativeSymbol) -> Vec<u8> {
    let mut p = Vec::with_capacity(24 + (sym.function.len() + sym.file.len()) * 2);
    p.extend_from_slice(&sym.address.to_le_bytes());
    p.extend_from_slice(&sym.start.to_le_bytes());
    p.extend_from_slice(&sym.line.to_le_bytes());
    utf16z(&mut p, &sym.function);
    utf16z(&mut p, &sym.file);
    p
}

/// The event id and payload of a heap operation.
pub fn heap_op(op: &HeapOp) -> (u32, Vec<u8>) {
    let words: &[u64] = match op {
        HeapOp::Alloc { heap, address, size } => &[*heap, *address, *size],
        HeapOp::Free { heap, address } => &[*heap, *address],
        HeapOp::ReAlloc {
            heap,
            new_address,
            old_address,
            size,
        } => &[*heap, *new_address, *old_address, *size],
    };
    let id = match op {
        HeapOp::Alloc { .. } => ids::HEAP_ALLOC,
        HeapOp::Free { .. } => ids::HEAP_FREE,
        HeapOp::ReAlloc { .. } => ids::HEAP_REALLOC,
    };
    (id, words.iter().flat_map(|w| w.to_le_bytes()).collect())
}

pub(super) fn decode_native_symbol(payload: &[u8], max_name: usize) -> Option<NativeSymbol> {
    let mut c = Cursor::new(payload, 0);
    Some(NativeSymbol {
        address: c.u64().ok()?,
        start: c.u64().ok()?,
        line: c.u32().ok()?,
        function: c.utf16z(max_name).ok()?,
        file: c.utf16z(max_name).ok()?,
    })
}

pub(super) fn decode_heap_op(id: u32, payload: &[u8]) -> Option<HeapOp> {
    let mut c = Cursor::new(payload, 0);
    let heap = c.u64().ok()?;
    match id {
        ids::HEAP_ALLOC => Some(HeapOp::Alloc {
            heap,
            address: c.u64().ok()?,
            size: c.u64().ok()?,
        }),
        ids::HEAP_FREE => Some(HeapOp::Free {
            heap,
            address: c.u64().ok()?,
        }),
        ids::HEAP_REALLOC => Some(HeapOp::ReAlloc {
            heap,
            new_address: c.u64().ok()?,
            old_address: c.u64().ok()?,
            size: c.u64().ok()?,
        }),
        _ => None,
    }
}

pub(super) fn decode_image_load(payload: &[u8], max_name: usize) -> Option<ImageLoad> {
    let mut c = Cursor::new(payload, 0);
    let base = c.u64().ok()?;
    let size = c.u64().ok()?;
    let path = c.utf16z(max_name.saturating_mul(4)).ok()?;
    Some(ImageLoad { base, size, path })
}

pub(super) fn decode_thread_wait(payload: &[u8]) -> Option<ThreadWait> {
    let mut c = Cursor::new(payload, 0);
    Some(ThreadWait {
        duration: c.u64().ok()?,
        reason: c.u8().ok()?,
        readied_by: c.u32().ok()?,
    })
}

pub(super) fn decode_thread_name(payload: &[u8], max_name: usize) -> Option<String> {
    Cursor::new(payload, 0).utf16z(max_name).ok()
}

/// What a wait was, in words, from its `KWAIT_REASON`.
pub fn wait_label(reason: u8) -> &'static str {
    match reason {
        4 | 11 => "Sleeping (delay)",
        0 | 6 | 7 | 13 => "Blocked waiting for an object",
        2 | 9 | 19 => "Blocked on paging I/O",
        15 => "Blocked on a queue (I/O completion, thread pool)",
        16 | 17 => "Blocked on an LPC/ALPC call",
        5 | 12 => "Suspended",
        21 | 37 => "Blocked on a lock (keyed event / SRW / critical section)",
        27..=29 | 34 | 35 => "Blocked on a kernel lock",
        30..=33 | 38 => "Ready, waiting for a CPU (preempted)",
        18 => "Blocked on virtual memory",
        _ => "Blocked (other wait)",
    }
}

/// Whether a wait reason means the thread was runnable (preempted), not blocked.
pub fn is_ready_wait(reason: u8) -> bool {
    matches!(reason, 30..=33 | 38)
}

/// The file name of an image path (`C:\x\y.dll` → `y.dll`).
pub fn image_file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_short_payloads() {
        let i = ImageLoad {
            base: 0x7ff0_0000,
            size: 0x1000,
            path: r"C:\Windows\System32\ntdll.dll".into(),
        };
        assert_eq!(decode_image_load(&image_load(&i), 100), Some(i.clone()));
        assert!(decode_image_load(&image_load(&i)[..15], 100).is_none());
        let w = ThreadWait {
            duration: 99,
            reason: 6,
            readied_by: 7,
        };
        assert_eq!(decode_thread_wait(&thread_wait(&w)), Some(w));
        assert!(decode_thread_wait(&thread_wait(&w)[..12]).is_none());
        assert_eq!(decode_thread_name(&thread_name("worker"), 100).as_deref(), Some("worker"));
        assert!(decode_thread_name(&[0x41], 100).is_none());
        assert_eq!(image_file_name(&i.path), "ntdll.dll");
        assert_eq!(image_file_name("a.so"), "a.so");
        let sym = NativeSymbol {
            address: 0x1234,
            start: 0x1200,
            line: 42,
            function: "busy::spin".into(),
            file: r"C:\src\main.rs".into(),
        };
        assert_eq!(decode_native_symbol(&native_symbol(&sym), 100), Some(sym.clone()));
        assert!(decode_native_symbol(&native_symbol(&sym)[..19], 100).is_none());
        for op in [
            HeapOp::Alloc {
                heap: 1,
                address: 2,
                size: 3,
            },
            HeapOp::Free { heap: 1, address: 2 },
            HeapOp::ReAlloc {
                heap: 1,
                new_address: 2,
                old_address: 3,
                size: 4,
            },
        ] {
            let (id, p) = heap_op(&op);
            assert_eq!(decode_heap_op(id, &p), Some(op));
            assert!(decode_heap_op(id, &p[..p.len() - 1]).is_none());
        }
        assert!(decode_heap_op(ids::THREAD_NAME, &[0; 32]).is_none());
    }

    #[test]
    fn labels_every_reason() {
        for r in 0..=u8::MAX {
            assert!(!wait_label(r).is_empty());
        }
        assert!(is_ready_wait(32));
        assert!(!is_ready_wait(6));
        assert_eq!(wait_label(4), "Sleeping (delay)");
    }
}
