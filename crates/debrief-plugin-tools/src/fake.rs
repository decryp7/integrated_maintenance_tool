//! A scripted [`debrief_collect::CommandRunner`] for tests.

use debrief_collect::{CommandError, CommandOutput, CommandRunner};
use std::sync::Mutex;
use std::time::Duration;

type Handler = Box<dyn Fn(&str, &[String]) -> Result<CommandOutput, CommandError> + Send + Sync>;

/// Records every call (`"program arg arg"`) and its timeout, and answers with a closure.
pub struct FakeRunner {
    calls: Mutex<Vec<String>>,
    timeouts: Mutex<Vec<Duration>>,
    handler: Handler,
}

impl FakeRunner {
    pub fn new(handler: impl Fn(&str, &[String]) -> Result<CommandOutput, CommandError> + Send + Sync + 'static) -> Self {
        FakeRunner {
            calls: Mutex::new(Vec::new()),
            timeouts: Mutex::new(Vec::new()),
            handler: Box::new(handler),
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    pub fn timeouts(&self) -> Vec<Duration> {
        self.timeouts.lock().unwrap().clone()
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[String], timeout: Duration) -> Result<CommandOutput, CommandError> {
        self.calls.lock().unwrap().push(format!("{program} {}", args.join(" ")));
        self.timeouts.lock().unwrap().push(timeout);
        (self.handler)(program, args)
    }
}

pub fn ok(stdout: &str) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        exit_code: Some(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: String::new(),
    })
}

pub fn fail(code: i32, stdout: &str, stderr: &str) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        exit_code: Some(code),
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.into(),
    })
}
