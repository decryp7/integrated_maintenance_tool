//! Views of a dump: frame labels, the likely culprit, what each thread is doing, the overview and the paged
//! thread and module lists. Pure functions of the model.

use crate::api::*;
use crate::exceptions;
use crate::model::{Dump, DumpKind, Frame, Module, Thread, Trust};
use crate::modules::{is_framework, wait_kind, WaitKind};

/// Frames looked at for a thread's wait state (waits are near the top of a stack).
const WAIT_DEPTH: usize = 8;

pub(crate) fn hex(v: u64) -> String {
    format!("{v:#x}")
}

fn module_of<'a>(d: &'a Dump, f: &Frame) -> Option<&'a Module> {
    f.module.and_then(|m| d.modules.get(m as usize))
}

/// `module!function+0x1d`, `module+0x1234` or the address.
pub fn frame_label(d: &Dump, f: &Frame) -> String {
    let module = module_of(d, f);
    match (module, &f.function) {
        (Some(m), Some(func)) => match f.function_offset.filter(|o| *o != 0) {
            Some(o) => format!("{}!{func}+{o:#x}", m.name()),
            None => format!("{}!{func}", m.name()),
        },
        (None, Some(func)) => func.clone(),
        (Some(m), None) => format!("{}+{:#x}", m.name(), f.address.wrapping_sub(m.base)),
        (None, None) => hex(f.address),
    }
}

/// Whether a frame is in a system or runtime module (or in no module at all, which is not user code we can
/// name either).
pub fn frame_is_framework(d: &Dump, f: &Frame) -> bool {
    module_of(d, f).is_some_and(|m| is_framework(&m.path))
}

/// The likely culprit on `thread`: the first frame in user code (a module outside the system and runtime),
/// preferring walked frames over scanned candidates.
pub fn culprit(d: &Dump, thread: &Thread) -> Option<usize> {
    let user = |f: &Frame| module_of(d, f).is_some_and(|m| !is_framework(&m.path));
    thread
        .frames
        .iter()
        .position(|f| user(f) && f.trust != Trust::Scan)
        .or_else(|| thread.frames.iter().position(user))
}

/// The thread that raised the dump's exception.
pub fn faulting_thread(d: &Dump) -> Option<&Thread> {
    let id = d.exception.as_ref()?.thread_id;
    d.threads.iter().find(|t| t.id == id)
}

/// What `thread` is doing.
pub fn thread_state(d: &Dump, thread: &Thread) -> ThreadState {
    if d.exception.as_ref().is_some_and(|e| e.thread_id == thread.id) {
        return ThreadState::Faulted;
    }
    let mut state = ThreadState::Running;
    for f in thread.frames.iter().take(WAIT_DEPTH) {
        match f.function.as_deref().and_then(wait_kind) {
            Some(WaitKind::Lock) => return ThreadState::LockWait,
            Some(WaitKind::Object) => state = ThreadState::Waiting,
            None if !frame_is_framework(d, f) => break,
            None => {}
        }
    }
    state
}

fn trust(t: Trust) -> FrameTrust {
    match t {
        Trust::Context => FrameTrust::Context,
        Trust::Cfi => FrameTrust::Cfi,
        Trust::FramePointer => FrameTrust::FramePointer,
        Trust::Scan => FrameTrust::Scan,
        Trust::Recorded => FrameTrust::Recorded,
    }
}

/// The frames of `thread` as rows.
pub fn stack(d: &Dump, thread: &Thread) -> StackDto {
    let culprit = (thread_state(d, thread) == ThreadState::Faulted)
        .then(|| culprit(d, thread))
        .flatten();
    StackDto {
        thread_id: thread.id,
        truncated: thread.truncated,
        frames: thread
            .frames
            .iter()
            .enumerate()
            .map(|(i, f)| FrameRow {
                index: i as u32,
                address: hex(f.address),
                module: module_of(d, f).map(|m| m.name().to_string()),
                function: f.function.clone(),
                label: frame_label(d, f),
                source: f.file.as_ref().map(|file| match f.line {
                    Some(l) => format!("{file}:{l}"),
                    None => file.clone(),
                }),
                trust: trust(f.trust),
                framework: frame_is_framework(d, f),
                culprit: culprit == Some(i),
            })
            .collect(),
    }
}

/// The thread's row.
pub fn thread_row(d: &Dump, t: &Thread) -> ThreadRow {
    ThreadRow {
        id: t.id,
        name: t.name.clone(),
        state: thread_state(d, t),
        frames: t.frames.len() as u32,
        top: t.frames.first().map(|f| frame_label(d, f)),
        user_frame: t
            .frames
            .iter()
            .find(|f| module_of(d, f).is_some_and(|m| !is_framework(&m.path)))
            .map(|f| frame_label(d, f)),
    }
}

fn matches(filter: &Option<String>, texts: impl IntoIterator<Item = String>) -> bool {
    match filter.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        None => true,
        Some(f) => {
            let f = f.to_lowercase();
            texts.into_iter().any(|t| t.to_lowercase().contains(&f))
        }
    }
}

