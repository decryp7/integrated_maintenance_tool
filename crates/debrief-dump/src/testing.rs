//! Small documents for tests (this crate's and the app's). Not used by Debrief itself.

use crate::api::{DocKind, DumpDocSummary, Severity};
use crate::model::*;

pub const APP_BASE: u64 = 0x7FF6_0000_0000;
pub const SHOP_BASE: u64 = 0x7FF7_0000_0000;
pub const NTDLL_BASE: u64 = 0x7FFA_0000_0000;
pub const UCRT_BASE: u64 = 0x7FFB_0000_0000;

fn module(base: u64, path: &str, symbols: bool) -> Module {
    let symbols = if symbols { SymbolStatus::Loaded } else { SymbolStatus::Missing };
    Module {
        base,
        size: 0x10_0000,
        path: path.into(),
        version: Some("1.0.0.0".into()),
        debug_file: Some(format!(
            "{}.pdb",
            crate::modules::file_name(path).rsplit_once('.').map_or(path, |p| p.0)
        )),
        debug_id: Some("0123456789ABCDEF0123456789ABCDEF1".into()),
        symbols,
    }
}

/// A frame in module `m` (index into [`sample_dump`]'s modules) at `rva`, in `function` at `offset`.
pub fn frame(m: u32, base: u64, rva: u64, function: &str, offset: u64, trust: Trust) -> Frame {
    Frame {
        address: base + rva,
        module: Some(m),
        function: Some(function.into()),
        function_offset: Some(offset),
        file: None,
        line: None,
        trust,
    }
}

fn lock_waiter(id: u32) -> Thread {
    Thread {
        id,
        name: Some(format!("worker-{id}")),
        frames: vec![
            frame(1, NTDLL_BASE, 0x2000, "NtWaitForAlertByThreadId", 0x14, Trust::Context),
            frame(1, NTDLL_BASE, 0x3000, "RtlpWaitOnCriticalSection", 0x80, Trust::Cfi),
            frame(1, NTDLL_BASE, 0x3400, "RtlEnterCriticalSection", 0x40, Trust::Cfi),
            frame(3, SHOP_BASE, 0x2100, "Ledger::Post", 0x22, Trust::Cfi),
            frame(0, APP_BASE, 0x1100, "main", 0x10, Trust::Cfi),
        ],
        truncated: false,
        suspend_count: 0,
        priority: 0,
    }
}

