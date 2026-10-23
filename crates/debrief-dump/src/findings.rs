//! Problem-first findings of a dump: the crash cause (decoded exception, faulting location), the likely culprit
//! frame (skipping system and runtime modules), stack overflows, heap corruption, threads waiting on locks
//! (deadlock hints), a loaded .NET runtime, missing symbols, handle counts and address-space exhaustion.
//! Findings are emitted only when the data supports them.

use crate::api::*;
use crate::exceptions::{self, AccessKind};
use crate::model::{Arch, Dump, Thread};
use crate::modules::is_framework;
use crate::umdh::fmt_bytes;
use crate::views::{culprit, faulting_thread, frame_is_framework, frame_label, hex, thread_state};
use std::collections::BTreeMap;

/// Addresses below this are a null pointer plus a field offset.
const NULL_PAGE: u64 = 0x1_0000;
/// A function repeated this often on a stack is recursion.
const RECURSION: usize = 8;
/// More open handles than this are worth a look.
const MANY_HANDLES: u64 = 10_000;
/// A 32-bit process whose largest free block is below this is out of address space.
const LOW_ADDRESS_SPACE: u64 = 64 << 20;
/// Below this share of frames with function names, symbols are reported missing.
const SYMBOLISED_SHARE: f64 = 0.5;

fn evidence(doc: u32) -> FindingEvidence {
    FindingEvidence {
        doc_id: doc,
        thread_id: None,
        frame: None,
        module: None,
        stack: None,
        threads: Vec::new(),
    }
}

fn at(doc: u32, t: &Thread, frame: Option<usize>) -> FindingEvidence {
    FindingEvidence {
        thread_id: Some(t.id),
        frame: frame.map(|f| f as u32),
        ..evidence(doc)
    }
}

/// The findings of dump `doc`, most severe first.
pub fn findings(doc: u32, d: &Dump) -> Vec<DumpFinding> {
    let mut out = Vec::new();
    crash(doc, d, &mut out);
    locks(doc, d, &mut out);
    if d.exception.is_none() {
        out.push(DumpFinding {
            kind: FindingKind::NoException,
            severity: Severity::Info,
            title: "No exception: the dump was written on request".into(),
            detail: "It shows what every thread was doing at that moment (a hang or a snapshot), not a crash. Look at threads waiting on locks or objects."
                .into(),
            evidence: evidence(doc),
        });
    }
    dotnet(doc, d, &mut out);
    resources(doc, d, &mut out);
    symbols(doc, d, &mut out);
    out.sort_by_key(|f| std::cmp::Reverse(f.severity));
    out
}

