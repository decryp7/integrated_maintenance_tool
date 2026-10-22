//! The suspend-and-sample recorder (Windows). Debugger-style, from outside the process only: per tick it suspends
//! one thread of the target at a time, copies its registers and the top of its stack, resumes it ([`Suspended`]
//! resumes on every path, including unwinding), and walks the copy with DbgHelp. Nothing is written into the
//! target, no thread is created in it and no code is loaded into it.

use super::{SampleSet, State, MAX_DEPTH};
use crate::process::{self, thread_name};
use crate::NativeError;
use debrief_symbols::walk::{Context, Memory, ProcessMemory, Snapshot, Walker};
use debrief_symbols::Module;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::Debug::{
    GetThreadContext, Wow64GetThreadContext, CONTEXT, CONTEXT_FULL_AMD64, WOW64_CONTEXT, WOW64_CONTEXT_FULL,
};
use windows::Win32::System::Threading::{
    GetThreadTimes, OpenProcess, OpenThread, ResumeThread, SuspendThread, WaitForSingleObject, PROCESS_QUERY_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_VM_READ, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION, THREAD_QUERY_LIMITED_INFORMATION,
    THREAD_SUSPEND_RESUME,
};

/// Bytes of stack copied per sample (from the stack pointer up).
pub const STACK_COPY: usize = 64 * 1024;
/// How often the thread and module lists are refreshed.
const THREAD_REFRESH: Duration = Duration::from_millis(500);
const MODULE_REFRESH: Duration = Duration::from_secs(2);
/// Most threads sampled per tick (round robin beyond).
pub const MAX_THREADS_PER_TICK: usize = 128;
/// `ERROR_ACCESS_DENIED`, `ERROR_INVALID_PARAMETER` (no such process).
const ACCESS_DENIED: i32 = 5;
const INVALID_PARAMETER: i32 = 87;

/// What to record.
#[derive(Debug, Clone, Copy)]
pub struct SamplingSpec {
    pub rate_hz: u32,
    pub duration: Duration,
    pub max_samples: usize,
}

/// A handle closed on drop.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: a handle this struct owns.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// A suspended thread, resumed when dropped (every return, error and unwinding path).
struct Suspended<'a>(&'a HANDLE);

impl<'a> Suspended<'a> {
    fn new(thread: &'a HANDLE) -> Option<Suspended<'a>> {
        // SAFETY: a thread handle with THREAD_SUSPEND_RESUME.
        let previous = unsafe { SuspendThread(*thread) };
        (previous != u32::MAX).then_some(Suspended(thread))
    }
}

impl Drop for Suspended<'_> {
    fn drop(&mut self) {
        // SAFETY: the thread this guard suspended.
        unsafe {
            ResumeThread(*self.0);
        }
    }
}

struct Thread {
    handle: Owned,
    /// Kernel + user time at the previous sample (100 ns units).
    cpu: u64,
}

fn cpu_time(thread: HANDLE) -> u64 {
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    // SAFETY: valid out pointers; the handle has THREAD_QUERY_INFORMATION.
    if unsafe { GetThreadTimes(thread, &mut c, &mut e, &mut k, &mut u) }.is_err() {
        return 0;
    }
    let ft = |f: FILETIME| (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime);
    ft(k).saturating_add(ft(u))
}

/// Copies the stack from `sp` up, a page at a time, until `max` bytes or an unreadable page.
fn copy_stack(mem: &dyn Memory, sp: u64, max: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(max.min(16 * 1024));
    let mut at = sp;
    while out.len() < max {
        let page_end = (at | 0xfff) + 1;
        let want = ((page_end - at) as usize).min(max - out.len());
        let start = out.len();
        out.resize(start + want, 0);
        let n = mem.read(at, &mut out[start..]);
        out.truncate(start + n);
        if n < want {
            break;
        }
        at += want as u64;
    }
    out
}

