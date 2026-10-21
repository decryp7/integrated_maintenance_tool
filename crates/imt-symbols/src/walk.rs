//! Stack walking with DbgHelp `StackWalkEx` over a [`Memory`]: a live process read with `ReadProcessMemory`
//! ([`ProcessMemory`]), a stack copied while its thread was suspended ([`Snapshot`]), or a dump.
//!
//! The walker loads the target's modules at their addresses so DbgHelp finds their unwind data (x64 `.pdata`) in
//! the image files; code without registered unwind data (JIT-compiled code on x64) ends the walk.

use crate::dbghelp::{fake_handle, set_options};
use crate::{dbghelp_lock, Module, SymbolError};
use std::cell::Cell;
use windows::core::{BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Diagnostics::Debug::*;
use windows::Win32::System::SystemInformation::{IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_I386};

/// Memory of the target, by address.
pub trait Memory {
    /// Fills `out` from `address`; returns how many bytes could be read (0: unreadable).
    fn read(&self, address: u64, out: &mut [u8]) -> usize;
}

/// The registers a walk starts from.
pub enum Context {
    /// A 64-bit thread (`GetThreadContext`).
    Amd64(Box<CONTEXT>),
    /// A 32-bit thread of a WOW64 process (`Wow64GetThreadContext`).
    X86(Box<WOW64_CONTEXT>),
}

impl Context {
    /// The stack pointer (where a stack snapshot starts).
    pub fn stack_pointer(&self) -> u64 {
        match self {
            Context::Amd64(c) => c.Rsp,
            Context::X86(c) => u64::from(c.Esp),
        }
    }

    /// The instruction pointer (the leaf frame).
    pub fn instruction_pointer(&self) -> u64 {
        match self {
            Context::Amd64(c) => c.Rip,
            Context::X86(c) => u64::from(c.Eip),
        }
    }
}

/// A process's memory through `ReadProcessMemory` (`PROCESS_VM_READ`).
pub struct ProcessMemory(pub HANDLE);

impl Memory for ProcessMemory {
    fn read(&self, address: u64, out: &mut [u8]) -> usize {
        let mut n = 0usize;
        // SAFETY: `out` is a valid buffer of its length; a failed read leaves `n` at what was read.
        let ok = unsafe {
            ReadProcessMemory(
                self.0,
                address as usize as *const core::ffi::c_void,
                out.as_mut_ptr() as *mut core::ffi::c_void,
                out.len(),
                Some(&mut n),
            )
        };
        if ok.is_ok() {
            n
        } else {
            n.min(out.len())
        }
    }
}

/// A stack copied while its thread was suspended (`bytes` from `base`), other addresses from `rest`.
pub struct Snapshot<'a> {
    pub base: u64,
    pub bytes: Vec<u8>,
    pub rest: &'a dyn Memory,
}

impl Memory for Snapshot<'_> {
    fn read(&self, address: u64, out: &mut [u8]) -> usize {
        let end = self.base + self.bytes.len() as u64;
        if address >= self.base && address < end {
            let from = (address - self.base) as usize;
            let n = (self.bytes.len() - from).min(out.len());
            out[..n].copy_from_slice(&self.bytes[from..from + n]);
            return n;
        }
        self.rest.read(address, out)
    }
}

