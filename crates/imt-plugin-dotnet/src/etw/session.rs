//! A real-time ETW session recording one .NET process (Windows 8+).
//!
//! One system-logger session ("IMT-dotnet-<imt pid>-<target pid>-<n>") gets
//! the kernel profile / context-switch / ready-thread / process / thread /
//! image-load events (with stack walks on samples and context switches) and
//! the CLR runtime provider filtered to the target. At stop the CLR rundown
//! provider is enabled for method names; the session ends after
//! `DCEndComplete` or a timeout. Sessions are always stopped (stop, cancel,
//! drop, panic); sessions left by a crashed IMT are stopped at the next start.

use super::convert::Converter;
use super::sys::{self, wide, Source};
use super::{win32_error, EtwError, RawEvent};
use crate::nettrace::{system_time, TraceHeader};
use crate::recording::{Control, ThreadRecording};
use crate::{clr, DotnetError, Finished, Limits, Preset, ProcessInfo, HEAP_SNAPSHOT_CAP};
use std::io::BufWriter;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Diagnostics::Etw::*;
use windows::Win32::System::Performance::QueryPerformanceCounter;
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

/// Session name prefix (orphans are recognised by it).
const PREFIX: &str = "IMT-dotnet-";
/// Room for the logger and file names after the properties.
const NAME_CHARS: usize = 1024;
/// How long to wait for the rundown at stop.
const RUNDOWN_TIMEOUT: Duration = Duration::from_secs(15);
/// After the heap walk's GC ended, wait this long for trailing type events.
const HEAP_GRACE: Duration = Duration::from_millis(1000);
const CLASSIC_EVENT_COUNT: usize = 2;

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
        // SAFETY: LoggerNameOffset points inside the buffer (set by us or by QueryAllTraces).
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

/// Whether a session name belongs to an IMT that is no longer running.
pub fn is_orphan(name: &str, alive: impl Fn(u32) -> bool) -> bool {
    let Some(rest) = name.strip_prefix(PREFIX) else { return false };
    match rest.split('-').next().and_then(|p| p.parse::<u32>().ok()) {
        Some(owner) => owner != std::process::id() && !alive(owner),
        None => false,
    }
}

