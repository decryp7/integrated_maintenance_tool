//! The Debrief plugin SDK (ADR 0017, ADR 0027): the contract between Debrief and its plugins. A plugin depends on this
//! crate alone, and its `main` is one line:
//!
//! ```no_run
//! use debrief_plugin_api::prelude::*;
//!
//! struct MyPlugin;
//!
//! impl Plugin for MyPlugin {
//!     fn id(&self) -> &str { "com.example.my-plugin" }
//!     fn name(&self) -> &str { "My plugin" }
//!     fn version(&self) -> &str { env!("CARGO_PKG_VERSION") }
//!     fn register(&self, c: &mut Contributions) -> anyhow::Result<()> {
//!         // c.add_importer(MyImporter); ...
//!         Ok(())
//!     }
//! }
//!
//! fn main() -> std::io::Result<()> {
//!     debrief_plugin_api::serve(std::sync::Arc::new(MyPlugin))
//! }
//! ```
//!
//! **Versions.** The crate follows semver; the plugin protocol it speaks is [`PROTOCOL`] (sent in the handshake and
//! the manifest). Debrief runs a plugin built for the same protocol major version and a minor version it knows, and
//! otherwise refuses it with a message saying whether the plugin or Debrief needs an update. Start with
//! `cargo debrief-plugin new`, test with `debrief-plugin-test`, package with `cargo debrief-plugin package`; the walkthrough
//! is `docs/plugin-sdk.md`.
//!
//! Modules:
//!
//! - [`plugin`]: the [`Plugin`] trait and the [`Contributions`] it registers (importers, configuration
//!   parsers, marker kinds, annotators, transforms, tools, settings, profile importers);
//! - the values plugins build: [`model`] (records), [`time`] (timestamps), [`text`] (decoding), [`config`]
//!   (configuration trees), [`settings`], [`tools`], [`markers`], [`annotate`], [`condition`], [`user_regex`];
//! - [`manifest`]: what a plugin is and contributes, sent to Debrief on start;
//! - [`protocol`]: the requests Debrief sends and the host services a plugin calls back, carried by `debrief-ipc`'s
//!   frames;
//! - [`output`]: output files a tool writes without a path (streamed to the host from the elevated process);
//! - [`serve()`]: a plugin process' `main` (`debrief_plugin_api::serve(Arc::new(MyPlugin))`);
//! - [`sandbox`]: the sandbox Debrief runs a plugin in, from its permissions;
//! - [`prelude`]: the names most plugins need.
//!
//! The engine (`debrief-core`) re-exports the shared values, so there is one definition of each.

#![deny(missing_docs)]

pub mod annotate;
pub mod clef;
pub mod condition;
pub mod config;
mod forward;
pub mod host_file;
pub mod importer;
pub mod manifest;
pub mod markers;
/// Records: what importers and transforms produce.
pub mod model;
pub mod output;
pub mod plugin;
pub mod problems;
pub mod process;
pub mod protocol;
pub mod sandbox;
pub mod serve;
pub mod settings;
pub mod text;
pub mod time;
pub mod tools;
pub mod units;
pub mod user_regex;

pub use host_file::RangeRead;
pub use importer::{BinaryImporter, ImportCtx, Imported, TextImporter};
pub use manifest::{Manifest, Permission, PermissionRequest};
pub use model::{Fields, Level, Record};
pub use plugin::{Contributions, DocumentImporter, Plugin, ProfileImporter, Transform, TransformCtx, Verdict};
pub use protocol::{ProtocolVersion, PROTOCOL};
pub use serve::serve;

/// The names most plugins need: `use debrief_plugin_api::prelude::*;`.
pub mod prelude {
    pub use crate::importer::{BinaryImporter, ImportCtx, Imported, TextImporter};
    pub use crate::manifest::{Permission, PermissionRequest};
    pub use crate::model::{Fields, Level, Record};
    pub use crate::plugin::{Contributions, Plugin, Transform, TransformCtx, Verdict};
    pub use crate::serve::serve;
    pub use crate::tools::{Tool, ToolCtx, ToolInfo};
}