thread_local! {
    /// The memory of the walk in progress on this thread (DbgHelp's read callback has no context argument).
    static MEMORY: Cell<Option<*const (dyn Memory + 'static)>> = const { Cell::new(None) };
}

/// Clears [`MEMORY`] when the walk ends, however it ends.
struct MemoryScope;

impl Drop for MemoryScope {
    fn drop(&mut self) {
        MEMORY.with(|m| m.set(None));
    }
}

unsafe extern "system" fn read_memory(_: HANDLE, address: u64, buffer: *mut core::ffi::c_void, size: u32, read: *mut u32) -> BOOL {
    let Some(mem) = MEMORY.with(Cell::get) else {
        return BOOL(0);
    };
    if buffer.is_null() {
        return BOOL(0);
    }
    // SAFETY: DbgHelp passes a writable buffer of `size` bytes; the memory pointer is valid while the walk runs
    // (set and cleared around it on this thread).
    let n = unsafe {
        let out = std::slice::from_raw_parts_mut(buffer as *mut u8, size as usize);
        (*mem).read(address, out)
    };
    if !read.is_null() {
        // SAFETY: DbgHelp's out parameter.
        unsafe { *read = n as u32 };
    }
    BOOL::from(n > 0)
}

unsafe extern "system" fn function_table(process: HANDLE, base: u64) -> *mut core::ffi::c_void {
    // SAFETY: called by StackWalkEx with our session, under the DbgHelp lock its caller holds.
    unsafe { SymFunctionTableAccess64(process, base) }
}

unsafe extern "system" fn module_base(process: HANDLE, address: u64) -> u64 {
    // SAFETY: as above.
    unsafe { SymGetModuleBase64(process, address) }
}

/// A DbgHelp session for walking one process's stacks.
pub struct Walker {
    handle: HANDLE,
    loaded: Vec<u64>,
}

// SAFETY: the handle is a key (or a process handle the caller keeps open); DbgHelp calls are serialised.
unsafe impl Send for Walker {}

impl Walker {
    /// A walker for the process `process` (kept open by the caller while the walker lives).
    pub fn live(process: HANDLE) -> Result<Walker, SymbolError> {
        Walker::init(process)
    }

    /// A walker without a process (memory comes only from the [`Memory`] passed to [`Walker::walk`]).
    pub fn offline() -> Result<Walker, SymbolError> {
        Walker::init(fake_handle())
    }

    fn init(handle: HANDLE) -> Result<Walker, SymbolError> {
        let _g = dbghelp_lock();
        set_options();
        // SAFETY: a process handle or a unique key; no module enumeration (we load modules ourselves).
        unsafe { SymInitializeW(handle, PCWSTR::null(), false) }.map_err(|e| SymbolError::DbgHelp {
            call: "SymInitializeW",
            message: e.message(),
        })?;
        Ok(Walker {
            handle,
            loaded: Vec::new(),
        })
    }

    /// Loads `m` (its unwind data); returns whether it is loaded.
    pub fn add_module(&mut self, m: &Module) -> bool {
        if self.loaded.contains(&m.base) {
            return true;
        }
        let path = HSTRING::from(m.path.as_os_str());
        let size = u32::try_from(m.size).unwrap_or(u32::MAX);
        let _g = dbghelp_lock();
        // SAFETY: our session; the path outlives the call.
        let base = unsafe { SymLoadModuleExW(self.handle, None, PCWSTR(path.as_ptr()), PCWSTR::null(), m.base, size, None, None) };
        if base != 0 {
            self.loaded.push(m.base);
        }
        base != 0
    }

    /// Walks the stack from `ctx`, leaf first: at most `max` frame addresses (the instruction pointer, then return
    /// addresses). Never panics; an unreadable or unknown frame ends the walk.
    pub fn walk(&mut self, ctx: &Context, memory: &dyn Memory, max: usize) -> Vec<u64> {
        let mut frames = Vec::new();
        if max == 0 {
            return frames;
        }
        let mut frame = STACKFRAME_EX {
            StackFrameSize: std::mem::size_of::<STACKFRAME_EX>() as u32,
            ..Default::default()
        };
        let flat = |offset: u64| ADDRESS64 {
            Offset: offset,
            Segment: 0,
            Mode: AddrModeFlat,
        };
        // StackWalkEx updates the context: walk a copy.
        let (machine, mut amd64, mut x86) = match ctx {
            Context::Amd64(c) => {
                frame.AddrPC = flat(c.Rip);
                frame.AddrFrame = flat(c.Rbp);
                frame.AddrStack = flat(c.Rsp);
                (IMAGE_FILE_MACHINE_AMD64.0 as u32, Some(c.clone()), None)
            }
            Context::X86(c) => {
                frame.AddrPC = flat(u64::from(c.Eip));
                frame.AddrFrame = flat(u64::from(c.Ebp));
                frame.AddrStack = flat(u64::from(c.Esp));
                (IMAGE_FILE_MACHINE_I386.0 as u32, None, Some(c.clone()))
            }
        };
        let ctx_ptr: *mut core::ffi::c_void = match (&mut amd64, &mut x86) {
            (Some(c), _) => &mut **c as *mut CONTEXT as *mut core::ffi::c_void,
            (_, Some(c)) => &mut **c as *mut WOW64_CONTEXT as *mut core::ffi::c_void,
            _ => return frames,
        };
        let _g = dbghelp_lock();
        // SAFETY: the reference outlives the walk; MemoryScope clears the pointer before this function returns.
        let erased: &'static dyn Memory = unsafe { std::mem::transmute::<&dyn Memory, &'static dyn Memory>(memory) };
        MEMORY.with(|m| m.set(Some(erased as *const dyn Memory)));
        let _scope = MemoryScope;
        while frames.len() < max {
            // SAFETY: our session; frame and context are valid and exclusively ours; the callbacks are sound.
            let ok = unsafe {
                StackWalkEx(
                    machine,
                    self.handle,
                    HANDLE::default(),
                    &mut frame,
                    ctx_ptr,
                    Some(read_memory),
                    Some(function_table),
                    Some(module_base),
                    None,
                    0,
                )
            };
            let pc = frame.AddrPC.Offset;
            if !ok.as_bool() || pc == 0 || frames.last() == Some(&pc) && frame.AddrReturn.Offset == pc {
                break;
            }
            frames.push(pc);
            if frame.AddrReturn.Offset == 0 {
                break;
            }
        }
        frames
    }
}

impl Drop for Walker {
    fn drop(&mut self) {
        let _g = dbghelp_lock();
        // SAFETY: our session.
        unsafe {
            let _ = SymCleanup(self.handle);
        }
    }
}
