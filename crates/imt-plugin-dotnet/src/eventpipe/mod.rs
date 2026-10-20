//! EventPipe over the .NET diagnostics IPC (.NET Core 3+ / .NET 5+, every
//! platform, no elevation): `CollectTracing2` on one connection streams the
//! NetTrace file, `StopTracing` on a second one ends the session (the runtime
//! then writes the rundown and closes the stream).
//!
//! The transport is a [`Connector`] (named pipe `\\.\pipe\dotnet-diagnostic-<pid>`
//! on Windows, Unix socket `$TMPDIR/dotnet-diagnostic-<pid>-<key>-socket`
//! elsewhere), so the protocol and the recording loop are tested against a
//! fake target.

pub mod protocol;
mod transport;

pub use transport::{Connector, Stream, SystemConnector};

use crate::recording::{Control, ThreadRecording};
use crate::{DotnetError, Finished, Limits, Preset, HEAP_SNAPSHOT_CAP};
use protocol::{IpcProcessInfo, ProtocolError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A heap snapshot ends when the stream has been quiet this long after data.
pub const HEAP_QUIET: Duration = Duration::from_millis(1500);
/// After `StopTracing`, the rundown and the rest of the stream must arrive within this.
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(60);
/// Stop is requested at this share of the size limit, so the runtime can end the file properly.
const STOP_AT: f64 = 0.9;

impl From<ProtocolError> for DotnetError {
    fn from(e: ProtocolError) -> Self {
        DotnetError::Protocol(e.to_string())
    }
}

/// Sends `request` and reads one reply; returns its payload (errors: the runtime refused, garbage).
pub fn exchange(s: &mut dyn Stream, request: &[u8]) -> Result<Vec<u8>, DotnetError> {
    s.write_all(request)?;
    s.flush()?;
    let mut head = [0u8; protocol::HEADER_LEN];
    s.read_exact(&mut head).map_err(|e| match e.kind() {
        std::io::ErrorKind::UnexpectedEof => DotnetError::Protocol("the target closed the connection without answering".into()),
        _ => DotnetError::Io(e),
    })?;
    let (size, set, id) = protocol::header(&head)?;
    let mut payload = vec![0u8; size - protocol::HEADER_LEN];
    s.read_exact(&mut payload)?;
    Ok(protocol::ok_payload(set, id, &payload)?.to_vec())
}

/// Runs `f` on a helper thread, giving up after `timeout` (a stuck target never blocks the caller).
fn with_timeout<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce() -> Result<T, DotnetError> + Send + 'static,
) -> Result<T, DotnetError> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("imt-dotnet-ipc".into())
        .spawn(move || {
            let _ = tx.send(f());
        })
        .map_err(DotnetError::Io)?;
    rx.recv_timeout(timeout).unwrap_or(Err(DotnetError::Timeout))
}

/// `ProcessInfo2` (falling back to `ProcessInfo` for older runtimes) of `pid`.
pub fn process_info(pid: u32, timeout: Duration) -> Result<IpcProcessInfo, DotnetError> {
    process_info_with(Arc::new(SystemConnector), pid, timeout)
}

pub fn process_info_with(c: Arc<dyn Connector>, pid: u32, timeout: Duration) -> Result<IpcProcessInfo, DotnetError> {
    with_timeout(timeout, move || {
        let mut s = c.connect(pid, timeout)?;
        match exchange(s.as_mut(), &protocol::process_info(true)) {
            Ok(p) => Ok(protocol::parse_process_info(&p, true)?),
            Err(DotnetError::Protocol(_)) => {
                let mut s = c.connect(pid, timeout)?;
                let p = exchange(s.as_mut(), &protocol::process_info(false))?;
                Ok(protocol::parse_process_info(&p, false)?)
            }
            Err(e) => Err(e),
        }
    })
}

/// Starts recording `pid` into `out`.
pub fn record(pid: u32, preset: Preset, out: &Path, limits: Limits) -> Result<ThreadRecording, DotnetError> {
    record_with(Arc::new(SystemConnector), pid, preset, out, limits)
}

/// [`record`] with a given transport.
pub fn record_with(c: Arc<dyn Connector>, pid: u32, preset: Preset, out: &Path, limits: Limits) -> Result<ThreadRecording, DotnetError> {
    // Start the session before returning, so refusals reach the caller.
    let request = protocol::collect_tracing(&protocol::providers(preset), preset.rundown());
    let c2 = c.clone();
    let timeout = limits.connect_timeout;
    let (stream, session) = with_timeout(timeout, move || {
        let mut s = c2.connect(pid, timeout)?;
        let p = exchange(s.as_mut(), &request)?;
        Ok((s, protocol::session_id(&p)?))
    })?;
    let file = std::fs::File::create(out)?;
    let path = out.to_path_buf();
    ThreadRecording::spawn("imt-dotnet-eventpipe", path.clone(), move |ctl| {
        let r = pump(stream, file, &ctl, c, pid, session, preset, limits, path.clone());
        if r.is_err() {
            let _ = std::fs::remove_file(&path);
        }
        r
    })
}

