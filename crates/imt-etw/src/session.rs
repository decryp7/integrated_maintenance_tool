//! Real-time (and file) ETW sessions, Windows 8+, as IMT's profilers start them.
//!
//! A [`Session`] is one system-logger session (kernel events allowed) named `<prefix><imt pid>-<target>-<n>`, with
//! the kernel flags of a [`KernelSpec`] (stack walks on the classic events it lists) and providers enabled for one
//! process ([`Provider`]). [`consume`] reads its events on the calling thread until it stops. Sessions are always
//! stopped (stop, drop, unwinding); sessions left by a crashed IMT are found by their prefix and stopped
//! ([`stop_orphans`]).

use crate::kernel::{guids, opcodes};
use crate::sys::{self, wide, Source};
use crate::{win32_error, EtwError, RawEvent};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::Etw::*;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

/// Room for the logger and file names after the properties.
const NAME_CHARS: usize = 1024;
/// Most classic events with stack walks.
const MAX_STACK_EVENTS: usize = 8;

/// The kernel events a session records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KernelSpec {
    /// CPU samples (`PROFILE`, 1 ms) with stack walks.
    pub samples: bool,
    /// Context switches and ready-thread events (`CSWITCH`, `DISPATCHER`) with stack walks on switches.
    pub waits: bool,
}

impl KernelSpec {
    fn flags(&self) -> u32 {
        let mut flags = EVENT_TRACE_FLAG_PROCESS.0 | EVENT_TRACE_FLAG_THREAD.0 | EVENT_TRACE_FLAG_IMAGE_LOAD.0;
        if self.samples {
            flags |= EVENT_TRACE_FLAG_PROFILE.0;
        }
        if self.waits {
            flags |= EVENT_TRACE_FLAG_CSWITCH.0 | EVENT_TRACE_FLAG_DISPATCHER.0;
        }
        flags
    }

    /// Classic events that get a stack walk.
    fn stack_events(&self) -> Vec<(u128, u8)> {
        let mut v = Vec::new();
        if self.samples {
            v.push((guids::PERF_INFO, opcodes::SAMPLED_PROFILE));
        }
        if self.waits {
            v.push((guids::THREAD, opcodes::CSWITCH));
        }
        v
    }
}

/// A provider enabled for one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provider {
    pub guid: u128,
    pub keywords: u64,
    pub level: u8,
    /// Only this process (a PID filter where the system supports it; events are filtered again when converted).
    pub pid: u32,
    /// Attach the user-mode stack to each event.
    pub stacks: bool,
}

/// `EVENT_TRACE_PROPERTIES` followed by room for two names, 8-byte aligned.
struct Properties(Vec<u64>);

impl Properties {
    fn new() -> Properties {
        let size = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + 2 * NAME_CHARS * 2;
        let mut p = Properties(vec![0u64; size.div_ceil(8)]);
        let props = p.get();
        props.Wnode.BufferSize = size as u32;
        props.LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        props.LogFileNameOffset = (std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + NAME_CHARS * 2) as u32;
        p
    }

    fn get(&mut self) -> &mut EVENT_TRACE_PROPERTIES {
        // SAFETY: the buffer is at least the struct's size and 8-byte aligned; all-zero is a valid value.
        unsafe { &mut *(self.0.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) }
    }

    fn ptr(&mut self) -> *mut EVENT_TRACE_PROPERTIES {
        self.0.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES
    }