/// One sample of `thread`: registers and stack copied while it is suspended, walked after it resumed.
fn sample_thread(t: &Thread, wow64: bool, mem: &ProcessMemory, walker: &mut Walker) -> Option<Vec<u64>> {
    let (ctx, stack) = {
        let _suspended = Suspended::new(&t.handle.0)?;
        let ctx = if wow64 {
            let mut c = Box::new(WOW64_CONTEXT {
                ContextFlags: WOW64_CONTEXT_FULL,
                ..Default::default()
            });
            // SAFETY: a suspended thread with THREAD_GET_CONTEXT and a sized context.
            unsafe { Wow64GetThreadContext(t.handle.0, &mut *c) }.ok()?;
            Context::X86(c)
        } else {
            let mut c = Box::new(CONTEXT {
                ContextFlags: CONTEXT_FULL_AMD64,
                ..Default::default()
            });
            // SAFETY: as above.
            unsafe { GetThreadContext(t.handle.0, &mut *c) }.ok()?;
            Context::Amd64(c)
        };
        let stack = copy_stack(mem, ctx.stack_pointer(), STACK_COPY);
        (ctx, stack)
        // The thread resumes here.
    };
    let snapshot = Snapshot {
        base: ctx.stack_pointer(),
        bytes: stack,
        rest: mem,
    };
    let mut frames = walker.walk(&ctx, &snapshot, MAX_DEPTH);
    if frames.is_empty() {
        frames.push(ctx.instruction_pointer());
    }
    Some(frames)
}

