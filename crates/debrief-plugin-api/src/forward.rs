//! The elevated process' standard error, forwarded to the host (ADR 0017 §7, amendment 2026-10-22): it has no
//! pipe the host could read, so it redirects its own standard error into a pipe ([`debrief_ipc::stderr_pipe`]) and
//! sends the lines in [`HostRequest::Log`] batches. Bounded: lines are cut at [`MAX_LOG_LINE`], at most
//! [`QUEUE`] lines wait, batches go out at most every [`BATCH_EVERY`], and after [`MAX_LOG_LINES`] lines the rest
//! are only counted. The pipe is always drained, so a chatty plugin never blocks on its own standard error.

use crate::protocol::{cut_line, HostRequest, MAX_LOG_BATCH, MAX_LOG_LINE, MAX_LOG_LINES};
use debrief_ipc::Peer;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, TrySendError};
use std::sync::Arc;
use std::time::Duration;

/// Lines waiting to be sent; more are dropped (and counted).
const QUEUE: usize = 1024;
/// Least time between two batches.
const BATCH_EVERY: Duration = Duration::from_millis(50);
/// How long one batch may take.
const SEND_TIMEOUT: Option<Duration> = Some(Duration::from_secs(10));

/// Splits a byte stream into lines of at most `max` bytes; the rest of a longer line is dropped.
pub(crate) struct Lines {
    cur: Vec<u8>,
    max: usize,
}

impl Lines {
    pub(crate) fn new(max: usize) -> Lines {
        Lines { cur: Vec::new(), max }
    }

    /// Feeds `bytes`; calls `emit` with each completed line (without its line end).
    pub(crate) fn feed(&mut self, bytes: &[u8], emit: &mut impl FnMut(String)) {
        for &b in bytes {
            if b == b'\n' {
                emit(self.take());
            } else if self.cur.len() < self.max {
                self.cur.push(b);
            }
        }
    }

    /// The unterminated last line, if any.
    pub(crate) fn rest(&mut self) -> Option<String> {
        (!self.cur.is_empty()).then(|| self.take())
    }

    fn take(&mut self) -> String {
        let text = String::from_utf8_lossy(&self.cur);
        let line = cut_line(text.trim_end_matches('\r'), self.max).to_string();
        self.cur.clear();
        line
    }
}

/// Redirects standard error and forwards its lines to the host over `peer`. Failing to redirect leaves standard
/// error as it was (the lines are lost, as before).
pub(crate) fn forward_stderr(peer: &Peer) {
    match debrief_ipc::stderr_pipe::redirect() {
        Ok(pipe) => forward(pipe, peer.clone()),
        Err(e) => {
            let _: Result<(), _> = peer.request(
                &HostRequest::Log {
                    lines: vec![format!("standard error is not forwarded: {e}")],
                    dropped: 0,
                },
                SEND_TIMEOUT,
            );
        }
    }
}

/// Forwards the lines read from `input` to the host (also used by tests with any reader).
pub(crate) fn forward(mut input: impl Read + Send + 'static, peer: Peer) {
    let (tx, rx) = sync_channel::<String>(QUEUE);
    let dropped = Arc::new(AtomicU64::new(0));
    let d = dropped.clone();
    let _ = std::thread::Builder::new().name("debrief-stderr-read".into()).spawn(move || {
        let mut lines = Lines::new(MAX_LOG_LINE);
        let mut buf = vec![0u8; 8192];
        let mut push = |line: String| {
            if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) = tx.try_send(line) {
                d.fetch_add(1, Ordering::Relaxed);
            }
        };
        loop {
            match input.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => lines.feed(&buf[..n], &mut push),
            }
        }
        if let Some(l) = lines.rest() {
            push(l);
        }
    });
    let _ = std::thread::Builder::new().name("debrief-stderr-send".into()).spawn(move || {
        let mut sent = 0u64;
        while let Ok(first) = rx.recv() {
            let mut lines = vec![first];
            while lines.len() < MAX_LOG_BATCH {
                match rx.try_recv() {
                    Ok(l) => lines.push(l),
                    Err(_) => break,
                }
            }
            // Past the cap the lines are only counted.
            let room = MAX_LOG_LINES.saturating_sub(sent) as usize;
            if lines.len() > room {
                dropped.fetch_add((lines.len() - room) as u64, Ordering::Relaxed);
                lines.truncate(room);
            }
            sent += lines.len() as u64;
            let req = HostRequest::Log {
                lines,
                dropped: dropped.swap(0, Ordering::Relaxed),
            };
            if peer.request::<_, ()>(&req, SEND_TIMEOUT).is_err() && peer.is_closed() {
                return;
            }
            std::thread::sleep(BATCH_EVERY);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_split_cut_and_decoded() {
        let mut got = Vec::new();
        let mut l = Lines::new(5);
        l.feed(b"ab\r\ncdefghij\nk", &mut |s| got.push(s));
        // Invalid UTF-8 is replaced, and the replacement still respects the cut (at a character boundary).
        l.feed(b"\xc3\xa9\xff\n", &mut |s| got.push(s));
        assert_eq!(got, ["ab", "cdefg", "k\u{e9}"]);
        l.feed(b"tail", &mut |s| got.push(s));
        assert_eq!(l.rest().as_deref(), Some("tail"));
        assert_eq!(l.rest(), None);
    }
}
