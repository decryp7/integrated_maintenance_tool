//! A real-time ETW session recording one .NET process (Windows 8+), on `imt-etw`'s sessions (ADR 0022 §6).
//!
//! One system-logger session ("IMT-dotnet-<imt pid>-<target pid>-<n>") gets
//! the kernel profile / context-switch / ready-thread / process / thread /
//! image-load events (with stack walks on samples and context switches) and
//! the CLR runtime provider filtered to the target. At stop the CLR rundown
//! provider is enabled for method names; the session ends after
//! `DCEndComplete` or a timeout. Sessions are always stopped (stop, cancel,
//! drop, panic); sessions left by a crashed IMT are stopped at the next start.

use super::convert::Converter;
use super::{EtwError, RawEvent};
use crate::nettrace::{system_time, TraceHeader};
use crate::recording::{CaptureSink, Control, ThreadRecording};
use crate::{clr, DotnetError, Finished, Limits, Preset, ProcessInfo, HEAP_SNAPSHOT_CAP};
use imt_etw::session::{self as etw, process_alive, qpc, qpc_frequency, session_name, KernelSpec, Provider};
use std::io::BufWriter;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Session name prefix (orphans are recognised by it).
const PREFIX: &str = "IMT-dotnet-";
/// How long to wait for the rundown at stop.
const RUNDOWN_TIMEOUT: Duration = Duration::from_secs(15);
/// After the heap walk's GC ended, wait this long for trailing type events.
const HEAP_GRACE: Duration = Duration::from_millis(1000);

/// A session with the kernel events of `preset`; stopped when dropped.
struct Session(etw::Session);

impl Session {
    fn start(name: &str, preset: Preset) -> Result<Session, EtwError> {
        Session::start_to(name, preset, None)
    }

    /// A real-time session, or one writing the `.etl` file `file`.
    fn start_to(name: &str, preset: Preset, file: Option<&Path>) -> Result<Session, EtwError> {
        let kernel = KernelSpec {
            samples: preset.samples(),
            waits: preset.samples(),
        };
        etw::Session::start(name, &kernel, file).map(Session)
    }

    /// Enables a provider for `pid` (PID filter when the system supports it; events are filtered again here).
    fn enable(&self, guid: u128, keywords: u64, level: u8, pid: u32, stacks: bool) -> Result<(), EtwError> {
        self.0.enable(&Provider {
            guid,
            keywords,
            level,
            pid,
            stacks,
        })
    }

    fn stop(&mut self) {
        self.0.stop();
    }
}

/// Stops sessions left by an IMT that crashed.
fn stop_orphans() {
    etw::stop_orphans(PREFIX);
}

/// The converter writing into the capture's sink.
type SinkConverter = Converter<BufWriter<Box<dyn CaptureSink>>>;

/// What the consumer thread shares with the controller.
struct Shared {
    conv: Mutex<Option<SinkConverter>>,
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

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Starts recording `target` into `sink`; `saved_at` is where the finished capture will be (reported in
/// [`Finished::path`]). Errors (rights, unsupported) are returned before any thread starts; a failed or cancelled
/// recording drops the sink unfinished, which leaves no partial file.
pub fn record(
    target: &ProcessInfo,
    preset: Preset,
    sink: Box<dyn CaptureSink>,
    saved_at: &Path,
    limits: Limits,
) -> Result<ThreadRecording, DotnetError> {
    stop_orphans();
    let name = session_name(PREFIX, target.pid);
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
    let mut conv = Converter::new(BufWriter::with_capacity(1 << 20, sink), header)?;
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
            let r = etw::consume(&name2, &mut |e| s2.on_event(e));
            done2.store(true, Ordering::Release);
            r
        })
        .map_err(DotnetError::Io)?;

    let pid = target.pid;
    let path = saved_at.to_path_buf();
    ThreadRecording::spawn("imt-dotnet-etw", path.clone(), move |ctl| {
        let r = control(&ctl, session, consumer, &consumer_done, &shared, pid, preset, limits, path.clone());
        if r.is_err() {
            // Dropping the converter drops the sink unfinished: nothing is left behind.
            drop(shared.conv.lock().unwrap_or_else(|p| p.into_inner()).take());
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
    let sink = w.into_inner().map_err(|e| DotnetError::Io(e.into_error()))?;
    let bytes = sink.finish()?;
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
    let name = session_name(PREFIX, target.pid);
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
        assert!(etw::is_orphan(PREFIX, "IMT-dotnet-999999-5-1", |_| false));
        assert!(!etw::is_orphan(PREFIX, "IMT-dotnet-999999-5-1", |_| true));
        assert!(
            !etw::is_orphan(PREFIX, &format!("IMT-dotnet-{me}-5-1"), |_| false),
            "our own sessions"
        );
        assert!(
            !etw::is_orphan(PREFIX, "IMT-native-999999-5-1", |_| false),
            "the native profiler's sessions"
        );
    }

    /// Without administrator rights, starting a session is refused with `Denied` (the UI then offers
    /// "Restart as administrator"). Run unelevated; skipped when elevated.
    #[test]
    fn unelevated_start_is_denied() {
        if can_record() {
            eprintln!("skipped: running elevated");
            return;
        }
        let e = Session::start(&session_name(PREFIX, 0), Preset::Cpu).err().unwrap();
        assert!(matches!(e, EtwError::Denied), "{e:?}");
        assert!(DotnetError::from(e).to_string().contains("administrator"));
    }

    #[test]
    fn a_dead_process_is_not_alive() {
        assert!(!process_alive(u32::MAX - 1));
        assert!(process_alive(std::process::id()));
    }
}
