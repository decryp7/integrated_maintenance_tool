//! A minidump (or full dump) into the engine's [`Dump`]: system, process, exception, threads with walked stacks,
//! modules, unloaded modules, handles, memory and the .NET runtime. Parsing is rust-minidump's (fuzzed
//! upstream); stack walks are minidump-unwind's (call frame information when symbols provide it, frame pointers,
//! stack scanning). Everything is bounded by [`Limits`], and a damaged optional stream becomes a note, not an
//! error.

use crate::source::{load, Source};
use imt_dump::limits::clip;
use imt_dump::model::*;
use imt_dump::{DumpError, ErrorCode, Limits, Result};
use minidump::system_info::{Cpu, PointerWidth};
use minidump::{
    Minidump, MinidumpException, MinidumpHandleDataStream, MinidumpMemoryInfoList, MinidumpMiscInfo, MinidumpModuleList,
    MinidumpSystemInfo, MinidumpThreadList, MinidumpThreadNames, MinidumpUnloadedModuleList, Module as _, UnifiedMemory,
};
use minidump_unwind::{CallStack, FrameTrust, MultiSymbolProvider};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// Time all stack walks of one dump may take together; later threads keep their context frame only.
pub const WALK_BUDGET: Duration = Duration::from_secs(60);

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_FREE: u32 = 0x10000;
const MEM_IMAGE: u32 = 0x100_0000;
const MEM_MAPPED: u32 = 0x4_0000;
const MEM_PRIVATE: u32 = 0x2_0000;

/// Runs a future that never waits (the walker's symbol provider answers at once) to completion.
fn block_on<F: std::future::Future>(f: F) -> Option<F::Output> {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    for _ in 0..1_000 {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return Some(v);
        }
    }
    None
}

fn arch(cpu: Cpu) -> Arch {
    match cpu {
        Cpu::X86 => Arch::X86,
        Cpu::X86_64 => Arch::Amd64,
        Cpu::Arm64 => Arch::Arm64,
        Cpu::Arm => Arch::Arm,
        other => Arch::Other(other.to_string()),
    }
}

fn trust(t: FrameTrust) -> Trust {
    match t {
        FrameTrust::Context => Trust::Context,
        FrameTrust::CallFrameInfo => Trust::Cfi,
        FrameTrust::FramePointer => Trust::FramePointer,
        FrameTrust::Scan | FrameTrust::CfiScan | FrameTrust::None => Trust::Scan,
        FrameTrust::PreWalked => Trust::Recorded,
    }
}

/// The note list, bounded.
struct Notes<'a> {
    list: Vec<String>,
    limits: &'a Limits,
}

impl Notes<'_> {
    fn add(&mut self, note: impl Into<String>) {
        if self.list.len() < self.limits.max_notes {
            self.list.push(clip(&note.into(), self.limits.max_text));
        }
    }
}