/// Threads, faulting first, then lock waits, then by id.
pub fn threads(d: &Dump, req: &ThreadsRequest, max_page: u32) -> ThreadsPage {
    let mut rows: Vec<ThreadRow> = d
        .threads
        .iter()
        .filter(|t| {
            matches(
                &req.filter,
                std::iter::once(t.id.to_string())
                    .chain(t.name.clone())
                    .chain(t.frames.iter().map(|f| frame_label(d, f))),
            )
        })
        .map(|t| thread_row(d, t))
        .collect();
    let rank = |s: ThreadState| match s {
        ThreadState::Faulted => 0,
        ThreadState::LockWait => 1,
        ThreadState::Running => 2,
        ThreadState::Waiting => 3,
    };
    rows.sort_by_key(|r| (rank(r.state), r.id));
    let total = rows.len() as u32;
    let rows = rows
        .into_iter()
        .skip(req.offset as usize)
        .take(req.limit.min(max_page) as usize)
        .collect();
    ThreadsPage { rows, total }
}

/// Modules by address.
pub fn modules(d: &Dump, req: &ModulesRequest, max_page: u32) -> ModulesPage {
    let mut list: Vec<&Module> = d
        .modules
        .iter()
        .filter(|m| {
            matches(
                &req.filter,
                [
                    m.path.clone(),
                    m.version.clone().unwrap_or_default(),
                    m.debug_file.clone().unwrap_or_default(),
                ],
            )
        })
        .collect();
    list.sort_by_key(|m| m.base);
    let total = list.len() as u32;
    let rows = list
        .into_iter()
        .skip(req.offset as usize)
        .take(req.limit.min(max_page) as usize)
        .map(|m| ModuleRow {
            name: m.name().to_string(),
            path: m.path.clone(),
            base: hex(m.base),
            size: m.size,
            version: m.version.clone(),
            debug_file: m.debug_file.clone(),
            debug_id: m.debug_id.clone(),
            symbols: m.symbols,
            framework: is_framework(&m.path),
        })
        .collect();
    ModulesPage { rows, total }
}

/// The exception card.
pub fn exception_card(d: &Dump) -> Option<ExceptionCard> {
    let e = d.exception.as_ref()?;
    let known = exceptions::lookup(e.code);
    let location = faulting_thread(d)
        .and_then(|t| t.frames.first())
        .filter(|f| f.address == e.address)
        .map(|f| frame_label(d, f))
        .or_else(|| {
            d.modules
                .iter()
                .find(|m| m.contains(e.address))
                .map(|m| format!("{}+{:#x}", m.name(), e.address - m.base))
        });
    Some(ExceptionCard {
        code: exceptions::hex_code(e.code),
        name: exceptions::name(e.code),
        meaning: known.map(|k| k.meaning.to_string()),
        access: exceptions::access(e.code, &e.parameters).map(|a| format!("{} {}", a.kind.verb(), hex(a.address))),
        address: hex(e.address),
        location,
        thread_id: e.thread_id,
        fast_fail: (e.code == 0xC000_0409)
            .then(|| e.parameters.first().copied())
            .flatten()
            .and_then(exceptions::fast_fail)
            .map(str::to_string),
    })
}

/// The one-line headline of a dump.
pub fn headline(d: &Dump) -> String {
    match exception_card(d) {
        Some(c) => match &c.location {
            Some(l) => format!("{} in {l}", c.name),
            None => c.name,
        },
        None => format!("{} threads, no exception", d.threads.len()),
    }
}

pub fn dump_kind(k: DumpKind) -> &'static str {
    match k {
        DumpKind::Mini => "mini",
        DumpKind::WithHeap => "withHeap",
        DumpKind::Full => "full",
    }
}

/// `.NET 8.0.1 (coreclr.dll)`.
pub fn dotnet_label(d: &Dump) -> Option<String> {
    let r = d.dotnet.as_ref()?;
    let flavor = match r.flavor {
        crate::model::DotnetFlavor::Framework => ".NET Framework",
        crate::model::DotnetFlavor::Core => ".NET",
    };
    let module = crate::modules::file_name(&r.module);
    Some(match &r.version {
        Some(v) => format!("{flavor} {v} ({module})"),
        None => format!("{flavor} ({module})"),
    })
}

