//! Builds small synthetic minidumps, following `minidumpapiset.h`: header, streams and blobs, then the stream
//! directory. For tests, the fuzz seeds and the committed samples; not used by IMT itself.

pub const THREAD_LIST: u32 = 3;
pub const MODULE_LIST: u32 = 4;
pub const MEMORY_LIST: u32 = 5;
pub const EXCEPTION: u32 = 6;
pub const SYSTEM_INFO: u32 = 7;
pub const MEMORY64_LIST: u32 = 9;
pub const HANDLE_DATA: u32 = 12;
pub const UNLOADED_MODULE_LIST: u32 = 14;
pub const MISC_INFO: u32 = 15;
pub const MEMORY_INFO_LIST: u32 = 16;
pub const THREAD_NAMES: u32 = 24;

pub const ARCH_X86: u16 = 0;
pub const ARCH_AMD64: u16 = 9;

/// Time the sample dumps were written: 2026-10-14T10:41:24Z (the OrderService crash of the samples).
pub const DUMP_TIME: u32 = 1_791_974_484;
pub const APP_BASE: u64 = 0x7FF6_0000_0000;
pub const SHOP_BASE: u64 = 0x7FF7_0000_0000;
pub const NTDLL_BASE: u64 = 0x7FFA_0000_0000;
pub const KERNEL32_BASE: u64 = 0x7FFB_0000_0000;
pub const CORECLR_BASE: u64 = 0x7FFC_0000_0000;
pub const STACK_BASE: u64 = 0x0000_00C0_0000_0000;

/// `MiniDumpWithFullMemory`.
pub const FLAG_FULL: u64 = 0x2;
/// `MiniDumpWithPrivateReadWriteMemory`.
pub const FLAG_HEAP: u64 = 0x200;

/// A module: base, size, path, file version `(a, b, c, d)`, PDB name (a CodeView record).
pub type ModuleSpec<'a> = (u64, u32, &'a str, Option<(u16, u16, u16, u16)>, Option<&'a str>);

/// A thread: id, stack start, stack bytes, CPU context, name.
pub struct ThreadSpec<'a> {
    pub id: u32,
    pub stack_base: u64,
    pub stack: Vec<u8>,
    pub context: Vec<u8>,
    pub name: Option<&'a str>,
}

