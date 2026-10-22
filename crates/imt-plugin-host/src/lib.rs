//! The host side of IMT's out-of-process plugins (ADR 0017): plugin processes look like ordinary
//! [`imt_core::Plugin`]s, so the registry's guards, quarantine, settings and the UI work unchanged.
//!
//! - [`process`]: one plugin's supervised process (and its elevated process), the calls to it and the host
//!   services it calls back (host files, an annotator's view, tool task reports);
//! - [`remote`]: [`RemotePlugin`], registering proxies of what the plugin's manifest declares;
//! - [`profile`]: profile importers in a plugin process (the profile worker);
//! - [`document`]: document importers (dumps, UMDH logs: ADR 0023), reading the file by ranges;
//! - [`sandbox`]: each process' sandbox from its manifest's permissions;
//! - [`builtins`]: the built-in plugins served by the `imt-plugins` binary;
//! - [`installed`]: third-party plugins installed from `.imtplugin` packages (ADR 0027).
//!
//! This is a library: the app links it, and the store worker links it to run importer plugins next to the store
//! (ADR 0014 6c). It links no plugin.

pub mod builtins;
pub mod document;
pub mod installed;
pub mod process;
pub mod profile;
pub mod remote;
pub mod sandbox;

pub use builtins::Builtins;
pub use process::{PluginLimits, PluginProcess, PluginSpec};
pub use remote::{deadline_for, RemotePlugin};
