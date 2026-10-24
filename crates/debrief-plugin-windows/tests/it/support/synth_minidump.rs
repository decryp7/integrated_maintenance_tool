//! Builds small synthetic minidumps for tests (shared by the unit and property tests via `#[path]`).
//!
//! Layout follows `minidumpapiset.h`: header, streams and blobs, then the stream directory.
#![allow(dead_code)]

pub const SYSTEM_INFO: u32 = 7;
pub const MISC_INFO: u32 = 15;
pub const EXCEPTION: u32 = 6;
pub const THREAD_LIST: u32 = 3;
pub const MODULE_LIST: u32 = 4;

pub const ARCH_X86: u16 = 0;
pub const ARCH_AMD64: u16 = 9;

/// Time the sample dump was written: 2026-10-14T10:00:00Z.
pub const DUMP_TIME: u32 = 1_791_972_000;
pub const APP_BASE: u64 = 0x7FF6_0000_0000;
pub const NTDLL_BASE: u64 = 0x7FFA_0000_0000;
pub const KERNEL32_BASE: u64 = 0x7FFB_0000_0000;
pub const STACK_BASE: u64 = 0x0000_00C0_0000_0000;

/// A module for [`DumpBuilder::module_list`]: base, size, path, file version `(a, b, c, d)` when present.
pub type ModuleSpec<'a> = (u64, u32, &'a str, Option<(u16, u16, u16, u16)>);

#[derive(Default)]
pub struct DumpBuilder {
    buf: Vec<u8>,
    dir: Vec<(u32, u32, u32)>,
}

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}
fn u64le(v: &mut Vec<u8>, x: u64) {
    v.extend_from_slice(&x.to_le_bytes());
}

impl DumpBuilder {
    pub fn new() -> Self {
        DumpBuilder {
            buf: vec![0; 32],
            dir: Vec::new(),
        }
    }

    /// Appends a blob (4-byte aligned) and returns its RVA.
    pub fn blob(&mut self, bytes: &[u8]) -> u32 {
        while !self.buf.len().is_multiple_of(4) {
            self.buf.push(0);
        }
        let rva = self.buf.len() as u32;
        self.buf.extend_from_slice(bytes);
        rva
    }

    pub fn stream(&mut self, kind: u32, bytes: &[u8]) -> &mut Self {
        let rva = self.blob(bytes);
        self.dir.push((kind, bytes.len() as u32, rva));
        self
    }

    /// A `MINIDUMP_STRING` (byte length + UTF-16 + terminator); returns its RVA.
    pub fn string(&mut self, s: &str) -> u32 {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut v = Vec::new();
        u32le(&mut v, (units.len() * 2) as u32);
        for u in units {
            u16le(&mut v, u);
        }
        u16le(&mut v, 0);
        self.blob(&v)
    }

    pub fn system_info(&mut self, arch: u16, (major, minor, build): (u32, u32, u32), cpus: u8, csd: &str) -> &mut Self {
        let csd_rva = self.string(csd);
        let mut v = Vec::new();
        u16le(&mut v, arch);
        u16le(&mut v, 6); // processor level
        u16le(&mut v, 0x9E0A); // revision
        v.push(cpus);
        v.push(1); // workstation
        u32le(&mut v, major);
        u32le(&mut v, minor);
        u32le(&mut v, build);
        u32le(&mut v, 2); // VER_PLATFORM_WIN32_NT
        u32le(&mut v, csd_rva);
        u16le(&mut v, 0x100);
        u16le(&mut v, 0);
        // CPU_INFORMATION (x86 view: vendor "GenuineIntel", version, features, AMD ext).
        for w in [0x756E_6547u32, 0x4965_6E69, 0x6C65_746E, 0x000906EA, 0xBFEB_FBFF, 0] {
            u32le(&mut v, w);
        }
        self.stream(SYSTEM_INFO, &v)
    }

    pub fn misc_info(&mut self, pid: u32, create_time: u32) -> &mut Self {
        let mut v = Vec::new();
        u32le(&mut v, 24);
        u32le(&mut v, 0x1 | 0x2); // PROCESS_ID | PROCESS_TIMES
        u32le(&mut v, pid);
        u32le(&mut v, create_time);
        u32le(&mut v, 3);
        u32le(&mut v, 1);
        self.stream(MISC_INFO, &v)
    }

    /// `modules`: (base, size, path, file version (a, b, c, d) when present).
    pub fn module_list(&mut self, modules: &[ModuleSpec]) -> &mut Self {
        let names: Vec<u32> = modules.iter().map(|m| self.string(m.2)).collect();
        let mut v = Vec::new();
        u32le(&mut v, modules.len() as u32);
        for (m, name) in modules.iter().zip(names) {
            u64le(&mut v, m.0);
            u32le(&mut v, m.1);
            u32le(&mut v, 0); // checksum
            u32le(&mut v, 0x6000_0000); // timestamp
            u32le(&mut v, name);
            // VS_FIXEDFILEINFO
            let (sig, ver) = match m.3 {
                Some((a, b, c, d)) => (
                    0xFEEF_04BD,
                    [(u32::from(a) << 16) | u32::from(b), (u32::from(c) << 16) | u32::from(d)],
                ),
                None => (0, [0, 0]),
            };
            u32le(&mut v, sig);
            u32le(&mut v, if sig == 0 { 0 } else { 0x0001_0000 });
            u32le(&mut v, ver[0]);
            u32le(&mut v, ver[1]);
            u32le(&mut v, ver[0]);
            u32le(&mut v, ver[1]);
            for _ in 0..7 {
                u32le(&mut v, 0);
            }
            for _ in 0..4 {
                u32le(&mut v, 0); // CvRecord, MiscRecord
            }
            u64le(&mut v, 0);
            u64le(&mut v, 0);
        }
        self.stream(MODULE_LIST, &v)
    }

