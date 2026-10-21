//! The IMT plugin SDK (ADR 0017, ADR 0027): the contract between IMT and its plugins. A plugin depends on this
//! crate alone, and its `main` is one line:
//!
//! ```no_run
//! use imt_plugin_api::prelude::*;
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
//!     imt_plugin_api::serve(std::sync::Arc::new(MyPlugin))
//! }
//! ```
//!
//! **Versions.** The crate follows semver; the plugin protocol it speaks is [`PROTOCOL`] (sent in the handshake and
//! the manifest). IMT runs a plugin built for the same protocol major version and a minor version it knows, and
//! otherwise refuses it with a message saying whether the plugin or IMT needs an update. Start with
//! `cargo imt-plugin new`, test with `imt-plugin-test`, package with `cargo imt-plugin package`; the walkthrough
//! is `docs/plugin-sdk.md`.
//!
//! Modules:
//!
//! - [`plugin`]: the [`Plugin`] trait and the [`Contributions`] it registers (importers, configuration
//!   parsers, marker kinds, annotators, transforms, tools, settings, profile importers);
//! - the values plugins build: [`model`] (records), [`time`] (timestamps), [`text`] (decoding), [`config`]
//!   (configuration trees), [`settings`], [`tools`], [`markers`], [`annotate`], [`condition`], [`user_regex`];
//! - [`manifest`]: what a plugin is and contributes, sent to IMT on start;
//! - [`protocol`]: the requests IMT sends and the host services a plugin calls back, carried by `imt-ipc`'s
//!   frames;
//! - [`serve()`]: a plugin process' `main` (`imt_plugin_api::serve(Arc::new(MyPlugin))`);
//! - [`sandbox`]: the sandbox IMT runs a plugin in, from its permissions;
//! - [`prelude`]: the names most plugins need.
//!
//! The engine (`imt-core`) re-exports the shared values, so there is one definition of each.

#![deny(missing_docs)]

pub mod annotate;
pub mod clef;
pub mod condition;
pub mod config;
pub mod host_file;
pub mod importer;
pub mod manifest;
pub mod markers;
/// Records: what importers and transforms produce.
pub mod model;
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
pub mod user_regex;

pub use importer::{BinaryImporter, ImportCtx, Imported, TextImporter};
pub use manifest::{Manifest, Permission, PermissionRequest};
pub use model::{Fields, Level, Record};
pub use plugin::{Contributions, Plugin, ProfileImporter, Transform, TransformCtx, Verdict};
pub use protocol::{ProtocolVersion, PROTOCOL};
pub use serve::serve;

/// The names most plugins need: `use imt_plugin_api::prelude::*;`.
pub mod prelude {
    pub use crate::importer::{BinaryImporter, ImportCtx, Imported, TextImporter};
    pub use crate::manifest::{Permission, PermissionRequest};
    pub use crate::model::{Fields, Level, Record};
    pub use crate::plugin::{Contributions, Plugin, Transform, TransformCtx, Verdict};
    pub use crate::serve::serve;
    pub use crate::tools::{Tool, ToolCtx, ToolInfo};
}
