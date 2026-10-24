//! Running external tools (`relog`, `logman`) behind a small trait, so callers can be tested with a fake.

use std::io::{self, Read};
use std::process::Stdio;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Default time allowed for one external command.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CommandOutput {
    /// Exit code; `None` when the process was killed by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Runs a program to completion. Implementations must kill it after `timeout` and
/// return an [`io::ErrorKind::TimedOut`] error.
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[String], timeout: Duration) -> io::Result<CommandOutput>;
}

/// Runs real processes with `std::process::Command`.
pub struct SystemRunner;

/// How long the output is still read after the program exited: a child it left running may hold the pipe open for
/// ever, and the answer must not wait for it.
const AFTER_EXIT: Duration = Duration::from_secs(2);

/// A pipe read on its own thread into a shared buffer, so what was read can be taken without waiting for the end.
struct Drain {
    buf: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

impl Drain {
    fn start<R: Read + Send + 'static>(mut pipe: R) -> Drain {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let (tx, done) = mpsc::channel();
        let b = buf.clone();
        thread::spawn(move || {
            let mut chunk = [0u8; 64 * 1024];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                b.lock().unwrap_or_else(|p| p.into_inner()).extend_from_slice(&chunk[..n]);
            }
            let _ = tx.send(());
        });
        Drain { buf, done }
    }

    /// What was read, once the pipe ended or `until` passed (whichever is first).
    fn take(self, until: Instant) -> String {
        let _ = self.done.recv_timeout(until.saturating_duration_since(Instant::now()));
        let buf = std::mem::take(&mut *self.buf.lock().unwrap_or_else(|p| p.into_inner()));
        String::from_utf8_lossy(&buf).into_owned()
    }
}

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[String], timeout: Duration) -> io::Result<CommandOutput> {
        let mut child = debrief_plugin_api::process::quiet_command(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let out = child.stdout.take().map(Drain::start);
        let err = child.stderr.take().map(Drain::start);
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{program} did not finish within {timeout:?}"),
                ));
            }
            thread::sleep(Duration::from_millis(20));
        };
        let until = Instant::now() + AFTER_EXIT;
        Ok(CommandOutput {
            code: status.code(),
            stdout: out.map(|d| d.take(until)).unwrap_or_default(),
            stderr: err.map(|d| d.take(until)).unwrap_or_default(),
        })
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    type Handler = Box<dyn Fn(&str, &[String]) -> io::Result<CommandOutput> + Send + Sync>;

    /// Records every call and answers with a closure.
    pub struct FakeRunner {
        pub calls: Mutex<Vec<String>>,
        handler: Handler,
    }

    impl FakeRunner {
        pub fn new(handler: impl Fn(&str, &[String]) -> io::Result<CommandOutput> + Send + Sync + 'static) -> Self {
            FakeRunner {
                calls: Mutex::new(Vec::new()),
                handler: Box::new(handler),
            }
        }

        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[String], _timeout: Duration) -> io::Result<CommandOutput> {
            self.calls.lock().unwrap().push(format!("{program} {}", args.join(" ")));
            (self.handler)(program, args)
        }
    }

    impl CommandRunner for std::sync::Arc<FakeRunner> {
        fn run(&self, program: &str, args: &[String], timeout: Duration) -> io::Result<CommandOutput> {
            self.as_ref().run(program, args, timeout)
        }
    }

    pub fn ok(stdout: &str) -> io::Result<CommandOutput> {
        Ok(CommandOutput {
            code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
        })
    }

    pub fn fail(code: i32, stdout: &str, stderr: &str) -> io::Result<CommandOutput> {
        Ok(CommandOutput {
            code: Some(code),
            stdout: stdout.into(),
            stderr: stderr.into(),
        })
    }
}

// The only test drives `sh`, so the module exists on Unix only.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn system_runner_captures_output_and_codes() {
        let sh = |script: &str, secs: u64| SystemRunner.run("sh", &["-c".into(), script.into()], Duration::from_secs(secs));
        let out = sh("echo hi; echo oops >&2; exit 3", 10).unwrap();
        assert_eq!((out.code, out.stdout.trim(), out.stderr.trim()), (Some(3), "hi", "oops"));
        assert!(!out.success());
        assert!(sh("true", 10).unwrap().success());
        let err = SystemRunner
            .run("sh", &["-c".into(), "sleep 5".into()], Duration::from_millis(100))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(SystemRunner.run("/nonexistent/debrief-tool", &[], Duration::from_secs(1)).is_err());
    }

    /// A tool that exits but left a child holding its output pipe (a service it started) is still answered in time,
    /// with what it wrote.
    #[test]
    fn a_grandchild_holding_the_pipe_does_not_hold_the_answer() {
        let start = Instant::now();
        let out = SystemRunner
            .run("sh", &["-c".into(), "sleep 30 & echo $!".into()], Duration::from_secs(60))
            .unwrap();
        let elapsed = start.elapsed();
        let text = out.stdout.clone();
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
