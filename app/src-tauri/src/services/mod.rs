//! Tauri-free services behind the commands (ADR 0016): the commands are thin adapters that check their
//! arguments, call a service and turn its result into the reply. A service knows no window, event or
//! `AppHandle`, so it is tested without a running app and reused by other commands without calling them.

pub mod profile;
pub mod project;