/// Reads the minidump in `src`.
pub fn read_dump(src: &dyn Source, limits: &Limits) -> Result<Dump> {
    let loaded = load(src, limits)?;
    let mut notes = Notes { list: Vec::new(), limits };
    for n in loaded.notes {
        notes.add(n);
    }
    let dump = Minidump::read(loaded.bytes.as_slice()).map_err(|e| match e {
        minidump::Error::VersionMismatch => DumpError::new(ErrorCode::NotADump, "unsupported minidump version"),
        minidump::Error::MissingHeader | minidump::Error::MissingDirectory => {
            DumpError::invalid("the dump is cut off: its header or stream directory is missing")
        }
        other => DumpError::invalid(format!("unreadable minidump: {other}")),
    })?;
    let text = |s: &str| clip(s, limits.max_text);
    let system = dump.get_stream::<MinidumpSystemInfo>().ok();
    let misc = dump.get_stream::<MinidumpMiscInfo>().ok();
    let modules = match dump.get_stream::<MinidumpModuleList>() {
        Ok(m) => Some(m),
        Err(minidump::Error::StreamNotFound) => None,
        Err(e) => {
            notes.add(format!("The module list is damaged: {e}"));
            None
        }
    };
    let threads = match dump.get_stream::<MinidumpThreadList>() {
        Ok(t) => Some(t),
        Err(minidump::Error::StreamNotFound) => None,
        Err(e) => {
            notes.add(format!("The thread list is damaged: {e}"));
            None
        }
    };
    let exception = dump.get_stream::<MinidumpException>().ok();
    let memory = dump.get_memory().unwrap_or_default();
    let flags = dump.header.flags;
    let kind = if loaded.full || flags & 0x2 != 0 {
        DumpKind::Full
    } else if flags & 0x200 != 0 {
        DumpKind::WithHeap
    } else {
        DumpKind::Mini
    };
    let time_ms = (dump.header.time_date_stamp != 0).then(|| i64::from(dump.header.time_date_stamp) * 1000);

    // Modules, by address, with an index for frames.
    let mut out_modules = Vec::new();
    let mut index: HashMap<u64, u32> = HashMap::new();
    if let Some(list) = &modules {
        for (i, m) in list.by_addr().enumerate() {
            if i >= limits.max_modules {
                notes.add(format!("Only the first {} modules were read.", limits.max_modules));
                break;
            }
            index.insert(m.base_address(), out_modules.len() as u32);
            out_modules.push(Module {
                base: m.base_address(),
                size: m.size(),
                path: text(&m.code_file()),
                version: m.version().map(|v| text(&v)).filter(|v| !v.is_empty()),
                debug_file: m.debug_file().map(|v| text(&v)).filter(|v| !v.is_empty()),
                debug_id: m.debug_identifier().map(|d| text(&d.breakpad().to_string())),
                symbols: false,
            });
        }
    }
    let pointer = system.as_ref().map(|s| s.cpu.pointer_width());
    let narrow = |a: u64| match pointer {
        Some(PointerWidth::Bits32) => a & 0xFFFF_FFFF,
        _ => a,
    };

    let out_exception = exception.as_ref().map(|ex| {
        let rec = &ex.raw.exception_record;
        let n = (rec.number_parameters as usize).min(15);
        ExceptionInfo {
            code: rec.exception_code,
            flags: rec.exception_flags,
            address: narrow(rec.exception_address),
            thread_id: ex.get_crashing_thread_id(),
            parameters: rec.exception_information[..n].iter().map(|p| narrow(*p)).collect(),
        }
    });

    let names = dump.get_stream::<MinidumpThreadNames>().ok();
    let mut out_threads = Vec::new();
    if let (Some(list), Some(sys)) = (&threads, &system) {
        let unwind_info = minidump_unwind::SystemInfo {
            os: sys.os,
            os_version: None,
            os_build: None,
            cpu: sys.cpu,
            cpu_info: None,
            cpu_microcode_version: None,
            cpu_count: usize::from(sys.raw.number_of_processors),
        };
        let provider = MultiSymbolProvider::new();
        let empty = MinidumpModuleList::new();
        let module_list = modules.as_ref().unwrap_or(&empty);
        let started = Instant::now();
        let mut out_of_time = false;
        for (i, t) in list.threads.iter().enumerate() {
            if i >= limits.max_threads {
                notes.add(format!("Only the first {} threads were read.", limits.max_threads));
                break;
            }
            let id = t.raw.thread_id;
            let crashed = out_exception.as_ref().is_some_and(|e| e.thread_id == id);
            let context = crashed
                .then(|| exception.as_ref().and_then(|ex| ex.context(sys, misc.as_ref())))
                .flatten()
                .or_else(|| t.context(sys, misc.as_ref()));
            let mut frames = Vec::new();
            let mut truncated = false;
            if let Some(ctx) = context {
                let mut stack = CallStack::with_context(ctx.into_owned());
                let sp = stack.frames[0].context.get_stack_pointer();
                let window = t.stack_memory(&memory).and_then(|m| stack_window(m, sp, limits.max_stack_bytes));
                if out_of_time || started.elapsed() > WALK_BUDGET {
                    if !out_of_time {
                        notes.add("Walking the stacks took too long: later threads show their top frame only.");
                    }
                    out_of_time = true;
                } else {
                    let walk = minidump_unwind::walk_stack(
                        i,
                        (),
                        &mut stack,
                        window.as_ref().map(UnifiedMemory::Memory),
                        module_list,
                        &unwind_info,
                        &provider,
                    );
                    if block_on(walk).is_none() {
                        notes.add(format!("The stack walk of thread {id} did not finish."));
                    }
                }
                truncated = stack.frames.len() > limits.max_frames;
                for f in stack.frames.iter().take(limits.max_frames) {
                    let address = narrow(f.resume_address);
                    let module = out_modules
                        .iter()
                        .position(|m| m.contains(address))
                        .map(|i| i as u32)
                        .or_else(|| f.module.as_ref().and_then(|m| index.get(&m.base_address()).copied()));
                    frames.push(Frame {
                        address,
                        module,
                        function: None,
                        function_offset: None,
                        file: None,
                        line: None,
                        trust: trust(f.trust),
                    });
                }
            }
            out_threads.push(Thread {
                id,
                name: names
                    .as_ref()
                    .and_then(|n| n.get_name(id))
                    .map(|n| text(&n))
                    .filter(|n| !n.is_empty()),
                frames,
                truncated,
                suspend_count: t.raw.suspend_count,
                priority: t.raw.priority as i32,
            });
        }
    } else if threads.is_some() {
        notes.add("The dump has no system information, so its stacks cannot be walked.");
    }

    let process = process(modules.as_ref(), misc.as_ref(), dump.header.time_date_stamp, &text);
    let unloaded = dump
        .get_stream::<MinidumpUnloadedModuleList>()
        .ok()
        .map(|u| {
            u.iter()
                .take(limits.max_modules)
                .map(|m| text(imt_dump::modules::file_name(&m.code_file())))
                .collect()
        })
        .unwrap_or_default();
    let handles = dump.get_stream::<MinidumpHandleDataStream>().ok().map(|h| handles(&h, limits));
    let memory_summary = dump.get_stream::<MinidumpMemoryInfoList>().ok().map(|m| memory_info(&m));
    let dotnet = dotnet(&out_modules);
    Ok(Dump {
        kind,
        time_ms,
        system: system.as_ref().map(|s| system_info(s, &text)),
        process,
        exception: out_exception,
        threads: out_threads,
        modules: out_modules,
        unloaded_modules: unloaded,
        handles,
        memory: memory_summary,
        dotnet,
        notes: notes.list,
    })
}