    fn logger_name(&self) -> String {
        // SAFETY: the buffer holds the struct (see `new`); LoggerNameOffset is clamped to the buffer below.
        let bytes = unsafe { std::slice::from_raw_parts(self.0.as_ptr() as *const u8, self.0.len() * 8) };
        let props = unsafe { &*(self.0.as_ptr() as *const EVENT_TRACE_PROPERTIES) };
        let start = (props.LoggerNameOffset as usize).min(bytes.len());
        let units: Vec<u16> = bytes[start..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take(NAME_CHARS)
            .take_while(|&u| u != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }

    /// Real time (or sequential file), system logger (kernel events), QPC clock, the kernel flags.
    fn fill(&mut self, kernel: &KernelSpec, file: Option<&Path>) -> Result<(), EtwError> {
        if let Some(path) = file {
            let name = wide(&path.to_string_lossy());
            if name.len() >= NAME_CHARS {
                return Err(EtwError::Unsupported("the .etl path is too long".into()));
            }
            let off = self.get().LogFileNameOffset as usize;
            // SAFETY: the offset and the name fit inside the buffer (checked above).
            unsafe {
                let dst = (self.0.as_mut_ptr() as *mut u8).add(off) as *mut u16;
                std::ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len());
            }
        }
        let props = self.get();
        props.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        props.Wnode.ClientContext = 1;
        props.Wnode.Guid = GUID::new().map_err(|e| EtwError::Unsupported(e.message()))?;
        props.LogFileMode = if file.is_some() {
            EVENT_TRACE_FILE_MODE_SEQUENTIAL | EVENT_TRACE_SYSTEM_LOGGER_MODE
        } else {
            EVENT_TRACE_REAL_TIME_MODE | EVENT_TRACE_SYSTEM_LOGGER_MODE
        };
        props.BufferSize = 256;
        props.MinimumBuffers = 64;
        props.MaximumBuffers = 512;
        props.FlushTimer = 1;
        props.EnableFlags = EVENT_TRACE_FLAG(kernel.flags());
        Ok(())
    }
}

fn stop_by_name(name: &str) {
    let mut p = Properties::new();
    let w = wide(name);
    // SAFETY: valid properties buffer and name.
    unsafe {
        let _ = ControlTraceW(
            CONTROLTRACE_HANDLE::default(),
            PCWSTR(w.as_ptr()),
            p.ptr(),
            EVENT_TRACE_CONTROL_STOP,
        );
    }
}

/// Whether a session name with `prefix` belongs to an IMT that is no longer running.
pub fn is_orphan(prefix: &str, name: &str, alive: impl Fn(u32) -> bool) -> bool {
    let Some(rest) = name.strip_prefix(prefix) else { return false };
    match rest.split('-').next().and_then(|p| p.parse::<u32>().ok()) {
        Some(owner) => owner != std::process::id() && !alive(owner),
        None => false,
    }
}

/// Whether process `pid` is running.
pub fn process_alive(pid: u32) -> bool {
    // SAFETY: the handle is closed below.
    match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
        Ok(h) => {
            let alive = unsafe { WaitForSingleObject(h, 0) } != WAIT_OBJECT_0;
            unsafe {
                let _ = CloseHandle(h);
            }
            alive
        }
        Err(_) => false,
    }
}