fn crash(doc: u32, d: &Dump, out: &mut Vec<DumpFinding>) {
    let Some(e) = &d.exception else { return };
    let thread = faulting_thread(d);
    let name = exceptions::name(e.code);
    let known = exceptions::lookup(e.code);
    let access = exceptions::access(e.code, &e.parameters);
    let top = thread.and_then(|t| t.frames.first()).map(|f| frame_label(d, f));
    let location = top.clone().or_else(|| {
        d.modules
            .iter()
            .find(|m| m.contains(e.address))
            .map(|m| format!("{}+{:#x}", m.name(), e.address - m.base))
    });
    let mut title = name.clone();
    if let Some(a) = access {
        title.push_str(&format!(" {} {}", a.kind.verb(), hex(a.address)));
    }
    if let Some(l) = &location {
        title.push_str(&format!(" in {l}"));
    }
    let mut detail = known.map_or_else(
        || format!("Exception code {} is not one Debrief knows.", exceptions::hex_code(e.code)),
        |k| k.meaning.to_string(),
    );
    if let Some(a) = access {
        if a.address < NULL_PAGE && a.kind != AccessKind::Execute {
            detail.push_str(&format!(
                " The address {} is near zero: a null pointer (plus a field offset {}).",
                hex(a.address),
                hex(a.address)
            ));
        }
        if a.kind == AccessKind::Execute || (a.kind != AccessKind::Other && a.address == e.address) {
            detail.push_str(" The CPU tried to execute that address: a call through a bad function pointer or into freed code.");
        }
    }
    if e.code == 0xC000_0409 {
        if let Some(f) = e.parameters.first().copied().and_then(exceptions::fast_fail) {
            detail.push_str(&format!(" Fast-fail code: {f}."));
        }
    }
    detail.push_str(&format!(" Thread {} raised it.", e.thread_id));
    out.push(DumpFinding {
        kind: FindingKind::CrashCause,
        severity: Severity::Critical,
        title,
        detail,
        evidence: match thread {
            Some(t) => at(doc, t, (!t.frames.is_empty()).then_some(0)),
            None => evidence(doc),
        },
    });
    if !d.modules.is_empty() && !d.modules.iter().any(|m| m.contains(e.address)) {
        let unloaded = if d.unloaded_modules.is_empty() {
            String::new()
        } else {
            format!(" Modules unloaded before the crash: {}.", d.unloaded_modules.join(", "))
        };
        out.push(DumpFinding {
            kind: FindingKind::BadAddress,
            severity: Severity::Warning,
            title: format!("The faulting address {} is in no loaded module", hex(e.address)),
            detail: format!("Code ran outside every loaded module: a call through a corrupted function pointer, a return to a smashed stack, or code that was unloaded.{unloaded}"),
            evidence: evidence(doc),
        });
    }
    if let Some(t) = thread {
        if let Some(i) = culprit(d, t).filter(|i| *i > 0) {
            let skipped: Vec<String> = {
                let mut names: Vec<String> = t.frames[..i]
                    .iter()
                    .filter_map(|f| f.module.and_then(|m| d.modules.get(m as usize)).map(|m| m.name().to_string()))
                    .collect();
                names.dedup();
                names
            };
            let trust = if t.frames[i].trust == crate::model::Trust::Scan {
                " It was found by scanning the stack, so it is a candidate rather than a certainty."
            } else {
                ""
            };
            out.push(DumpFinding {
                kind: FindingKind::Culprit,
                severity: Severity::Critical,
                title: format!("Likely culprit: {}", frame_label(d, &t.frames[i])),
                detail: format!(
                    "The first frame in your code on the faulting thread; the {} frame(s) above it are in {}, which crashed on what this code passed in.{trust}",
                    i,
                    if skipped.is_empty() { "system modules".into() } else { skipped.join(", ") }
                ),
                evidence: FindingEvidence {
                    module: t.frames[i].module.and_then(|m| d.modules.get(m as usize)).map(|m| m.name().to_string()),
                    ..at(doc, t, Some(i))
                },
            });
        }
        if e.code == 0xC000_00FD || e.code == 0xE053_534F {
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for f in &t.frames {
                *counts.entry(frame_label_without_offset(d, f)).or_default() += 1;
            }
            if let Some((func, n)) = counts.into_iter().filter(|(_, n)| *n >= RECURSION).max_by_key(|(_, n)| *n) {
                out.push(DumpFinding {
                    kind: FindingKind::StackOverflow,
                    severity: Severity::Critical,
                    title: format!("Recursion in {func}"),
                    detail: format!(
                        "{func} appears {n} times on the overflowing stack{}: it calls itself without reaching its end condition.",
                        if t.truncated { " (and the stack was cut)" } else { "" }
                    ),
                    evidence: at(doc, t, None),
                });
            }
        }
    }
    let heap_corruption = e.code == 0xC000_0374 || (e.code == 0xC000_0409 && e.parameters.first() == Some(&38));
    if heap_corruption {
        let exe = d.process.as_ref().map_or("the program", |p| p.name.as_str());
        out.push(DumpFinding {
            kind: FindingKind::HeapCorruption,
            severity: Severity::Critical,
            title: "The heap was corrupted earlier than the crash".into(),
            detail: format!(
                "The code that damaged the heap ran before this point and is usually not on this stack. Turn on full page heap for {exe} (gflags /p /enable {exe} /full) and reproduce: the next crash stops at the bad write."
            ),
            evidence: evidence(doc),
        });
    }
}

fn frame_label_without_offset(d: &Dump, f: &crate::model::Frame) -> String {
    let label = frame_label(d, f);
    match (&f.function, label.rfind('+')) {
        (Some(_), Some(i)) => label[..i].to_string(),
        _ => label,
    }
}