/// The stack from the stack pointer up, at most `max` bytes (the walker reads no further).
fn stack_window<'a>(m: UnifiedMemory<'a, 'a>, sp: u64, max: u64) -> Option<minidump::MinidumpMemory<'a>> {
    let (base, bytes, endian) = match m {
        UnifiedMemory::Memory(m) => (m.base_address, m.bytes, m.endian),
        UnifiedMemory::Memory64(m) => (m.base_address, m.bytes, m.endian),
    };
    let off = sp.checked_sub(base).filter(|o| *o < bytes.len() as u64).unwrap_or(0) as usize;
    let len = (bytes.len() - off).min(usize::try_from(max).unwrap_or(usize::MAX));
    if len == 0 {
        return None;
    }
    let desc = minidump::format::MINIDUMP_MEMORY_DESCRIPTOR {
        start_of_memory_range: base + off as u64,
        memory: minidump::format::MINIDUMP_LOCATION_DESCRIPTOR {
            data_size: len as u32,
            rva: 0,
        },
    };
    Some(minidump::MinidumpMemory {
        desc,
        base_address: base + off as u64,
        size: len as u64,
        bytes: &bytes[off..off + len],
        endian,
    })
}

fn system_info(s: &MinidumpSystemInfo, text: &dyn Fn(&str) -> String) -> SystemInfo {
    let raw = &s.raw;
    SystemInfo {
        os: text(&s.os.long_name()),
        os_version: format!("{}.{}.{}", raw.major_version, raw.minor_version, raw.build_number),
        os_build: s.csd_version().map(|c| text(&c)).filter(|c| !c.is_empty()),
        arch: arch(s.cpu),
        processors: u32::from(raw.number_of_processors),
        cpu_info: s.cpu_info().map(|c| text(&c)).filter(|c| !c.is_empty()),
    }
}

