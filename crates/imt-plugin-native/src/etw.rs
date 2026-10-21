//! The ETW capture of a native process (ADR 0022), run by the plugin's elevated process: one real-time
//! system-logger session with kernel CPU samples, context switches and image loads (stack walks on samples and
//! switches), optionally the heap provider for the target, and the CLR providers when the target runs .NET
//! (mixed stacks). Events go through `imt-etw`'s converter into NetTrace; the symbolisation step names the
//! collected addresses before the file is closed. Sessions are always stopped (stop, cancel, drop, panic), and
//! sessions of a crashed IMT are stopped at the next start.

use crate::captures::Control;
use crate::{Finished, NativeError};
use imt_symbols::SymbolConfig;
use std::path::Path;
use std::time::Duration;

/// Session name prefix (orphans are recognised by it).
pub const PREFIX: &str = "IMT-native-";
/// How long to wait for the CLR rundown at stop.
pub const RUNDOWN_TIMEOUT: Duration = Duration::from_secs(15);

/// What to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EtwSpec {
    /// The heap provider's allocations and frees of the target.
    pub heap: bool,
    pub duration: Duration,
    pub max_bytes: u64,
    pub max_addresses: usize,
}

/// Records `pid` into `out` until `ctl` stops, the duration ends, the size limit is reached or the process
/// exits; then names the code addresses with `symbols` and closes the file.
#[cfg(windows)]
pub fn record(pid: u32, spec: &EtwSpec, out: &Path, symbols: &SymbolConfig, ctl: &Control) -> Result<Finished, NativeError> {
    let r = windows_impl::record(pid, spec, out, symbols, ctl);
    if r.is_err() {
        let _ = std::fs::remove_file(out);
    }
    r
}

#[cfg(not(windows))]
pub fn record(_: u32, _: &EtwSpec, _: &Path, _: &SymbolConfig, _: &Control) -> Result<Finished, NativeError> {
    Err(NativeError::Unsupported("ETW recording is only available on Windows".into()))
}

#[cfg(windows)]
mod windows_impl {
    use super::{EtwSpec, PREFIX, RUNDOWN_TIMEOUT};
    use crate::captures::Control;
    use crate::{process, symbolize, Finished, NativeError};
    use imt_etw::convert::{Converter, Options};
    use imt_etw::nettrace::{system_time, TraceHeader};
    use imt_etw::session::{self, KernelSpec, Provider, Session};
    use imt_etw::{clr, heap, EtwError, RawEvent};
    use imt_symbols::SymbolConfig;
    use std::io::BufWriter;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    const RUNDOWN_DONE: u8 = 1;
    const FULL: u8 = 4;
    const WRITE_FAILED: u8 = 8;

    type Conv = Converter<BufWriter<std::fs::File>>;

    /// What the consumer thread shares with the controller.
    struct Shared {
        conv: Mutex<Option<Conv>>,
        bytes: AtomicU64,
        flags: AtomicU8,
        max_bytes: u64,
    }

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
            let mut f = 0;
            if conv.milestones().rundown_complete {
                f |= RUNDOWN_DONE;
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

    fn pointer_size(pid: u32) -> Result<u32, NativeError> {
        // SAFETY: the handle is closed below.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.map_err(|_| NativeError::NotFound { pid })?;
        let arch = process::arch_of(h);
        // SAFETY: opened above.
        unsafe {
            let _ = CloseHandle(h);
        }
        Ok(if arch == Some("x86") { 4 } else { 8 })
    }

    fn unix_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    }

