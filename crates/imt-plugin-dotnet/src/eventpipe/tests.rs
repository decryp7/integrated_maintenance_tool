//! The recording loop against a fake diagnostics server.

use super::protocol::{self, commands, reply};
use super::*;
use crate::nettrace::{system_time, EventType, NetTraceWriter, TraceHeader};
use crate::Recording;
use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// Streams the first half, the rest after StopTracing.
    Normal,
    Refuse,
    Garbage,
    /// Closes right after accepting.
    CloseAtOnce,
    /// Sends the first half and closes (target exited).
    CloseMidStream,
    /// Sends data forever until stopped.
    Endless,
    /// Sends the first half, then is quiet until stopped (heap snapshot end detection).
    Quiet,
    /// Never answers.
    Hang,
    /// ProcessInfo2 unknown (old runtime): answers ProcessInfo only.
    OldRuntime,
}

struct Fake {
    behaviour: Behaviour,
    trace: Vec<u8>,
    stopped: Arc<AtomicBool>,
    stops: AtomicUsize,
}

impl Fake {
    fn new(behaviour: Behaviour) -> Arc<Fake> {
        Arc::new(Fake {
            behaviour,
            trace: sample_trace(),
            stopped: Arc::new(AtomicBool::new(false)),
            stops: AtomicUsize::new(0),
        })
    }
}

fn sample_trace() -> Vec<u8> {
    let h = TraceHeader {
        sync_time: system_time(1_792_000_000_000).unwrap(),
        sync_ticks: 0,
        ticks_per_second: 10_000_000,
        pointer_size: 8,
        process_id: 4242,
        processors: 2,
        sampling_ns: 1_000_000,
    };
    let mut w = NetTraceWriter::new(Vec::new(), &h).unwrap();
    let ty = EventType {
        provider: crate::clr::SAMPLE_PROFILER,
        id: 0,
        version: 0,
        name: "ThreadSample",
    };
    for i in 0..2000 {
        w.event(ty, 1, 10 + i, &[0x1000 + (i as u64 % 7)], &2u32.to_le_bytes()).unwrap();
    }
    w.finish().unwrap()
}