fn process_alive(pid: u32) -> bool {
    // SAFETY: handle closed below.
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

/// Stops sessions left by an IMT that crashed.
pub fn stop_orphans() {
    let mut bufs: Vec<Properties> = (0..64).map(|_| Properties::new()).collect();
    let mut ptrs: Vec<*mut EVENT_TRACE_PROPERTIES> = bufs.iter_mut().map(Properties::ptr).collect();
    let mut count = 0u32;
    // SAFETY: 64 valid property buffers.
    if unsafe { QueryAllTracesW(&mut ptrs, &mut count) }.0 != 0 {
        return;
    }
    for b in bufs.iter().take(count as usize) {
        let name = b.logger_name();
        if is_orphan(&name, process_alive) {
            stop_by_name(&name);
        }
    }
}

/// A started session; stopped when dropped.
struct Session {
    handle: CONTROLTRACE_HANDLE,
    stopped: bool,
}

impl Session {
    fn start(name: &str, preset: Preset) -> Result<Session, EtwError> {
        Session::start_to(name, preset, None)
    }

    /// A real-time session, or one writing the `.etl` file `file`.
    fn start_to(name: &str, preset: Preset, file: Option<&Path>) -> Result<Session, EtwError> {
        let w = wide(name);
        let mut handle = CONTROLTRACE_HANDLE::default();
        let mut props = Properties::new();
        fill(&mut props, preset, file)?;
        // SAFETY: valid buffers.
        let mut status = unsafe { StartTraceW(&mut handle, PCWSTR(w.as_ptr()), props.ptr()) }.0;
        if status == 183 {
            // ERROR_ALREADY_EXISTS: a stale session of this name.
            stop_by_name(name);
            let mut props = Properties::new();
            fill(&mut props, preset, file)?;
            status = unsafe { StartTraceW(&mut handle, PCWSTR(w.as_ptr()), props.ptr()) }.0;
        }
        if status != 0 {
            return Err(win32_error("StartTraceW", status));
        }
        let session = Session { handle, stopped: false };
        if preset.samples() {
            let ids = [
                CLASSIC_EVENT_ID {
                    EventGuid: GUID::from_u128(super::kernel::guids::PERF_INFO),
                    Type: super::kernel::opcodes::SAMPLED_PROFILE,
                    Reserved: [0; 7],
                },
                CLASSIC_EVENT_ID {
                    EventGuid: GUID::from_u128(super::kernel::guids::THREAD),
                    Type: super::kernel::opcodes::CSWITCH,
                    Reserved: [0; 7],
                },
            ];
            // SAFETY: an array of CLASSIC_EVENT_COUNT ids.
            let s = unsafe {
                TraceSetInformation(
                    handle,
                    TraceStackTracingInfo,
                    ids.as_ptr() as *const core::ffi::c_void,
                    (std::mem::size_of::<CLASSIC_EVENT_ID>() * CLASSIC_EVENT_COUNT) as u32,
                )
            };
            if s.0 != 0 {
                return Err(win32_error("TraceSetInformation", s.0));
            }
        }
        Ok(session)
    }

    /// Enables a provider for `pid` (PID filter when the system supports it; events are filtered again here).
    fn enable(&self, provider: u128, keywords: u64, level: u8, pid: u32, stacks: bool) -> Result<(), EtwError> {
        let guid = GUID::from_u128(provider);
        let pid_value = pid;
        let mut filter = EVENT_FILTER_DESCRIPTOR {
            Ptr: &pid_value as *const u32 as u64,
            Size: 4,
            Type: EVENT_FILTER_TYPE_PID,
        };
        let mut params = ENABLE_TRACE_PARAMETERS {
            Version: ENABLE_TRACE_PARAMETERS_VERSION_2,
            EnableProperty: if stacks { EVENT_ENABLE_PROPERTY_STACK_TRACE } else { 0 },
            EnableFilterDesc: &mut filter,
            FilterDescCount: 1,
            ..Default::default()
        };
        // SAFETY: params and filter outlive the call.
        let mut s = unsafe {
            EnableTraceEx2(
                self.handle,
                &guid,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                level,
                keywords,
                0,
                0,
                Some(&params),
            )
        }
        .0;
        if s != 0 && s != 5 {
            // The PID filter is not supported here: enable for everyone, filter while converting.
            params.EnableFilterDesc = std::ptr::null_mut();
            params.FilterDescCount = 0;
            s = unsafe {
                EnableTraceEx2(
                    self.handle,
                    &guid,
                    EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                    level,
                    keywords,
                    0,
                    0,
                    Some(&params),
                )
            }
            .0;
        }
        if s != 0 {
            return Err(win32_error("EnableTraceEx2", s));
        }
        Ok(())
    }

    fn stop(&mut self) {
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

/// The session settings: real time, system logger (kernel events), QPC clock, kernel flags per preset.
fn fill(p: &mut Properties, preset: Preset, file: Option<&Path>) -> Result<(), EtwError> {
    if let Some(path) = file {
        let name = wide(&path.to_string_lossy());
        if name.len() >= NAME_CHARS {
            return Err(EtwError::Unsupported("the .etl path is too long".into()));
        }
        let off = p.get().LogFileNameOffset as usize;
        // SAFETY: the offset and the name fit inside the buffer (checked above).
        unsafe {
            let dst = (p.0.as_mut_ptr() as *mut u8).add(off) as *mut u16;
            std::ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len());
        }
    }
    let props = p.get();
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
    let mut flags = EVENT_TRACE_FLAG_PROCESS.0 | EVENT_TRACE_FLAG_THREAD.0 | EVENT_TRACE_FLAG_IMAGE_LOAD.0;
    if preset.samples() {
        flags |= EVENT_TRACE_FLAG_PROFILE.0 | EVENT_TRACE_FLAG_CSWITCH.0 | EVENT_TRACE_FLAG_DISPATCHER.0;
    }
    props.EnableFlags = EVENT_TRACE_FLAG(flags);
    Ok(())
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// What the consumer thread shares with the controller.
struct Shared {
    conv: Mutex<Option<Converter<BufWriter<std::fs::File>>>>,
    bytes: AtomicU64,
    /// Bits: 1 rundown complete, 2 heap walk done, 4 size limit reached, 8 write failed.
    flags: AtomicU8,
    max_bytes: u64,
}

const RUNDOWN_DONE: u8 = 1;
const HEAP_DONE: u8 = 2;
const FULL: u8 = 4;
const WRITE_FAILED: u8 = 8;

impl Shared {
    fn on_event(&self, e: &RawEvent<'_>) -> bool {
        let mut guard = self.conv.lock().unwrap_or_else(|p| p.into_inner());
        let Some(conv) = guard.as_mut() else { return true };
        if self.flags.load(Ordering::Relaxed) & (FULL | WRITE_FAILED) != 0 {
            return true;
        }
        if conv.event(e).is_err() {
            self.flags.fetch_or(WRITE_FAILED, Ordering::Relaxed);
            return true;
        }
        let bytes = conv.bytes();
        self.bytes.store(bytes, Ordering::Relaxed);
        let m = conv.milestones();
        let mut f = 0;
        if m.rundown_complete {
            f |= RUNDOWN_DONE;
        }
        if m.heap_walk_done {
            f |= HEAP_DONE;
        }
        if bytes >= self.max_bytes {
            f |= FULL;
        }
        if f != 0 {
            self.flags.fetch_or(f, Ordering::Relaxed);
        }
        true
    }

    fn has(&self, flag: u8) -> bool {
        self.flags.load(Ordering::Relaxed) & flag != 0
    }
}

fn qpc() -> i64 {
    let mut v = 0i64;
    // SAFETY: valid out pointer.
    unsafe {
        let _ = QueryPerformanceCounter(&mut v);
    }
    v
}

fn qpc_frequency() -> i64 {
    let mut v = 0i64;
    // SAFETY: valid out pointer.
    unsafe {
        let _ = windows::Win32::System::Performance::QueryPerformanceFrequency(&mut v);
    }
    v.max(1)
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Starts recording `target` into `out`. Errors (rights, unsupported) are returned before any thread starts.
pub fn record(target: &ProcessInfo, preset: Preset, out: &Path, limits: Limits) -> Result<ThreadRecording, DotnetError> {
    stop_orphans();
    let name = format!(
        "{PREFIX}{}-{}-{}",
        std::process::id(),
        target.pid,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let session = Session::start(&name, preset)?;
    let (mut keywords, level) = preset.runtime();
    if preset == Preset::HeapSnapshot {
        // GCStart/GCEnd tell when the induced GC (and its heap walk) is over.
        keywords |= clr::keywords::GC;
    }
    session.enable(
        clr::RUNTIME_GUID,
        keywords | clr::keywords::STACK,
        level,
        target.pid,
        preset.samples(),
    )?;

    let header = TraceHeader {
        sync_time: system_time(unix_ms()).unwrap_or([1970, 1, 4, 1, 0, 0, 0, 0]),
        sync_ticks: qpc(),
        ticks_per_second: qpc_frequency(),
        pointer_size: target.pointer_size(),
        process_id: target.pid,
        processors: std::thread::available_parallelism().map_or(1, |n| n.get() as u32),
        sampling_ns: 1_000_000,
    };
    let file = std::fs::File::create(out)?;
    let mut conv = Converter::new(BufWriter::with_capacity(1 << 20, file), header)?;
    conv.seed_threads(crate::process::threads(target.pid));
    let shared = Arc::new(Shared {
        conv: Mutex::new(Some(conv)),
        bytes: AtomicU64::new(0),
        flags: AtomicU8::new(0),
        max_bytes: limits.max_bytes,
    });

    // The consumer: ProcessTrace on its own thread until the session stops.
    let consumer_done = Arc::new(AtomicBool::new(false));
    let (s2, done2, name2) = (shared.clone(), consumer_done.clone(), name.clone());
    let consumer = std::thread::Builder::new()
        .name("imt-dotnet-etw-consumer".into())
        .spawn(move || {
            let r = sys::consume(Source::Session(&name2), |_| {}, &mut |e| s2.on_event(e));
            done2.store(true, Ordering::Release);
            r
        })
        .map_err(DotnetError::Io)?;

    let pid = target.pid;
    let path = out.to_path_buf();
    ThreadRecording::spawn("imt-dotnet-etw", path.clone(), move |ctl| {
        let r = control(&ctl, session, consumer, &consumer_done, &shared, pid, preset, limits, path.clone());
        if r.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        r
    })
}

#[allow(clippy::too_many_arguments)]
fn control(
    ctl: &Control,
    mut session: Session,
    consumer: std::thread::JoinHandle<Result<bool, EtwError>>,
    consumer_done: &AtomicBool,
    shared: &Shared,
    pid: u32,
    preset: Preset,
    limits: Limits,
    path: std::path::PathBuf,
) -> Result<Finished, DotnetError> {
    let started = Instant::now();
    let cap = if preset == Preset::HeapSnapshot {
        limits.max_duration.min(HEAP_SNAPSHOT_CAP)
    } else {
        limits.max_duration
    };
    let mut notes = Vec::new();
    let mut heap_done_at: Option<Instant> = None;
    let mut exited = false;
    loop {
        ctl.set_bytes(shared.bytes.load(Ordering::Relaxed));
        if ctl.stopping() || started.elapsed() >= cap || shared.has(FULL) || shared.has(WRITE_FAILED) {
            break;
        }
        if consumer_done.load(Ordering::Acquire) {
            notes.push("The ETW session ended early (stopped by another tool?)".to_string());
            break;
        }
        if shared.has(HEAP_DONE) {
            let t = *heap_done_at.get_or_insert_with(Instant::now);
            if t.elapsed() >= HEAP_GRACE {
                break;
            }
        }
        if !process_alive(pid) {
            exited = true;
            notes.push("The process exited during the recording".to_string());
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if ctl.cancelled() {
        session.stop();
        let _ = consumer.join();
        return Err(DotnetError::Cancelled);
    }
    if preset.rundown() && !exited && !shared.has(FULL) {
        match session.enable(clr::RUNDOWN_GUID, clr::keywords::RUNDOWN_END, clr::level::VERBOSE, pid, false) {
            Ok(()) => {
                let t = Instant::now();
                while !shared.has(RUNDOWN_DONE) && t.elapsed() < RUNDOWN_TIMEOUT && !ctl.cancelled() && !shared.has(FULL) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                if !shared.has(RUNDOWN_DONE) {
                    notes.push("The runtime did not finish listing its methods in time; some frames may show as addresses".into());
                }
            }
            Err(e) => notes.push(format!("Method names could not be collected at the end ({e})")),
        }
    }
    session.stop();
    let consumed = consumer
        .join()
        .unwrap_or_else(|_| Err(EtwError::Unsupported("the ETW consumer thread failed".into())));
    if ctl.cancelled() {
        return Err(DotnetError::Cancelled);
    }
    match consumed {
        Ok(true) => notes.push("Some events could not be converted (a bug in IMT); the capture may be incomplete".into()),
        Ok(false) => {}
        Err(e) => notes.push(format!("Reading the session failed: {e}")),
    }
    if shared.has(WRITE_FAILED) {
        return Err(DotnetError::Protocol("writing the capture file failed (is the disk full?)".into()));
    }
    let conv = shared
        .conv
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .ok_or_else(|| DotnetError::Protocol("the capture was already finished".into()))?;
    let stats = conv.stats();
    let (w, _) = conv.finish()?;
    let file = w.into_inner().map_err(|e| DotnetError::Io(e.into_error()))?;
    file.sync_all()?;
    let bytes = file.metadata()?.len();
    let truncated = shared.has(FULL);
    if truncated {
        notes.push(format!(
            "The recording reached the size limit of {} MB and was cut",
            limits.max_bytes >> 20
        ));
    }
    if preset.samples() && stats.samples == 0 {
        notes.push("No CPU samples were recorded for the process (was it idle?)".into());
    }
    if preset == Preset::HeapSnapshot && !shared.has(HEAP_DONE) {
        notes.push("The runtime did not complete a heap walk (is the process a .NET 4.6.2+ process?)".into());
    }
    Ok(Finished {
        path,
        bytes,
        truncated,
        notes,
    })
}

/// Records `target` for `duration` (CPU preset, then rundown) into an `.etl` file, as WPR or
/// PerfView would. Used by the elevated tests to produce `.etl` input for [`super::etl`].
#[doc(hidden)]
pub fn record_etl(target: &ProcessInfo, etl: &Path, duration: Duration) -> Result<(), DotnetError> {
    let name = format!(
        "{PREFIX}{}-{}-etl{}",
        std::process::id(),
        target.pid,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let mut session = Session::start_to(&name, Preset::Cpu, Some(etl))?;
    let (keywords, level) = Preset::Cpu.runtime();
    session.enable(clr::RUNTIME_GUID, keywords | clr::keywords::STACK, level, target.pid, true)?;
    std::thread::sleep(duration);
    session.enable(
        clr::RUNDOWN_GUID,
        clr::keywords::RUNDOWN_END,
        clr::level::VERBOSE,
        target.pid,
        false,
    )?;
    std::thread::sleep(Duration::from_secs(3));
    session.stop();
    Ok(())
}

/// Whether ETW sessions can be started here (administrator or Performance Log Users).
pub fn can_record() -> bool {
    crate::process::elevated()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orphans_are_sessions_of_dead_imts_only() {
        let me = std::process::id();
        assert!(is_orphan("IMT-dotnet-999999-5-1", |_| false));
        assert!(!is_orphan("IMT-dotnet-999999-5-1", |_| true));
        assert!(!is_orphan(&format!("IMT-dotnet-{me}-5-1"), |_| false), "our own sessions");
        assert!(!is_orphan("NT Kernel Logger", |_| false));
        assert!(!is_orphan("IMT-dotnet-x-5-1", |_| false));
    }

    #[test]
    fn properties_hold_the_logger_name() {
        let mut p = Properties::new();
        let off = p.get().LoggerNameOffset as usize;
        let name = wide("IMT-dotnet-1-2-3");
        // SAFETY: inside the buffer.
        unsafe {
            let dst = (p.0.as_mut_ptr() as *mut u8).add(off) as *mut u16;
            std::ptr::copy_nonoverlapping(name.as_ptr(), dst, name.len());
        }
        assert_eq!(p.logger_name(), "IMT-dotnet-1-2-3");
        assert!(p.get().Wnode.BufferSize as usize >= std::mem::size_of::<EVENT_TRACE_PROPERTIES>());
    }

    /// Without administrator rights, starting a session is refused with `Denied` (the UI then offers
    /// "Restart as administrator"). Run unelevated; skipped when elevated.
    #[test]
    fn unelevated_start_is_denied() {
        if can_record() {
            eprintln!("skipped: running elevated");
            return;
        }
        let e = Session::start(&format!("{PREFIX}{}-0-test", std::process::id()), Preset::Cpu)
            .err()
            .unwrap();
        assert!(matches!(e, EtwError::Denied), "{e:?}");
        assert!(DotnetError::from(e).to_string().contains("administrator"));
    }

    #[test]
    fn a_dead_process_is_not_alive() {
        assert!(!process_alive(u32::MAX - 1));
        assert!(process_alive(std::process::id()));
    }
}
