//! Payload decoders for the NT kernel logger's classic events (MOF classes
//! `PerfInfo`, `Thread`, `Process`, `Image`, `StackWalk`), as documented in
//! the Windows SDK and TraceEvent's `KernelTraceEventParser`. Pointer-sized
//! fields follow the event header's 32/64-bit flag. Every decoder returns
//! `None` for a payload too short for its fields; none panics.

/// Event class GUIDs (the provider id of a classic kernel event).
pub mod guids {
    pub const PERF_INFO: u128 = 0xce1dbfb4_137e_4da6_87b0_3f59aa102cbc;
    pub const THREAD: u128 = 0x3d6fa8d1_fe05_11d0_9dda_00c04fd7ba7c;
    pub const PROCESS: u128 = 0x3d6fa8d0_fe05_11d0_9dda_00c04fd7ba7c;
    pub const IMAGE: u128 = 0x2cb15d1d_5fc1_11d2_abe1_00a0c911f518;
    pub const STACK_WALK: u128 = 0xdef2fe46_7bd6_4b80_bd94_f57fe20d0ce3;
    /// The trace header event of a log file.
    pub const EVENT_TRACE: u128 = 0x68fdd900_4a3e_11d1_84f4_0000f80464e3;
}

/// Opcodes (event types) of the kernel classes.
pub mod opcodes {
    pub const START: u8 = 1;
    pub const END: u8 = 2;
    pub const DC_START: u8 = 3;
    pub const DC_END: u8 = 4;
    pub const IMAGE_LOAD: u8 = 10;
    pub const PROCESS_DEFUNCT: u8 = 39;
    pub const CSWITCH: u8 = 36;
    pub const READY_THREAD: u8 = 50;
    pub const THREAD_SET_NAME: u8 = 72;
    pub const SAMPLED_PROFILE: u8 = 46;
    pub const STACK_WALK: u8 = 32;
}

/// Lowest kernel-mode address on 64-bit Windows (canonical upper half).
pub const KERNEL_START: u64 = 0xffff_8000_0000_0000;

/// A little-endian reader that never panics.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data }
    }

    pub fn remaining(&self) -> usize {
        self.data.len()
    }

    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > self.data.len() {
            return None;
        }
        let (head, rest) = self.data.split_at(n);
        self.data = rest;
        Some(head)
    }

    pub fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    pub fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|b| {
            let mut a = [0u8; 8];
            a.copy_from_slice(b);
            u64::from_le_bytes(a)
        })
    }

    /// A pointer of `size` bytes (4 or 8).
    pub fn ptr(&mut self, size: usize) -> Option<u64> {
        if size == 4 {
            self.u32().map(u64::from)
        } else {
            self.u64()
        }
    }

    /// NUL-terminated UTF-16 (at most `max` characters kept; the rest is skipped).
    pub fn utf16z(&mut self, max: usize) -> Option<String> {
        let mut units = Vec::new();
        loop {
            let u = self.u16()?;
            if u == 0 {
                break;
            }
            if units.len() < max {
                units.push(u);
            }
        }
        Some(String::from_utf16_lossy(&units))
    }

    /// NUL-terminated 8-bit text (Latin-1 as the kernel writes image names).
    pub fn ansiz(&mut self, max: usize) -> Option<String> {
        let mut s = String::new();
        loop {
            let b = self.u8()?;
            if b == 0 {
                break;
            }
            if s.len() < max {
                s.push(char::from(b));
            }
        }
        Some(s)
    }
}

/// `PerfInfo/SampledProfile`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampledProfile {
    pub ip: u64,
    pub thread: u32,
}

pub fn sampled_profile(p: &[u8], ptr: usize) -> Option<SampledProfile> {
    let mut r = Reader::new(p);
    Some(SampledProfile {
        ip: r.ptr(ptr)?,
        thread: r.u32()?,
    })
}

/// `Thread` start/end/rundown: (process id, thread id).
pub fn thread_group1(p: &[u8]) -> Option<(u32, u32)> {
    let mut r = Reader::new(p);
    Some((r.u32()?, r.u32()?))
}

/// `Thread/CSwitch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CSwitch {
    pub new_thread: u32,
    pub old_thread: u32,
    /// `KWAIT_REASON` of the old thread.
    pub old_wait_reason: u8,
    /// `KTHREAD_STATE` of the old thread (5 = waiting, 1 = ready).
    pub old_state: u8,
}

pub fn cswitch(p: &[u8]) -> Option<CSwitch> {
    let mut r = Reader::new(p);
    let new_thread = r.u32()?;
    let old_thread = r.u32()?;
    let _priorities = r.take(4)?;
    let old_wait_reason = r.u8()?;
    let _mode = r.u8()?;
    let old_state = r.u8()?;
    Some(CSwitch {
        new_thread,
        old_thread,
        old_wait_reason,
        old_state,
    })
}

/// `Thread/ReadyThread`: the readied thread (the header's thread readied it).
pub fn ready_thread(p: &[u8]) -> Option<u32> {
    Reader::new(p).u32()
}