    /// One thread with its stack memory at `stack_base` and a CPU context.
    pub fn thread_list(&mut self, thread_id: u32, stack_base: u64, stack: &[u8], context: &[u8]) -> &mut Self {
        let stack_rva = self.blob(stack);
        let ctx_rva = self.blob(context);
        let mut v = Vec::new();
        u32le(&mut v, 1);
        u32le(&mut v, thread_id);
        u32le(&mut v, 0);
        u32le(&mut v, 0x20);
        u32le(&mut v, 0);
        u64le(&mut v, 0x00C0_0000_1000); // TEB
        u64le(&mut v, stack_base);
        u32le(&mut v, stack.len() as u32);
        u32le(&mut v, stack_rva);
        u32le(&mut v, context.len() as u32);
        u32le(&mut v, ctx_rva);
        self.stream(THREAD_LIST, &v)
    }

    pub fn exception(&mut self, thread_id: u32, code: u32, address: u64, params: &[u64], context: &[u8]) -> &mut Self {
        let ctx_rva = self.blob(context);
        let mut v = Vec::new();
        u32le(&mut v, thread_id);
        u32le(&mut v, 0);
        u32le(&mut v, code);
        u32le(&mut v, 0); // flags
        u64le(&mut v, 0); // nested record
        u64le(&mut v, address);
        u32le(&mut v, params.len() as u32);
        u32le(&mut v, 0);
        for i in 0..15 {
            u64le(&mut v, params.get(i).copied().unwrap_or(0));
        }
        u32le(&mut v, context.len() as u32);
        u32le(&mut v, ctx_rva);
        self.stream(EXCEPTION, &v)
    }

    /// Writes the directory and header.
    pub fn finish(&mut self, time: u32) -> Vec<u8> {
        let dir_rva = self.blob(&[]);
        for &(kind, size, rva) in &self.dir.clone() {
            u32le(&mut self.buf, kind);
            u32le(&mut self.buf, size);
            u32le(&mut self.buf, rva);
        }
        let mut h = Vec::new();
        h.extend_from_slice(b"MDMP");
        u32le(&mut h, 0x0000_A793);
        u32le(&mut h, self.dir.len() as u32);
        u32le(&mut h, dir_rva);
        u32le(&mut h, 0);
        u32le(&mut h, time);
        u64le(&mut h, 0x0000_0002); // MiniDumpWithFullMemory... flags are informational
        self.buf[..32].copy_from_slice(&h);
        std::mem::take(&mut self.buf)
    }
}

/// An AMD64 `CONTEXT` (1232 bytes) with `rip` and `rsp` set.
pub fn amd64_context(rip: u64, rsp: u64) -> Vec<u8> {
    let mut c = vec![0u8; 1232];
    c[0x30..0x34].copy_from_slice(&0x0010_000Bu32.to_le_bytes()); // CONTEXT_AMD64 | CONTROL | INTEGER | SEGMENTS
    c[0x98..0xA0].copy_from_slice(&rsp.to_le_bytes());
    c[0xF8..0x100].copy_from_slice(&rip.to_le_bytes());
    c
}

/// An x86 `CONTEXT` (716 bytes) with `eip` and `esp` set.
pub fn x86_context(eip: u32, esp: u32) -> Vec<u8> {
    let mut c = vec![0u8; 716];
    c[0..4].copy_from_slice(&0x0001_0007u32.to_le_bytes()); // CONTEXT_i386 | CONTROL | INTEGER | SEGMENTS
    c[0xB8..0xBC].copy_from_slice(&eip.to_le_bytes());
    c[0xC4..0xC8].copy_from_slice(&esp.to_le_bytes());
    c
}

/// A stack of pointer-sized little-endian words.
pub fn stack64(words: &[u64]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The sample crash: `app.exe` (pid 4242, started an hour before the dump) dies with an
/// access violation reading 0x0 at `app.exe+0x1234` on thread 7; the stack holds return
/// addresses into `app.exe` and `kernel32.dll` between non-code words.
pub fn sample_crash() -> Vec<u8> {
    let rsp = STACK_BASE + 0x10;
    let stack = stack64(&[
        0xDEAD_BEEF,          // below rsp: not scanned
        0x1,                  // below rsp: not scanned
        0x0,                  // rsp
        APP_BASE + 0x2000,    // return into app.exe
        0x1234_5678,          // data
        KERNEL32_BASE + 0x17, // return into kernel32.dll
        NTDLL_BASE + 0x5AA,   // return into ntdll.dll
        0xFFFF_FFFF_FFFF_FFFF,
    ]);
    let ctx = amd64_context(APP_BASE + 0x1234, rsp);
    DumpBuilder::new()
        .system_info(ARCH_AMD64, (10, 0, 19045), 8, "")
        .misc_info(4242, DUMP_TIME - 3723)
        .module_list(&[
            (APP_BASE, 0x10000, r"C:\Program Files\App\app.exe", Some((1, 2, 3, 4))),
            (NTDLL_BASE, 0x20000, r"C:\Windows\System32\ntdll.dll", Some((10, 0, 19041, 3636))),
            (KERNEL32_BASE, 0x10000, r"C:\Windows\System32\kernel32.dll", None),
        ])
        .thread_list(7, STACK_BASE, &stack, &ctx)
        .exception(7, 0xC000_0005, APP_BASE + 0x1234, &[0, 0], &ctx)
        .finish(DUMP_TIME)
}