fn process(
    modules: Option<&MinidumpModuleList>,
    misc: Option<&MinidumpMiscInfo>,
    dump_time: u32,
    text: &dyn Fn(&str) -> String,
) -> Option<ProcessInfo> {
    let path = modules.and_then(|m| m.main_module()).map(|m| text(&m.code_file()));
    let pid = misc.and_then(|m| m.raw.process_id().copied());
    let created = misc
        .and_then(|m| m.process_create_time())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .filter(|c| *c > 0);
    if path.is_none() && pid.is_none() {
        return None;
    }
    Some(ProcessInfo {
        pid,
        name: path
            .as_deref()
            .map(imt_dump::modules::file_name)
            .unwrap_or("unknown process")
            .to_string(),
        path,
        started_ms: created.map(|c| c as i64 * 1000),
        uptime_s: created.and_then(|c| u64::from(dump_time).checked_sub(c)).filter(|_| dump_time != 0),
    })
}

fn handles(h: &MinidumpHandleDataStream, limits: &Limits) -> HandleSummary {
    let mut by: BTreeMap<String, u64> = BTreeMap::new();
    let mut total = 0u64;
    for d in h.iter() {
        total += 1;
        let t = d.type_name.as_deref().filter(|t| !t.is_empty()).unwrap_or("Unknown");
        if by.len() < limits.max_handle_types || by.contains_key(t) {
            *by.entry(clip(t, limits.max_text)).or_default() += 1;
        } else {
            *by.entry("Other".into()).or_default() += 1;
        }
    }
    let mut by_type: Vec<(String, u64)> = by.into_iter().collect();
    by_type.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    HandleSummary { total, by_type }
}

fn memory_info(m: &MinidumpMemoryInfoList) -> MemorySummary {
    let mut s = MemorySummary::default();
    for r in m.iter() {
        let size = r.raw.region_size;
        s.regions += 1;
        match r.raw.state {
            MEM_COMMIT => {
                s.committed = s.committed.saturating_add(size);
                match r.raw._type {
                    MEM_IMAGE => s.image = s.image.saturating_add(size),
                    MEM_MAPPED => s.mapped = s.mapped.saturating_add(size),
                    MEM_PRIVATE => s.private = s.private.saturating_add(size),
                    _ => {}
                }
            }
            MEM_RESERVE => s.reserved = s.reserved.saturating_add(size),
            MEM_FREE => {
                s.free = s.free.saturating_add(size);
                s.largest_free = s.largest_free.max(size);
            }
            _ => {}
        }
    }
    s
}

