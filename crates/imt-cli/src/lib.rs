//! The command line (`docs/cli.md`): headless import, queries, analyses and the incident report over the same
//! engine and store worker the app uses. [`args`] parses (pure), [`output`] renders (pure, bounded), [`run`]
//! drives a store through the ports, and [`host`] says how a store is opened (the worker; in process in tests).
//!
//! The executable's name is the one constant [`BIN_NAME`].

pub mod args;
pub mod host;
pub mod output;
pub mod run;
pub mod time_arg;
pub mod watch;

/// The executable's name in usage text and messages (one place to change when the product is renamed).
pub const BIN_NAME: &str = "imt";

/// What the process exits with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// Done.
    Ok = 0,
    /// The work failed (a file, the store, a query).
    Failure = 1,
    /// The command line was wrong.
    Usage = 2,
    /// Done, but a `--fail-on-findings` check found something (for CI).
    Findings = 3,
}

impl Exit {
    pub fn code(self) -> i32 {
        self as i32
    }
}