#[derive(Default)]
pub struct DumpBuilder {
    buf: Vec<u8>,
    dir: Vec<(u32, u32, u32)>,
    flags: u64,
    /// Full dump: thread stacks go into a Memory64List at the end.
    full: bool,
    memory64: Vec<(u64, Vec<u8>)>,
    /// Where thread stack RVAs in the thread list point when the stack is in the Memory64List.
    stack_fixups: Vec<(usize, usize)>,
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
            ..DumpBuilder::default()
        }
    }

    /// A full-memory dump: stacks are written into a `Memory64List` after every other stream.
    pub fn full(mut self) -> Self {
        self.full = true;
        self.flags |= FLAG_FULL;
        self
    }

    pub fn flags(mut self, flags: u64) -> Self {
        self.flags |= flags;
        self
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

    /// A CodeView PDB 7.0 record (`RSDS`, GUID, age, PDB name); returns its location.
    fn codeview(&mut self, pdb: &str, seed: u8) -> (u32, u32) {
        let mut v = b"RSDS".to_vec();
        v.extend((0..16u8).map(|i| i.wrapping_mul(17).wrapping_add(seed)));
        u32le(&mut v, 1);
        v.extend_from_slice(pdb.as_bytes());
        v.push(0);
        (v.len() as u32, self.blob(&v))
    }

    pub fn module_list(&mut self, modules: &[ModuleSpec]) -> &mut Self {
        let names: Vec<u32> = modules.iter().map(|m| self.string(m.2)).collect();
        let cvs: Vec<(u32, u32)> = modules
            .iter()
            .enumerate()
            .map(|(i, m)| m.4.map_or((0, 0), |pdb| self.codeview(pdb, i as u8)))
            .collect();
        let mut v = Vec::new();
        u32le(&mut v, modules.len() as u32);
        for ((m, name), cv) in modules.iter().zip(names).zip(cvs) {
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
            u32le(&mut v, cv.0); // CvRecord
            u32le(&mut v, cv.1);
            u32le(&mut v, 0); // MiscRecord
            u32le(&mut v, 0);
            u64le(&mut v, 0);
            u64le(&mut v, 0);
        }
        self.stream(MODULE_LIST, &v)
    }

    /// Threads with their stacks (a `MemoryList` for a minidump, the `Memory64List` for a full dump) and CPU
    /// contexts; names in a thread names stream.
    pub fn threads(&mut self, threads: &[ThreadSpec]) -> &mut Self {
        let mut rows = Vec::new();
        let mut memory = Vec::new();
        for t in threads {
            let stack_rva = if self.full {
                self.memory64.push((t.stack_base, t.stack.clone()));
                0
            } else {
                let rva = self.blob(&t.stack);
                memory.push((t.stack_base, t.stack.len() as u32, rva));
                rva
            };
            let ctx_rva = self.blob(&t.context);
            rows.push((t, stack_rva, ctx_rva));
        }
        let mut v = Vec::new();
        u32le(&mut v, threads.len() as u32);
        for (i, (t, stack_rva, ctx_rva)) in rows.iter().enumerate() {
            u32le(&mut v, t.id);
            u32le(&mut v, 0);
            u32le(&mut v, 0x20);
            u32le(&mut v, 0);
            u64le(&mut v, 0x00C0_0000_1000 + i as u64 * 0x2000); // TEB
            u64le(&mut v, t.stack_base);
            u32le(&mut v, t.stack.len() as u32);
            u32le(&mut v, *stack_rva);
            u32le(&mut v, t.context.len() as u32);
            u32le(&mut v, *ctx_rva);
        }
        let list_rva = self.blob(&v);
        self.dir.push((THREAD_LIST, v.len() as u32, list_rva));
        if self.full {
            // Like Windows, point the stack descriptors at their bytes inside the full-memory data.
            for (i, _) in rows.iter().enumerate() {
                self.stack_fixups
                    .push((list_rva as usize + 4 + i * 48 + 36, self.memory64.len() - rows.len() + i));
            }
        } else {
            let mut m = Vec::new();
            u32le(&mut m, memory.len() as u32);
            for (start, size, rva) in memory {
                u64le(&mut m, start);
                u32le(&mut m, size);
                u32le(&mut m, rva);
            }
            self.stream(MEMORY_LIST, &m);
        }
        let named: Vec<(u32, &str)> = threads.iter().filter_map(|t| t.name.map(|n| (t.id, n))).collect();
        if !named.is_empty() {
            let rvas: Vec<u32> = named.iter().map(|(_, n)| self.string(n)).collect();
            let mut n = Vec::new();
            u32le(&mut n, named.len() as u32);
            for ((id, _), rva) in named.iter().zip(rvas) {
                u32le(&mut n, *id);
                u64le(&mut n, u64::from(rva));
            }
            self.stream(THREAD_NAMES, &n);
        }
        self
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

    /// Handles as `(type name, count)`.
    pub fn handles(&mut self, by_type: &[(&str, u32)]) -> &mut Self {
        let names: Vec<u32> = by_type.iter().map(|(t, _)| self.string(t)).collect();
        let total: u32 = by_type.iter().map(|(_, n)| n).sum();
        let mut v = Vec::new();
        u32le(&mut v, 16);
        u32le(&mut v, 32);
        u32le(&mut v, total);
        u32le(&mut v, 0);
        let mut handle = 4u64;
        for ((_, n), name) in by_type.iter().zip(names) {
            for _ in 0..*n {
                u64le(&mut v, handle);
                handle += 4;
                u32le(&mut v, name);
                u32le(&mut v, 0);
                u32le(&mut v, 0);
                u32le(&mut v, 0x1F_0003);
                u32le(&mut v, 1);
                u32le(&mut v, 1);
            }
        }
        self.stream(HANDLE_DATA, &v)
    }

    /// Memory regions as `(base, size, state, type)`.
    pub fn memory_info(&mut self, regions: &[(u64, u64, u32, u32)]) -> &mut Self {
        let mut v = Vec::new();
        u32le(&mut v, 16);
        u32le(&mut v, 48);
        u64le(&mut v, regions.len() as u64);
        for &(base, size, state, kind) in regions {
            u64le(&mut v, base);
            u64le(&mut v, base);
            u32le(&mut v, 4);
            u32le(&mut v, 0);
            u64le(&mut v, size);
            u32le(&mut v, state);
            u32le(&mut v, 4);
            u32le(&mut v, kind);
            u32le(&mut v, 0);
        }
        self.stream(MEMORY_INFO_LIST, &v)
    }

    pub fn unloaded(&mut self, modules: &[(u64, u32, &str)]) -> &mut Self {
        let names: Vec<u32> = modules.iter().map(|m| self.string(m.2)).collect();
        let mut v = Vec::new();
        u32le(&mut v, 12);
        u32le(&mut v, 24);
        u32le(&mut v, modules.len() as u32);
        for (m, name) in modules.iter().zip(names) {
            u64le(&mut v, m.0);
            u32le(&mut v, m.1);
            u32le(&mut v, 0);
            u32le(&mut v, 0);
            u32le(&mut v, name);
        }
        self.stream(UNLOADED_MODULE_LIST, &v)
    }

    /// Writes the full-memory data (full dumps), the directory and the header.
    pub fn finish(&mut self, time: u32) -> Vec<u8> {
        if self.full {
            let list_size = 16 + self.memory64.len() * 16;
            let mut v = Vec::new();
            u64le(&mut v, self.memory64.len() as u64);
            let list_rva = self.blob(&vec![0; list_size]);
            u64le(&mut v, 0); // the data's offset, set below
            for (start, bytes) in &self.memory64 {
                u64le(&mut v, *start);
                u64le(&mut v, bytes.len() as u64);
            }
            self.buf[list_rva as usize..list_rva as usize + list_size].copy_from_slice(&v);
            self.dir.push((MEMORY64_LIST, list_size as u32, list_rva));
            // The directory goes before the memory data, as Windows writes it.
            let dir_rva = self.blob(&[]);
            let mut dir = Vec::new();
            for &(kind, size, rva) in &self.dir {
                u32le(&mut dir, kind);
                u32le(&mut dir, size);
                u32le(&mut dir, rva);
            }
            self.buf.extend_from_slice(&dir);
            let data_at = self.buf.len() as u64;
            let mut offsets = Vec::new();
            for (_, bytes) in &self.memory64 {
                offsets.push(self.buf.len());
                self.buf.extend_from_slice(bytes);
            }
            self.buf[list_rva as usize + 8..list_rva as usize + 16].copy_from_slice(&data_at.to_le_bytes());
            for (at, i) in self.stack_fixups.clone() {
                let rva = offsets.get(i).copied().unwrap_or(0) as u32;
                self.buf[at..at + 4].copy_from_slice(&rva.to_le_bytes());
            }
            self.header(dir_rva, time);
        } else {
            let dir_rva = self.blob(&[]);
            for &(kind, size, rva) in &self.dir.clone() {
                u32le(&mut self.buf, kind);
                u32le(&mut self.buf, size);
                u32le(&mut self.buf, rva);
            }
            self.header(dir_rva, time);
        }
        std::mem::take(&mut self.buf)
    }

    fn header(&mut self, dir_rva: u32, time: u32) {
        let mut h = Vec::new();
        h.extend_from_slice(b"MDMP");
        u32le(&mut h, 0x0000_A793);
        u32le(&mut h, self.dir.len() as u32);
        u32le(&mut h, dir_rva);
        u32le(&mut h, 0);
        u32le(&mut h, time);
        u64le(&mut h, self.flags);
        self.buf[..32].copy_from_slice(&h);
    }
}

/// An AMD64 `CONTEXT` (1232 bytes) with `rip`, `rsp` and `rbp` set.
pub fn amd64_context(rip: u64, rsp: u64, rbp: u64) -> Vec<u8> {
    let mut c = vec![0u8; 1232];
    c[0x30..0x34].copy_from_slice(&0x0010_000Bu32.to_le_bytes()); // CONTEXT_AMD64 | CONTROL | INTEGER | SEGMENTS
    c[0x98..0xA0].copy_from_slice(&rsp.to_le_bytes());
    c[0xA0..0xA8].copy_from_slice(&rbp.to_le_bytes());
    c[0xF8..0x100].copy_from_slice(&rip.to_le_bytes());
    c
}

/// An x86 `CONTEXT` (716 bytes) with `eip`, `esp` and `ebp` set.
pub fn x86_context(eip: u32, esp: u32, ebp: u32) -> Vec<u8> {
    let mut c = vec![0u8; 716];
    c[0..4].copy_from_slice(&0x0001_0007u32.to_le_bytes()); // CONTEXT_i386 | CONTROL | INTEGER | SEGMENTS
    c[0xB4..0xB8].copy_from_slice(&ebp.to_le_bytes());
    c[0xB8..0xBC].copy_from_slice(&eip.to_le_bytes());
    c[0xC4..0xC8].copy_from_slice(&esp.to_le_bytes());
    c
}

/// A stack of 64-bit little-endian words.
pub fn stack64(words: &[u64]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// A stack of 32-bit little-endian words.
pub fn stack32(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// A frame-pointer chain on a stack at `base`: `frames` return addresses, each frame `[saved rbp, return]`,
/// padded to `size` bytes. Returns the stack and the first frame's rbp.
pub fn chain64(base: u64, returns: &[u64], size: usize) -> (Vec<u8>, u64) {
    let mut words = vec![0u64; size / 8];
    // Frame i lives at word 2 + 4*i: [saved rbp, return address], with two locals in between.
    for (i, ret) in returns.iter().enumerate() {
        let at = 2 + 4 * i;
        if at + 1 >= words.len() {
            break;
        }
        let next = if i + 1 < returns.len() {
            base + ((2 + 4 * (i + 1)) * 8) as u64
        } else {
            0
        };
        words[at] = next;
        words[at + 1] = *ret;
    }
    (stack64(&words), base + 16)
}

fn modules() -> Vec<ModuleSpec<'static>> {
    vec![
        (
            APP_BASE,
            0x10000,
            r"C:\Program Files\Shop\OrderService.exe",
            Some((1, 4, 0, 0)),
            Some("OrderService.pdb"),
        ),
        (
            SHOP_BASE,
            0x20000,
            r"C:\Program Files\Shop\Shop.Orders.dll",
            Some((1, 4, 0, 0)),
            Some("Shop.Orders.pdb"),
        ),
        (
            NTDLL_BASE,
            0x20000,
            r"C:\Windows\System32\ntdll.dll",
            Some((10, 0, 19041, 3636)),
            Some("ntdll.pdb"),
        ),
        (
            KERNEL32_BASE,
            0x10000,
            r"C:\Windows\System32\kernel32.dll",
            Some((10, 0, 19041, 3636)),
            Some("kernel32.pdb"),
        ),
    ]
}

/// A thread stopped at `rip` with a frame-pointer chain through `returns`.
pub fn thread(id: u32, slot: u64, rip: u64, returns: &[u64], name: Option<&'static str>) -> ThreadSpec<'static> {
    let base = STACK_BASE + slot * 0x10_0000;
    let (stack, rbp) = chain64(base, returns, 512);
    ThreadSpec {
        id,
        stack_base: base,
        stack,
        context: amd64_context(rip, base, rbp),
        name,
    }
}

/// The sample crash: `OrderService.exe` (pid 4120, x64) dies with an access violation writing `0x10` in
/// `ntdll.dll+0x1234`, called from `Shop.Orders.dll+0x2100` (the culprit) and `OrderService.exe+0x1100`; two worker
/// threads wait in `ntdll.dll+0x3000`, both called from `Shop.Orders.dll+0x4000`. Handles, memory info, an
/// unloaded module.
pub fn sample_crash() -> Vec<u8> {
    crash_builder(DumpBuilder::new())
}

fn crash_builder(mut b: DumpBuilder) -> Vec<u8> {
    let crash = thread(
        4121,
        0,
        NTDLL_BASE + 0x1234,
        &[SHOP_BASE + 0x2100, APP_BASE + 0x1100, KERNEL32_BASE + 0x17],
        Some("main"),
    );
    let ctx = crash.context.clone();
    let threads = [
        crash,
        thread(
            4122,
            1,
            NTDLL_BASE + 0x3000,
            &[SHOP_BASE + 0x4000, KERNEL32_BASE + 0x17],
            Some("worker-1"),
        ),
        thread(
            4123,
            2,
            NTDLL_BASE + 0x3000,
            &[SHOP_BASE + 0x4000, KERNEL32_BASE + 0x17],
            Some("worker-2"),
        ),
    ];
    b.system_info(ARCH_AMD64, (10, 0, 19045), 8, "")
        .misc_info(4120, DUMP_TIME - 3723)
        .module_list(&modules())
        .threads(&threads)
        .exception(4121, 0xC000_0005, NTDLL_BASE + 0x1234, &[1, 0x10], &ctx)
        .handles(&[("Event", 3), ("File", 2)])
        .memory_info(&[
            (0x10000, 0x10000, 0x1000, 0x20000),
            (APP_BASE, 0x10000, 0x1000, 0x100_0000),
            (0x7FFE_0000, 0x1000, 0x2000, 0x40000),
            (0x8000_0000, 0x4000_0000, 0x10000, 0),
        ])
        .unloaded(&[(0x7FF8_0000_0000, 0x10000, r"C:\Program Files\Shop\Plugins\Legacy.dll")]);
    b.finish(DUMP_TIME)
}

/// The sample crash as a full dump (stacks in a `Memory64List`) with `bulk` bytes of other memory after them.
pub fn sample_full(bulk: usize) -> Vec<u8> {
    let mut b = DumpBuilder::new().full();
    b.memory64.push((0x1_0000_0000, vec![0xAB; bulk]));
    crash_builder(b)
}

/// A hang dump of a 32-bit process: no exception, the main thread and a worker wait.
pub fn sample_hang_x86() -> Vec<u8> {
    let app = 0x0040_0000u64;
    let ntdll = 0x7700_0000u64;
    let stack = |base: u32, ret: &[u32]| {
        let mut words = vec![0u32; 64];
        for (i, r) in ret.iter().enumerate() {
            let at = 2 + 4 * i;
            words[at] = if i + 1 < ret.len() {
                base + ((2 + 4 * (i + 1)) * 4) as u32
            } else {
                0
            };
            words[at + 1] = *r;
        }
        stack32(&words)
    };
    let threads = [
        ThreadSpec {
            id: 100,
            stack_base: 0x0019_0000,
            stack: stack(0x0019_0000, &[app as u32 + 0x1500]),
            context: x86_context(ntdll as u32 + 0x100, 0x0019_0000, 0x0019_0008),
            name: None,
        },
        ThreadSpec {
            id: 101,
            stack_base: 0x0029_0000,
            stack: stack(0x0029_0000, &[app as u32 + 0x1600]),
            context: x86_context(ntdll as u32 + 0x200, 0x0029_0000, 0x0029_0008),
            name: None,
        },
    ];
    let mut b = DumpBuilder::new().flags(FLAG_HEAP);
    b.system_info(ARCH_X86, (6, 1, 7601), 2, "Service Pack 1")
        .misc_info(900, DUMP_TIME - 60)
        .module_list(&[
            (app, 0x10000, r"C:\Legacy\Billing.exe", Some((2, 0, 0, 0)), Some("Billing.pdb")),
            (ntdll, 0x100000, r"C:\Windows\SysWOW64\ntdll.dll", Some((6, 1, 7601, 1)), None),
            (
                0x6000_0000,
                0x10000,
                r"C:\Windows\Microsoft.NET\Framework\v4.0.30319\clr.dll",
                Some((4, 8, 4645, 0)),
                None,
            ),
        ])
        .threads(&threads);
    b.finish(DUMP_TIME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_samples_parse() {
        for bytes in [sample_crash(), sample_full(1024), sample_hang_x86()] {
            let d = minidump::Minidump::read(bytes.as_slice()).unwrap();
            let t = d.get_stream::<minidump::MinidumpThreadList>().unwrap();
            assert!(!t.threads.is_empty());
            let mem = d.get_memory().unwrap();
            assert!(t.threads.iter().all(|t| t.stack_memory(&mem).is_some()));
        }
    }
}
