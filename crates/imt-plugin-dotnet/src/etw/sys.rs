//! The Win32 ETW consumer glue shared by the live session and the `.etl`
//! reader: `OpenTraceW` + `ProcessTrace` on the calling thread, each
//! `EVENT_RECORD` copied into a [`RawEvent`] for a Rust callback. A panic in
//! the callback is caught (it must not unwind into the OS); later events are
//! then ignored and the caller learns about it.

use super::{win32_error, EtwError, RawEvent};
use std::panic::{catch_unwind, AssertUnwindSafe};
use windows::core::PWSTR;
use windows::Win32::System::Diagnostics::Etw::*;

struct Ctx<'a> {
    sink: &'a mut dyn FnMut(&RawEvent<'_>) -> bool,
    stop: bool,
    panicked: bool,
    stack: Vec<u64>,
}

/// Header flag of events logged by a 32-bit process.
const FLAG_32_BIT: u16 = EVENT_HEADER_FLAG_32_BIT_HEADER as u16;
/// Most frames copied from an extended-data stack.
const MAX_STACK: usize = 1024;

/// Copies `rec`'s extended-data stack (32- or 64-bit) into `out`, leaf first.
///
/// # Safety
/// `rec` must be a valid record from `ProcessTrace`.
unsafe fn copy_stack(rec: &EVENT_RECORD, out: &mut Vec<u64>) {
    out.clear();
    if rec.ExtendedData.is_null() {
        return;
    }
    // SAFETY: ExtendedData points at ExtendedDataCount items for the callback's duration.
    let items = unsafe { std::slice::from_raw_parts(rec.ExtendedData, usize::from(rec.ExtendedDataCount)) };
    for item in items {
        let width = match u32::from(item.ExtType) {
            EVENT_HEADER_EXT_TYPE_STACK_TRACE32 => 4,
            EVENT_HEADER_EXT_TYPE_STACK_TRACE64 => 8,
            _ => continue,
        };
        let size = usize::from(item.DataSize);
        if item.DataPtr == 0 || size < 8 {
            continue;
        }
        // SAFETY: DataPtr points at DataSize bytes: MatchId (u64) then the addresses.
        let data = unsafe { std::slice::from_raw_parts(item.DataPtr as *const u8, size) };
        for w in data[8..].chunks_exact(width).take(MAX_STACK) {
            let mut a = [0u8; 8];
            a[..width].copy_from_slice(w);
            out.push(u64::from_le_bytes(a));
        }
        return;
    }
}

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    // SAFETY: ProcessTrace passes a valid record whose UserContext is the Ctx we registered.
    let Some(rec) = (unsafe { rec.as_ref() }) else { return };
    let Some(ctx) = (unsafe { (rec.UserContext as *mut Ctx<'_>).as_mut() }) else {
        return;
    };
    if ctx.stop || ctx.panicked {
        return;
    }
    let mut stack = std::mem::take(&mut ctx.stack);
    unsafe { copy_stack(rec, &mut stack) };
    let payload: &[u8] = if rec.UserData.is_null() || rec.UserDataLength == 0 {
        &[]
    } else {
        // SAFETY: UserData points at UserDataLength bytes for the callback's duration.
        unsafe { std::slice::from_raw_parts(rec.UserData as *const u8, usize::from(rec.UserDataLength)) }
    };
    let h = &rec.EventHeader;
    let raw = RawEvent {
        provider: h.ProviderId.to_u128(),
        id: h.EventDescriptor.Id,
        version: h.EventDescriptor.Version,
        opcode: h.EventDescriptor.Opcode,
        process_id: h.ProcessId,
        thread_id: h.ThreadId,
        timestamp: h.TimeStamp,
        pointer_size: if h.Flags & FLAG_32_BIT != 0 { 4 } else { 8 },
        payload,
        stack: &stack,
    };
    match catch_unwind(AssertUnwindSafe(|| (ctx.sink)(&raw))) {
        Ok(true) => {}
        Ok(false) => ctx.stop = true,
        Err(_) => ctx.panicked = true,
    }
    ctx.stack = stack;
}

unsafe extern "system" fn on_buffer(log: *mut EVENT_TRACE_LOGFILEW) -> u32 {
    // SAFETY: ProcessTrace passes the logfile we opened; Context is our Ctx.
    let stop = unsafe { log.as_ref() }
        .and_then(|l| unsafe { (l.Context as *const Ctx<'_>).as_ref() })
        .is_some_and(|c| c.stop || c.panicked);
    // TRUE continues processing.
    u32::from(!stop)
}

/// The log file header of an opened trace (clock and start time of `.etl` files).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LogHeader {
    /// 1 QPC, 2 system time, 3 CPU cycle counter.
    pub clock: u32,
    pub perf_freq: i64,
    pub cpu_mhz: u32,
    /// FILETIME (100 ns since 1601) of the start.
    pub start_time: i64,
    pub processors: u32,
    pub events_lost: u32,
}

impl LogHeader {
    /// Ticks per second of the raw timestamps.
    pub fn ticks_per_second(&self) -> i64 {
        match self.clock {
            2 => 10_000_000,
            3 if self.cpu_mhz > 0 => i64::from(self.cpu_mhz) * 1_000_000,
            _ if self.perf_freq > 0 => self.perf_freq,
            _ => 10_000_000,
        }
    }
}

/// How to open the trace.
pub(crate) enum Source<'a> {
    /// A real-time session by name.
    Session(&'a str),
    /// An `.etl` file.
    File(&'a std::path::Path),
}

/// Opens `source`, calls `on_header` once with its header, then feeds every event to `sink`
/// until the session stops, the file ends, or `sink` returns `false`.
/// Returns whether the callback panicked (events after that were ignored).
pub(crate) fn consume(
    source: Source<'_>,
    on_header: impl FnOnce(LogHeader),
    sink: &mut dyn FnMut(&RawEvent<'_>) -> bool,
) -> Result<bool, EtwError> {
    let (mut name, real_time) = match source {
        Source::Session(n) => (wide(n), true),
        Source::File(p) => (wide(&p.to_string_lossy()), false),
    };
    let mut ctx = Ctx {
        sink,
        stop: false,
        panicked: false,
        stack: Vec::with_capacity(256),
    };
    let mut log = EVENT_TRACE_LOGFILEW::default();
    if real_time {
        log.LoggerName = PWSTR(name.as_mut_ptr());
    } else {
        log.LogFileName = PWSTR(name.as_mut_ptr());
    }
    let mut mode = PROCESS_TRACE_MODE_EVENT_RECORD | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
    if real_time {
        mode |= PROCESS_TRACE_MODE_REAL_TIME;
    }
    log.Anonymous1.ProcessTraceMode = mode;
    log.Anonymous2.EventRecordCallback = Some(on_event);
    log.BufferCallback = Some(on_buffer);
    log.Context = &mut ctx as *mut Ctx<'_> as *mut core::ffi::c_void;
    // SAFETY: `log`, `name` and `ctx` outlive the trace handle (closed below before they drop).
    let handle = unsafe { OpenTraceW(&mut log) };
    if handle.Value == u64::MAX || handle.Value == 0xffff_ffff {
        // SAFETY: reads the calling thread's last error.
        let code = unsafe { windows::Win32::Foundation::GetLastError() }.0;
        return Err(win32_error("OpenTraceW", code));
    }
    let h = &log.LogfileHeader;
    // SAFETY: the union holds plain integers filled by OpenTraceW.
    let pointer_info = unsafe { h.Anonymous2.Anonymous };
    on_header(LogHeader {
        clock: h.ReservedFlags,
        perf_freq: h.PerfFreq,
        cpu_mhz: pointer_info.CpuSpeedInMHz,
        start_time: h.StartTime,
        processors: h.NumberOfProcessors,
        events_lost: pointer_info.EventsLost,
    });
    // SAFETY: a valid handle; ProcessTrace calls back on this thread only.
    let status = unsafe { ProcessTrace(&[handle], None, None) };
    // SAFETY: the handle came from OpenTraceW.
    unsafe {
        let _ = CloseTrace(handle);
    }
    let panicked = ctx.panicked;
    // ERROR_CANCELLED (1223) when the buffer callback stopped the file on purpose.
    if status.0 != 0 && status.0 != 1223 && !ctx.stop {
        return Err(win32_error("ProcessTrace", status.0));
    }
    Ok(panicked)
}

/// NUL-terminated UTF-16.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