    pub(super) fn record(pid: u32, spec: &EtwSpec, out: &Path, symbols: &SymbolConfig, ctl: &Control) -> Result<Finished, NativeError> {
        session::stop_orphans(PREFIX);
        let pointer_size = pointer_size(pid)?;
        let clr_loaded = process::has_clr(pid);
        let name = session::session_name(PREFIX, pid);
        let mut session = Session::start(
            &name,
            &KernelSpec {
                samples: true,
                waits: true,
            },
            None,
        )?;
        if spec.heap {
            session.enable(&Provider {
                guid: heap::PROVIDER,
                keywords: u64::MAX,
                level: clr::level::VERBOSE,
                pid,
                stacks: true,
            })?;
        }
        if clr_loaded {
            // Method load events give the JIT-compiled code ranges of managed frames.
            session.enable(&Provider {
                guid: clr::RUNTIME_GUID,
                keywords: clr::keywords::LOADER | clr::keywords::JIT,
                level: clr::level::VERBOSE,
                pid,
                stacks: false,
            })?;
        }
        let header = TraceHeader {
            sync_time: system_time(unix_ms()).unwrap_or([1970, 1, 4, 1, 0, 0, 0, 0]),
            sync_ticks: session::qpc(),
            ticks_per_second: session::qpc_frequency(),
            pointer_size,
            process_id: pid,
            processors: std::thread::available_parallelism().map_or(1, |n| n.get() as u32),
            sampling_ns: 1_000_000,
        };
        let file = std::fs::File::create(out)?;
        let options = Options {
            heap: spec.heap,
            max_addresses: spec.max_addresses,
        };
        let mut conv = Converter::with_options(BufWriter::with_capacity(1 << 20, file), header, options)?;
        conv.seed_threads(process::threads(pid));
        let shared = Arc::new(Shared {
            conv: Mutex::new(Some(conv)),
            bytes: AtomicU64::new(0),
            flags: AtomicU8::new(0),
            max_bytes: spec.max_bytes,
        });
        let consumer_done = Arc::new(AtomicBool::new(false));
        let (s2, done2, name2) = (shared.clone(), consumer_done.clone(), name.clone());
        let consumer = std::thread::Builder::new().name("imt-native-etw-consumer".into()).spawn(move || {
            let r = session::consume(&name2, &mut |e| s2.on_event(e));
            done2.store(true, Ordering::Release);
            r
        })?;

        let started = Instant::now();
        let mut notes = Vec::new();
        let mut exited = false;
        loop {
            ctl.set_bytes(shared.bytes.load(Ordering::Relaxed));
            if ctl.stopping() || started.elapsed() >= spec.duration || shared.has(FULL) || shared.has(WRITE_FAILED) {
                break;
            }
            if consumer_done.load(Ordering::Acquire) {
                notes.push("The ETW session ended early (stopped by another tool?)".to_string());
                break;
            }
            if !session::process_alive(pid) {
                exited = true;
                notes.push("The process exited during the capture".to_string());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if ctl.cancelled() {
            session.stop();
            let _ = consumer.join();
            return Err(NativeError::Cancelled);
        }
        if clr_loaded && !exited && !shared.has(FULL) {
            let rundown = Provider {
                guid: clr::RUNDOWN_GUID,
                keywords: clr::keywords::RUNDOWN_END,
                level: clr::level::VERBOSE,
                pid,
                stacks: false,
            };
            match session.enable(&rundown) {
                Ok(()) => {
                    let t = Instant::now();
                    while !shared.has(RUNDOWN_DONE) && t.elapsed() < RUNDOWN_TIMEOUT && !ctl.cancelled() && !shared.has(FULL) {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    if !shared.has(RUNDOWN_DONE) {
                        notes.push(
                            "The .NET runtime did not finish listing its methods in time; some managed frames may show as addresses".into(),
                        );
                    }
                }
                Err(e) => notes.push(format!("Managed method names could not be collected ({e})")),
            }
        }
        session.stop();
        let consumed = consumer
            .join()
            .unwrap_or_else(|_| Err(EtwError::Unsupported("the ETW consumer thread failed".into())));
        if ctl.cancelled() {
            return Err(NativeError::Cancelled);
        }
        match consumed {
            Ok(true) => notes.push("Some events could not be converted (a bug in IMT); the capture may be incomplete".into()),
            Ok(false) => {}
            Err(e) => notes.push(format!("Reading the session failed: {e}")),
        }
        if shared.has(WRITE_FAILED) {
            return Err(NativeError::Failed("writing the capture file failed (is the disk full?)".into()));
        }
        let mut conv = shared
            .conv
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .ok_or_else(|| NativeError::Failed("the capture was already finished".into()))?;
        let (syms, sym_notes) = symbolize::symbolize(symbols, conv.images(), &conv.addresses(), spec.max_addresses);
        conv.symbols(&syms)?;
        let stats = conv.stats();
        let (w, _) = conv.finish()?;
        let file = w.into_inner().map_err(|e| NativeError::Io(e.into_error()))?;
        file.sync_all()?;
        let bytes = file.metadata()?.len();
        let truncated = shared.has(FULL);
        if truncated {
            notes.push(format!(
                "The capture reached the size limit of {} MB and was cut",
                spec.max_bytes >> 20
            ));
        }
        if stats.samples == 0 {
            notes.push("No CPU samples were recorded for the process (was it idle?)".into());
        }
        if spec.heap && stats.heap_ops == 0 {
            notes.push(
                "No heap events arrived: heap tracing may need the process to be started with tracing enabled on this Windows version"
                    .into(),
            );
        }
        if clr_loaded {
            notes.push(".NET and native frames are merged where the runtime published its methods; see ADR 0022 for the limits".into());
        }
        if stats.addresses_dropped > 0 {
            notes.push(format!(
                "{} code addresses beyond the limit were not symbolised",
                stats.addresses_dropped
            ));
        }
        notes.extend(sym_notes);
        Ok(Finished {
            path: out.to_path_buf(),
            bytes,
            truncated,
            notes,
        })
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Without administrator rights the session is refused with `Denied` (the host then shows the
    /// "Restart as administrator" hint). Skipped when elevated.
    #[test]
    fn unelevated_recording_is_denied() {
        if imt_etw::session::can_record() {
            eprintln!("skipped: running elevated");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("x.nettrace");
        let spec = EtwSpec {
            heap: false,
            duration: Duration::from_secs(1),
            max_bytes: 1 << 20,
            max_addresses: 10,
        };
        let e = record(std::process::id(), &spec, &out, &SymbolConfig::default(), &Control::default()).unwrap_err();
        assert!(matches!(e, NativeError::Denied(_)), "{e:?}");
        assert!(!out.exists(), "a failed capture leaves no file");
    }
}
