//! Running external tools (`wevtutil`, `reg`, `typeperf`, ...) behind a trait,
//! so the collection logic can be tested with a fake.

use std::io::Read;
use std::process::Stdio;
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

fn drain<R: Read + Send + 'static>(r: R, cap: u64) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut limited = r.take(cap);
        let _ = limited.read_to_end(&mut buf);
        // Keep draining so the child never blocks on a full pipe.
        let _ = std::io::copy(&mut limited.into_inner(), &mut std::io::sink());
        buf
    })
}

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[String], timeout: Duration) -> Result<CommandOutput, CommandError> {
        let mut child = imt_core::process::quiet_command(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => CommandError::NotFound(program.into()),
                _ => CommandError::Io(program.into(), e.to_string()),
            })?;
        let out = child.stdout.take().map(|s| drain(s, MAX_STDOUT));
        let err = child.stderr.take().map(|s| drain(s, 1 << 20));
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
        let stdout = out.and_then(|h| h.join().ok()).unwrap_or_default();
        let stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
        Ok(CommandOutput {
            exit_code: status.code(),
            stdout,
            stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
        })
    }
}
