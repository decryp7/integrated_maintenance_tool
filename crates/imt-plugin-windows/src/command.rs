//! Running external tools (`relog`, `logman`) behind a small trait, so callers can be tested with a fake.

use std::io::{self, Read};
use std::process::{Command, Stdio};
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

fn drain<R: Read + Send + 'static>(mut pipe: R) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    })
}

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[String], timeout: Duration) -> io::Result<CommandOutput> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let out = child.stdout.take().map(drain);
        let err = child.stderr.take().map(drain);
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
        let join = |h: Option<thread::JoinHandle<String>>| h.and_then(|h| h.join().ok()).unwrap_or_default();
        Ok(CommandOutput {
            code: status.code(),
            stdout: join(out),
            stderr: join(err),
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
        assert!(SystemRunner.run("/nonexistent/imt-tool", &[], Duration::from_secs(1)).is_err());
    }
}