/// An x64 crash dump of `OrderService.exe`: an access violation writing `0x10` in `ntdll!RtlFreeHeap`, called
/// from `ucrtbase!free` by `Shop.Orders.dll!Cache::Evict` (the culprit); two threads wait on a critical section
/// in `Ledger::Post`, one waits on an object.
pub fn sample_dump() -> Dump {
    let mut evict = frame(3, SHOP_BASE, 0x1200, "Cache::Evict", 0x40, Trust::Cfi);
    evict.file = Some("cache.cpp".into());
    evict.line = Some(42);
    Dump {
        kind: DumpKind::Mini,
        time_ms: Some(1_791_974_484_000),
        system: Some(SystemInfo {
            os: "Windows NT".into(),
            os_version: "10.0.19045".into(),
            os_build: None,
            arch: Arch::Amd64,
            processors: 8,
            cpu_info: Some("GenuineIntel family 6".into()),
        }),
        process: Some(ProcessInfo {
            pid: Some(4120),
            name: "OrderService.exe".into(),
            path: Some("C:\\Program Files\\Shop\\OrderService.exe".into()),
            started_ms: Some(1_791_960_000_000),
            uptime_s: Some(14_484),
        }),
        exception: Some(ExceptionInfo {
            code: 0xC000_0005,
            flags: 0,
            address: NTDLL_BASE + 0x101D,
            thread_id: 4121,
            parameters: vec![1, 0x10],
        }),
        threads: vec![
            Thread {
                id: 4121,
                name: None,
                frames: vec![
                    frame(1, NTDLL_BASE, 0x101D, "RtlFreeHeap", 0x1d, Trust::Context),
                    frame(2, UCRT_BASE, 0x5014, "free", 0x14, Trust::Cfi),
                    evict,
                    frame(0, APP_BASE, 0x1110, "main", 0x10, Trust::Cfi),
                ],
                truncated: false,
                suspend_count: 0,
                priority: 0,
            },
            lock_waiter(4122),
            lock_waiter(4123),
            Thread {
                id: 4124,
                name: Some("timer".into()),
                frames: vec![
                    frame(1, NTDLL_BASE, 0x2400, "NtWaitForSingleObject", 0x14, Trust::Context),
                    frame(0, APP_BASE, 0x1200, "Timer::Run", 0x10, Trust::Cfi),
                ],
                truncated: false,
                suspend_count: 0,
                priority: 0,
            },
        ],
        modules: vec![
            module(APP_BASE, "C:\\Program Files\\Shop\\OrderService.exe", true),
            module(NTDLL_BASE, "C:\\Windows\\System32\\ntdll.dll", true),
            module(UCRT_BASE, "C:\\Windows\\System32\\ucrtbase.dll", true),
            module(SHOP_BASE, "C:\\Program Files\\Shop\\Shop.Orders.dll", true),
        ],
        unloaded_modules: Vec::new(),
        handles: Some(HandleSummary {
            total: 412,
            by_type: vec![("Event".into(), 200), ("File".into(), 120), ("Key".into(), 92)],
        }),
        memory: Some(MemorySummary {
            regions: 900,
            committed: 300 << 20,
            reserved: 600 << 20,
            free: 100 << 40,
            image: 80 << 20,
            mapped: 20 << 20,
            private: 200 << 20,
            largest_free: 90 << 40,
        }),
        dotnet: None,
        notes: Vec::new(),
    }
}

const FUNCTIONS: [&str; 3] = ["Cache::Add", "Session::New", "Report::Build"];

fn umdh_frame(module: &str, function: &str, offset: u64) -> UmdhFrame {
    UmdhFrame {
        module: Some(module.into()),
        function: Some(function.into()),
        offset: Some(offset),
        address: None,
        file: None,
        line: None,
    }
}

/// A UMDH log taken at minute `t` with `(back trace id, bytes, count)` stacks. Stack `id` allocates from
/// `Shop.Orders.dll!` [`FUNCTIONS`]`[(id - 1) % 3]` through `malloc` and `RtlAllocateHeap`. A diff has half of every
/// value on its older side.
pub fn sample_umdh(kind: UmdhKind, t: i64, stacks: &[(u64, u64, u64)]) -> UmdhLog {
    UmdhLog {
        kind,
        process: Some(UmdhProcess {
            pid: Some(4120),
            name: Some("OrderService.exe".into()),
        }),
        modules: Vec::new(),
        time_ms: Some(1_791_970_000_000 + t * 60_000),
        notes: Vec::new(),
        stacks: stacks
            .iter()
            .map(|&(id, bytes, count)| UmdhStack {
                id,
                bytes,
                count,
                old_bytes: (kind == UmdhKind::Diff).then_some(bytes / 2),
                old_count: (kind == UmdhKind::Diff).then_some(count / 2),
                largest: bytes / count.max(1),
                frames: vec![
                    umdh_frame("ntdll", "RtlpAllocateHeapInternal", 0xe),
                    umdh_frame("ntdll", "RtlAllocateHeap", 0x21),
                    umdh_frame("ucrtbase", "malloc", 0x10),
                    umdh_frame("Shop.Orders", FUNCTIONS[(id.wrapping_sub(1) % 3) as usize], 0x34),
                    umdh_frame("OrderService", "main", 0x10),
                ],
            })
            .collect(),
    }
}

/// A summary for overview tests.
pub fn summary() -> DumpDocSummary {
    DumpDocSummary {
        id: 1,
        name: "OrderService.dmp".into(),
        path: None,
        kind: DocKind::Dump,
        time_ms: Some(1_791_974_484_000),
        headline: crate::api::DumpHeadline::NoException { threads: 0 },
        process: Some("OrderService.exe".into()),
        severity: Severity::Critical,
    }
}