/// `Thread/SetName`: (process id, thread id, name).
pub fn thread_set_name(p: &[u8], max: usize) -> Option<(u32, u32, String)> {
    let mut r = Reader::new(p);
    Some((r.u32()?, r.u32()?, r.utf16z(max)?))
}

/// `Image/Load` and rundown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageLoad {
    pub base: u64,
    pub size: u64,
    pub process_id: u32,
    pub path: String,
}

pub fn image_load(p: &[u8], ptr: usize, max: usize) -> Option<ImageLoad> {
    let mut r = Reader::new(p);
    let base = r.ptr(ptr)?;
    let size = r.ptr(ptr)?;
    let process_id = r.u32()?;
    let _checksum = r.u32()?;
    let _time_date_stamp = r.u32()?;
    let _reserved0 = r.u32()?;
    let _default_base = r.ptr(ptr)?;
    r.take(16)?;
    let path = r.utf16z(max)?;
    Some(ImageLoad {
        base,
        size,
        process_id,
        path,
    })
}

/// `Process` start/end/rundown (V3 and V4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEvent {
    pub process_id: u32,
    pub parent_id: u32,
    pub image: String,
    pub command_line: String,
}

/// Skips a `UserSID` field: a TOKEN_USER (two pointers) followed by the SID, or a 4-byte zero.
fn skip_sid(r: &mut Reader<'_>, ptr: usize) -> Option<()> {
    let mut probe = r.clone();
    if probe.u32()? == 0 {
        r.take(4)?;
        return Some(());
    }
    r.take(2 * ptr)?;
    let mut sid = r.clone();
    let _revision = sid.u8()?;
    let authorities = sid.u8()?;
    r.take(8 + 4 * usize::from(authorities))?;
    Some(())
}

pub fn process(p: &[u8], ptr: usize, version: u8, max: usize) -> Option<ProcessEvent> {
    let mut r = Reader::new(p);
    let _key = r.ptr(ptr)?;
    let process_id = r.u32()?;
    let parent_id = r.u32()?;
    let _session = r.u32()?;
    let _exit = r.u32()?;
    let _dtb = r.ptr(ptr)?;
    if version >= 4 {
        let _flags = r.u32()?;
    }
    skip_sid(&mut r, ptr)?;
    let image = r.ansiz(max)?;
    let command_line = r.utf16z(max.saturating_mul(8)).unwrap_or_default();
    Some(ProcessEvent {
        process_id,
        parent_id,
        image,
        command_line,
    })
}

/// `StackWalk/Stack`: the event's timestamp, process, thread and addresses (leaf first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackWalk {
    pub event_ts: i64,
    pub process_id: u32,
    pub thread: u32,
    pub frames: Vec<u64>,
}

/// Most frames read from one stack walk.
pub const MAX_FRAMES: usize = 1024;

pub fn stack_walk(p: &[u8], ptr: usize) -> Option<StackWalk> {
    let mut r = Reader::new(p);
    let event_ts = r.u64()? as i64;
    let process_id = r.u32()?;
    let thread = r.u32()?;
    let n = (r.remaining() / ptr).min(MAX_FRAMES);
    let mut frames = Vec::with_capacity(n);
    for _ in 0..n {
        frames.push(r.ptr(ptr)?);
    }
    Some(StackWalk {
        event_ts,
        process_id,
        thread,
        frames,
    })
}

/// Payload encoders (tests, fuzz seeds): the same layouts the decoders read.
#[doc(hidden)]
pub mod encode {
    use crate::nettrace::utf16z;

    fn ptr(p: &mut Vec<u8>, v: u64, size: usize) {
        p.extend_from_slice(&v.to_le_bytes()[..size]);
    }