/// Copies the stream to the file until it ends; stops the session on request or at a limit.
#[allow(clippy::too_many_arguments)]
fn pump(
    stream: Box<dyn Stream>,
    mut file: std::fs::File,
    ctl: &Control,
    c: Arc<dyn Connector>,
    pid: u32,
    session: u64,
    preset: Preset,
    limits: Limits,
    path: PathBuf,
) -> Result<Finished, DotnetError> {
    let (tx, rx) = mpsc::sync_channel::<std::io::Result<Vec<u8>>>(64);
    let mut stream = stream;
    // A reader thread: reads block, the loop below must not.
    std::thread::Builder::new()
        .name("imt-dotnet-eventpipe-read".into())
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(Ok(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    // Windows reports the end of a pipe as ERROR_BROKEN_PIPE / ERROR_PIPE_NOT_CONNECTED.
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe || matches!(e.raw_os_error(), Some(109 | 233)) => break,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        })
        .map_err(DotnetError::Io)?;
    let started = Instant::now();
    let cap = if preset == Preset::HeapSnapshot {
        limits.max_duration.min(HEAP_SNAPSHOT_CAP)
    } else {
        limits.max_duration
    };
    let mut bytes = 0u64;
    let mut truncated = false;
    let mut notes = Vec::new();
    let mut stop_sent: Option<Instant> = None;
    let mut last_data: Option<Instant> = None;
    let mut ended = false;
    // StopTracing runs on its own thread: the runtime answers only after it wrote the rest of the
    // stream (the rundown), which this loop must keep draining meanwhile.
    let (stop_tx, stop_rx) = mpsc::channel::<String>();
    let send_stop = || {
        let (c, tx, timeout) = (c.clone(), stop_tx.clone(), limits.connect_timeout.max(DRAIN_TIMEOUT));
        let _ = std::thread::Builder::new().name("imt-dotnet-eventpipe-stop".into()).spawn(move || {
            if let Err(e) = stop_session(&c, pid, session, timeout) {
                let _ = tx.send(format!(
                    "The session could not be stopped cleanly ({e}); the target may have exited"
                ));
            }
        });
    };
    while !ended {
        let want_stop = ctl.stopping()
            || started.elapsed() >= cap
            || bytes as f64 >= limits.max_bytes as f64 * STOP_AT
            || (preset == Preset::HeapSnapshot && last_data.is_some_and(|t| t.elapsed() >= HEAP_QUIET));
        if want_stop && stop_sent.is_none() {
            send_stop();
            stop_sent = Some(Instant::now());
        }
        if stop_sent.is_some_and(|t| t.elapsed() > DRAIN_TIMEOUT) {
            notes.push("The target did not finish the trace in time; it may be incomplete".into());
            break;
        }
        if ctl.cancelled() {
            return Err(DotnetError::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(chunk)) => {
                let room = limits.max_bytes.saturating_sub(bytes) as usize;
                if chunk.len() > room {
                    file.write_all(&chunk[..room])?;
                    bytes += room as u64;
                    truncated = true;
                    if stop_sent.is_none() {
                        send_stop();
                        stop_sent = Some(Instant::now());
                    }
                    break;
                }
                file.write_all(&chunk)?;
                bytes += chunk.len() as u64;
                ctl.set_bytes(bytes);
                last_data = Some(Instant::now());
            }
            Ok(Err(e)) => {
                if bytes == 0 {
                    return Err(DotnetError::Io(e));
                }
                notes.push(format!("The connection to the target broke ({e}); the trace may be incomplete"));
                ended = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => ended = true,
        }
    }
    file.flush()?;
    ctl.set_bytes(bytes);
    notes.extend(stop_rx.try_iter());
    if bytes == 0 {
        return Err(DotnetError::Protocol("the target sent no trace data (it may have exited)".into()));
    }
    if stop_sent.is_none() && !truncated {
        notes.push("The target ended the trace (it may have exited)".into());
    }
    if truncated {
        notes.push(format!(
            "The recording reached the size limit of {} MB and was cut",
            limits.max_bytes >> 20
        ));
    }
    Ok(Finished {
        path,
        bytes,
        truncated,
        notes,
    })
}

fn stop_session(c: &Arc<dyn Connector>, pid: u32, session: u64, timeout: Duration) -> Result<(), DotnetError> {
    let c = c.clone();
    with_timeout(timeout, move || {
        let mut s = c.connect(pid, timeout)?;
        exchange(s.as_mut(), &protocol::stop_tracing(session)).map(|_| ())
    })
}

#[cfg(test)]
mod tests;