/// The .NET runtime among the modules.
pub fn dotnet(modules: &[Module]) -> Option<DotnetRuntime> {
    modules.iter().find_map(|m| {
        let name = m.name().to_ascii_lowercase();
        let flavor = match name.as_str() {
            "clr.dll" | "mscorwks.dll" => DotnetFlavor::Framework,
            "coreclr.dll" | "libcoreclr.so" | "libcoreclr.dylib" => DotnetFlavor::Core,
            _ => return None,
        };
        Some(DotnetRuntime {
            flavor,
            version: m.version.clone(),
            module: m.path.clone(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::*;

    #[test]
    fn the_sample_crash_reads_completely() {
        let d = read_dump(&sample_crash(), &Limits::default()).unwrap();
        assert_eq!(d.kind, DumpKind::Mini);
        assert_eq!(d.time_ms, Some(i64::from(DUMP_TIME) * 1000));
        let s = d.system.as_ref().unwrap();
        assert_eq!(
            (s.arch.clone(), s.os_version.as_str(), s.processors),
            (Arch::Amd64, "10.0.19045", 8)
        );
        let p = d.process.as_ref().unwrap();
        assert_eq!((p.pid, p.name.as_str(), p.uptime_s), (Some(4120), "OrderService.exe", Some(3723)));
        let e = d.exception.as_ref().unwrap();
        assert_eq!(
            (e.code, e.thread_id, e.parameters.as_slice()),
            (0xC000_0005, 4121, &[1u64, 0x10][..])
        );
        assert_eq!(d.modules.len(), 4);
        assert_eq!(d.modules[0].name(), "OrderService.exe");
        assert_eq!(d.modules[0].debug_file.as_deref(), Some("OrderService.pdb"));
        assert!(
            d.modules[0].debug_id.as_ref().is_some_and(|id| id.len() == 33),
            "{:?}",
            d.modules[0].debug_id
        );
        assert_eq!(d.threads.len(), 3);
        let crash = &d.threads[0];
        assert_eq!(crash.name.as_deref(), Some("main"));
        assert_eq!(crash.frames[0].address, NTDLL_BASE + 0x1234);
        assert_eq!(crash.frames[0].trust, Trust::Context);
        let addrs: Vec<u64> = crash.frames.iter().map(|f| f.address).collect();
        assert!(addrs.contains(&(SHOP_BASE + 0x2100)), "{addrs:x?}");
        assert!(crash.frames.iter().all(|f| f.module.is_some()));
        let h = d.handles.as_ref().unwrap();
        assert_eq!((h.total, h.by_type[0].0.as_str()), (5, "Event"));
        let m = d.memory.unwrap();
        assert_eq!((m.regions, m.free, m.largest_free), (4, 0x4000_0000, 0x4000_0000));
        assert_eq!(m.image, 0x10000);
        assert_eq!(d.unloaded_modules, ["Legacy.dll"]);
        assert!(d.dotnet.is_none());
        imt_dump::wire::check(&Document::Dump(d.clone()), &Limits::default()).unwrap();
        // The engine's findings name the culprit.
        let f = imt_dump::findings::findings(1, &d);
        assert!(f
            .iter()
            .any(|f| f.kind == imt_dump::api::FindingKind::Culprit && f.title.contains("Shop.Orders.dll+0x2100")));
    }

    #[test]
    fn full_and_x86_dumps() {
        let full = read_dump(&sample_full(256 << 10), &Limits::default()).unwrap();
        assert_eq!(full.kind, DumpKind::Full);
        assert!(
            full.threads[0].frames.len() > 1,
            "the stacks were kept: {:?}",
            full.threads[0].frames
        );
        let x86 = read_dump(&sample_hang_x86(), &Limits::default()).unwrap();
        assert_eq!(x86.kind, DumpKind::WithHeap);
        assert_eq!(x86.system.as_ref().unwrap().arch, Arch::X86);
        assert_eq!(x86.system.as_ref().unwrap().os_build.as_deref(), Some("Service Pack 1"));
        assert!(x86.exception.is_none());
        assert_eq!(x86.threads.len(), 2);
        assert!(
            x86.threads[0].frames.iter().any(|f| f.address == 0x0040_1500),
            "{:x?}",
            x86.threads[0].frames
        );
        let net = x86.dotnet.as_ref().unwrap();
        assert_eq!((net.flavor, net.version.as_deref()), (DotnetFlavor::Framework, Some("4.8.4645.0")));
    }

    #[test]
    fn limits_bound_threads_frames_and_modules() {
        let small = Limits {
            max_threads: 1,
            max_frames: 1,
            max_modules: 2,
            max_notes: 10,
            ..Limits::default()
        };
        let d = read_dump(&sample_crash(), &small).unwrap();
        assert_eq!((d.threads.len(), d.modules.len()), (1, 2));
        assert_eq!(d.threads[0].frames.len(), 1);
        assert!(d.threads[0].truncated);
        assert!(d.notes.iter().any(|n| n.contains("threads")), "{:?}", d.notes);
    }

    #[test]
    fn damaged_dumps_fail_with_a_message() {
        let mut v = sample_crash();
        v[4] = 0;
        assert!(read_dump(&v, &Limits::default()).is_err());
        assert_eq!(
            read_dump(&b"hello world, this is not a dump!!".to_vec(), &Limits::default())
                .unwrap_err()
                .code,
            ErrorCode::NotADump
        );
        assert!(block_on(std::future::pending::<()>()).is_none());
    }
}
