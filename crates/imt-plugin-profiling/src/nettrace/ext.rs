//! IMT's extension events in NetTrace files (ADR 0019).
//!
//! ETW captures (live or converted from `.etl`) carry OS facts that NetTrace
//! has no event for. IMT writes them under the provider [`PROVIDER`] so the
//! same importer reads ETW and EventPipe captures:
//!
//! | Event | Id | Payload (little-endian) | Thread / stack of the row |
//! |---|---|---|---|
//! | `ImageLoad` | 1 | base `u64`, size `u64`, file path UTF-16 NUL-terminated | any / none |
//! | `ThreadWait` | 2 | duration in timestamp ticks `u64`, wait reason `u8`, readying thread `u32` (0: unknown) | the waiting thread; row time = when it stopped running; stack = where it blocked |
//! | `ThreadName` | 3 | name UTF-16 NUL-terminated | the named thread |
//!
//! Decoders return `None` for payloads too short for their fields.

use super::cursor::Cursor;

/// Provider name of IMT's extension events.
pub const PROVIDER: &str = "IMT-Windows-Kernel";

/// Event ids.
pub mod ids {
    pub const IMAGE_LOAD: u32 = 1;
    pub const THREAD_WAIT: u32 = 2;
    pub const THREAD_NAME: u32 = 3;
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
