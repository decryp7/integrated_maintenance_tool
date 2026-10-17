//! Out-of-process plugins for IMT (ADR 0002, ADR 0004).
//!
//! - [`RemotePlugin`]: the host-side adapter. A plugin process (any executable speaking `imt.plugin.v1`,
//!   typically built with `imt-plugin-sdk`) appears as an ordinary [`imt_core::Plugin`]: same manifest,
//!   guard, quarantine, settings, status and health as an in-process one.
//! - [`isolation`]: which plugins run out of process ("Run isolated" per plugin, automatically after a
//!   quarantine, or all of them in isolated mode) and the permissions the user granted.
//! - [`builtins`]: the built-in plugins and the `imt-plugin-host` binary that serves any of them, so a
//!   built-in can run isolated without a binary of its own.

pub mod builtins;
pub mod connection;
pub mod isolation;
pub mod remote;

pub use connection::{PluginConnection, PluginSpec};
pub use remote::{deadline_for, RemotePlugin};
