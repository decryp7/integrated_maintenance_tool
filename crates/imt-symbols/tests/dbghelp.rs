//! DbgHelp for real (Windows): this test binary's own functions are named with their source file, and the
//! stack of a suspended thread of this process is walked and symbolised.
#![cfg(all(windows, feature = "dbghelp"))]

use imt_symbols::walk::{Context, Memory, ProcessMemory, Snapshot, Walker};
use imt_symbols::{resolve_all, resolver, Module, ModuleSymbols, SymbolConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::Debug::{GetThreadContext, CONTEXT, CONTEXT_FULL_AMD64};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, OpenThread, ResumeThread, SuspendThread, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME,
};

fn this_module() -> Module {
    let exe = std::env::current_exe().unwrap();
    let info = imt_symbols::pe::parse_file(&exe).unwrap();
    // SAFETY: the module handle of this executable.
    let base = unsafe { GetModuleHandleW(None) }.unwrap().0 as u64;
    Module {
        path: exe,
        base,
        size: u64::from(info.image_size),
    }
}

#[inline(never)]
fn imt_symbols_marker(x: u64) -> u64 {
    std::hint::black_box(x.wrapping_mul(31).rotate_left(7))
}

#[inline(never)]
fn imt_symbols_spin(stop: &AtomicBool) -> u64 {
    let mut acc = 0u64;
    while !stop.load(Ordering::Relaxed) {
        acc = imt_symbols_marker(acc.wrapping_add(1));
    }
    acc
}

#[test]
fn names_functions_of_this_binary_with_their_source_file() {
    let module = this_module();
    let mut r = resolver(&SymbolConfig::default()).unwrap();
    assert_eq!(r.add_module(&module), ModuleSymbols::Pdb, "the test binary's PDB sits next to it");
    let addr = imt_symbols_marker as *const () as u64;
    let s = r.resolve(addr + 1).expect("a symbol");
    assert!(s.function.ends_with("imt_symbols_marker"), "{s:?}");
    assert_eq!(s.start, addr, "{s:?}");
    assert!(s.file.as_deref().is_some_and(|f| f.ends_with("dbghelp.rs")), "{s:?}");
    assert!(s.line.is_some_and(|l| l > 0));
    assert_eq!(s.module, module.name());
    // Addresses outside every loaded module, and the bounded batch helper.
    assert!(r.resolve(1).is_none());
    let all = resolve_all(r.as_mut(), std::slice::from_ref(&module), &[addr, 1, 2], 10);
    assert_eq!(all.symbols.len(), 1);
    assert_eq!(all.unresolved, 2);
    // A missing image is reported, not fatal.
    let gone = Module {
        path: std::env::temp_dir().join("imt-no-such-module.dll"),
        base: 0x7700_0000_0000,
        size: 0x1000,
    };
    assert!(matches!(r.add_module(&gone), ModuleSymbols::None { .. }));
}

#[test]
fn walks_a_suspended_thread_of_this_process() {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let s2 = stop.clone();
    let worker = std::thread::spawn(move || {
        // SAFETY: plain query of the calling thread.
        tx.send(unsafe { GetCurrentThreadId() }).unwrap();
        imt_symbols_spin(&s2)
    });
    let tid = rx.recv().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    // SAFETY: handles are closed below; the thread is resumed right after its context and stack are copied.
    let (process, thread) = unsafe {
        (
            OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, std::process::id()).unwrap(),
            OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, false, tid).unwrap(),
        )
    };
    let mut ctx = Box::new(CONTEXT {
        ContextFlags: CONTEXT_FULL_AMD64,
        ..Default::default()
    });
    let memory = ProcessMemory(process);
    // SAFETY: as above.
    let snapshot = unsafe {
        assert_ne!(SuspendThread(thread), u32::MAX);
        let ok = GetThreadContext(thread, &mut *ctx).is_ok();
        let mut bytes = vec![0u8; 64 * 1024];
        let n = memory.read(ctx.Rsp, &mut bytes);
        bytes.truncate(n);
        ResumeThread(thread);
        assert!(ok);
        Snapshot {
            base: ctx.Rsp,
            bytes,
            rest: &memory,
        }
    };
    let module = this_module();
    let mut walker = Walker::live(process).unwrap();
    assert!(walker.add_module(&module));
    let ctx = Context::Amd64(ctx);
    assert!(ctx.stack_pointer() > 0 && ctx.instruction_pointer() > 0);
    let frames = walker.walk(&ctx, &snapshot, 64);
    assert!(!frames.is_empty());
    assert!(walker.walk(&ctx, &snapshot, 0).is_empty());
    let mut r = resolver(&SymbolConfig::default()).unwrap();
    r.add_module(&module);
    let names: Vec<String> = frames.iter().filter_map(|&a| r.resolve(a)).map(|s| s.function).collect();
    assert!(
        names
            .iter()
            .any(|n| n.ends_with("imt_symbols_spin") || n.ends_with("imt_symbols_marker")),
        "{names:?}"
    );
    stop.store(true, Ordering::Relaxed);
    worker.join().unwrap();
    // SAFETY: opened above.
    unsafe {
        let _ = CloseHandle(thread);
        let _ = CloseHandle(process);
    }
}