fn locks(doc: u32, d: &Dump, out: &mut Vec<DumpFinding>) {
    // Threads waiting on locks, grouped by the first frame of user code (where the lock is taken).
    let mut groups: BTreeMap<String, Vec<&Thread>> = BTreeMap::new();
    for t in &d.threads {
        if thread_state(d, t) != ThreadState::LockWait {
            continue;
        }
        let at = t
            .frames
            .iter()
            .find(|f| !frame_is_framework(d, f))
            .map_or_else(|| "system code".to_string(), |f| frame_label_without_offset(d, f));
        groups.entry(at).or_default().push(t);
    }
    if groups.is_empty() {
        return;
    }
    let deadlock = groups.len() >= 2 || d.exception.as_ref().is_some_and(|e| e.code == 0xC000_0194);
    for (place, threads) in &groups {
        let ids: Vec<u32> = threads.iter().map(|t| t.id).collect();
        let list = ids.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
        out.push(DumpFinding {
            kind: FindingKind::LockWait,
            severity: if deadlock || threads.len() >= 2 {
                Severity::Warning
            } else {
                Severity::Info
            },
            title: format!("{} thread(s) wait on a lock in {place}", threads.len()),
            detail: if deadlock {
                format!("Threads {list} are blocked acquiring a lock here while other threads wait on locks elsewhere: a possible deadlock (each holds what the other needs). Compare the lock order of these code paths.")
            } else {
                format!("Threads {list} are blocked acquiring the same kind of lock here: contention, or a deadlock with a thread that holds it while waiting on something else.")
            },
            evidence: FindingEvidence {
                threads: ids.clone(),
                ..at(doc, threads[0], None)
            },
        });
    }
}

fn dotnet(doc: u32, d: &Dump, out: &mut Vec<DumpFinding>) {
    let clr_exception = d.exception.as_ref().is_some_and(|e| matches!(e.code, 0xE043_4352 | 0xE043_4F4D));
    let Some(label) = crate::views::dotnet_label(d) else {
        if clr_exception {
            out.push(DumpFinding {
                kind: FindingKind::Dotnet,
                severity: Severity::Info,
                title: "A .NET exception, but no .NET runtime module in the dump".into(),
                detail: "The module list is missing or damaged, so the runtime version is unknown.".into(),
                evidence: evidence(doc),
            });
        }
        return;
    };
    out.push(DumpFinding {
        kind: FindingKind::Dotnet,
        severity: if clr_exception { Severity::Warning } else { Severity::Info },
        title: format!("The process runs {label}"),
        detail: format!(
            "Managed frames show as {} code without names, and managed exception objects are not read yet: that needs the runtime's data access component (mscordaccore) of exactly this version, which Debrief does not load in this version. Open the dump in Visual Studio or WinDbg with SOS (!clrstack, !pe) for the managed side.{}",
            if d.dotnet.as_ref().is_some_and(|r| r.flavor == crate::model::DotnetFlavor::Core) { "JIT-compiled" } else { "jitted or NGEN" },
            if clr_exception { " The crash is an unhandled .NET exception." } else { "" }
        ),
        evidence: evidence(doc),
    });
}

fn resources(doc: u32, d: &Dump, out: &mut Vec<DumpFinding>) {
    if let Some(h) = d.handles.as_ref().filter(|h| h.total > MANY_HANDLES) {
        let top = h
            .by_type
            .first()
            .map(|(t, n)| format!(" Most are {t} handles ({n})."))
            .unwrap_or_default();
        out.push(DumpFinding {
            kind: FindingKind::Handles,
            severity: Severity::Warning,
            title: format!("{} open handles", h.total),
            detail: format!("Far more than a process usually needs: a handle leak.{top}"),
            evidence: evidence(doc),
        });
    }
    let x86 = d.system.as_ref().is_some_and(|s| s.arch == Arch::X86 || s.arch == Arch::Arm);
    if let Some(m) = d.memory.filter(|m| x86 && m.regions > 0 && m.largest_free < LOW_ADDRESS_SPACE) {
        out.push(DumpFinding {
            kind: FindingKind::AddressSpace,
            severity: Severity::Warning,
            title: format!(
                "The 32-bit address space is nearly exhausted (largest free block {})",
                fmt_bytes(m.largest_free as i64)
            ),
            detail: format!(
                "{} committed, {} reserved: large allocations fail before memory runs out. Find what reserves memory, or build for 64-bit.",
                fmt_bytes(m.committed as i64),
                fmt_bytes(m.reserved as i64)
            ),
            evidence: evidence(doc),
        });
    }
}

