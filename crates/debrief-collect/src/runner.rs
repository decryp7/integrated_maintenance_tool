//! Running external tools (`wevtutil`, `reg`, `typeperf`, ...) behind a trait,
//! so the collection logic can be tested with a fake.

use std::io::Read;
use std::process::Stdio;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

/// Captured output of a finished process.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandOutput {
    /// `None` when the process was ended by a signal.
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum CommandError {
    #[error("{0} was not found on this machine")]
    NotFound(String),
    #[error("{0} did not finish within {1} s and was stopped")]
    TimedOut(String, u64),
    #[error("could not run {0}: {1}")]
    Io(String, String),
}

/// Contract: runs `program` with `args` (no shell), waits at most `timeout`,
/// and returns its output or a typed error. Never panics.
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[String], timeout: Duration) -> Result<CommandOutput, CommandError>;
}

/// Largest stdout kept per command.
const MAX_STDOUT: u64 = 256 << 20;

/// Runs real processes with `std::process::Command`.
pub struct SystemRunner;

/// How long the output is still read after the tool exited: a child it left running (a service it started) may hold
/// the pipe open for ever, and the answer must not wait for it.
const AFTER_EXIT: Duration = Duration::from_secs(2);

/// A pipe read on its own thread into a shared buffer, so what was read can be taken without waiting for the end.
struct Drain {
    buf: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

impl Drain {
    fn start<R: Read + Send + 'static>(mut r: R, cap: u64) -> Drain {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (tx, done) = mpsc::channel();
        let b = buf.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 64 * 1024];
            // Keep reading past the cap (discarding) so the child never blocks on a full pipe.
            while let Ok(n) = r.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let mut g = b.lock().unwrap_or_else(|p| p.into_inner());
                let room = (cap as usize).saturating_sub(g.len()).min(n);
                g.extend_from_slice(&chunk[..room]);
            }
            let _ = tx.send(());
        });
        Drain { buf, done }
    }

    /// What was read, once the pipe ended or `until` passed (whichever is first).
    fn take(self, until: Instant) -> Vec<u8> {
        let _ = self.done.recv_timeout(until.saturating_duration_since(Instant::now()));
        std::mem::take(&mut *self.buf.lock().unwrap_or_else(|p| p.into_inner()))
    }
}

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[String], timeout: Duration) -> Result<CommandOutput, CommandError> {
        let mut child = debrief_core::process::quiet_command(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => CommandError::NotFound(program.into()),
                _ => CommandError::Io(program.into(), e.to_string()),
            })?;
        let out = child.stdout.take().map(|s| Drain::start(s, MAX_STDOUT));
        let err = child.stderr.take().map(|s| Drain::start(s, 1 << 20));
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if start.elapsed() >= timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(CommandError::TimedOut(program.into(), timeout.as_secs()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(CommandError::Io(program.into(), e.to_string())),
            }
        };
        let until = Instant::now() + AFTER_EXIT;
        let stdout = out.map(|d| d.take(until)).unwrap_or_default();
        let stderr = err.map(|d| d.take(until)).unwrap_or_default();
        Ok(CommandOutput {
            exit_code: status.code(),
            stdout,
            stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
        })
    }
}

// The tests drive `sh`, so the module exists on Unix only.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A tool that exits but left a child holding its output pipe (a service it started) is still answered in time,
    /// with what it wrote.
    #[test]
    fn a_grandchild_holding_the_pipe_does_not_hold_the_answer() {
        let start = Instant::now();
        let out = SystemRunner
            .run("sh", &["-c".into(), "sleep 30 & echo $!".into()], Duration::from_secs(60))
            .unwrap();
        let elapsed = start.elapsed();
        let text = String::from_utf8_lossy(&out.stdout);
        let pid: i32 = text.trim().parse().unwrap_or(0);
        if pid > 0 {
            // Ends the test's own `sleep` grandchild.
            #[allow(clippy::disallowed_methods)]
            let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
        }
        assert!(pid > 0, "what it wrote is kept: {text:?}");
        assert!(elapsed < Duration::from_secs(10), "answered after {elapsed:?}");
    }
}