fn now_ticks() -> i64 {
    debrief_etw::session::qpc()
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Records `pid` by suspend-and-sample until `stop()` is true, the duration ends, the sample limit is reached
/// or the process exits. Returns the samples and notes for the user; `on_progress` gets the sample count.
pub fn record(
    pid: u32,
    spec: &SamplingSpec,
    stop: &dyn Fn() -> bool,
    on_progress: &dyn Fn(u64),
) -> Result<(SampleSet, Vec<String>), NativeError> {
    if !cfg!(target_arch = "x86_64") {
        return Err(NativeError::Unsupported("suspend-and-sample needs the 64-bit (x64) Debrief".into()));
    }
    // SAFETY: owned and closed.
    let process = match unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(h) => Owned(h),
        Err(e) if e.code().0 & 0xffff == ACCESS_DENIED => {
            return Err(NativeError::Denied(format!(
                "Debrief cannot read process {pid} (it runs elevated or as another user); record it with ETW (administrator)"
            )))
        }
        Err(e) if e.code().0 & 0xffff == INVALID_PARAMETER => return Err(NativeError::NotFound { pid }),
        Err(e) => return Err(NativeError::Failed(format!("opening process {pid}: {}", e.message()))),
    };
    let arch = crate::process::arch_of(process.0);
    let wow64 = match arch {
        Some("x64") => false,
        Some("x86") => true,
        other => {
            return Err(NativeError::Unsupported(format!(
                "process {pid} is {}; suspend-and-sample records x64 and x86 processes",
                other.unwrap_or("of an unknown architecture")
            )))
        }
    };
    let mem = ProcessMemory(process.0);
    let mut walker = Walker::live(process.0).map_err(|e| NativeError::Failed(e.to_string()))?;
    let interval = Duration::from_nanos(1_000_000_000 / u64::from(spec.rate_hz.max(1)));
    let mut set = SampleSet::new(
        pid,
        if wow64 { 4 } else { 8 },
        unix_ms(),
        now_ticks(),
        debrief_etw::session::qpc_frequency(),
        interval.as_nanos() as u64,
        spec.max_samples,
    );
    let mut notes = Vec::new();
    let mut threads: HashMap<u32, Thread> = HashMap::new();
    let mut unopened: HashSet<u32> = HashSet::new();
    let mut loaded: HashSet<u64> = HashSet::new();
    let started = Instant::now();
    let (mut last_threads, mut last_modules) = (started - THREAD_REFRESH, started - MODULE_REFRESH);
    let mut next_tick = started;
    let mut cursor = 0usize;
    let mut exited = false;
    loop {
        if stop() || started.elapsed() >= spec.duration || set.is_full() {
            break;
        }
        // SAFETY: a process handle with SYNCHRONIZE.
        if unsafe { WaitForSingleObject(process.0, 0) } == WAIT_OBJECT_0 {
            exited = true;
            break;
        }
        if last_modules.elapsed() >= MODULE_REFRESH {
            last_modules = Instant::now();
            for m in process::modules(pid) {
                if loaded.insert(m.base) {
                    walker.add_module(&Module {
                        path: m.path.clone().into(),
                        base: m.base,
                        size: m.size,
                    });
                    set.images.push(m);
                }
            }
        }
        if last_threads.elapsed() >= THREAD_REFRESH {
            last_threads = Instant::now();
            let live: HashSet<u32> = process::threads(pid).into_iter().collect();
            threads.retain(|tid, _| live.contains(tid));
            for &tid in &live {
                if threads.contains_key(&tid) || unopened.contains(&tid) {
                    continue;
                }
                let rights = THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION | THREAD_QUERY_LIMITED_INFORMATION;
                // SAFETY: owned and closed.
                match unsafe { OpenThread(rights, false, tid) } {
                    Ok(h) => {
                        let handle = Owned(h);
                        if let Some(name) = thread_name(handle.0) {
                            set.thread_names.insert(tid, name);
                        }
                        let cpu = cpu_time(handle.0);
                        threads.insert(tid, Thread { handle, cpu });
                    }
                    Err(_) => {
                        unopened.insert(tid);
                    }
                }
            }
        }
        // One tick: each thread once (round robin when there are many).
        let mut ids: Vec<u32> = threads.keys().copied().collect();
        ids.sort_unstable();
        let n = ids.len().min(MAX_THREADS_PER_TICK);
        for i in 0..n {
            let Some(&tid) = ids.get((cursor + i) % ids.len().max(1)) else {
                break;
            };
            let Some(t) = threads.get_mut(&tid) else { continue };
            let Some(frames) = sample_thread(t, wow64, &mem, &mut walker) else {
                continue;
            };
            let cpu = cpu_time(t.handle.0);
            let state = if cpu > t.cpu { State::Running } else { State::Waiting };
            t.cpu = cpu;
            if !set.push(tid, now_ticks(), &frames, state) {
                break;
            }
        }
        cursor = cursor.wrapping_add(n);
        on_progress(set.samples.len() as u64);
        next_tick += interval;
        let now = Instant::now();
        if next_tick > now {
            std::thread::sleep(next_tick - now);
        } else {
            // Behind (many threads, slow walks): do not burst to catch up.
            next_tick = now;
        }
    }
    if exited {
        notes.push("The process exited during the capture".to_string());
    }
    if !unopened.is_empty() {
        notes.push(format!("{} threads could not be opened and were not sampled", unopened.len()));
    }
    if set.dropped > 0 {
        notes.push(format!("{} samples beyond the limit were dropped", set.dropped));
    }
    if wow64 {
        notes.push("A 32-bit (x86) process: its 32-bit stacks were sampled through WOW64".to_string());
    }
    notes.push(format!(
        "Suspend-and-sample at {} Hz per thread: CPU samples where a thread used CPU since its previous sample, waits otherwise (wall-clock, approximate)",
        spec.rate_hz
    ));
    Ok((set, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Pages(u64);
    impl Memory for Pages {
        fn read(&self, address: u64, out: &mut [u8]) -> usize {
            // Readable below `self.0` only.
            if address >= self.0 {
                return 0;
            }
            let n = ((self.0 - address) as usize).min(out.len());
            out[..n].fill(1);
            n
        }
    }

    #[test]
    fn stack_copies_stop_at_the_limit_or_an_unreadable_page() {
        assert_eq!(copy_stack(&Pages(u64::MAX), 0x1000_0010, 10_000).len(), 10_000);
        assert_eq!(copy_stack(&Pages(0x1000_2000), 0x1000_0010, 1 << 20).len(), 0x2000 - 0x10);
        assert!(copy_stack(&Pages(0), 0x10, 100).is_empty());
    }

    #[test]
    fn missing_and_own_processes_are_refused() {
        let spec = SamplingSpec {
            rate_hz: 100,
            duration: Duration::from_millis(100),
            max_samples: 10,
        };
        let e = record(u32::MAX - 3, &spec, &|| false, &|_| {}).unwrap_err();
        assert!(matches!(e, NativeError::NotFound { .. } | NativeError::Failed(_)), "{e:?}");
        assert!(cpu_time(HANDLE::default()) == 0);
    }
}