fn symbols(doc: u32, d: &Dump, out: &mut Vec<DumpFinding>) {
    let frames: Vec<&crate::model::Frame> = d.threads.iter().flat_map(|t| &t.frames).filter(|f| f.module.is_some()).collect();
    if frames.is_empty() {
        return;
    }
    let named = frames.iter().filter(|f| f.function.is_some()).count();
    if (named as f64) / (frames.len() as f64) >= SYMBOLISED_SHARE {
        return;
    }
    let mut missing: Vec<&str> = d
        .modules
        .iter()
        .filter(|m| !m.symbols.is_loaded() && !is_framework(&m.path))
        .map(|m| m.name())
        .collect();
    missing.truncate(5);
    let servers_off = d.modules.iter().any(|m| m.symbols == crate::model::SymbolStatus::ServersDisabled);
    let next_step = if servers_off {
        "Enable symbol servers in Settings ▸ Symbols, or add the folder with the PDB files in Settings ▸ Dumps ▸ Symbol folders, and open the dump again."
    } else {
        "Add the folder with the PDB files in Settings ▸ Dumps ▸ Symbol folders and open the dump again."
    };
    out.push(DumpFinding {
        kind: FindingKind::MissingSymbols,
        severity: Severity::Info,
        title: format!("{named} of {} frames have function names", frames.len()),
        detail: format!(
            "Frames without symbols show as module+offset, and waits in system modules cannot be recognised. {}{}",
            next_step,
            if missing.is_empty() {
                String::new()
            } else {
                format!(" Missing for: {}.", missing.join(", "))
            }
        ),
        evidence: evidence(doc),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::testing::*;

    fn kinds(f: &[DumpFinding]) -> Vec<FindingKind> {
        f.iter().map(|f| f.kind).collect()
    }

    #[test]
    fn a_crash_leads_with_its_cause_and_culprit() {
        let f = findings(7, &sample_dump());
        assert_eq!(&kinds(&f)[..2], [FindingKind::CrashCause, FindingKind::Culprit]);
        assert_eq!(f[0].title, "ACCESS_VIOLATION writing 0x10 in ntdll.dll!RtlFreeHeap+0x1d");
        assert!(f[0].detail.contains("null pointer"));
        assert_eq!(
            (f[0].evidence.doc_id, f[0].evidence.thread_id, f[0].evidence.frame),
            (7, Some(4121), Some(0))
        );
        assert_eq!(f[1].title, "Likely culprit: Shop.Orders.dll!Cache::Evict+0x40");
        assert!(f[1].detail.contains("ntdll.dll, ucrtbase.dll"));
        assert_eq!(f[1].evidence.frame, Some(2));
        let lock = f.iter().find(|f| f.kind == FindingKind::LockWait).unwrap();
        assert_eq!(lock.title, "2 thread(s) wait on a lock in Shop.Orders.dll!Ledger::Post");
        assert_eq!(lock.evidence.threads, [4122, 4123]);
        assert_eq!(lock.severity, Severity::Warning);
        assert!(!kinds(&f).contains(&FindingKind::NoException));
        assert!(!kinds(&f).contains(&FindingKind::MissingSymbols));
    }

    #[test]
    fn hang_dumps_report_waits_and_deadlocks() {
        let mut d = sample_dump();
        d.exception = None;
        // A third waiter elsewhere makes it a possible deadlock.
        let mut other = d.threads[1].clone();
        other.id = 5000;
        other.frames[3] = frame(0, APP_BASE, 0x1300, "Inventory::Reserve", 0x8, Trust::Cfi);
        d.threads.push(other);
        let f = findings(1, &d);
        let locks: Vec<&DumpFinding> = f.iter().filter(|f| f.kind == FindingKind::LockWait).collect();
        assert_eq!(locks.len(), 2);
        assert!(locks.iter().all(|l| l.detail.contains("possible deadlock")));
        assert!(kinds(&f).contains(&FindingKind::NoException));
        let mut single = sample_dump();
        single.exception = None;
        single.threads.truncate(2);
        let one = findings(1, &single);
        assert_eq!(
            one.iter().find(|f| f.kind == FindingKind::LockWait).unwrap().severity,
            Severity::Info
        );
    }

    #[test]
    fn special_crashes() {
        let mut d = sample_dump();
        let e = d.exception.as_mut().unwrap();
        e.code = 0xC000_00FD;
        e.parameters.clear();
        let rec = frame(3, SHOP_BASE, 0x3000, "Tree::Walk", 0x12, Trust::Cfi);
        d.threads[0].frames.extend(std::iter::repeat_n(rec, 10));
        d.threads[0].truncated = true;
        let f = findings(1, &d);
        let so = f.iter().find(|f| f.kind == FindingKind::StackOverflow).unwrap();
        assert_eq!(so.title, "Recursion in Shop.Orders.dll!Tree::Walk");
        assert!(so.detail.contains("10 times") && so.detail.contains("cut"));

        let mut h = sample_dump();
        h.exception.as_mut().unwrap().code = 0xC000_0409;
        h.exception.as_mut().unwrap().parameters = vec![38];
        let f = findings(1, &h);
        assert!(f[0].detail.contains("HEAP_METADATA_CORRUPTION"));
        assert!(kinds(&f).contains(&FindingKind::HeapCorruption));

        let mut bad = sample_dump();
        bad.exception.as_mut().unwrap().address = 0x1234;
        bad.exception.as_mut().unwrap().parameters = vec![8, 0x1234];
        bad.unloaded_modules = vec!["plugin.dll".into()];
        let f = findings(1, &bad);
        let b = f.iter().find(|f| f.kind == FindingKind::BadAddress).unwrap();
        assert!(b.detail.contains("plugin.dll"));
        assert!(f[0].detail.contains("execute"));

        let mut unknown = sample_dump();
        unknown.exception.as_mut().unwrap().code = 0x1234_5678;
        unknown.threads[0].frames.clear();
        let f = findings(1, &unknown);
        assert!(f[0].detail.contains("0x12345678") && f[0].title.starts_with("0x12345678"));
        assert_eq!(f[0].evidence.frame, None);
    }

    #[test]
    fn runtime_resources_and_symbols() {
        let mut d = sample_dump();
        d.exception.as_mut().unwrap().code = 0xE043_4352;
        d.dotnet = Some(DotnetRuntime {
            flavor: DotnetFlavor::Core,
            version: Some("8.0.1".into()),
            module: "C:\\dotnet\\coreclr.dll".into(),
        });
        d.handles.as_mut().unwrap().total = 50_000;
        d.system.as_mut().unwrap().arch = Arch::X86;
        d.memory.as_mut().unwrap().largest_free = 10 << 20;
        for f in d.threads.iter_mut().flat_map(|t| t.frames.iter_mut()) {
            f.function = None;
        }
        d.modules[3].symbols = SymbolStatus::Missing;
        let f = findings(1, &d);
        let net = f.iter().find(|f| f.kind == FindingKind::Dotnet).unwrap();
        assert_eq!(net.title, "The process runs .NET 8.0.1 (coreclr.dll)");
        assert_eq!(net.severity, Severity::Warning);
        assert!(net.detail.contains("mscordaccore"));
        assert!(f.iter().any(|f| f.kind == FindingKind::Handles && f.detail.contains("Event")));
        assert!(f.iter().any(|f| f.kind == FindingKind::AddressSpace));
        let sym = f.iter().find(|f| f.kind == FindingKind::MissingSymbols).unwrap();
        assert!(sym.detail.contains("Shop.Orders.dll"));
        assert!(!sym.detail.contains("Settings ▸ Symbols"));
        d.modules[3].symbols = SymbolStatus::ServersDisabled;
        let f = findings(1, &d);
        let sym = f.iter().find(|f| f.kind == FindingKind::MissingSymbols).unwrap();
        assert!(sym.detail.contains("Enable symbol servers in Settings ▸ Symbols"), "{}", sym.detail);

        let mut no_runtime = sample_dump();
        no_runtime.exception.as_mut().unwrap().code = 0xE043_4352;
        assert!(findings(1, &no_runtime).iter().any(|f| f.title.contains("no .NET runtime module")));
        let mut fx = sample_dump();
        fx.dotnet = Some(DotnetRuntime {
            flavor: DotnetFlavor::Framework,
            version: None,
            module: "clr.dll".into(),
        });
        let f = findings(1, &fx);
        assert_eq!(f.iter().find(|f| f.kind == FindingKind::Dotnet).unwrap().severity, Severity::Info);
        let mut empty = sample_dump();
        empty.threads.clear();
        empty.exception = None;
        assert_eq!(kinds(&findings(1, &empty)), [FindingKind::NoException]);
    }
}
