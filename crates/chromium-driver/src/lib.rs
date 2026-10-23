//! A small, self-contained Chromium renderer driver. It has no application types, so it can be copied between apps.
//!
//! * [`discover`]: Edge, Chrome or Chromium on Windows, macOS and Linux (pure candidate lists, one thin `find`).
//! * [`launch`]: the command line (the automation banner stays on: no stealth), a throwaway or persistent profile
//!   with an accent theme, and a supervised child that is killed on drop ([`process`]).
//! * [`cdp`] / [`page`]: a minimal synchronous DevTools-protocol client and a page handle (navigate, evaluate,
//!   wait for readiness, show or hide the window, block heavy resources).
//! * [`ready`] and [`challenge`]: pure rules for "the page can be read" and "this page needs the user".
//! * [`cue`]: the text of the title that marks a window as AI-driven. Nothing is ever injected into a page.
//! * [`install`]: verified install of a pinned Chromium archive.

// This crate depends on nothing of Debrief, so it cannot use `debrief_core::process::quiet_command`; the processes it starts
// are the browser (a GUI program) and `taskkill`, which is started without a console window.
#![allow(clippy::disallowed_methods)]

pub mod cdp;
pub mod challenge;
pub mod cue;
pub mod discover;
pub mod install;
pub mod launch;
pub mod page;
pub mod process;
pub mod ready;

pub use challenge::{detect as detect_challenge, Challenge};
pub use discover::{find, find_for_tests, Browser as BrowserExe, Flavor};
pub use launch::{Chromium, LaunchOptions, Profile, Visibility};
pub use page::Page;

/// Everything that can go wrong; messages tell the user what to do.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Chromium-based browser was found; install Edge, Chrome or Chromium, or download one in Settings")]
    NotFound,
    #[error("the browser could not be started: {0}")]
    Launch(String),
    #[error("the browser did not answer in time ({0})")]
    Timeout(&'static str),
    #[error("the browser connection failed: {0}")]
    Connection(String),
    #[error("the browser reported an error: {0}")]
    Protocol(String),
    #[error("the browser was closed")]
    Closed,
    #[error("{0}")]
    Install(String),
}

pub type Result<T> = std::result::Result<T, Error>;