struct Conn {
    fake: Arc<Fake>,
    written: Vec<u8>,
    out: Arc<Mutex<VecDeque<u8>>>,
    closed: Arc<AtomicBool>,
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            {
                let mut q = self.out.lock().unwrap();
                if !q.is_empty() {
                    let n = buf.len().min(q.len());
                    for b in buf.iter_mut().take(n) {
                        *b = q.pop_front().unwrap();
                    }
                    return Ok(n);
                }
            }
            if self.closed.load(Ordering::SeqCst) {
                return Ok(0);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.written.extend_from_slice(buf);
        if self.written.len() >= protocol::HEADER_LEN {
            let size = usize::from(u16::from_le_bytes([self.written[14], self.written[15]]));
            if self.written.len() >= size {
                let (set, id) = (self.written[16], self.written[17]);
                self.handle(set, id);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Conn {
    fn send(&self, bytes: &[u8]) {
        self.out.lock().unwrap().extend(bytes.iter().copied());
    }

    fn handle(&mut self, set: u8, id: u8) {
        let f = self.fake.clone();
        match (set, id) {
            (commands::PROCESS, commands::PROCESS_INFO_2) if f.behaviour == Behaviour::OldRuntime => {
                self.send(&reply::error(0x8013_1384));
                self.closed.store(true, Ordering::SeqCst);
            }
            (commands::PROCESS, _) => {
                self.send(&reply::process_info(4242, "Shop.exe --serve", "X64", "10.0.1"));
                self.closed.store(true, Ordering::SeqCst);
            }
            (commands::EVENTPIPE, commands::STOP_TRACING) => {
                f.stops.fetch_add(1, Ordering::SeqCst);
                f.stopped.store(true, Ordering::SeqCst);
                self.send(&reply::ok(&7u64.to_le_bytes()));
                self.closed.store(true, Ordering::SeqCst);
            }
            (commands::EVENTPIPE, commands::COLLECT_TRACING_2) => match f.behaviour {
                Behaviour::Refuse => {
                    self.send(&reply::error(0x8007_0057));
                    self.closed.store(true, Ordering::SeqCst);
                }
                Behaviour::Garbage => {
                    self.send(&[0x42; 40]);
                    self.closed.store(true, Ordering::SeqCst);
                }
                Behaviour::Hang => {}
                b => {
                    self.send(&reply::ok(&7u64.to_le_bytes()));
                    let (out, closed) = (self.out.clone(), self.closed.clone());
                    std::thread::spawn(move || {
                        let half = f.trace.len() / 2;
                        if b == Behaviour::CloseAtOnce {
                            closed.store(true, Ordering::SeqCst);
                            return;
                        }
                        out.lock().unwrap().extend(f.trace[..half].iter().copied());
                        if b == Behaviour::CloseMidStream {
                            closed.store(true, Ordering::SeqCst);
                            return;
                        }
                        while !f.stopped.load(Ordering::SeqCst) {
                            if b == Behaviour::Endless {
                                out.lock().unwrap().extend(std::iter::repeat_n(0u8, 64 * 1024));
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        if b != Behaviour::Endless {
                            out.lock().unwrap().extend(f.trace[half..].iter().copied());
                        }
                        closed.store(true, Ordering::SeqCst);
                    });
                }
            },
            _ => {
                self.send(&reply::error(1));
                self.closed.store(true, Ordering::SeqCst);
            }
        }
    }
}

struct FakeConnector(Arc<Fake>);

impl Connector for FakeConnector {
    fn connect(&self, _pid: u32, _timeout: Duration) -> Result<Box<dyn Stream>, DotnetError> {
        Ok(Box::new(Conn {
            fake: self.0.clone(),
            written: Vec::new(),
            out: Arc::new(Mutex::new(VecDeque::new())),
            closed: Arc::new(AtomicBool::new(false)),
        }))
    }
}

struct Refusing(fn() -> DotnetError);

impl Connector for Refusing {
    fn connect(&self, _: u32, _: Duration) -> Result<Box<dyn Stream>, DotnetError> {
        Err((self.0)())
    }
}

fn limits() -> Limits {
    Limits {
        connect_timeout: Duration::from_millis(500),
        ..Limits::default()
    }
}

fn start(b: Behaviour, preset: Preset, limits: Limits) -> (Arc<Fake>, tempfile::TempDir, PathBuf, Result<ThreadRecording, DotnetError>) {
    let fake = Fake::new(b);
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("t.nettrace");
    let r = record_with(Arc::new(FakeConnector(fake.clone())), 4242, preset, &out, limits);
    (fake, dir, out, r)
}

fn wait_finished(r: &ThreadRecording, within: Duration) -> bool {
    let t = Instant::now();
    while !r.is_finished() && t.elapsed() < within {
        std::thread::sleep(Duration::from_millis(10));
    }
    r.is_finished()
}

#[test]
fn records_until_stopped_and_the_file_imports() {
    let (fake, _dir, out, r) = start(Behaviour::Normal, Preset::Cpu, limits());
    let r = r.unwrap();
    let t = Instant::now();
    while r.bytes() == 0 && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(r.bytes() > 0);
    assert!(!r.is_finished());
    let f = Box::new(r).stop().unwrap();
    assert_eq!(fake.stops.load(Ordering::SeqCst), 1);
    assert!(!f.truncated);
    assert_eq!(f.bytes as usize, fake.trace.len());
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(bytes, fake.trace);
    use imt_profile::CaptureImporter;
    assert!(imt_plugin_profiling::nettrace::NettraceImporter
        .import("t.nettrace", &bytes, &imt_profile::Limits::default())
        .is_ok());
}

#[test]
fn refusals_garbage_and_hangs_are_errors() {
    let (_, _d, out, r) = start(Behaviour::Refuse, Preset::Cpu, limits());
    assert!(matches!(r, Err(DotnetError::Protocol(m)) if m.contains("refused")));
    assert!(!out.exists());
    let (_, _d, _, r) = start(Behaviour::Garbage, Preset::Cpu, limits());
    assert!(matches!(r, Err(DotnetError::Protocol(_))));
    let (_, _d, _, r) = start(Behaviour::Hang, Preset::Cpu, limits());
    assert!(matches!(r, Err(DotnetError::Timeout)));
    for e in [|| DotnetError::Denied, || DotnetError::NotFound { pid: 1 }] {
        let dir = tempfile::tempdir().unwrap();
        let r = record_with(Arc::new(Refusing(e)), 1, Preset::Cpu, &dir.path().join("x"), limits());
        assert!(matches!(r, Err(DotnetError::Denied | DotnetError::NotFound { .. })));
    }
}

#[test]
fn a_peer_closing_early_is_reported() {
    let (_, _d, out, r) = start(Behaviour::CloseAtOnce, Preset::Cpu, limits());
    let r = r.unwrap();
    assert!(wait_finished(&r, Duration::from_secs(5)));
    assert!(matches!(Box::new(r).stop(), Err(DotnetError::Protocol(m)) if m.contains("no trace data")));
    assert!(!out.exists(), "an empty capture is not kept");

    let (fake, _d, _, r) = start(Behaviour::CloseMidStream, Preset::Cpu, limits());
    let r = r.unwrap();
    assert!(wait_finished(&r, Duration::from_secs(5)), "ends by itself");
    let f = Box::new(r).stop().unwrap();
    assert_eq!(f.bytes as usize, fake.trace.len() / 2);
    assert!(f.notes.iter().any(|n| n.contains("ended the trace")), "{:?}", f.notes);
}

#[test]
fn the_size_limit_stops_and_cuts() {
    let l = Limits {
        max_bytes: 512 * 1024,
        ..limits()
    };
    let (fake, _d, _, r) = start(Behaviour::Endless, Preset::Cpu, l);
    let r = r.unwrap();
    assert!(wait_finished(&r, Duration::from_secs(20)));
    let f = Box::new(r).stop().unwrap();
    assert!(f.bytes <= l.max_bytes);
    let t = Instant::now();
    while fake.stops.load(Ordering::SeqCst) == 0 && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(fake.stops.load(Ordering::SeqCst) >= 1, "the session is stopped at the limit");
    assert!(f.truncated || f.bytes >= (l.max_bytes as f64 * 0.9) as u64);
}

#[test]
fn heap_snapshots_end_when_the_stream_is_quiet_and_duration_caps_apply() {
    let (fake, _d, _, r) = start(Behaviour::Quiet, Preset::HeapSnapshot, limits());
    let r = r.unwrap();
    assert!(wait_finished(&r, Duration::from_secs(10)), "quiet after data ends a heap snapshot");
    let f = Box::new(r).stop().unwrap();
    assert_eq!(fake.stops.load(Ordering::SeqCst), 1);
    assert_eq!(f.bytes as usize, fake.trace.len());

    let l = Limits {
        max_duration: Duration::from_millis(300),
        ..limits()
    };
    let (_, _d, _, r) = start(Behaviour::Normal, Preset::Cpu, l);
    let r = r.unwrap();
    assert!(wait_finished(&r, Duration::from_secs(10)), "the duration limit stops it");
}

#[test]
fn cancel_stops_the_session_and_deletes_the_file() {
    let (fake, _d, out, r) = start(Behaviour::Normal, Preset::Allocations, limits());
    let r = r.unwrap();
    std::thread::sleep(Duration::from_millis(50));
    Box::new(r).cancel();
    assert!(!out.exists());
    // StopTracing is sent from its own thread.
    let t = Instant::now();
    while fake.stops.load(Ordering::SeqCst) == 0 && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fake.stops.load(Ordering::SeqCst), 1);
}

#[test]
fn process_info_falls_back_to_v1_and_times_out() {
    let p = process_info_with(Arc::new(FakeConnector(Fake::new(Behaviour::Normal))), 4242, Duration::from_secs(2)).unwrap();
    assert_eq!(p.command_line, "Shop.exe --serve");
    assert_eq!(p.runtime_version.as_deref(), Some("10.0.1"));
    let p = process_info_with(
        Arc::new(FakeConnector(Fake::new(Behaviour::OldRuntime))),
        4242,
        Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(p.runtime_version, None);
    assert_eq!(p.arch, "X64");
    let e = process_info_with(Arc::new(Refusing(|| DotnetError::Timeout)), 1, Duration::from_secs(1));
    assert!(matches!(e, Err(DotnetError::Timeout)));
}