    pub fn sampled_profile(ip: u64, thread: u32, size: usize) -> Vec<u8> {
        let mut p = Vec::new();
        ptr(&mut p, ip, size);
        p.extend_from_slice(&thread.to_le_bytes());
        p.extend_from_slice(&1u16.to_le_bytes());
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    pub fn thread_group1(pid: u32, tid: u32) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&pid.to_le_bytes());
        p.extend_from_slice(&tid.to_le_bytes());
        p.extend_from_slice(&[0u8; 40]);
        p
    }

    pub fn cswitch(new_thread: u32, old_thread: u32, reason: u8, state: u8) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&new_thread.to_le_bytes());
        p.extend_from_slice(&old_thread.to_le_bytes());
        p.extend_from_slice(&[8, 8, 0, 0, reason, 1, state, 0]);
        p.extend_from_slice(&[0u8; 8]);
        p
    }

    pub fn ready_thread(tid: u32) -> Vec<u8> {
        let mut p = tid.to_le_bytes().to_vec();
        p.extend_from_slice(&[0u8; 4]);
        p
    }

    pub fn thread_set_name(pid: u32, tid: u32, name: &str) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&pid.to_le_bytes());
        p.extend_from_slice(&tid.to_le_bytes());
        utf16z(&mut p, name);
        p
    }

    pub fn image_load(base: u64, size: u64, pid: u32, path: &str, s: usize) -> Vec<u8> {
        let mut p = Vec::new();
        ptr(&mut p, base, s);
        ptr(&mut p, size, s);
        p.extend_from_slice(&pid.to_le_bytes());
        p.extend_from_slice(&[0u8; 12]);
        ptr(&mut p, base, s);
        p.extend_from_slice(&[0u8; 16]);
        utf16z(&mut p, path);
        p
    }

    /// `Process` V4 with a SID of `authorities` sub-authorities (or none).
    pub fn process(pid: u32, parent: u32, image: &str, command_line: &str, sid: Option<u8>, s: usize) -> Vec<u8> {
        let mut p = Vec::new();
        ptr(&mut p, 0xffff_0000, s);
        p.extend_from_slice(&pid.to_le_bytes());
        p.extend_from_slice(&parent.to_le_bytes());
        p.extend_from_slice(&1u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        ptr(&mut p, 0x1000, s);
        p.extend_from_slice(&0u32.to_le_bytes()); // flags (V4)
        match sid {
            None => p.extend_from_slice(&0u32.to_le_bytes()),
            Some(n) => {
                ptr(&mut p, 0x1234, s);
                ptr(&mut p, 0, s);
                p.extend_from_slice(&[1, n, 0, 0, 0, 0, 0, 5]);
                for i in 0..n {
                    p.extend_from_slice(&u32::from(i).to_le_bytes());
                }
            }
        }
        p.extend_from_slice(image.as_bytes());
        p.push(0);
        utf16z(&mut p, command_line);
        p
    }

    pub fn stack_walk(ts: i64, pid: u32, tid: u32, frames: &[u64], s: usize) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&ts.to_le_bytes());
        p.extend_from_slice(&pid.to_le_bytes());
        p.extend_from_slice(&tid.to_le_bytes());
        for f in frames {
            ptr(&mut p, *f, s);
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::encode as e;
    use super::*;

    #[test]
    fn decodes_every_kernel_event_for_both_pointer_sizes() {
        for s in [4usize, 8] {
            assert_eq!(
                sampled_profile(&e::sampled_profile(0x1234, 7, s), s),
                Some(SampledProfile { ip: 0x1234, thread: 7 })
            );
            let img = image_load(&e::image_load(0x40_0000, 0x2000, 9, r"C:\a\app.exe", s), s, 100).unwrap();
            assert_eq!(
                (img.base, img.size, img.process_id, img.path.as_str()),
                (0x40_0000, 0x2000, 9, r"C:\a\app.exe")
            );
            for sid in [None, Some(0), Some(5)] {
                let p = process(&e::process(9, 1, "app.exe", "app.exe --x", sid, s), s, 4, 100).unwrap();
                assert_eq!(
                    (p.process_id, p.parent_id, p.image.as_str(), p.command_line.as_str()),
                    (9, 1, "app.exe", "app.exe --x")
                );
            }
            let w = stack_walk(&e::stack_walk(55, 9, 7, &[1, 2, 3], s), s).unwrap();
            assert_eq!((w.event_ts, w.process_id, w.thread, w.frames), (55, 9, 7, vec![1, 2, 3]));
        }
        assert_eq!(thread_group1(&e::thread_group1(9, 7)), Some((9, 7)));
        assert_eq!(
            cswitch(&e::cswitch(7, 8, 6, 5)),
            Some(CSwitch {
                new_thread: 7,
                old_thread: 8,
                old_wait_reason: 6,
                old_state: 5
            })
        );
        assert_eq!(ready_thread(&e::ready_thread(7)), Some(7));
        assert_eq!(
            thread_set_name(&e::thread_set_name(9, 7, "worker"), 100),
            Some((9, 7, "worker".into()))
        );
    }

    #[test]
    fn short_payloads_are_none_never_panics() {
        let s = 8;
        let all: Vec<Vec<u8>> = vec![
            e::sampled_profile(1, 2, s),
            e::cswitch(1, 2, 3, 4),
            e::image_load(1, 2, 3, "x", s),
            e::process(1, 2, "x", "y", Some(3), s),
            e::thread_set_name(1, 2, "n"),
        ];
        for p in &all {
            for n in 0..p.len().saturating_sub(1) {
                let cut = &p[..n];
                let _ = sampled_profile(cut, s);
                let _ = cswitch(cut);
                let _ = image_load(cut, s, 10);
                let _ = process(cut, s, 4, 10);
                let _ = thread_set_name(cut, 10);
                let _ = stack_walk(cut, s);
            }
        }
        assert!(sampled_profile(&[0; 11], 8).is_none());
        assert!(cswitch(&[0; 10]).is_none());
        assert!(stack_walk(&[0; 15], 8).is_none());
        assert!(thread_group1(&[0; 7]).is_none());
        assert!(ready_thread(&[0; 3]).is_none());
        // Names longer than the limit are cut, the rest is skipped.
        let p = e::thread_set_name(1, 2, "abcdef");
        assert_eq!(thread_set_name(&p, 3).unwrap().2, "abc");
        let mut r = Reader::new(b"ab\0");
        assert_eq!(r.ansiz(1).as_deref(), Some("a"));
        assert!(Reader::new(b"ab").ansiz(5).is_none());
    }
}