pub fn overview(d: &Dump, summary: DumpDocSummary) -> DumpOverview {
    let frames: u32 = d.threads.iter().map(|t| t.frames.len() as u32).sum();
    let named: u32 = d.threads.iter().flat_map(|t| &t.frames).filter(|f| f.function.is_some()).count() as u32;
    DumpOverview {
        summary,
        dump_kind: dump_kind(d.kind).into(),
        os: d.system.as_ref().map(|s| match &s.os_build {
            Some(b) if !b.is_empty() => format!("{} {} ({b})", s.os, s.os_version),
            _ => format!("{} {}", s.os, s.os_version),
        }),
        arch: d.system.as_ref().map(|s| s.arch.label().to_string()),
        processors: d.system.as_ref().map(|s| s.processors),
        cpu: d.system.as_ref().and_then(|s| s.cpu_info.clone()),
        process_name: d.process.as_ref().map(|p| p.name.clone()),
        process_path: d.process.as_ref().and_then(|p| p.path.clone()),
        pid: d.process.as_ref().and_then(|p| p.pid),
        started_ms: d.process.as_ref().and_then(|p| p.started_ms),
        uptime_s: d.process.as_ref().and_then(|p| p.uptime_s),
        exception: exception_card(d),
        threads: d.threads.len() as u32,
        modules: d.modules.len() as u32,
        modules_with_symbols: d.modules.iter().filter(|m| m.symbols).count() as u32,
        frames_symbolised: named,
        frames,
        handles: d.handles.as_ref().map(|h| HandlesDto {
            total: h.total,
            by_type: h
                .by_type
                .iter()
                .map(|(name, count)| HandleTypeDto {
                    name: name.clone(),
                    count: *count,
                })
                .collect(),
        }),
        memory: d.memory.map(|m| MemoryDto {
            regions: m.regions,
            committed: m.committed,
            reserved: m.reserved,
            free: m.free,
            image: m.image,
            mapped: m.mapped,
            private: m.private,
            largest_free: m.largest_free,
        }),
        dotnet: dotnet_label(d),
        unloaded_modules: d.unloaded_modules.clone(),
        notes: d.notes.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sample_dump;

    #[test]
    fn frames_are_labelled_and_the_culprit_skips_system_modules() {
        let d = sample_dump();
        let t = faulting_thread(&d).unwrap();
        assert_eq!(frame_label(&d, &t.frames[0]), "ntdll.dll!RtlFreeHeap+0x1d");
        assert_eq!(culprit(&d, t), Some(2));
        let s = stack(&d, t);
        assert!(s.frames[2].culprit && !s.frames[0].culprit && s.frames[0].framework);
        assert_eq!(s.frames[2].source.as_deref(), Some("cache.cpp:42"));
        assert_eq!(thread_state(&d, t), ThreadState::Faulted);
        let waits: Vec<ThreadState> = d.threads[1..].iter().map(|t| thread_state(&d, t)).collect();
        assert_eq!(waits, [ThreadState::LockWait, ThreadState::LockWait, ThreadState::Waiting]);
        let mut unnamed = sample_dump();
        unnamed.threads[0].frames[2].function = None;
        unnamed.threads[0].frames[2].trust = Trust::Scan;
        assert_eq!(frame_label(&unnamed, &unnamed.threads[0].frames[2]), "Shop.Orders.dll+0x1200");
        assert_eq!(culprit(&unnamed, &unnamed.threads[0]), Some(3), "walked frames are preferred");
        unnamed.threads[0].frames[3].trust = Trust::Scan;
        assert_eq!(
            culprit(&unnamed, &unnamed.threads[0]),
            Some(2),
            "a scanned user frame is still a candidate"
        );
        let bare = Frame {
            address: 0x42,
            module: None,
            function: None,
            function_offset: None,
            file: None,
            line: None,
            trust: Trust::Scan,
        };
        assert_eq!(frame_label(&d, &bare), "0x42");
    }

    #[test]
    fn exception_card_and_overview() {
        let d = sample_dump();
        let c = exception_card(&d).unwrap();
        assert_eq!(c.name, "ACCESS_VIOLATION");
        assert_eq!(c.access.as_deref(), Some("writing 0x10"));
        assert_eq!(c.location.as_deref(), Some("ntdll.dll!RtlFreeHeap+0x1d"));
        assert_eq!(headline(&d), "ACCESS_VIOLATION in ntdll.dll!RtlFreeHeap+0x1d");
        let page = threads(
            &d,
            &ThreadsRequest {
                id: 1,
                offset: 0,
                limit: 10,
                filter: None,
            },
            1000,
        );
        assert_eq!(page.total, 4);
        assert_eq!(page.rows[0].state, ThreadState::Faulted);
        assert_eq!(page.rows[0].user_frame.as_deref(), Some("Shop.Orders.dll!Cache::Evict+0x40"));
        let ledger = threads(
            &d,
            &ThreadsRequest {
                id: 1,
                offset: 0,
                limit: 10,
                filter: Some("LEDGER".into()),
            },
            1000,
        );
        assert_eq!(ledger.total, 2);
        let m = modules(
            &d,
            &ModulesRequest {
                id: 1,
                offset: 1,
                limit: 1,
                filter: None,
            },
            1000,
        );
        assert_eq!((m.total, m.rows.len()), (4, 1));
        let o = overview(&d, crate::testing::summary());
        assert_eq!(o.dump_kind, "mini");
        assert_eq!(o.arch.as_deref(), Some("x64"));
        assert!(o.frames >= o.frames_symbolised);
        assert_eq!(o.dotnet, None);
        let mut no_ex = sample_dump();
        no_ex.exception = None;
        assert_eq!(headline(&no_ex), "4 threads, no exception");
    }
}