/// Stops sessions with `prefix` left by an IMT that crashed.
pub fn stop_orphans(prefix: &str) {
    let mut bufs: Vec<Properties> = (0..64).map(|_| Properties::new()).collect();
    let mut ptrs: Vec<*mut EVENT_TRACE_PROPERTIES> = bufs.iter_mut().map(Properties::ptr).collect();
    let mut count = 0u32;
    // SAFETY: 64 valid property buffers.
    if unsafe { QueryAllTracesW(&mut ptrs, &mut count) }.0 != 0 {
        return;
    }
    for b in bufs.iter().take(count as usize) {
        let name = b.logger_name();
        if is_orphan(prefix, &name, process_alive) {
            stop_by_name(&name);
        }
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// A unique session name: `<prefix><imt pid>-<target>-<n>`.
pub fn session_name(prefix: &str, target: u32) -> String {
    format!(
        "{prefix}{}-{target}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// A started session; stopped when dropped.
pub struct Session {
    name: String,
    handle: CONTROLTRACE_HANDLE,
    stopped: bool,
}

impl Session {
    /// Starts a real-time session `name` (or one writing the `.etl` file `file`) with `kernel`'s events.
    /// A stale session of the same name is stopped first. Missing rights are [`EtwError::Denied`].
    pub fn start(name: &str, kernel: &KernelSpec, file: Option<&Path>) -> Result<Session, EtwError> {
        let w = wide(name);
        let mut handle = CONTROLTRACE_HANDLE::default();
        let mut props = Properties::new();
        props.fill(kernel, file)?;
        // SAFETY: valid buffers.
        let mut status = unsafe { StartTraceW(&mut handle, PCWSTR(w.as_ptr()), props.ptr()) }.0;
        if status == 183 {
            // ERROR_ALREADY_EXISTS: a stale session of this name.
            stop_by_name(name);
            let mut props = Properties::new();
            props.fill(kernel, file)?;
            // SAFETY: as above.
            status = unsafe { StartTraceW(&mut handle, PCWSTR(w.as_ptr()), props.ptr()) }.0;
        }
        if status != 0 {
            return Err(win32_error("StartTraceW", status));
        }
        let session = Session {
            name: name.to_string(),
            handle,
            stopped: false,
        };
        let stack_events = kernel.stack_events();
        if !stack_events.is_empty() {
            let ids: Vec<CLASSIC_EVENT_ID> = stack_events
                .iter()
                .take(MAX_STACK_EVENTS)
                .map(|(guid, opcode)| CLASSIC_EVENT_ID {
                    EventGuid: GUID::from_u128(*guid),
                    Type: *opcode,
                    Reserved: [0; 7],
                })
                .collect();
            // SAFETY: a slice of CLASSIC_EVENT_IDs and its byte size.
            let s = unsafe {
                TraceSetInformation(
                    handle,
                    TraceStackTracingInfo,
                    ids.as_ptr() as *const core::ffi::c_void,
                    (std::mem::size_of::<CLASSIC_EVENT_ID>() * ids.len()) as u32,
                )
            };
            if s.0 != 0 {
                return Err(win32_error("TraceSetInformation", s.0));
            }
        }
        Ok(session)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Enables `p` (with a PID filter when the system supports it).
    pub fn enable(&self, p: &Provider) -> Result<(), EtwError> {
        let guid = GUID::from_u128(p.guid);
        let pid_value = p.pid;
        let mut filter = EVENT_FILTER_DESCRIPTOR {
            Ptr: &pid_value as *const u32 as u64,
            Size: 4,
            Type: EVENT_FILTER_TYPE_PID,
        };
        let mut params = ENABLE_TRACE_PARAMETERS {
            Version: ENABLE_TRACE_PARAMETERS_VERSION_2,
            EnableProperty: if p.stacks { EVENT_ENABLE_PROPERTY_STACK_TRACE } else { 0 },
            EnableFilterDesc: &mut filter,
            FilterDescCount: 1,
            ..Default::default()
        };
        let enable = |params: &ENABLE_TRACE_PARAMETERS| {
            // SAFETY: params and the filter outlive the call.
            unsafe {
                EnableTraceEx2(
                    self.handle,
                    &guid,
                    EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                    p.level,
                    p.keywords,
                    0,
                    0,
                    Some(params),
                )
            }
            .0
        };
        let mut s = enable(&params);
        if s != 0 && s != 5 {
            // The PID filter is not supported here: enable for everyone, filter while converting.
            params.EnableFilterDesc = std::ptr::null_mut();
            params.FilterDescCount = 0;
            s = enable(&params);
        }
        if s != 0 {
            return Err(win32_error("EnableTraceEx2", s));
        }
        Ok(())
    }

    /// Stops the session (consumers then see the end of the stream).
    pub fn stop(&mut self) {
        if !self.stopped {
            self.stopped = true;
            let mut p = Properties::new();
            // SAFETY: our session handle and a valid buffer.
            unsafe {
                let _ = ControlTraceW(self.handle, PCWSTR::null(), p.ptr(), EVENT_TRACE_CONTROL_STOP);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Reads the real-time session `name` on the calling thread until it stops or `sink` returns `false`.
/// Returns whether `sink` panicked (events after that were ignored).
pub fn consume(name: &str, sink: &mut dyn FnMut(&RawEvent<'_>) -> bool) -> Result<bool, EtwError> {
    sys::consume(Source::Session(name), |_| {}, sink)
}

/// The QPC clock (what IMT's sessions stamp events with).
pub fn qpc() -> i64 {
    let mut v = 0i64;
    // SAFETY: a valid out pointer.
    unsafe {
        let _ = QueryPerformanceCounter(&mut v);
    }
    v
}

/// Ticks per second of [`qpc`].
pub fn qpc_frequency() -> i64 {
    let mut v = 0i64;
    // SAFETY: a valid out pointer.
    unsafe {
        let _ = QueryPerformanceFrequency(&mut v);
    }
    v.max(1)
}

/// Whether this process can start ETW sessions (it is elevated).
pub fn can_record() -> bool {
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token = windows::Win32::Foundation::HANDLE::default();
    // SAFETY: the pseudo handle of this process; the token is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let mut e = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    // SAFETY: a TOKEN_ELEVATION buffer of its size.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut e as *mut TOKEN_ELEVATION as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    }
    .is_ok();
    // SAFETY: opened above.
    unsafe {
        let _ = CloseHandle(token);
    }
    ok && e.TokenIsElevated != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "IMT-test-";

    #[test]
    fn orphans_are_sessions_of_dead_imts_only() {
        let me = std::process::id();
        assert!(is_orphan(PREFIX, "IMT-test-999999-5-1", |_| false));
        assert!(!is_orphan(PREFIX, "IMT-test-999999-5-1", |_| true));
        assert!(!is_orphan(PREFIX, &format!("IMT-test-{me}-5-1"), |_| false), "our own sessions");
        assert!(!is_orphan(PREFIX, "NT Kernel Logger", |_| false));
        assert!(!is_orphan(PREFIX, "IMT-test-x-5-1", |_| false));
        let a = session_name(PREFIX, 7);
        assert!(a.starts_with(&format!("{PREFIX}{me}-7-")) && a != session_name(PREFIX, 7));
    }

    #[test]
    fn properties_hold_the_logger_name_and_kernel_flags() {
        let mut p = Properties::new();
        let off = p.get().LoggerNameOffset as usize;
        let name = wide("IMT-test-1-2-3");
        // SAFETY: inside the buffer.
        unsafe {
            let dst = (p.0.as_mut_ptr() as *mut u8).add(off) as *mut u16;
            std::ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len());
        }
        assert_eq!(p.logger_name(), "IMT-test-1-2-3");
        let k = KernelSpec {
            samples: true,
            waits: true,
        };
        p.fill(&k, None).unwrap();
        assert_ne!(p.get().EnableFlags.0 & EVENT_TRACE_FLAG_PROFILE.0, 0);
        assert_eq!(k.stack_events().len(), 2);
        assert!(KernelSpec::default().stack_events().is_empty());
        let long = std::path::PathBuf::from("x".repeat(2000));
        assert!(Properties::new().fill(&k, Some(&long)).is_err());
    }

    /// Without administrator rights, starting a session is refused with `Denied` (the UI then offers
    /// "Restart as administrator"). Run unelevated; skipped when elevated.
    #[test]
    fn unelevated_start_is_denied() {
        if can_record() {
            eprintln!("skipped: running elevated");
            return;
        }
        let e = Session::start(&session_name(PREFIX, 0), &KernelSpec::default(), None)
            .err()
            .unwrap();
        assert!(matches!(e, EtwError::Denied), "{e:?}");
    }

    #[test]
    fn a_dead_process_is_not_alive_and_the_clock_runs() {
        assert!(!process_alive(u32::MAX - 1));
        assert!(process_alive(std::process::id()));
        assert!(qpc_frequency() > 0 && qpc() > 0);
    }
}
